//! A live tree-sitter document: the tree is the source of truth, edits are
//! applied to it directly, and highlights are queried straight off it for
//! whatever byte range is on screen.
//!
//! This is the opposite arrangement to [`super::prepared`], which identifies a
//! document by a hash of its whole text and materializes per-line tokens into
//! 64-line chunks on a worker thread. That design suits the diff views, where
//! the text is immutable and the same document is scrolled repeatedly. It suits
//! an *editable* buffer badly: every keystroke changes the hash, so the document
//! and all of its chunks are discarded and the viewport falls back to heuristic
//! tokens until the worker catches up.
//!
//! Here the document outlives its edits. `tree.edit()` shifts the existing tree
//! into the new coordinates synchronously, the reparse reuses it, and rendering
//! walks a `QueryCursor` over the visible range. Nothing is materialized per
//! line, so there is no cache to invalidate and no pending state to report.
//!
//! The document holds a [`Rope`] rather than a contiguous buffer: the parser
//! reads it a chunk at a time, query predicates read node text the same way,
//! and edit positions come off the rope's summaries. So neither parsing nor
//! querying ever needs the buffer assembled into one string, and holding a
//! snapshot across a background reparse costs an atomic increment.
//!
//! Used by the merge tool's editable resolved output.

use super::super::{SyntaxHighlightPalette, syntax_highlight_palette};
use super::*;
use crate::kit::rope::Rope;
use std::sync::atomic::{AtomicU64, Ordering};

/// Served in place of the real bytes for masked spans. See [`masked_read`].
static BLANKS: [u8; 64] = [b' '; 64];

/// Distinguishes every document ever built on this process, so a version can be
/// used directly as a `TextInput` highlight-provider binding key: rebinding must
/// be detected across a document swap, not just across an edit.
static NEXT_LIVE_SYNTAX_VERSION: AtomicU64 = AtomicU64::new(1);

fn next_live_syntax_version() -> u64 {
    NEXT_LIVE_SYNTAX_VERSION.fetch_add(1, Ordering::Relaxed)
}

fn clamp_to_len(range: Range<usize>, len: usize) -> Range<usize> {
    let start = range.start.min(len);
    let end = range.end.min(len).max(start);
    start..end
}

/// Feeds the parser blanks for the masked spans and real text everywhere else.
///
/// The spans are the unresolved-conflict placeholder rows — `<Merge Conflict>`
/// and friends — which are a drawing of an open decision, not text the file will
/// ever contain. Handed to a grammar verbatim they are a syntax error, and the
/// error recovery does not stay local: in HTML and TSX `<Merge Conflict>` parses
/// as an *opening element* and swallows everything after it, so already-resolved
/// code far below an open conflict loses its highlighting.
///
/// Blanking them keeps the parse honest without moving a single byte. Offsets
/// coming back out of the tree are offsets into the real text, so nothing
/// downstream has to remap, and ASCII space is ignorable in every grammar we
/// wire up — unlike a comment, which would produce a `@comment` capture we would
/// then have to overpaint, and which in block-comment languages can swallow
/// following lines exactly the way the placeholder does.
///
/// Note this cannot invent the code *inside* an unresolved block. A conflict
/// that straddles a brace leaves the parse genuinely unbalanced and the tail
/// below it genuinely mis-coloured; that is inherent, not a limitation of
/// masking. What it removes is the additional, spurious damage.
fn masked_read<'a>(
    rope: &'a Rope,
    mask: &'a [Range<usize>],
) -> impl FnMut(usize, tree_sitter::Point) -> &'a [u8] {
    let len = rope.len();
    move |offset, _position| {
        if offset >= len {
            return &[];
        }
        // `mask` is sorted and disjoint, so the first span ending past `offset`
        // is the only one that can contain or follow it.
        let ix = mask.partition_point(|span| span.end <= offset);
        match mask.get(ix) {
            Some(span) if span.start <= offset => {
                let masked_end = span.end.min(len);
                &BLANKS[..(masked_end - offset).min(BLANKS.len())]
            }
            // The parser is happy with however many bytes it gets, so handing
            // it one rope chunk at a time means a parse never needs the
            // document as a single buffer.
            Some(span) => rope.bytes_at(offset, span.start.min(len)),
            None => rope.bytes_at(offset, len),
        }
    }
}

/// A tree-sitter `Point` for a byte offset, read off the rope's summaries.
///
/// Replaces the line-starts array the document used to carry: the row and
/// column are a single O(log n) descent, and there is no index to keep in step
/// with the text across edits.
fn rope_ts_point(rope: &Rope, offset: usize) -> tree_sitter::Point {
    let point = rope.offset_to_point(offset);
    tree_sitter::Point::new(point.row as usize, point.column as usize)
}

/// Lets tree-sitter query predicates (`#eq?` and friends) read node text
/// straight from the rope, one chunk at a time.
struct RopeTextProvider<'a>(&'a Rope);

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeTextProvider<'a> {
    type I = RopeChunkBytes<'a>;

    fn text(&mut self, node: tree_sitter::Node<'_>) -> Self::I {
        RopeChunkBytes(self.0.chunks_in_range(node.byte_range()))
    }
}

struct RopeChunkBytes<'a>(crate::kit::rope::Chunks<'a>);

impl<'a> Iterator for RopeChunkBytes<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(str::as_bytes)
    }
}

fn parse_masked_tree(
    spec: &TreesitterHighlightSpec,
    rope: &Rope,
    mask: &[Range<usize>],
    old_tree: Option<&tree_sitter::Tree>,
    budget: Option<Duration>,
) -> Option<tree_sitter::Tree> {
    with_ts_parser_parse_result(&spec.ts_language, |parser| {
        let mut read = masked_read(rope, mask);
        let Some(budget) = budget else {
            return parser.parse_with_options(&mut read, old_tree, None);
        };
        let started = Instant::now();
        let mut progress = |_state: &tree_sitter::ParseState| {
            if started.elapsed() >= budget {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
        parser.parse_with_options(&mut read, old_tree, Some(options))
    })
}

/// Collapse overlapping tree-sitter captures into non-overlapping styled runs.
///
/// Ported from Zed's `BufferChunks::next`. `next_capture` yields captures in the
/// order the query cursor emitted them, which is ascending start order; the
/// stack holds those still open at the cursor and the last pushed wins, so **the
/// later capture wins** — the same rule `normalize_non_overlapping_tokens`
/// applies in the read-only panes, and the rule upstream `highlights.scm` files
/// are written against. A `self` inside a parameter list still reads as a
/// keyword rather than inheriting the enclosing function-signature capture,
/// because it starts later and is therefore emitted later.
///
/// Resolving by *span* instead — innermost wins — is what this used to do, and
/// it silently inverts any query that colours a node by capturing its parent
/// afterwards. TOML's `(bare_key) @type` followed by `(pair (bare_key))
/// @property` is the worst case: every key in the file took the `@type` colour,
/// which in the shipped themes is the same green as `@string`, so an entire
/// `Cargo.toml` rendered in one colour.
///
/// The stack tolerates a longer capture sitting on top of a shorter one — the
/// shorter one is buried, never read, and always ends first, so the pop loop
/// below never surfaces it while it is stale.
///
/// The capture source is a closure rather than a concrete iterator so that
/// depth-1 injections can be added by merging several layers' cursors into one
/// ordered stream, without touching this function.
fn sweep_runs(
    mut next_capture: impl FnMut() -> Option<(Range<usize>, SyntaxTokenKind)>,
    palette: &SyntaxHighlightPalette,
    range: Range<usize>,
    out: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
) {
    let mut stack: Vec<(usize, SyntaxTokenKind)> = Vec::new();
    let mut pending = next_capture();
    let mut offset = range.start;

    while offset < range.end {
        while stack.last().is_some_and(|(end, _)| *end <= offset) {
            stack.pop();
        }

        while let Some((capture_range, kind)) = pending.clone() {
            if offset < capture_range.start {
                break;
            }
            if capture_range.end > offset {
                stack.push((capture_range.end, kind));
            }
            pending = next_capture();
        }

        let mut run_end = range.end;
        if let Some((capture_range, _)) = pending.as_ref() {
            run_end = run_end.min(capture_range.start);
        }
        if let Some((end, _)) = stack.last() {
            run_end = run_end.min(*end);
        }
        if run_end <= offset {
            // No capture can advance the cursor and none is open: bail rather
            // than spin. Reachable only if the source yields an unordered or
            // empty range, which the clamping below already rules out.
            break;
        }

        if let Some(style) = stack.last().and_then(|(_, kind)| palette.style(*kind)) {
            match out.last_mut() {
                // Passes are contiguous, so a run split at a pass boundary
                // arrives here as two halves of one span.
                Some((last_range, last_style))
                    if last_range.end == offset && *last_style == style =>
                {
                    last_range.end = run_end;
                }
                _ => out.push((offset..run_end, style)),
            }
        }
        offset = run_end;
    }
}

/// One capture from one layer's tree, carrying everything the sweep needs to
/// order it: the layer's `depth` and the position the cursor emitted it at.
///
/// `seq` is what preserves the query's own precedence. tree-sitter emits
/// captures ordered by `(node start byte, pattern index)`, so for two patterns
/// matching at the same offset the later pattern arrives later — and the later
/// pattern is the one that must win. See [`sweep_runs`].
struct LayerCapture {
    range: Range<usize>,
    kind: SyntaxTokenKind,
    depth: u8,
    seq: u32,
}

/// Captures from one layer's tree that intersect `pass`, tagged with `depth`.
/// `clip` is the layer's included ranges. With a single range (or none, for the
/// root) a capture cannot escape the region, so the fast path pushes it whole.
/// A *combined* layer is parsed over disjoint ranges and tree-sitter reports
/// document offsets, so a node straddling two of them spans the host bytes in
/// between — see `combined_injection_gaps` in `prepared.rs` for the same problem
/// on the batch path. Those captures are split into their intersections with
/// `clip`, all pieces keeping one `seq` so the query's own precedence survives
/// the stable sort in `highlights_for_byte_range`.
fn collect_layer_captures(
    spec: &TreesitterHighlightSpec,
    tree: &tree_sitter::Tree,
    rope: &Rope,
    pass: Range<usize>,
    text_len: usize,
    depth: u8,
    clip: &[Range<usize>],
    out: &mut Vec<LayerCapture>,
) {
    catch_treesitter_query_panic(|| {
        TS_CURSOR.with(|cursor| {
            let mut cursor = cursor.borrow_mut();
            cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
            cursor.set_byte_range(pass.clone());
            cursor.set_containing_byte_range(0..usize::MAX);
            // `set_byte_range` yields every capture *intersecting* the window,
            // so a string or block comment opened far above it still arrives —
            // no look-behind needed. Query predicates read node text through the
            // rope, so this path never needs a contiguous buffer either.
            let mut captures =
                cursor.captures(&spec.query, tree.root_node(), RopeTextProvider(rope));
            tree_sitter::StreamingIterator::advance(&mut captures);
            let capture_kinds = spec.capture_kinds.as_slice();
            // Counts every capture the cursor yields, including the ones dropped
            // below, so the numbering is the emission order itself rather than
            // the order of what survived.
            let mut seq: u32 = 0;
            while let Some((m, capture_ix)) = captures.get() {
                if let Some(capture) = m.captures.get(*capture_ix)
                    && let Some(kind) = capture_kinds.get(capture.index as usize).copied().flatten()
                {
                    let range = clamp_to_len(capture.node.byte_range(), text_len);
                    if !range.is_empty() {
                        if clip.len() <= 1 {
                            out.push(LayerCapture {
                                range,
                                kind,
                                depth,
                                seq,
                            });
                        } else {
                            // `clip` is sorted and disjoint, so skip straight to
                            // the first range that can intersect and stop at the
                            // first that cannot. Scanning from the front instead
                            // would be O(captures x ranges) on a template with
                            // hundreds of text runs.
                            let first =
                                clip.partition_point(|clip_range| clip_range.end <= range.start);
                            for clip_range in &clip[first..] {
                                if clip_range.start >= range.end {
                                    break;
                                }
                                let start = range.start.max(clip_range.start);
                                let end = range.end.min(clip_range.end);
                                if start < end {
                                    out.push(LayerCapture {
                                        range: start..end,
                                        kind,
                                        depth,
                                        seq,
                                    });
                                }
                            }
                        }
                    }
                }
                seq = seq.saturating_add(1);
                tree_sitter::StreamingIterator::advance(&mut captures);
            }
        });
    });
}

/// One injected sub-grammar region: a `<script>` body inside HTML, SQL inside a
/// Rust string literal.
///
/// The tree is parsed with `included_ranges` set to the injected span, so its
/// node offsets are already *document* coordinates and merging its captures with
/// the root's needs no remapping.
///
/// A `(#set! injection.combined)` pattern produces *one* layer covering every
/// match of it, so `ranges` can hold more than one span. [`Self::hull`] spans
/// them all, which is what the coarse overlap tests want; anything that has to
/// know whether a specific byte belongs to the layer must consult `ranges`.
pub(in crate::view) struct LiveSyntaxLayer {
    spec: &'static TreesitterHighlightSpec,
    tree: tree_sitter::Tree,
    ranges: Vec<Range<usize>>,
}

impl LiveSyntaxLayer {
    /// The span covering every range in the layer.
    ///
    /// Derived rather than stored: as a field it had to be kept in step by hand at
    /// three construction sites, and a hull that stopped covering its members would
    /// show up only as a layer silently skipped by the overlap gate below.
    fn hull(&self) -> Range<usize> {
        let start = self.ranges.first().map(|range| range.start).unwrap_or(0);
        let end = self.ranges.last().map(|range| range.end).unwrap_or(0);
        start..end.max(start)
    }
}

/// Depth-1 only. An injection inside an injection is not pursued: it is rare,
/// it multiplies parse cost on the keystroke path, and the read-only diff panes
/// draw the same line at the same depth (`TS_MAX_INJECTION_DEPTH`).
/// Parse the depth-1 injected grammars found in `tree`.
///
/// `budget` is a ceiling for *all* layers together, not per layer. Handing each
/// one its own copy let a document with N injections spend N × budget on the
/// keystroke path while the root parse it was protecting stayed capped at one —
/// so a markdown file with many fenced blocks blocked the frame in proportion to
/// how many it had.
///
/// Returns the layers plus whether any were dropped because the deadline ran
/// out. A dropped layer leaves its region on the enclosing grammar, so the
/// caller has to mark the document stale and let the background reparse — which
/// runs unbudgeted — put it back. Silently keeping `stale = false` stranded the
/// region until the user happened to type again.
fn parse_injection_layers(
    rope: &Rope,
    spec: &TreesitterHighlightSpec,
    tree: &tree_sitter::Tree,
    mask: &[Range<usize>],
    budget: Option<Duration>,
) -> (Vec<LiveSyntaxLayer>, bool) {
    let Some(query) = spec.injection_query.as_ref() else {
        return (Vec::new(), false);
    };
    let Some(content_ix) = query.capture_index_for_name("injection.content") else {
        return (Vec::new(), false);
    };
    let language_ix = query
        .capture_index_for_name("injection.language")
        .or_else(|| query.capture_index_for_name("language"));

    // The gate that keeps this a no-op for grammars with no combined pattern.
    let has_combined = spec.has_combined_injections;

    // Collect first, parse second: the query cursor is a thread-local, so
    // parsing a layer while still holding it would re-enter the borrow.
    let mut found: Vec<(DiffSyntaxLanguage, Range<usize>)> = Vec::new();
    let mut combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>> =
        FxHashMap::default();
    let mut truncated = false;
    catch_treesitter_query_panic(|| {
        TS_CURSOR.with(|cursor| {
            let mut cursor = cursor.borrow_mut();
            cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
            cursor.set_byte_range(0..rope.len());
            cursor.set_containing_byte_range(0..usize::MAX);
            let mut matches = cursor.matches(query, tree.root_node(), RopeTextProvider(rope));
            tree_sitter::StreamingIterator::advance(&mut matches);
            while let Some(m) = matches.get() {
                if let Some(language) = injection_language_for_match(rope, query, m, language_ix) {
                    let pattern_ix = m.pattern_index;
                    let is_combined = spec.is_combined_injection_pattern(pattern_ix);
                    for capture in m.captures.iter().filter(|c| c.index == content_ix) {
                        if let Some(range) =
                            normalized_injection_content_byte_range(capture.node, rope.len())
                            && !range.is_empty()
                        {
                            if is_combined {
                                combined_ranges
                                    .entry((language, pattern_ix))
                                    .or_default()
                                    .push(range);
                            } else {
                                found.push((language, range));
                            }
                        }
                    }
                }
                tree_sitter::StreamingIterator::advance(&mut matches);
            }
            if has_combined {
                truncated = cursor.did_exceed_match_limit();
            }
        });
    });

    // Sort by the whole entry, language included: `dedup` only removes adjacent
    // duplicates, so keying the sort on the range alone lets a different
    // language at the same span sit between two identical entries and defeat it
    // — leaving two layers for one region, parsed twice and merged twice.
    found.sort_by(|(a_language, a_range), (b_language, b_range)| {
        (a_range.start, a_range.end, *a_language).cmp(&(b_range.start, b_range.end, *b_language))
    });
    found.dedup();

    // One deadline for the whole set, so the cost of injections is bounded by
    // the budget rather than by how many there are.
    let deadline = budget.map(|budget| Instant::now() + budget);
    let mut layers = Vec::with_capacity(found.len());
    let mut dropped = false;
    for (language, range) in found {
        let Some(layer_spec) = tree_sitter_highlight_spec(language) else {
            continue;
        };
        // A layer that fails to parse is dropped: only its own span loses
        // highlighting, the document around it is untouched. `dropped` carries
        // that up so it can be repaired off-thread.
        match parse_included_range(
            layer_spec,
            rope,
            mask,
            std::slice::from_ref(&range),
            deadline,
        ) {
            Some(tree) => layers.push(LiveSyntaxLayer {
                spec: layer_spec,
                tree,
                ranges: vec![range],
            }),
            None => dropped = true,
        }
    }

    // One layer per combined pattern, covering every match of it. A truncated
    // query means tree-sitter silently discarded matches; dropping a range out of
    // a combined set changes the document the injected grammar sees, so the whole
    // group is abandoned to the host grammar rather than parsed half-complete.
    if has_combined && !truncated {
        let groups = combined_injection_groups_in_apply_order(combined_ranges);
        // No TS_COMBINED_INJECTION_MAX_* ceiling here, deliberately, and do not add
        // one. Those are the prepared path's stand-in for a budget and are measured
        // against a 64-row window; this path is not windowed -- `set_byte_range`
        // above is the whole rope -- so here they capped on document size, costing a
        // 600-line `.njk` all of its HTML while the diff pane still highlighted it.
        // `deadline` already bounds this parse, and a layer it drops sets `dropped`
        // so the off-thread reparse restores it.
        for (language, _, ranges) in groups {
            let Some(layer_spec) = tree_sitter_highlight_spec(language) else {
                continue;
            };
            match parse_included_range(layer_spec, rope, mask, &ranges, deadline) {
                Some(tree) => layers.push(LiveSyntaxLayer {
                    spec: layer_spec,
                    tree,
                    ranges,
                }),
                None => dropped = true,
            }
        }
    }
    (layers, dropped)
}

/// Resolve the language for an injection match, reading capture text from the
/// rope. Language names are short, so materializing one is trivially bounded.
fn injection_language_for_match(
    rope: &Rope,
    query: &tree_sitter::Query,
    query_match: &tree_sitter::QueryMatch<'_, '_>,
    language_ix: Option<u32>,
) -> Option<DiffSyntaxLanguage> {
    let capture_text = |capture_ix: u32| -> Option<String> {
        query_match
            .captures
            .iter()
            .find(|capture| capture.index == capture_ix)
            .map(|capture| rope.text_for_range(capture.node.byte_range()))
    };

    query
        .property_settings(query_match.pattern_index)
        .iter()
        .filter(|setting| matches!(setting.key.as_ref(), "injection.language" | "language"))
        .find_map(|setting| {
            setting
                .value
                .as_deref()
                .and_then(injection_language_from_name)
                .or_else(|| {
                    setting
                        .capture_id
                        .and_then(|id| capture_text(id as u32))
                        .as_deref()
                        .and_then(injection_language_from_name)
                })
        })
        .or_else(|| {
            language_ix
                .and_then(capture_text)
                .as_deref()
                .and_then(injection_language_from_name)
        })
}

/// Parse `ranges` with `spec`'s grammar, leaving node offsets in document
/// coordinates.
///
/// `set_included_ranges` is what buys that: the parser still reads the whole
/// (masked) document, but only builds nodes inside the ranges. The ranges are
/// cleared again before returning — the parser is pooled and its included
/// ranges are sticky, so leaving them set would silently truncate the next
/// root parse.
///
/// `ranges` must be sorted, non-overlapping and **non-empty**: an empty slice is
/// tree-sitter's reset to "the whole document", which would silently promote the
/// injected grammar to the entire file.
fn parse_included_range(
    spec: &TreesitterHighlightSpec,
    rope: &Rope,
    mask: &[Range<usize>],
    ranges: &[Range<usize>],
    deadline: Option<Instant>,
) -> Option<tree_sitter::Tree> {
    if ranges.is_empty() {
        return None;
    }
    with_ts_parser_parse_result(&spec.ts_language, |parser| {
        let included = ranges
            .iter()
            .map(|range| tree_sitter::Range {
                start_byte: range.start,
                end_byte: range.end,
                start_point: rope_ts_point(rope, range.start),
                end_point: rope_ts_point(rope, range.end),
            })
            .collect::<Vec<_>>();
        // RAII rather than a clear per exit path: a panic in the progress callback or
        // in `masked_read` would otherwise leave the pooled parser's sticky ranges
        // set, truncating every later root parse on this thread.
        let mut guard = IncludedRangesGuard::set(parser, &included)?;
        let mut read = masked_read(rope, mask);
        match deadline {
            None => guard.parser().parse_with_options(&mut read, None, None),
            Some(deadline) => {
                let mut progress = |_state: &tree_sitter::ParseState| {
                    if Instant::now() >= deadline {
                        std::ops::ControlFlow::Break(())
                    } else {
                        std::ops::ControlFlow::Continue(())
                    }
                };
                let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
                guard
                    .parser()
                    .parse_with_options(&mut read, None, Some(options))
            }
        }
    })
}

/// Whether a live document could be built for this language and size at all.
///
/// The two permanent reasons [`LiveSyntaxDocument::new`] returns `None` — no
/// wired grammar, and text past the parse ceiling — are both cheap to ask
/// directly. Callers check them up front rather than inferring them from a
/// failed build, so a *transient* failure is never mistaken for a permanent one
/// and latched.
pub(in crate::view) fn live_syntax_document_supported(
    language: DiffSyntaxLanguage,
    len: usize,
) -> bool {
    len <= PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES
        && tree_sitter_highlight_spec(language).is_some()
}

/// A live tree-sitter document, owned by the view that edits it.
pub(in crate::view) struct LiveSyntaxDocument {
    language: DiffSyntaxLanguage,
    spec: &'static TreesitterHighlightSpec,
    rope: Rope,
    mask: Arc<[Range<usize>]>,
    tree: tree_sitter::Tree,
    /// Depth-1 injected grammars, rebuilt whenever the root tree is reparsed.
    injections: Vec<LiveSyntaxLayer>,
    stale: bool,
    version: u64,
}

/// What a parse attempt managed to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum LiveSyntaxSyncOutcome {
    /// The tree describes the current text exactly.
    Reparsed,
    /// The budget ran out. The edited tree is live and positionally correct, but
    /// semantically stale near the edit; the caller should reparse off-thread.
    Deferred,
    /// The edit pushed the buffer past
    /// [`PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES`], the same ceiling
    /// [`LiveSyntaxDocument::new`] refuses to build over. The document is left
    /// untouched and describes text that no longer exists; the caller must drop
    /// it and fall back to heuristic tokens.
    Abandoned,
}

impl LiveSyntaxDocument {
    /// `None` when the language has no wired grammar, the text is over
    /// [`PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES`], or the initial parse
    /// exhausts `budget` (there is no tree yet to fall back on). `budget: None`
    /// parses unbounded, for background threads and tests.
    pub(in crate::view) fn new(
        language: DiffSyntaxLanguage,
        rope: Rope,
        mask: Arc<[Range<usize>]>,
        budget: Option<Duration>,
    ) -> Option<Self> {
        if rope.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
            return None;
        }
        let spec = tree_sitter_highlight_spec(language)?;
        let tree = parse_masked_tree(spec, &rope, mask.as_ref(), None, budget)?;
        let (injections, dropped) =
            parse_injection_layers(&rope, spec, &tree, mask.as_ref(), budget);
        Some(Self {
            language,
            spec,
            rope,
            mask,
            tree,
            injections,
            // A first parse that ran out of budget before finishing every layer
            // is not finished. Reporting it as settled leaves those regions on
            // the enclosing grammar for the rest of the session.
            stale: dropped,
            version: next_live_syntax_version(),
        })
    }

    pub(in crate::view) fn language(&self) -> DiffSyntaxLanguage {
        self.language
    }

    /// Fold one coalesced edit into the tree and reparse.
    ///
    /// `rope` must already reflect the edit. `edit` is
    /// `(replaced, inserted)` — the replaced span in the *old* text's
    /// coordinates and the inserted span in the new text's, sharing a start.
    /// `None` means the text was replaced wholesale, which reparses from
    /// scratch: a conflict resolution rewrites structure, and there is no
    /// keystroke latency to protect on that path.
    ///
    /// The version advances either way, so a caller keying a highlight provider
    /// on it always rebinds.
    ///
    /// Returns [`LiveSyntaxSyncOutcome::Abandoned`] when the document cannot be
    /// carried forward and the caller must drop it: the edit takes the buffer
    /// past the size ceiling (nothing is touched, exactly as if it had never
    /// been built at that size), or the text was replaced *wholesale* —
    /// `edit: None` — and the budgeted parse did not finish, leaving no tree
    /// that describes the new text.
    ///
    /// [`LiveSyntaxSyncOutcome::Deferred`] is the softer failure and is reserved
    /// for a **seeded** sync, where `tree.edit()` has already moved the old tree
    /// into the new coordinates so it still paints while the background reparse
    /// catches up.
    pub(in crate::view) fn sync(
        &mut self,
        rope: Rope,
        mask: Arc<[Range<usize>]>,
        edit: Option<(Range<usize>, Range<usize>)>,
        budget: Option<Duration>,
    ) -> LiveSyntaxSyncOutcome {
        // The ceiling bounds the *document*, not just the incremental step, so
        // it has to be rechecked on every edit. Parsing past it here would let
        // a single paste buy an unbounded background reparse for the rest of
        // the session.
        if rope.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
            return LiveSyntaxSyncOutcome::Abandoned;
        }

        let seed = match edit {
            Some((replaced, inserted)) => {
                // Positions come straight off the summaries — an O(log n)
                // descent each, with no line-start array to keep in step.
                let replaced = clamp_to_len(replaced, self.rope.len());
                let inserted = clamp_to_len(inserted, rope.len());
                self.tree.edit(&tree_sitter::InputEdit {
                    start_byte: replaced.start,
                    old_end_byte: replaced.end,
                    new_end_byte: inserted.end,
                    start_position: rope_ts_point(&self.rope, replaced.start),
                    old_end_position: rope_ts_point(&self.rope, replaced.end),
                    new_end_position: rope_ts_point(&rope, inserted.end),
                });
                true
            }
            None => false,
        };

        self.rope = rope;
        self.mask = mask;
        self.version = next_live_syntax_version();

        let old_tree = seed.then_some(&self.tree);
        match parse_masked_tree(self.spec, &self.rope, self.mask.as_ref(), old_tree, budget) {
            Some(tree) => {
                self.tree = tree;
                let (injections, dropped) = parse_injection_layers(
                    &self.rope,
                    self.spec,
                    &self.tree,
                    self.mask.as_ref(),
                    budget,
                );
                self.injections = injections;
                // The root tree is current either way; `dropped` says only that
                // some injected region did not fit in the budget, which the
                // background reparse finishes.
                self.stale = dropped;
                LiveSyntaxSyncOutcome::Reparsed
            }
            None if !seed => {
                // Nothing moved this tree: `edit` was `None`, so it still
                // describes the text that was here *before* the replacement,
                // while `self.rope` is already the new text. Keeping it would
                // pair a document with a tree for a different string, and every
                // query over it answers for that other string — for a buffer
                // replaced from empty, a tree spanning nothing and therefore no
                // highlighting at all.
                //
                // Hand the document back instead. Both callers drop it and fall
                // to heuristic tokens, then finish the parse off-thread with no
                // budget, which is the path a blown budget was always meant to
                // take.
                LiveSyntaxSyncOutcome::Abandoned
            }
            None => {
                // The root tree was edited into the new coordinates but not
                // reparsed, so the injection *ranges* it reported are stale.
                // Drop the layers rather than paint with spans that have moved:
                // the enclosing grammar still highlights the region, which is a
                // smaller error than an inner grammar in the wrong place. The
                // background reparse restores them.
                self.injections.clear();
                // Keep the edited tree. Its node positions already moved with
                // the edit, so it paints correctly everywhere the edit did not
                // change the structure — which is the overwhelming majority of
                // the viewport, and strictly better than blanking it.
                self.stale = true;
                LiveSyntaxSyncOutcome::Deferred
            }
        }
    }

    pub(in crate::view) fn version(&self) -> u64 {
        self.version
    }

    /// Everything an off-thread reparse needs, or `None` if the tree is already
    /// current. All `Send`, so it can cross into a background task.
    pub(in crate::view) fn background_reparse_request(&self) -> Option<LiveSyntaxReparseRequest> {
        self.stale.then(|| LiveSyntaxReparseRequest {
            spec: self.spec,
            // O(1): the rope is persistent, so the background parse reads a
            // snapshot that later edits cannot disturb.
            rope: self.rope.clone(),
            mask: Arc::clone(&self.mask),
            // The edited tree, as a seed: the background parse is incremental
            // too, so it starts from what the keystrokes already shifted.
            old_tree: self.tree.clone(),
            version: self.version,
        })
    }

    /// Install a tree parsed off-thread.
    ///
    /// Returns false when the document moved on while the parse was in flight,
    /// in which case the tree describes text that no longer exists and must be
    /// discarded — the caller should re-issue from the current state.
    pub(in crate::view) fn adopt_background_tree(
        &mut self,
        for_version: u64,
        tree: tree_sitter::Tree,
        injections: Vec<LiveSyntaxLayer>,
    ) -> bool {
        if self.version != for_version {
            return false;
        }
        self.tree = tree;
        // The layers come with the tree, already parsed off-thread. A `Deferred`
        // sync drops them (their ranges moved with the edit) on the promise that
        // the background reparse restores them; without adopting them here the
        // promise is not kept, and every injected region — a `<script>` body, a
        // fenced code block — silently loses its inner grammar until the user
        // happens to type again.
        self.injections = injections;
        self.stale = false;
        self.version = next_live_syntax_version();
        true
    }

    pub(in crate::view) fn snapshot(&self, theme: AppTheme) -> LiveSyntaxSnapshot {
        LiveSyntaxSnapshot(Arc::new(LiveSyntaxSnapshotInner {
            spec: self.spec,
            rope: self.rope.clone(),
            tree: self.tree.clone(),
            injections: self
                .injections
                .iter()
                .map(|layer| LiveSyntaxLayer {
                    spec: layer.spec,
                    tree: layer.tree.clone(),
                    ranges: layer.ranges.clone(),
                })
                .collect(),
            palette: syntax_highlight_palette(theme),
        }))
    }
}

/// A deferred reparse, detached from the document so it can run off-thread.
pub(in crate::view) struct LiveSyntaxReparseRequest {
    spec: &'static TreesitterHighlightSpec,
    rope: Rope,
    mask: Arc<[Range<usize>]>,
    old_tree: tree_sitter::Tree,
    version: u64,
}

/// Run a deferred reparse to completion. Safe to call under `smol::unblock`.
///
/// Returns the version it was parsed for, so the caller can tell whether the
/// document moved on in the meantime.
/// Reparse off-thread, layers included.
///
/// The injected layers are built here rather than at adoption because adoption
/// runs inside a `view.update`, i.e. on the main thread. Parsing every injected
/// region there — unbudgeted, since a dropped layer would be lost until the next
/// edit — would block a frame for exactly the work this job exists to move off
/// it.
pub(in crate::view) fn live_syntax_reparse(
    request: LiveSyntaxReparseRequest,
) -> Option<(u64, tree_sitter::Tree, Vec<LiveSyntaxLayer>)> {
    let tree = parse_masked_tree(
        request.spec,
        &request.rope,
        request.mask.as_ref(),
        Some(&request.old_tree),
        None,
    )?;
    let (injections, _dropped) = parse_injection_layers(
        &request.rope,
        request.spec,
        &tree,
        request.mask.as_ref(),
        None,
    );
    Some((request.version, tree, injections))
}

struct LiveSyntaxSnapshotInner {
    spec: &'static TreesitterHighlightSpec,
    rope: Rope,
    injections: Vec<LiveSyntaxLayer>,
    tree: tree_sitter::Tree,
    palette: SyntaxHighlightPalette,
}

/// An immutable view of a document, cheap to clone into a highlight-provider
/// closure. It never observes an edit — a new one is minted per version — so it
/// is always exactly right for the text it carries, and callers never have to
/// interpolate stale ranges or report a pending state.
#[derive(Clone)]
pub(in crate::view) struct LiveSyntaxSnapshot(Arc<LiveSyntaxSnapshotInner>);

impl LiveSyntaxSnapshot {
    /// Styled runs covering `byte_range`: sorted, non-overlapping, clipped.
    ///
    /// Nothing is cached here. A viewport is a few thousand bytes and a
    /// windowed tree-sitter query over it is microseconds, so re-running it is
    /// cheaper than tracking what would invalidate a cache. `TextInput`'s own
    /// `ProviderHighlightCache` already memoizes repeated identical windows.
    pub(in crate::view) fn highlights_for_byte_range(
        &self,
        byte_range: Range<usize>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        let inner = self.0.as_ref();
        let text_len = inner.rope.len();
        let range = clamp_to_len(byte_range, text_len);
        if range.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        let mut pass_start = range.start;
        while pass_start < range.end {
            let pass_end = pass_start
                .saturating_add(TS_MAX_BYTES_TO_QUERY)
                .min(range.end);
            let pass = pass_start..pass_end;

            // Gather the root's captures and those of every injected layer
            // overlapping this pass, then sweep them as one ordered stream.
            // Collected rather than merged lazily because the query cursor is a
            // thread-local: querying a second layer inside the first's borrow
            // would re-enter it.
            let mut hits: Vec<LayerCapture> = Vec::new();
            collect_layer_captures(
                inner.spec,
                &inner.tree,
                &inner.rope,
                pass.clone(),
                text_len,
                0,
                &[],
                &mut hits,
            );
            for layer in &inner.injections {
                let hull = layer.hull();
                if hull.start < pass.end && hull.end > pass.start {
                    collect_layer_captures(
                        layer.spec,
                        &layer.tree,
                        &inner.rope,
                        pass.clone(),
                        text_len,
                        1,
                        &layer.ranges,
                        &mut hits,
                    );
                }
            }

            // Start ascending, then depth, then emission order — the order
            // `sweep_runs` needs to make the *later* capture win.
            //
            // Depth ahead of `seq` is what keeps an injected grammar's capture
            // above its host's over the same span: the two layers number their
            // captures independently, so `seq` alone cannot rank across them.
            // Everything below that is the query's own precedence, preserved by
            // `seq`. Sorting by span length instead — longest first, so the
            // innermost lands on top — is what used to invert it.
            //
            // Merging by start is enough to keep the stack honest because a
            // cursor emits in start order, so within a layer `seq` is already
            // monotone in `start`.
            hits.sort_by(|left, right| {
                left.range
                    .start
                    .cmp(&right.range.start)
                    .then(left.depth.cmp(&right.depth))
                    .then(left.seq.cmp(&right.seq))
            });

            let mut hits = hits.into_iter();
            let next_capture = || hits.next().map(|hit| (hit.range, hit.kind));
            sweep_runs(next_capture, &inner.palette, pass.clone(), &mut out);

            pass_start = pass_end;
        }
        out
    }

    /// Everywhere the document names the token at `offset`.
    ///
    /// Bounded, unlike [`Self::syntax_pair_at`]: that is O(tree depth), this is
    /// O(document), and the editor recomputes highlights on every caret move.
    /// Past the ceiling the answer is empty rather than slow -- a missing
    /// highlight beats a stuttering keystroke.
    ///
    /// Injected regions are not searched: the root tree has their bodies as one
    /// unparsed leaf, so a name inside one matches nothing there.
    pub(in crate::view) fn occurrences_at(&self, offset: usize) -> Vec<Range<usize>> {
        let inner = self.0.as_ref();
        let len = inner.rope.len();
        if len > OCCURRENCE_MAX_TEXT_BYTES {
            return Vec::new();
        }
        let offset = offset.min(len);
        // Ask the tree first. Deciding "not a name" costs a descent; deciding it
        // after flattening the rope costs the whole document, and most caret
        // moves land on punctuation, whitespace or a literal.
        if name_token_at(&inner.tree, offset, |range| {
            Some(inner.rope.text_for_range(range))
        })
        .is_none()
        {
            return Vec::new();
        }
        let text = inner.rope.text_for_range(0..len);
        syntax_occurrences_in_tree(&inner.tree, &text, offset)
            .map(|found| found.ranges)
            .unwrap_or_default()
    }

    /// The matching open/close pair the caret at `offset` belongs to: the
    /// delimiter it sits on (or immediately after), otherwise the innermost
    /// pair enclosing it.
    ///
    /// Brackets, element tags and quotes all count -- see [`super::pairs`] for
    /// what each covers and how they are read off the tree.
    ///
    /// O(tree depth) -- cheap enough to run on the caret-move path.
    pub(in crate::view) fn syntax_pair_at(&self, offset: usize) -> Option<SyntaxPair> {
        let inner = self.0.as_ref();
        let offset = offset.min(inner.rope.len());
        let source_ranges_equal = |left: Range<usize>, right: Range<usize>| {
            inner.rope.text_for_range(left) == inner.rope.text_for_range(right)
        };
        // Injected grammars first: a brace inside an interpolated region is the
        // inner grammar's. Layer trees are parsed with `included_ranges`, so
        // their node offsets are already document coordinates.
        for layer in &inner.injections {
            // Membership, not the hull: a caret sitting in a `{% ... %}` gap between
            // two ranges of a combined layer is host-grammar territory, and the
            // combined tree has no nodes there to answer with.
            if layer
                .ranges
                .binary_search_by(|range| {
                    if offset < range.start {
                        std::cmp::Ordering::Greater
                    } else if offset > range.end {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .is_ok()
                && let Some(pair) = syntax_pair_in_tree(&layer.tree, offset, &source_ranges_equal)
            {
                return Some(pair);
            }
        }
        syntax_pair_in_tree(&inner.tree, offset, &source_ranges_equal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_starts_for(text: &str) -> Arc<[usize]> {
        let mut starts = vec![0usize];
        for (ix, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(ix + 1);
            }
        }
        if starts.last() == Some(&text.len()) && !text.is_empty() {
            starts.pop();
        }
        starts.into()
    }

    fn document(text: &str, mask: Vec<Range<usize>>) -> LiveSyntaxDocument {
        document_in(DiffSyntaxLanguage::Rust, text, mask)
    }

    fn document_in(
        language: DiffSyntaxLanguage,
        text: &str,
        mask: Vec<Range<usize>>,
    ) -> LiveSyntaxDocument {
        LiveSyntaxDocument::new(language, Rope::from_str(text), mask.into(), None)
            .unwrap_or_else(|| panic!("{language:?} live document should build"))
    }

    fn styles_at(
        highlights: &[(Range<usize>, gpui::HighlightStyle)],
        offset: usize,
    ) -> Option<gpui::HighlightStyle> {
        highlights
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .map(|(_, style)| *style)
    }

    #[test]
    fn runs_are_sorted_disjoint_and_clipped() {
        let text = "fn main() {\n    let value = 1;\n}\n";
        let doc = document(text, Vec::new());
        let snapshot = doc.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());

        assert!(!highlights.is_empty(), "rust source should highlight");
        let mut previous_end = 0usize;
        for (range, _) in &highlights {
            assert!(
                range.start >= previous_end,
                "runs must not overlap: {highlights:?}"
            );
            assert!(range.start < range.end, "runs must be non-empty");
            assert!(range.end <= text.len(), "runs must stay inside the text");
            previous_end = range.end;
        }
    }

    #[test]
    fn window_query_matches_the_same_span_of_a_full_query() {
        let text = "fn main() {\n    let value = 1;\n    let other = 2;\n}\n";
        let doc = document(text, Vec::new());
        let snapshot = doc.snapshot(AppTheme::gitcomet_dark());

        let window = 12..47;
        let full = snapshot.highlights_for_byte_range(0..text.len());
        let windowed = snapshot.highlights_for_byte_range(window.clone());

        for offset in window.clone() {
            assert_eq!(
                styles_at(&windowed, offset),
                styles_at(&full, offset),
                "byte {offset} should style identically whether queried whole or windowed"
            );
        }
    }

    #[test]
    fn masked_placeholder_does_not_poison_following_lines() {
        // Same document twice: once with the placeholder row masked, once with
        // the row already replaced by spaces. Masking should make these agree.
        let with_placeholder = "fn a() {}\n<Merge Conflict>\nfn b() -> u32 { 7 }\n";
        let with_spaces = "fn a() {}\n                \nfn b() -> u32 { 7 }\n";
        assert_eq!(with_placeholder.len(), with_spaces.len());

        let placeholder_span = 10..26;
        assert_eq!(
            &with_placeholder[placeholder_span.clone()],
            "<Merge Conflict>"
        );

        let masked = document(with_placeholder, vec![placeholder_span]);
        let masked = masked.snapshot(AppTheme::gitcomet_dark());
        let spaced = document(with_spaces, Vec::new());
        let spaced = spaced.snapshot(AppTheme::gitcomet_dark());

        let tail = 27..with_placeholder.len();
        for offset in tail {
            assert_eq!(
                styles_at(
                    &masked.highlights_for_byte_range(0..with_placeholder.len()),
                    offset
                ),
                styles_at(
                    &spaced.highlights_for_byte_range(0..with_spaces.len()),
                    offset
                ),
                "byte {offset} after a masked placeholder should match the spaces-only parse"
            );
        }
    }

    #[test]
    fn unmasked_placeholder_is_what_masking_protects_against() {
        // Guards the premise: without the mask the tail really does change.
        let text = "fn a() {}\n<Merge Conflict>\nfn b() -> u32 { 7 }\n";
        let masked =
            document(text, Vec::from([10..26usize; 1])).snapshot(AppTheme::gitcomet_dark());
        let unmasked = document(text, Vec::new()).snapshot(AppTheme::gitcomet_dark());

        let full = 0..text.len();
        assert_ne!(
            masked.highlights_for_byte_range(full.clone()),
            unmasked.highlights_for_byte_range(full),
            "masking should change the parse; if not, the fixture stopped exercising it"
        );
    }

    #[test]
    fn edit_keeps_the_document_exact() {
        let before = "fn main() {\n    let value = 1;\n}\n";
        let mut doc = document(before, Vec::new());
        let first_version = doc.version();

        // Insert "let extra = 2;\n    " at the start of the body line.
        let inserted_text = "let extra = 2;\n    ";
        let after = "fn main() {\n    let extra = 2;\n    let value = 1;\n}\n";
        let at = 16usize;
        assert_eq!(&after[at..at + inserted_text.len()], inserted_text);

        let outcome = doc.sync(
            Rope::from_str(after),
            Vec::new().into(),
            Some((at..at, at..at + inserted_text.len())),
            None,
        );
        assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);
        assert!(
            doc.background_reparse_request().is_none(),
            "a document that finished its reparse has nothing to defer"
        );
        assert_ne!(
            doc.version(),
            first_version,
            "version must advance per edit"
        );

        let incremental = doc.snapshot(AppTheme::gitcomet_dark());
        let scratch = document(after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
        assert_eq!(
            incremental.highlights_for_byte_range(0..after.len()),
            scratch.highlights_for_byte_range(0..after.len()),
            "an incrementally reparsed tree must match a cold parse of the same text"
        );
    }

    #[test]
    fn an_edit_past_the_size_ceiling_abandons_the_document() {
        // `new` refuses to build over the ceiling, so an edit that crosses it
        // has to refuse too — otherwise one paste buys a full parse now and an
        // unbounded background reparse for the rest of the session.
        let before = "fn main() {}\n";
        let mut doc = document(before, Vec::new());
        let version_before = doc.version();

        let at = before.len();
        let padding =
            "// ".to_string() + &"x".repeat(PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES) + "\n";
        let after = format!("{before}{padding}");
        assert!(after.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES);
        assert!(
            LiveSyntaxDocument::new(
                DiffSyntaxLanguage::Rust,
                Rope::from_str(after.as_str()),
                Vec::new().into(),
                None,
            )
            .is_none(),
            "the fixture must be past the ceiling for this test to mean anything"
        );

        let outcome = doc.sync(
            Rope::from_str(after.as_str()),
            Vec::new().into(),
            Some((at..at, at..at + padding.len())),
            None,
        );

        assert_eq!(outcome, LiveSyntaxSyncOutcome::Abandoned);
        assert_eq!(
            doc.version(),
            version_before,
            "an abandoned sync must leave the document untouched"
        );
        assert!(
            doc.background_reparse_request().is_none(),
            "an abandoned document must not owe an unbounded background parse"
        );
    }

    #[test]
    fn an_edit_that_outruns_the_budget_defers_and_the_background_pass_catches_up() {
        let before = "fn main() {\n    let value = 1;\n}\n".repeat(400);
        let mut doc = document(&before, Vec::new());

        let at = 11usize; // just inside the first body
        let inserted = "\n    let extra = 2;";
        let mut after = before.clone();
        after.insert_str(at, inserted);

        let outcome = doc.sync(
            Rope::from_str(after.as_str()),
            Vec::new().into(),
            Some((at..at, at..at + inserted.len())),
            Some(Duration::ZERO),
        );
        assert_eq!(
            outcome,
            LiveSyntaxSyncOutcome::Deferred,
            "a zero budget cannot finish a reparse"
        );

        // Deferred still renders: the edited tree moved with the edit, so the
        // provider has something positionally correct to paint right now.
        let deferred = doc.snapshot(AppTheme::gitcomet_dark());
        assert!(
            !deferred.highlights_for_byte_range(0..200).is_empty(),
            "a deferred document must keep painting rather than blank the viewport"
        );

        let request = doc
            .background_reparse_request()
            .expect("a deferred document owes a background reparse");
        let (version, tree, injections) =
            live_syntax_reparse(request).expect("unbudgeted reparse succeeds");
        assert!(
            doc.adopt_background_tree(version, tree, injections),
            "the version has not moved, so the tree should be adopted"
        );
        assert!(doc.background_reparse_request().is_none());

        let caught_up = doc.snapshot(AppTheme::gitcomet_dark());
        let scratch = document(&after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
        assert_eq!(
            caught_up.highlights_for_byte_range(0..400),
            scratch.highlights_for_byte_range(0..400),
            "after the background pass the tree must match a cold parse"
        );
    }

    /// A wholesale replacement that outruns its budget must not keep the tree.
    ///
    /// `Deferred` is sound only for a *seeded* sync: there `tree.edit()` has
    /// moved the old tree into the new coordinates, so it still paints. With
    /// `edit: None` nothing moved it, so keeping it pairs the new rope with a
    /// tree describing text that is gone — and every query over it answers for
    /// the wrong document. The file editor reached exactly that on a file
    /// switch: the buffer is blanked, a document is built over the empty text,
    /// then the file lands as a wholesale replacement whose budgeted parse
    /// fails, leaving a full rope with a 0-byte tree and no highlighting at all.
    #[test]
    fn a_wholesale_replacement_that_outruns_the_budget_is_abandoned() {
        // The empty buffer the editor blanks to before a file lands.
        let mut doc = document("", Vec::new());
        assert!(
            doc.snapshot(AppTheme::gitcomet_dark())
                .0
                .tree
                .root_node()
                .end_byte()
                == 0,
            "the fixture must start with a tree that spans nothing"
        );

        let landed = "fn main() {\n    let value = 1;\n}\n".repeat(400);
        let outcome = doc.sync(
            Rope::from_str(landed.as_str()),
            Vec::new().into(),
            // `None` -- the text was replaced, not edited.
            None,
            Some(Duration::ZERO),
        );

        assert_eq!(
            outcome,
            LiveSyntaxSyncOutcome::Abandoned,
            "an unseeded sync that cannot parse must hand the document back, so \
             the caller falls back to heuristics and rebuilds off-thread"
        );
    }

    /// The same shape as the test above, but *seeded*: this one must stay
    /// `Deferred`, because the tree really did move with the edit.
    #[test]
    fn a_seeded_edit_that_outruns_the_budget_still_defers() {
        let before = "fn main() {\n    let value = 1;\n}\n".repeat(400);
        let mut doc = document(&before, Vec::new());
        let inserted = "\n    let extra = 2;";
        let at = 11usize;
        let mut after = before.clone();
        after.insert_str(at, inserted);

        assert_eq!(
            doc.sync(
                Rope::from_str(after.as_str()),
                Vec::new().into(),
                Some((at..at, at..at + inserted.len())),
                Some(Duration::ZERO),
            ),
            LiveSyntaxSyncOutcome::Deferred
        );
    }

    #[test]
    fn a_background_tree_for_a_superseded_version_is_rejected() {
        let before = "fn main() {}\n";
        let mut doc = document(before, Vec::new());
        let stale_version = doc.version();
        let tree = doc.snapshot(AppTheme::gitcomet_dark()).0.tree.clone();

        let after = "fn main() { let x = 1; }\n";
        doc.sync(Rope::from_str(after), Vec::new().into(), None, None);

        assert!(
            !doc.adopt_background_tree(stale_version, tree, Vec::new()),
            "a tree parsed for text that has since changed must be discarded"
        );
    }

    #[test]
    fn wholesale_replacement_reparses_from_scratch() {
        let mut doc = document("fn main() {}\n", Vec::new());
        let after = "struct Point { x: u32, y: u32 }\n";

        let outcome = doc.sync(Rope::from_str(after), Vec::new().into(), None, None);
        assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);

        let replaced = doc.snapshot(AppTheme::gitcomet_dark());
        let scratch = document(after, Vec::new()).snapshot(AppTheme::gitcomet_dark());
        assert_eq!(
            replaced.highlights_for_byte_range(0..after.len()),
            scratch.highlights_for_byte_range(0..after.len()),
        );
    }

    #[test]
    fn masked_read_serves_blanks_then_real_text() {
        let rope = Rope::from_str("abcdefghij");
        let mask = [2..5usize; 1];
        let mut read = masked_read(&rope, &mask);

        assert_eq!(read(0, tree_sitter::Point::new(0, 0)), b"ab");
        assert_eq!(read(2, tree_sitter::Point::new(0, 2)), b"   ");
        assert_eq!(read(5, tree_sitter::Point::new(0, 5)), b"fghij");
        assert_eq!(read(10, tree_sitter::Point::new(0, 10)), b"");
    }

    #[test]
    fn masked_read_spans_longer_than_the_blank_buffer_are_served_in_pieces() {
        let rope = Rope::from_str(&"x".repeat(200));
        let mask = [0..200usize; 1];
        let mut read = masked_read(&rope, &mask);

        let mut served = 0usize;
        while served < 200 {
            let chunk = read(served, tree_sitter::Point::new(0, served));
            assert!(!chunk.is_empty(), "reader must always make progress");
            assert!(chunk.iter().all(|byte| *byte == b' '));
            served += chunk.len();
        }
        assert_eq!(served, 200);
    }

    #[test]
    fn oversized_text_has_no_live_document() {
        let huge: Arc<str> =
            Arc::from("fn a() {}\n".repeat(PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES / 5));
        assert!(
            LiveSyntaxDocument::new(
                DiffSyntaxLanguage::Rust,
                Rope::from_str(&huge),
                Vec::new().into(),
                None,
            )
            .is_none(),
            "text past the ceiling should not get a live document"
        );
    }

    #[test]
    fn language_without_a_grammar_has_no_live_document() {
        let text: Arc<str> = Arc::from("x = 1\n");
        assert!(
            LiveSyntaxDocument::new(
                DiffSyntaxLanguage::VisualBasic,
                Rope::from_str(&text),
                Vec::new().into(),
                None,
            )
            .is_none(),
            "a language with no wired grammar should fall back rather than build"
        );
    }

    /// The editable buffers — the merge tool's resolved output and the file
    /// editor — and the read-only diff panes must colour the same code the same
    /// way, and they do not share an engine: this one sweeps a `QueryCursor`
    /// over the viewport ([`sweep_runs`]), the diff panes materialize per-line
    /// tokens and resolve overlaps with `normalize_non_overlapping_tokens`. Hold
    /// the tree constant and check the two derivations agree byte for byte.
    ///
    /// `probes` are `(needle, expected kind)` pairs asserted against the live
    /// side first, so a fixture that stopped being tree-sitter-highlighted
    /// cannot make the comparison pass by leaving both sides empty. They are
    /// also where each language's *precedence* is pinned: the divergence this
    /// guards against is a query that colours a node by capturing its parent
    /// afterwards, which only shows up as one kind rather than another.
    ///
    /// Fixtures must avoid constructs that trigger `*_injections.scm`:
    /// [`super::prepared`] is driven here with the root tree alone, while the
    /// live snapshot merges its injected layers, so an injected region is a
    /// known divergence rather than a regression.
    fn assert_engines_agree(
        language: DiffSyntaxLanguage,
        text: &str,
        probes: &[(&str, SyntaxTokenKind)],
    ) {
        // A fixture inside one rope chunk never exercises the chunked feed this
        // engine parses through (`masked_read` hands the parser one chunk at a
        // time, `prepared` hands it a contiguous slice), so it compares the two
        // engines on the one input where they cannot differ. Every fixture here
        // used to be under 512 bytes, which is why several rounds of "the
        // engines agree" said nothing about files the app actually opens.
        assert!(
            text.len() > crate::kit::rope::MAX_CHUNK_BYTES,
            "an equivalence fixture must span more than one rope chunk \
             ({} bytes); this one is {} bytes",
            crate::kit::rope::MAX_CHUNK_BYTES,
            text.len(),
        );
        let theme = AppTheme::gitcomet_dark();
        let palette = syntax_highlight_palette(theme);
        let snapshot = document_in(language, text, Vec::new()).snapshot(theme);

        let mut live_by_byte = vec![None; text.len()];
        for (range, style) in snapshot.highlights_for_byte_range(0..text.len()) {
            for byte in range {
                live_by_byte[byte] = Some(style);
            }
        }

        // Same tree, so any disagreement below is in how the captures are turned
        // into styles -- which is exactly what differs between the two engines.
        let line_starts = line_starts_for(text);
        let spec = tree_sitter_highlight_spec(language)
            .unwrap_or_else(|| panic!("{language:?} should have a wired grammar"));
        let per_line = collect_treesitter_document_line_tokens_for_line_window(
            &snapshot.0.tree,
            spec,
            text.as_bytes(),
            line_starts.as_ref(),
            0,
            line_starts.len(),
            treesitter_text_hash(text),
        );
        let mut prepared_by_byte = vec![None; text.len()];
        for (line_ix, tokens) in per_line.iter().enumerate() {
            let line_start = line_starts[line_ix];
            for token in tokens {
                let Some(style) = palette.style(token.kind) else {
                    continue;
                };
                let span = (line_start + token.range.start)..(line_start + token.range.end);
                prepared_by_byte[span].fill(Some(style));
            }
        }

        for (needle, kind) in probes {
            let at = text.find(needle).expect("fixture should contain the probe");
            assert_eq!(
                live_by_byte[at],
                palette.style(*kind),
                "{language:?}: {needle:?} at {at} should carry {kind:?}; without these \
                 classes the comparison below cannot tell tree-sitter from the \
                 heuristic tokenizer"
            );
        }

        // Newlines are excluded: `prepared` clips every token to
        // `line_content_end_byte`, so a capture spanning a line break stops at
        // the `\n`, while a swept run carries straight through it. Invisible in
        // rendering -- a newline has no glyph -- and not worth reshaping either
        // engine over.
        let mismatched = (0..text.len())
            .filter(|byte| text.as_bytes()[*byte] != b'\n')
            .filter(|byte| live_by_byte[*byte] != prepared_by_byte[*byte])
            .map(|byte| {
                let line_ix = line_starts.partition_point(|start| *start <= byte) - 1;
                format!(
                    "byte {byte} (line {line_ix}, {:?}): live={:?} prepared={:?}",
                    text.as_bytes()[byte] as char,
                    live_by_byte[byte].and_then(|style| style.color),
                    prepared_by_byte[byte].and_then(|style| style.color),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            mismatched.is_empty(),
            "{language:?}: the editable buffers and the diff panes must colour \
             identical text identically; diverging bytes:\n  {}",
            mismatched.join("\n  ")
        );
    }

    #[test]
    fn the_live_engine_agrees_with_the_prepared_engine_the_diff_panes_use() {
        let text = concat!(
            "use std::fmt;\n",
            "\n",
            "/// A stage in the pipeline.\n",
            "pub struct Stage<'a> {\n",
            "    pub name: &'a str,\n",
            "    pub retries: usize,\n",
            "}\n",
            "\n",
            "impl Stage<'_> {\n",
            "    pub fn bump(&mut self) -> usize {\n",
            "        self.retries = self.retries.wrapping_add(1);\n",
            "        self.retries\n",
            "    }\n",
            "}\n",
        );

        // Repeated so the fixture spans several rope chunks: the chunked parser
        // feed is only exercised past 512 bytes.
        let text = text.repeat(6);
        let text = text.as_str();
        assert_engines_agree(
            DiffSyntaxLanguage::Rust,
            text,
            &[
                ("Stage<'a>", SyntaxTokenKind::Type),
                ("usize", SyntaxTokenKind::TypeBuiltin),
                ("retries: usize", SyntaxTokenKind::Property),
                ("wrapping_add", SyntaxTokenKind::FunctionMethod),
            ],
        );
    }

    /// The regression that made an entire `Cargo.toml` render in one colour.
    ///
    /// `tree-sitter-toml-ng` colours keys by capturing the enclosing node —
    /// `(bare_key) @type` first, then `(pair (bare_key)) @property` — so a
    /// resolver that prefers the innermost capture gives every key `@type`,
    /// which is the same green as `@string` in the shipped themes. The `@property`
    /// probe below is what pins the precedence.
    #[test]
    fn the_two_engines_agree_on_toml_keys() {
        let text = concat!(
            "[package]\n",
            "name = \"gitcomet\"\n",
            "version = \"0.1.16\"\n",
            "edition = \"2024\"\n",
            "\n",
            "# A comment.\n",
            "[dependencies]\n",
            "serde = { version = \"1\", features = [\"derive\"] }\n",
            "retries = 3\n",
            "strict = true\n",
        );

        let text = text.repeat(6);
        let text = text.as_str();
        assert_engines_agree(
            DiffSyntaxLanguage::Toml,
            text,
            &[
                ("name", SyntaxTokenKind::Property),
                ("\"gitcomet\"", SyntaxTokenKind::String),
                ("# A comment.", SyntaxTokenKind::Comment),
                ("3", SyntaxTokenKind::Number),
                ("true", SyntaxTokenKind::Boolean),
            ],
        );
    }

    /// Python's `highlights.scm` uses the same capture-the-parent idiom for
    /// f-string interpolations and docstrings.
    #[test]
    fn the_two_engines_agree_on_python() {
        let text = concat!(
            "import os\n",
            "\n",
            "\n",
            "class Stage:\n",
            "    \"\"\"A stage in the pipeline.\"\"\"\n",
            "\n",
            "    def __init__(self, name):\n",
            "        self.name = name\n",
            "        self.retries = 0\n",
            "\n",
            "    def bump(self):\n",
            "        self.retries += 1\n",
            "        return f\"{self.name}: {self.retries}\"\n",
        );

        let text = text.repeat(6);
        let text = text.as_str();
        assert_engines_agree(
            DiffSyntaxLanguage::Python,
            text,
            &[
                ("import", SyntaxTokenKind::Keyword),
                ("Stage", SyntaxTokenKind::Type),
                ("__init__", SyntaxTokenKind::FunctionSpecial),
                ("0", SyntaxTokenKind::Number),
            ],
        );
    }

    /// The shape of the file this was reported on: a shebang, a quoted heredoc,
    /// a `case` block and `${var:-}` expansions, over several rope chunks.
    ///
    /// Heredocs are the construct most likely to tell the two engines apart —
    /// tree-sitter-bash matches the delimiter in an external scanner, which is
    /// exactly the sort of thing that can read differently through a chunked
    /// feed than through one contiguous slice.
    #[test]
    fn the_two_engines_agree_on_shell_with_heredocs() {
        let text = concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail\n",
            "\n",
            "usage() {\n",
            "  cat <<'USAGE'\n",
            "Usage: scripts/update.sh --dir PATH --version VERSION [--verify]\n",
            "USAGE\n",
            "}\n",
            "\n",
            "dir=\"\"\n",
            "version=\"\"\n",
            "verify=\"false\"\n",
            "\n",
            "while [[ $# -gt 0 ]]; do\n",
            "  case \"$1\" in\n",
            "    --dir) dir=\"${2:-}\"; shift 2 ;;\n",
            "    --version) version=\"${2:-}\"; shift 2 ;;\n",
            "    --verify) verify=\"true\"; shift ;;\n",
            "    *) echo \"unknown option: $1\" >&2; usage; exit 1 ;;\n",
            "  esac\n",
            "done\n",
            "\n",
            "if [[ -z \"$dir\" ]]; then\n",
            "  echo \"--dir is required\" >&2\n",
            "  exit 1\n",
            "fi\n",
        )
        .repeat(3);

        assert_engines_agree(
            DiffSyntaxLanguage::Bash,
            text.as_str(),
            &[
                ("while", SyntaxTokenKind::KeywordControl),
                ("esac", SyntaxTokenKind::KeywordControl),
                ("USAGE\n", SyntaxTokenKind::String),
            ],
        );
    }

    /// `(open, close)` as `&str` slices, so failures read as source text rather
    /// than as byte offsets.
    fn syntax_pair_text<'a>(
        document: &LiveSyntaxDocument,
        text: &'a str,
        offset: usize,
    ) -> Option<(&'a str, &'a str)> {
        document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(offset)
            .map(|pair| (&text[pair.open], &text[pair.close]))
    }

    #[test]
    fn syntax_pair_matches_a_delimiter_the_caret_sits_on() {
        let text = "fn main() {\n    let value = compute(1, 2);\n}\n";
        let document = document(text, Vec::new());

        let open_paren = text.find("(1").expect("call paren");
        assert_eq!(
            syntax_pair_text(&document, text, open_paren),
            Some(("(", ")")),
            "the caret on an opening paren must find its own closer"
        );

        // Caret immediately *after* the closer: an editor caret touches the
        // character to its left too.
        let close_paren = text.find(");").expect("call close");
        assert_eq!(
            syntax_pair_text(&document, text, close_paren + 1),
            Some(("(", ")"))
        );
    }

    #[test]
    fn syntax_pair_matches_the_innermost_block_around_the_caret() {
        let text = "fn main() {\n    let value = compute(1, 2);\n}\n";
        let document = document(text, Vec::new());

        let inside_call = text.find("1, 2").expect("call args") + 2;
        assert_eq!(
            syntax_pair_text(&document, text, inside_call),
            Some(("(", ")")),
            "inside the argument list the call parens win over the block braces"
        );

        let inside_block = text.find("let").expect("statement");
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(inside_block)
            .expect("the body braces enclose the statement");
        assert_eq!(
            (&text[pair.open.clone()], &text[pair.close.clone()]),
            ("{", "}")
        );
        assert_eq!(pair.kind, SyntaxPairKind::Bracket);
        assert_eq!(pair.open.start, text.find('{').expect("body open"));
    }

    #[test]
    fn syntax_pair_ignores_braces_inside_strings_and_comments() {
        let text = "fn main() {\n    let s = \"a } b\";\n    // ) not a paren\n}\n";
        let document = document(text, Vec::new());

        // Sitting on the brace inside the string literal: it is a byte of the
        // string node, not a delimiter, so it pairs with nothing and the answer
        // comes from the quotes enclosing it instead.
        let in_string = text.find("} b").expect("brace in string");
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(in_string)
            .expect("the string's own quotes enclose the brace");
        assert_eq!(pair.kind, SyntaxPairKind::Quote);
        assert_eq!(
            (&text[pair.open.clone()], &text[pair.close.clone()]),
            ("\"", "\"")
        );
        assert_eq!(pair.open.start, text.find('"').expect("string open"));

        let in_comment = text.find(") not").expect("paren in comment");
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(in_comment)
            .expect("the function body still encloses the comment");
        assert_eq!(pair.kind, SyntaxPairKind::Bracket);
        assert_eq!(pair.open.start, text.find('{').expect("body open"));
    }

    #[test]
    fn syntax_pair_distinguishes_sibling_pairs_of_the_same_kind() {
        let text = "fn main() {\n    f((1), (2));\n}\n";
        let document = document(text, Vec::new());

        let first_open = text.find("(1").expect("first inner");
        let first = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(first_open)
            .expect("first inner pair");
        let second_open = text.find("(2").expect("second inner");
        let second = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(second_open)
            .expect("second inner pair");

        assert_eq!(first.open.start, first_open);
        assert_eq!(second.open.start, second_open);
        assert_ne!(
            first.close, second.close,
            "two sibling pairs must not share a closer"
        );
    }

    #[test]
    fn syntax_pair_is_correct_after_an_incremental_edit() {
        // The case the feature exists for: typing into the middle of the file
        // must not leave the pair pointing at pre-edit offsets.
        let before = "fn main() {\n    let value = 1;\n}\n";
        let mut document = document(before, Vec::new());

        let insert_at = before.find("let").expect("statement");
        let inserted = "if x { }\n    ";
        let after = format!(
            "{}{}{}",
            &before[..insert_at],
            inserted,
            &before[insert_at..]
        );
        let outcome = document.sync(
            Rope::from_str(&after),
            Arc::default(),
            Some((insert_at..insert_at, insert_at..insert_at + inserted.len())),
            None,
        );
        assert_eq!(outcome, LiveSyntaxSyncOutcome::Reparsed);

        let new_block_open = after.find("{ }").expect("new block");
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(new_block_open)
            .expect("the freshly typed block must pair");
        assert_eq!(pair.open.start, new_block_open);
        assert_eq!(&after[pair.close.clone()], "}");

        // And the statement that moved down still resolves to the outer body.
        let moved_statement = after.rfind("let").expect("moved statement");
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(moved_statement)
            .expect("the body still encloses the moved statement");
        assert_eq!(pair.open.start, after.find('{').expect("body open"));
    }

    /// `(open, close)` in `language`, as `&str` slices.
    fn pair_text_in(
        language: DiffSyntaxLanguage,
        text: &str,
        offset: usize,
    ) -> Option<(&str, &str)> {
        let document = document_in(language, text, Vec::new());
        syntax_pair_text(&document, text, offset)
    }

    fn pair_kind_in(
        language: DiffSyntaxLanguage,
        text: &str,
        offset: usize,
    ) -> Option<SyntaxPairKind> {
        document_in(language, text, Vec::new())
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(offset)
            .map(|pair| pair.kind)
    }

    /// A tag pair covers both tags in full, attributes included -- not just the
    /// angle brackets and not just the element name.
    #[test]
    fn syntax_pair_matches_whole_html_tags() {
        let text = "<div class=\"card\">\n  <span>hi</span>\n</div>\n";

        // Caret on the element name of the outer start tag.
        let on_outer_name = text.find("div").expect("outer name");
        assert_eq!(
            pair_text_in(DiffSyntaxLanguage::Html, text, on_outer_name),
            Some(("<div class=\"card\">", "</div>")),
            "the whole start tag pairs with the whole end tag"
        );
        assert_eq!(
            pair_kind_in(DiffSyntaxLanguage::Html, text, on_outer_name),
            Some(SyntaxPairKind::Tag)
        );

        // Caret in the inner element's text content takes the inner pair.
        let in_inner_text = text.find("hi").expect("inner text");
        assert_eq!(
            pair_text_in(DiffSyntaxLanguage::Html, text, in_inner_text),
            Some(("<span>", "</span>")),
            "the innermost element wins over the one enclosing it"
        );

        // Caret in the outer element's content, outside the inner element.
        let between = text.find("\n  <span").expect("gap before inner");
        assert_eq!(
            pair_text_in(DiffSyntaxLanguage::Html, text, between),
            Some(("<div class=\"card\">", "</div>"))
        );
    }

    /// The three tag naming schemes in the wired grammars all resolve, with no
    /// per-language dispatch: `start_tag`/`end_tag`, `STag`/`ETag`, and JSX's
    /// `jsx_opening_element`/`jsx_closing_element`.
    #[test]
    fn syntax_pair_matches_every_tag_naming_scheme() {
        let xml = "<root><item>x</item></root>\n";
        assert_eq!(
            pair_text_in(DiffSyntaxLanguage::Xml, xml, xml.find("item").expect("tag")),
            Some(("<item>", "</item>")),
            "xml names them STag and ETag"
        );

        let tsx = "const a = <Foo bar={1}>text</Foo>;\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Tsx,
                tsx,
                tsx.find("text").expect("child")
            ),
            Some(("<Foo bar={1}>", "</Foo>")),
            "jsx names them jsx_opening_element and jsx_closing_element"
        );

        let tsx_fragment = "const a = <>text</>;\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Tsx,
                tsx_fragment,
                tsx_fragment.find("text").expect("fragment child")
            ),
            Some(("<>", "</>")),
            "nameless JSX fragments should remain pairable"
        );

        let vue = "<template><p>hi</p></template>\n";
        assert_eq!(
            pair_text_in(DiffSyntaxLanguage::Vue, vue, vue.find("hi").expect("text")),
            Some(("<p>", "</p>"))
        );

        let svelte = "<main><b>hi</b></main>\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Svelte,
                svelte,
                svelte.find("hi").expect("text")
            ),
            Some(("<b>", "</b>"))
        );
    }

    /// A tag with no partner matches nothing rather than pairing with the wrong
    /// element. Both cases are decisions, not gaps.
    #[test]
    fn syntax_pair_leaves_invalid_or_self_closing_tags_unpaired() {
        let self_closing = "<br/>\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Html,
                self_closing,
                self_closing.find("br").expect("name")
            ),
            None,
            "a self-closing tag has no partner"
        );

        let unclosed = "<div>\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Html,
                unclosed,
                unclosed.find("div").expect("name")
            ),
            None,
            "an unclosed tag must not reach for some other element's end tag"
        );

        let jsx_self_closing = "const a = <Foo />;\n";
        assert_eq!(
            pair_text_in(
                DiffSyntaxLanguage::Tsx,
                jsx_self_closing,
                jsx_self_closing.find("Foo").expect("name")
            ),
            None
        );

        let jsx_mismatched = "const a = <Foo>text</Bar>;\n";
        for needle in ["Foo", "text", "Bar"] {
            assert_eq!(
                pair_text_in(
                    DiffSyntaxLanguage::Tsx,
                    jsx_mismatched,
                    jsx_mismatched.find(needle).expect("mismatched JSX part")
                ),
                None,
                "mismatched JSX tags must not pair when clicking {needle:?}"
            );
        }
    }

    /// Quotes flanking a string's content pair with each other, in the two
    /// shapes grammars use for them.
    #[test]
    fn syntax_pair_matches_quote_delimiters() {
        // Anonymous same-kind tokens: JSON, Rust, JS.
        let json = "{\"alpha\": \"beta\"}\n";
        let in_value = json.find("beta").expect("value");
        assert_eq!(
            pair_kind_in(DiffSyntaxLanguage::Json, json, in_value),
            Some(SyntaxPairKind::Quote)
        );
        let document = document_in(DiffSyntaxLanguage::Json, json, Vec::new());
        let pair = document
            .snapshot(AppTheme::gitcomet_dark())
            .syntax_pair_at(in_value)
            .expect("the value's own quotes");
        assert_eq!(
            pair.open.start,
            json.find("\"beta").expect("value open quote"),
            "the value pairs with its own quotes, not the key's"
        );

        let rust = "let s = \"abc\";\n";
        assert_eq!(
            pair_kind_in(
                DiffSyntaxLanguage::Rust,
                rust,
                rust.find("abc").expect("body")
            ),
            Some(SyntaxPairKind::Quote)
        );

        let js = "const s = 'abc';\n";
        assert_eq!(
            pair_kind_in(
                DiffSyntaxLanguage::JavaScript,
                js,
                js.find("abc").expect("body")
            ),
            Some(SyntaxPairKind::Quote)
        );

        // Explicit named marker nodes: Python's string_start / string_end.
        let python = "s = \"abc\"\n";
        assert_eq!(
            pair_kind_in(
                DiffSyntaxLanguage::Python,
                python,
                python.find("abc").expect("body")
            ),
            Some(SyntaxPairKind::Quote)
        );
    }

    /// Two documented gaps, asserted so they read as decisions.
    #[test]
    fn syntax_pair_leaves_raw_strings_and_lifetimes_unpaired() {
        // A lone `'` has no partner among its parent's children.
        let lifetime = "struct S<'a> { r: &'a str }\n";
        let on_tick = lifetime.find("'a>").expect("lifetime");
        assert_ne!(
            pair_kind_in(DiffSyntaxLanguage::Rust, lifetime, on_tick),
            Some(SyntaxPairKind::Quote),
            "a lifetime tick is not half of a quote pair"
        );

        // Raw string delimiters are hidden external tokens: they are not in the
        // tree at all, so there is nothing to pair.
        let raw = "let s = r#\"abc\"#;\n";
        assert_ne!(
            pair_kind_in(
                DiffSyntaxLanguage::Rust,
                raw,
                raw.find("abc").expect("body")
            ),
            Some(SyntaxPairKind::Quote)
        );
    }

    /// When several kinds enclose the caret, the innermost one answers.
    #[test]
    fn syntax_pair_takes_the_innermost_enclosing_kind() {
        let text = "<div class=\"card\">x</div>\n";
        let in_attribute = text.find("card").expect("attribute value");
        assert_eq!(
            pair_kind_in(DiffSyntaxLanguage::Html, text, in_attribute),
            Some(SyntaxPairKind::Quote),
            "inside the attribute value the quotes are nearer than the tags"
        );

        let rust = "fn f() { let s = \"a\"; }\n";
        assert_eq!(
            pair_kind_in(
                DiffSyntaxLanguage::Rust,
                rust,
                rust.find('a').expect("body")
            ),
            Some(SyntaxPairKind::Quote),
            "the string's quotes are nearer than the enclosing block braces"
        );
    }

    #[test]
    fn syntax_pair_is_none_outside_any_pair() {
        let text = "fn main() {\n    let value = 1;\n}\n";
        let document = document(text, Vec::new());
        assert_eq!(
            document
                .snapshot(AppTheme::gitcomet_dark())
                .syntax_pair_at(0),
            None,
            "the caret before `fn` is inside nothing"
        );
    }
}

/// Injected sub-grammars: a `<script>` body must be highlighted as JavaScript,
/// not left as opaque HTML raw text.
///
/// This is what the editable resolved output was missing relative to the
/// read-only diff panes above it, which have had depth-1 injections all along.
#[cfg(test)]
mod injection_tests {
    use super::*;

    fn html_document(text: &str) -> LiveSyntaxDocument {
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Html,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("html live document should build")
    }

    /// Styles covering `needle`, if any.
    fn styles_for<'a>(
        highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
        text: &str,
        needle: &str,
    ) -> Vec<&'a gpui::HighlightStyle> {
        let at = text
            .find(needle)
            .expect("fixture should contain the needle");
        let span = at..at + needle.len();
        highlights
            .iter()
            .filter(|(range, _)| range.start < span.end && range.end > span.start)
            .map(|(_, style)| style)
            .collect()
    }

    fn fsharp_document(text: &str) -> LiveSyntaxDocument {
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::FSharp,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("fsharp live document should build")
    }

    /// F# XML doc comments are the tree's only `(#set! injection.combined)` rule.
    /// `xml_doc` is a per-line token, so without combined support a three-line
    /// doc comment produced three layers instead of one.
    #[test]
    fn combined_injection_produces_one_layer_covering_every_range() {
        let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
        let document = fsharp_document(text);

        assert_eq!(
            document.injections.len(),
            1,
            "three xml_doc lines belong to one combined layer, got {} layers",
            document.injections.len()
        );
        let layer = &document.injections[0];
        assert_eq!(
            layer.ranges.len(),
            3,
            "the combined layer should carry one range per xml_doc line: {:?}",
            layer.ranges
        );
        assert_eq!(
            layer.hull(),
            layer.ranges[0].start..layer.ranges[2].end,
            "`range` is the hull of `ranges`"
        );
        assert!(
            layer.hull().end <= text.find("let add").expect("let binding"),
            "the layer must not reach the F# code below it: {:?}",
            layer.hull()
        );
    }

    /// A node straddling two included ranges reports a byte range covering the
    /// host bytes between them, so its captures have to be split. Here the XML
    /// element spans all three `///` lines, and the `/// ` prefixes are F#.
    #[test]
    fn combined_layer_captures_are_clipped_to_its_ranges() {
        let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
        let document = fsharp_document(text);
        let layer = &document.injections[0];
        let ranges = layer.ranges.clone();

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());

        let gap_start = ranges[0].end;
        let gap_end = ranges[1].start;
        assert!(
            gap_start < gap_end,
            "fixture should have a real gap between the first two ranges"
        );
        for (range, _) in &highlights {
            assert!(
                range.start >= gap_end || range.end <= gap_start,
                "highlight {range:?} crosses into the host-grammar gap \
                 {gap_start}..{gap_end} between two combined ranges"
            );
        }
    }

    /// A combined layer may answer for a caret on its own boundary, but neither
    /// end of the pair it returns may straddle a host-grammar gap.
    ///
    /// The gaps here are one byte wide -- the newline between two `///` lines --
    /// so a caret "in the gap" is always also on some range's edge, which the
    /// membership check treats as inside the layer on purpose: a caret directly
    /// after an injected region's last character still belongs to it. What must
    /// never happen is a delimiter range that covers bytes the layer does not
    /// own, because those bytes are host text the layer never parsed.
    #[test]
    fn syntax_pair_at_never_straddles_a_combined_layer_gap() {
        let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
        let document = fsharp_document(text);
        let ranges = document.injections[0].ranges.clone();
        let gap = ranges[0].end..ranges[1].start;
        assert!(gap.start < gap.end, "fixture should have a real gap");
        let snapshot = document.snapshot(AppTheme::gitcomet_dark());

        let pair = snapshot
            .syntax_pair_at(gap.start)
            .expect("the caret sits just past `<summary>`, which pairs with `</summary>`");
        for span in [&pair.open, &pair.close] {
            assert!(
                ranges
                    .iter()
                    .any(|range| range.start <= span.start && span.end <= range.end),
                "delimiter {span:?} is not contained in any injected range {ranges:?}"
            );
        }

        // And a caret out in the host grammar is never answered from the layer.
        let host = text.find("let add").expect("host code");
        if let Some(pair) = snapshot.syntax_pair_at(host) {
            let in_layer = |offset: usize| {
                ranges
                    .iter()
                    .any(|range| range.start <= offset && offset <= range.end)
            };
            assert!(
                !(in_layer(pair.open.start) && in_layer(pair.close.start)),
                "a caret in host code was answered by the combined XML layer"
            );
        }
    }

    /// A caret strictly inside a wide gap belongs to the host grammar, and the
    /// combined tree -- which has no nodes there -- must not answer for it.
    ///
    /// The `///` fixture's gaps are one byte wide, so every offset in them is
    /// also on some range's boundary, which membership counts as inside the
    /// layer on purpose (a caret directly after an injected region's last
    /// character still belongs to it). Interposing a plain `//` comment makes a
    /// ten-byte gap with a real interior, which is the case this guards.
    #[test]
    fn syntax_pair_at_ignores_a_wide_combined_layer_gap() {
        let text = "/// <summary>\n// plain\n/// </summary>\nlet add x y = x + y\n";
        let document = fsharp_document(text);
        let ranges = document.injections[0].ranges.clone();
        let gap = ranges[0].end..ranges[1].start;
        assert!(
            gap.end - gap.start > 2,
            "fixture must have a gap with a strict interior, got {gap:?}"
        );
        let snapshot = document.snapshot(AppTheme::gitcomet_dark());

        let inside = gap.start + (gap.end - gap.start) / 2;
        assert!(inside > gap.start && inside < gap.end);
        if let Some(pair) = snapshot.syntax_pair_at(inside) {
            let in_layer = |offset: usize| {
                ranges
                    .iter()
                    .any(|range| range.start <= offset && offset <= range.end)
            };
            assert!(
                !(in_layer(pair.open.start) && in_layer(pair.close.start)),
                "a caret at {inside}, strictly inside the host-grammar gap {gap:?}, \
                 was answered by the combined XML layer: {:?}/{:?}",
                pair.open,
                pair.close
            );
        }
    }

    /// The clip path must be inert for ordinary single-range layers, which is
    /// every injection in the tree except F#'s.
    #[test]
    fn single_range_layers_are_unaffected_by_the_clip_parameter() {
        let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
        let document = html_document(text);
        let layer = &document.injections[0];
        assert_eq!(
            layer.ranges.len(),
            1,
            "an ordinary injection is one range, so collect_layer_captures takes \
             the unsplit fast path"
        );
        assert_eq!(layer.ranges[0], layer.hull());

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());
        assert!(
            !styles_for(&highlights, text, "const").is_empty(),
            "clipping must not have eaten the injected JavaScript keyword"
        );
    }

    #[test]
    fn script_bodies_are_highlighted_as_javascript() {
        let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
        let document = html_document(text);
        assert_eq!(
            document.injections.len(),
            1,
            "the script body should produce exactly one injected layer"
        );

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());

        // `const` is a JavaScript keyword. The HTML grammar sees the whole
        // script body as one `raw_text` node and has no keyword concept, so a
        // keyword-coloured run here can only come from the injected layer.
        let keyword = styles_for(&highlights, text, "const");
        assert!(
            !keyword.is_empty(),
            "expected the injected JavaScript layer to highlight `const`: {highlights:?}"
        );

        // And the enclosing HTML is still highlighted by the root layer.
        assert!(
            !styles_for(&highlights, text, "script").is_empty(),
            "the host grammar must keep highlighting its own tags"
        );
    }

    #[test]
    fn injected_layers_cover_only_their_own_span() {
        let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
        let document = html_document(text);
        let layer = document
            .injections
            .first()
            .expect("expected one injected layer");

        let body_start = text.find("\nconst").expect("script body") + 1;
        assert!(
            layer.hull().start <= body_start && layer.hull().end >= body_start + "const".len(),
            "layer {:?} should cover the script body at {body_start}",
            layer.hull()
        );
        assert!(
            layer.hull().start > text.find("<script>").expect("open tag"),
            "the layer must not swallow the opening tag"
        );
        assert!(
            layer.hull().end <= text.find("</script>").expect("close tag"),
            "the layer must not swallow the closing tag"
        );
    }

    /// The layer's tree is parsed with `included_ranges`, so its node offsets
    /// are already document coordinates. If that ever regressed to
    /// injection-local offsets, highlights would land near the top of the file.
    #[test]
    fn injected_capture_offsets_are_document_coordinates() {
        let prefix = "<html>\n<body>\n<p>filler</p>\n".repeat(20);
        let text = format!("{prefix}<script>\nconst answer = 42;\n</script>\n</html>\n");
        let document = html_document(&text);
        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());

        let keyword_at = text.find("const").expect("keyword");
        assert!(
            keyword_at > 200,
            "fixture should place the injection well into the document"
        );
        let covering = styles_for(&highlights, &text, "const");
        assert!(
            !covering.is_empty(),
            "the injected keyword should be highlighted at its real offset {keyword_at}"
        );
    }

    fn jinja_document(text: &str) -> LiveSyntaxDocument {
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Jinja,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("jinja live document should build")
    }

    fn dense_jinja_table(rows: usize, cells: usize) -> String {
        let mut lines = vec!["{% block body %}".to_string()];
        for row in 0..rows {
            let mut line = String::from("<tr>");
            for cell in 0..cells {
                line.push_str(&format!("<td>{{{{ r{row}.c{cell} }}}}</td>"));
            }
            line.push_str("</tr>");
            lines.push(line);
        }
        lines.push("{% endblock %}".to_string());
        lines.join("\n")
    }

    /// This path is not windowed, so the prepared path's ceilings must not reach it:
    /// applied here they capped on document size, and a 600-line `.njk` lost all of
    /// its HTML in the editor while the diff pane still highlighted it.
    #[test]
    fn combined_layer_survives_a_template_past_the_prepared_range_ceiling() {
        let text = dense_jinja_table(600, 4);
        let document = jinja_document(&text);

        // Pinned as a literal, not read from the prepared path's constant: the point
        // is that a document of this density used to be dropped.
        const CEILING_THAT_USED_TO_APPLY: usize = 512;
        let layer = document
            .injections
            .iter()
            .find(|layer| layer.ranges.len() > CEILING_THAT_USED_TO_APPLY)
            .unwrap_or_else(|| {
                panic!(
                    "fixture must produce more than {CEILING_THAT_USED_TO_APPLY} combined \
                     ranges to be a regression test; layers: {:?}",
                    document
                        .injections
                        .iter()
                        .map(|layer| layer.ranges.len())
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            layer.hull().end > 0,
            "the combined html layer must actually cover the template"
        );

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let probe = text.find("<td>").expect("a table cell");
        let highlights = snapshot.highlights_for_byte_range(probe..probe + "<td>".len());
        assert!(
            !highlights.is_empty(),
            "the editor dropped HTML highlighting that the diff pane keeps"
        );
    }

    /// Same defect on the byte ceiling: `live_syntax_document_supported` admits
    /// documents up to 8MB, so a 200KB template is valid but used to lose all its
    /// HTML to the 128KB ceiling.
    #[test]
    fn combined_layer_survives_a_template_past_the_prepared_byte_ceiling() {
        let mut lines = vec!["{% block body %}".to_string()];
        for ix in 0..3_000 {
            lines.push(format!(
                "  <span class=\"cell\" data-row=\"{ix}\">value {ix} padded out</span>"
            ));
        }
        lines.push("{% endblock %}".to_string());
        let text = lines.join("\n");
        assert!(
            text.len() > TS_COMBINED_INJECTION_MAX_BYTES,
            "fixture must exceed the byte ceiling ({} bytes)",
            text.len()
        );
        assert!(
            live_syntax_document_supported(DiffSyntaxLanguage::Jinja, text.len()),
            "and must still be a document the live path accepts"
        );

        let document = jinja_document(&text);
        assert!(
            document
                .injections
                .iter()
                .any(|layer| layer.ranges.len() > 1
                    || layer.hull().end - layer.hull().start > TS_COMBINED_INJECTION_MAX_BYTES),
            "the combined html layer must cover a span past the prepared byte ceiling"
        );

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let probe = text.rfind("<span").expect("a span near the end");
        let highlights = snapshot.highlights_for_byte_range(probe..probe + "<span".len());
        assert!(
            !highlights.is_empty(),
            "a {}-byte template lost its HTML highlighting in the editor",
            text.len()
        );
    }

    /// A root parse must not inherit an injected layer's clipping. `TS_PARSER` is
    /// pooled and its included ranges are sticky, so ranges left set would silently
    /// truncate the next root parse on this thread, for any language.
    #[test]
    fn an_included_range_parse_leaves_the_pooled_parser_unclipped() {
        let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
        let rope = Rope::from_str("<div class=\"a\">text</div>\n");
        let head: Range<usize> = 0..5;
        let _ = parse_included_range(html, &rope, &[], std::slice::from_ref(&head), None);

        let text = "fn main() { let value = 1; }\n";
        let document = LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Rust,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("rust live document should build");
        assert_eq!(
            document.tree.root_node().end_byte(),
            text.len(),
            "the next root parse was truncated to the previous layer's ranges"
        );
    }

    /// The half of that contract a successful parse cannot exercise. Neither the
    /// parser nor `masked_read` has a panic a test can inject, so the unwind path is
    /// pinned on the guard directly; what binds it to `parse_included_range` is that
    /// there is no other way for that function to set ranges.
    #[test]
    fn the_included_ranges_guard_clears_them_on_unwind() {
        let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
        let rope = Rope::from_str("<div class=\"a\">text</div>\n");

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_ts_parser_parse_result(&html.ts_language, |parser| {
                let included = [tree_sitter::Range {
                    start_byte: 0,
                    end_byte: 5,
                    start_point: rope_ts_point(&rope, 0),
                    end_point: rope_ts_point(&rope, 5),
                }];
                let _guard = IncludedRangesGuard::set(parser, &included)?;
                panic!("simulated parse panic");
                #[allow(unreachable_code)]
                None::<tree_sitter::Tree>
            })
        }));
        assert!(unwound.is_err(), "the probe must actually unwind");

        let text = "fn main() { let value = 1; }\n";
        let document = LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Rust,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("rust live document should build");
        assert_eq!(
            document.tree.root_node().end_byte(),
            text.len(),
            "an unwinding parse left its included ranges set on the pooled parser"
        );
    }

    fn vue_document(text: &str) -> LiveSyntaxDocument {
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Vue,
            Rope::from_str(text),
            Vec::new().into(),
            None,
        )
        .expect("vue live document should build")
    }

    /// The editor uses this layer engine rather than `prepared.rs`, so Vue's
    /// script/style/interpolation injections need covering here too.
    #[test]
    fn vue_sections_are_highlighted_by_their_own_grammars() {
        let text = concat!(
            "<template>\n",
            "  <p v-if=\"count > 10\">{{ count }}</p>\n",
            "</template>\n",
            "\n",
            "<script setup lang=\"ts\">\n",
            "const count = 42;\n",
            "</script>\n",
            "\n",
            "<style lang=\"scss\">\n",
            ".wrapper { color: red; }\n",
            "</style>\n",
        );
        let document = vue_document(text);
        assert!(
            !document.injections.is_empty(),
            "the SFC should produce injected layers"
        );

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..text.len());

        // `const` can only be coloured by the injected TypeScript layer: the Vue
        // grammar sees the whole script body as one `raw_text` node.
        assert!(
            !styles_for(&highlights, text, "const").is_empty(),
            "expected the injected TypeScript layer to highlight `const`: {highlights:?}"
        );
        // Likewise `color` comes from the injected CSS layer.
        assert!(
            !styles_for(&highlights, text, "color").is_empty(),
            "expected the injected CSS layer to highlight `color`: {highlights:?}"
        );
        // And the root Vue grammar still highlights the template markup.
        assert!(
            !styles_for(&highlights, text, "template").is_empty(),
            "the host grammar must keep highlighting its own tags"
        );
    }

    /// A document whose language has no injection query must be unaffected.
    #[test]
    fn languages_without_injections_build_no_layers() {
        let document = LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Json,
            Rope::from_str("{\"a\": 1}\n"),
            Vec::new().into(),
            None,
        )
        .expect("json live document should build");
        assert!(document.injections.is_empty());
    }

    /// Editing inside the host must not leave the injected layer painting at
    /// stale offsets — the failure mode that would look like highlighting
    /// "sliding" away from the code.
    #[test]
    fn layers_are_rebuilt_after_an_edit_moves_them() {
        let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
        let mut document = html_document(text);

        let inserted = "<div>pushed down</div>\n";
        let after = format!("<html>\n{inserted}<script>\nconst answer = 42;\n</script>\n</html>\n");
        let at = "<html>\n".len();
        document.sync(
            Rope::from_str(&after),
            Vec::new().into(),
            Some((at..at, at..at + inserted.len())),
            None,
        );

        let layer = document
            .injections
            .first()
            .expect("expected the layer to survive the edit");
        let body_start = after.find("\nconst").expect("script body") + 1;
        assert!(
            layer.hull().start <= body_start && layer.hull().end >= body_start,
            "layer {:?} should have moved with the edit to cover {body_start}",
            layer.hull()
        );

        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        let highlights = snapshot.highlights_for_byte_range(0..after.len());
        assert!(
            !styles_for(&highlights, &after, "const").is_empty(),
            "the injected keyword should still be highlighted after the edit"
        );
    }

    /// A layer the budget could not finish has to be reported, not swallowed.
    ///
    /// The budget is now shared across all layers, so a document with many
    /// injections can exhaust it partway through. Both callers assign
    /// `stale = dropped`, so if this flag were dropped on the floor
    /// `background_reparse_request` would return `None` and those regions would
    /// keep only the enclosing grammar until the user happened to type again.
    ///
    /// Driven at the function rather than through `new`/`sync`: a budget small
    /// enough to starve layers also starves the root parse, and one tuned to sit
    /// between the two would be a timing race.
    #[test]
    fn layers_the_budget_could_not_finish_are_reported_as_dropped() {
        // Script bodies large enough that their parse reaches a progress
        // callback — tree-sitter polls periodically, so a handful of bytes
        // finishes before any deadline is consulted.
        let body = (0..4_000)
            .map(|ix| format!("const answer{ix} = {ix} + compute{ix}(1, 2, 3);"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut text = String::from("<html>\n");
        for _ in 0..4 {
            text.push_str("<script>\n");
            text.push_str(&body);
            text.push_str("\n</script>\n");
        }
        text.push_str("</html>\n");

        let document = html_document(&text);
        let rope = Rope::from_str(&text);

        let (complete, dropped) =
            parse_injection_layers(&rope, document.spec, &document.tree, &[], None);
        assert_eq!(
            complete.len(),
            4,
            "an unbudgeted pass should find every script body"
        );
        assert!(!dropped, "nothing is dropped when there is no deadline");
        assert!(
            document.background_reparse_request().is_none(),
            "a complete document owes no reparse"
        );

        // A deadline already in the past breaks every layer at its first
        // progress callback.
        let (starved, dropped) = parse_injection_layers(
            &rope,
            document.spec,
            &document.tree,
            &[],
            Some(Duration::ZERO),
        );
        assert!(
            starved.len() < complete.len(),
            "an exhausted budget should not have finished every layer"
        );
        assert!(
            dropped,
            "layers skipped for want of budget must be reported so the caller \
             can mark the document stale"
        );
    }

    /// A background reparse must restore the layers a deferred sync dropped.
    ///
    /// `sync` clears `injections` when it cannot afford to reparse, on the
    /// stated promise that the background parse brings them back. Adopting the
    /// finished tree without rebuilding them leaves every injected region on the
    /// enclosing grammar — `const` renders as plain HTML text — until the user
    /// types again, which is both wrong and invisible to the reparse tests.
    #[test]
    fn adopting_a_background_tree_restores_the_injected_layers() {
        let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
        let mut document = html_document(text);
        assert!(
            !document.injections.is_empty(),
            "fixture should start with an injected script layer"
        );

        // Stand where a `Deferred` sync leaves the document: the root tree has
        // been edited forward, but the layers whose ranges moved were dropped.
        document.injections.clear();
        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        assert!(
            styles_for(
                &snapshot.highlights_for_byte_range(0..text.len()),
                text,
                "const"
            )
            .is_empty(),
            "with the layers dropped the injected keyword is unhighlighted — the \
             state the background parse exists to repair"
        );

        // The background parse finishes and hands its tree back.
        document.stale = true;
        let request = document
            .background_reparse_request()
            .expect("a stale document owes a background reparse");
        let (version, tree, injections) =
            live_syntax_reparse(request).expect("an unbudgeted reparse should succeed");
        assert!(document.adopt_background_tree(version, tree, injections));

        assert!(
            !document.injections.is_empty(),
            "adopting the caught-up tree must rebuild the injected layers"
        );
        let snapshot = document.snapshot(AppTheme::gitcomet_dark());
        assert!(
            !styles_for(
                &snapshot.highlights_for_byte_range(0..text.len()),
                text,
                "const"
            )
            .is_empty(),
            "the injected keyword must be highlighted again after the background parse"
        );
    }
}
