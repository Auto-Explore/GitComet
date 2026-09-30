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
//! Used by file editors and the merge tool's editable resolved output.

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
/// injected layers can be added by merging several layers' cursors into one
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
/// `clip` is the layer's current ownership. Only the root (an empty `clip`)
/// bypasses clipping: after an edit even a single old tree can span a new host
/// boundary while its replacement parse is deferred.
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
                if let Some(capture) = m.captures().get(*capture_ix)
                    && let Some(kind) = capture_kinds.get(capture.index as usize).copied().flatten()
                {
                    let range = clamp_to_len(capture.node.byte_range(), text_len);
                    if !range.is_empty() {
                        if clip.is_empty() {
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
#[derive(Clone)]
pub(in crate::view) struct LiveSyntaxLayer {
    spec: &'static TreesitterHighlightSpec,
    tree: tree_sitter::Tree,
    ranges: Vec<Range<usize>>,
    /// 1 for a layer injected by the root, 2 for one injected by a layer.
    depth: u8,
    /// Its edited tree still needs parsing. Nested ownership must be retained
    /// from the previous layers until this parent's query is current again.
    pending: bool,
}

impl LiveSyntaxLayer {
    fn edit(&mut self, edit: &tree_sitter::InputEdit) {
        self.tree.edit(edit);
        self.pending = true;
        // Layer ownership has left affinity at its start: text typed directly
        // after `<script>` or at a Markdown paragraph start belongs to that
        // layer. Tree-sitter's included-range start has right affinity, which
        // would exclude that text and prevent reuse on the next parse.
        for range in &mut self.ranges {
            let shifted = |offset: usize| offset - edit.old_end_byte + edit.new_end_byte;
            range.start = if range.start <= edit.start_byte {
                range.start
            } else if range.start < edit.old_end_byte {
                edit.start_byte
            } else {
                shifted(range.start)
            };
            range.end = if range.end < edit.start_byte {
                range.end
            } else if range.end < edit.old_end_byte {
                edit.start_byte
            } else {
                shifted(range.end)
            };
        }
        self.ranges.retain(|range| !range.is_empty());
    }

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

/// Parse the injected grammars found in `tree`, to `TS_MAX_INJECTION_DEPTH` so
/// the editor agrees with the diff panes: a template's HTML is depth 1, and its
/// `<script>` bodies are depth 2.
///
/// `budget` is a ceiling for *all* layers together, not per layer. Handing each
/// one its own copy let a document with N injections spend N × budget on the
/// keystroke path while the root parse it was protecting stayed capped at one —
/// so a markdown file with many fenced blocks blocked the frame in proportion to
/// how many it had.
///
/// Returns the layers plus whether any need a background reparse. Previously
/// edited layers seed their replacements. A finished layer is published at once;
/// only unfinished regions retain their previous trees. The edited region gets
/// the first attempt so unrelated layers cannot starve held-key highlighting.
fn parse_injection_layers(
    rope: &Rope,
    spec: &TreesitterHighlightSpec,
    tree: &tree_sitter::Tree,
    mask: &[Range<usize>],
    budget: Option<Duration>,
    previous: Vec<LiveSyntaxLayer>,
    priority: Option<Range<usize>>,
) -> (Vec<LiveSyntaxLayer>, bool) {
    // One deadline for the whole set, so the cost of injections is bounded by
    // the budget rather than by how many there are.
    let mut parser = InjectionParser::new(rope, mask, budget, previous, priority);
    let mut layers = Vec::new();
    let targets = collect_injection_targets(rope, spec, tree, 0..rope.len());
    let mut dropped = parser.parse_layers_for_targets(targets, None, 1, &mut layers);

    // Each layer's own injections, clipped to the layer's ranges: a raw_text
    // spanning a `{% if %}` gap must not hand the template bytes to JSON.
    let mut parents = 0..layers.len();
    for depth in 2..=TS_MAX_INJECTION_DEPTH as u8 {
        let mut nested = Vec::new();
        for parent in &layers[parents.clone()] {
            if parent.spec.injection_query.is_none() {
                continue;
            }
            if parent.pending {
                parser.retain_nested_layers(parent, depth, &mut nested);
                continue;
            }
            let targets = collect_injection_targets(rope, parent.spec, &parent.tree, parent.hull());
            dropped |=
                parser.parse_layers_for_targets(targets, Some(&parent.ranges), depth, &mut nested);
        }
        if nested.is_empty() {
            break;
        }
        parents = layers.len()..layers.len() + nested.len();
        layers.extend(nested);
    }
    (layers, dropped)
}

/// What one tree's injection query asks for: single layers, one per match, and
/// combined groups, one per pattern. A truncated query drops the groups
/// entirely — losing one range out of a combined set changes the document the
/// injected grammar sees, so half a group is worse than none.
struct InjectionTargets {
    singles: Vec<(DiffSyntaxLanguage, Range<usize>)>,
    groups: Vec<(DiffSyntaxLanguage, usize, Vec<Range<usize>>)>,
}

fn collect_injection_targets(
    rope: &Rope,
    spec: &TreesitterHighlightSpec,
    tree: &tree_sitter::Tree,
    scope: Range<usize>,
) -> InjectionTargets {
    let mut targets = InjectionTargets {
        singles: Vec::new(),
        groups: Vec::new(),
    };
    let Some(query) = spec.injection_query.as_ref() else {
        return targets;
    };
    let Some(content_ix) = query.capture_index_for_name("injection.content") else {
        return targets;
    };
    let language_ix = query
        .capture_index_for_name("injection.language")
        .or_else(|| query.capture_index_for_name("language"));

    // The gate that keeps this a no-op for grammars with no combined pattern.
    let has_combined = spec.has_combined_injections;

    // Collect first, parse second: the query cursor is a thread-local, so
    // parsing a layer while still holding it would re-enter the borrow.
    let mut combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>> =
        FxHashMap::default();
    let mut truncated = false;
    catch_treesitter_query_panic(|| {
        TS_CURSOR.with(|cursor| {
            let mut cursor = cursor.borrow_mut();
            cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
            cursor.set_byte_range(scope);
            cursor.set_containing_byte_range(0..usize::MAX);
            let mut matches = cursor.matches(query, tree.root_node(), RopeTextProvider(rope));
            tree_sitter::StreamingIterator::advance(&mut matches);
            while let Some(m) = matches.get() {
                if let Some(language) = injection_language_for_match(rope, query, m, language_ix) {
                    let pattern_ix = m.pattern_index;
                    let is_combined = spec.is_combined_injection_pattern(pattern_ix);
                    for capture in m.captures().iter().filter(|c| c.index == content_ix) {
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
                                targets.singles.push((language, range));
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
    targets
        .singles
        .sort_by(|(a_language, a_range), (b_language, b_range)| {
            (a_range.start, a_range.end, *a_language).cmp(&(
                b_range.start,
                b_range.end,
                *b_language,
            ))
        });
    targets.singles.dedup();

    // No TS_COMBINED_INJECTION_MAX_* ceiling here, deliberately, and do not add
    // one. Those are the prepared path's windowed fallback, measured against a
    // 64-row window; this path is not windowed, so they capped on document size,
    // costing a 600-line `.njk` all of its HTML while the diff pane still
    // highlighted it. `deadline` already bounds this parse, and a layer it drops
    // sets `dropped` so the off-thread reparse restores it.
    if has_combined && !truncated {
        targets.groups = combined_injection_groups_in_apply_order(combined_ranges);
    }
    targets
}

/// A sorted interval index finds old layers even when an edit splits or joins
/// paragraphs. Prefix maximum ends also cover overlapping combined injections.
struct PreviousLayerSpan {
    start: usize,
    max_end: usize,
    index: usize,
}

struct InjectionParser<'a> {
    rope: &'a Rope,
    mask: &'a [Range<usize>],
    deadline: Option<Instant>,
    priority: Option<Range<usize>>,
    previous: Vec<Option<LiveSyntaxLayer>>,
    previous_by_grammar: FxHashMap<(u8, *const TreesitterHighlightSpec), Vec<PreviousLayerSpan>>,
}

impl<'a> InjectionParser<'a> {
    fn new(
        rope: &'a Rope,
        mask: &'a [Range<usize>],
        budget: Option<Duration>,
        previous: Vec<LiveSyntaxLayer>,
        priority: Option<Range<usize>>,
    ) -> Self {
        let deadline = budget.map(|budget| Instant::now() + budget);
        let mut by_grammar: FxHashMap<_, Vec<PreviousLayerSpan>> = FxHashMap::default();
        for (index, layer) in previous.iter().enumerate() {
            let hull = layer.hull();
            by_grammar
                .entry((layer.depth, std::ptr::from_ref(layer.spec)))
                .or_default()
                .push(PreviousLayerSpan {
                    start: hull.start,
                    max_end: hull.end,
                    index,
                });
        }
        for spans in by_grammar.values_mut() {
            spans.sort_by_key(|span| (span.start, span.index));
            let mut max_end = 0;
            for span in spans {
                max_end = max_end.max(span.max_end);
                span.max_end = max_end;
            }
        }
        Self {
            rope,
            mask,
            deadline,
            priority,
            previous: previous.into_iter().map(Some).collect(),
            previous_by_grammar: by_grammar,
        }
    }

    fn overlapping_previous(
        &self,
        depth: u8,
        spec: &TreesitterHighlightSpec,
        ranges: &[Range<usize>],
    ) -> Vec<usize> {
        let Some(spans) = self
            .previous_by_grammar
            .get(&(depth, std::ptr::from_ref(spec)))
        else {
            return Vec::new();
        };
        self.overlapping_spans(spans, ranges)
    }

    fn overlapping_spans(
        &self,
        spans: &[PreviousLayerSpan],
        ranges: &[Range<usize>],
    ) -> Vec<usize> {
        let end = spans.partition_point(|span| span.start < ranges.last().unwrap().end);
        let start = spans[..end].partition_point(|span| span.max_end <= ranges[0].start);
        spans[start..end]
            .iter()
            .filter_map(|span| {
                let layer = self.previous[span.index].as_ref()?;
                (!intersect_sorted_ranges(&layer.ranges, ranges).is_empty()).then_some(span.index)
            })
            .collect()
    }

    fn retain_nested_layers(
        &mut self,
        parent: &LiveSyntaxLayer,
        depth: u8,
        out: &mut Vec<LiveSyntaxLayer>,
    ) {
        // Querying an edited, unparsed parent can shift a nested range past
        // text inserted at its start. Keep the children's existing ownership
        // until the parent can describe its new injection boundaries.
        let mut candidates = self
            .previous_by_grammar
            .iter()
            .filter(|((layer_depth, _), _)| *layer_depth == depth)
            .flat_map(|(_, spans)| self.overlapping_spans(spans, &parent.ranges))
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for index in candidates {
            let previous = self.previous[index].as_ref().unwrap();
            let ranges = intersect_sorted_ranges(&previous.ranges, &parent.ranges);
            let mut layer = if ranges == previous.ranges {
                self.previous[index].take().unwrap()
            } else {
                LiveSyntaxLayer {
                    ranges,
                    ..previous.clone()
                }
            };
            layer.pending = true;
            out.push(layer);
        }
    }

    /// Parse in edit-priority order, but publish in query order so precedence
    /// stays unchanged. Retain only the layers whose own parse was deferred.
    fn parse_layers_for_targets(
        &mut self,
        targets: InjectionTargets,
        clip_to: Option<&[Range<usize>]>,
        depth: u8,
        out: &mut Vec<LiveSyntaxLayer>,
    ) -> bool {
        let singles = targets
            .singles
            .into_iter()
            .map(|(language, range)| (language, vec![range]));
        let groups = targets
            .groups
            .into_iter()
            .map(|(language, _, ranges)| (language, ranges));
        let mut targets = singles.chain(groups).enumerate().collect::<Vec<_>>();
        let mut results = (0..targets.len()).map(|_| Vec::new()).collect::<Vec<_>>();
        if let Some(priority) = &self.priority {
            targets.sort_by_key(|(_, (_, ranges))| {
                !ranges
                    .iter()
                    .any(|range| range.start <= priority.end && range.end >= priority.start)
            });
        }
        let mut dropped = false;
        for (order, (language, ranges)) in targets {
            let Some(layer_spec) = tree_sitter_highlight_spec(language) else {
                continue;
            };
            let ranges = match clip_to {
                Some(parent) => intersect_sorted_ranges(&ranges, parent),
                None => ranges,
            };
            if ranges.is_empty() {
                continue;
            }
            let candidates = self.overlapping_previous(depth, layer_spec, &ranges);
            let exact = candidates.iter().copied().find(|&index| {
                self.previous[index]
                    .as_ref()
                    .is_some_and(|layer| layer.ranges == ranges)
            });
            // Moving an exact match avoids cloning hundreds of trees on the
            // usual timeout path. A changed boundary can still use an
            // overlapping tree as an incremental seed.
            let previous = exact.and_then(|index| self.previous[index].take());
            let seed = previous.as_ref().or_else(|| {
                candidates
                    .first()
                    .and_then(|&index| self.previous[index].as_ref())
            });
            let tree = parse_included_range(
                layer_spec,
                self.rope,
                self.mask,
                &ranges,
                self.deadline,
                seed.map(|layer| &layer.tree),
            );
            match tree {
                Some(tree) => results[order].push(LiveSyntaxLayer {
                    spec: layer_spec,
                    tree,
                    ranges,
                    depth,
                    pending: false,
                }),
                None => {
                    dropped = true;
                    if let Some(mut previous) = previous {
                        previous.pending = true;
                        results[order].push(previous);
                    } else {
                        // A split may need pieces of one old tree in multiple
                        // new layers; a join may need several old trees. Clone
                        // only those boundary changes, clipped to their new
                        // ownership, until a complete parse replaces them.
                        for index in candidates {
                            if let Some(previous) = self.previous[index].as_ref() {
                                let ranges = intersect_sorted_ranges(&previous.ranges, &ranges);
                                if !ranges.is_empty() {
                                    results[order].push(LiveSyntaxLayer {
                                        ranges,
                                        pending: true,
                                        ..previous.clone()
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        out.extend(results.into_iter().flatten());
        dropped
    }
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
            .captures()
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
    old_tree: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree> {
    #[cfg(test)]
    if !injection_tests::take_parse_slot(old_tree.is_some()) {
        return None;
    }
    if ranges.is_empty() || deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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
            None => guard.parser().parse_with_options(&mut read, old_tree, None),
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
                    .parse_with_options(&mut read, old_tree, Some(options))
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
    /// Injected grammars, depth 1 then 2, edited and reparsed with the root tree.
    injections: Vec<LiveSyntaxLayer>,
    stale: bool,
    version: u64,
}

/// What a parse attempt managed to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum LiveSyntaxSyncOutcome {
    /// The root tree describes the current text. Injected layers may still
    /// require the background reparse reported by the document.
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
            parse_injection_layers(&rope, spec, &tree, mask.as_ref(), budget, Vec::new(), None);
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

        let priority = edit.as_ref().map(|(_, inserted)| inserted.clone());
        let seed = match edit {
            Some((replaced, inserted)) => {
                // Positions come straight off the summaries — an O(log n)
                // descent each, with no line-start array to keep in step.
                let replaced = clamp_to_len(replaced, self.rope.len());
                let inserted = clamp_to_len(inserted, rope.len());
                let edit = tree_sitter::InputEdit {
                    start_byte: replaced.start,
                    old_end_byte: replaced.end,
                    new_end_byte: inserted.end,
                    start_position: rope_ts_point(&self.rope, replaced.start),
                    old_end_position: rope_ts_point(&self.rope, replaced.end),
                    new_end_position: rope_ts_point(&rope, inserted.end),
                };
                self.tree.edit(&edit);
                for layer in &mut self.injections {
                    layer.edit(&edit);
                }
                self.injections.retain(|layer| !layer.ranges.is_empty());
                true
            }
            None => {
                self.injections.clear();
                false
            }
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
                    std::mem::take(&mut self.injections),
                    priority,
                );
                self.injections = injections;
                // The root tree is current either way; `dropped` says only that
                // some injected region still uses its edited tree or has no
                // tree yet. The background reparse finishes those regions.
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
                // Keep every edited layer, including inline Markdown. Their
                // nodes and clipping ranges moved together, so unchanged tokens
                // retain their colors while the background parse catches up.
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
            injections: self.injections.clone(),
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
        // Replace the provisional layers along with their parent. Both describe
        // this exact version, including any changed injection boundaries.
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
            injections: self.injections.clone(),
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
    injections: Vec<LiveSyntaxLayer>,
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
        request.injections,
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
/// its node ranges use the same coordinates as the text it carries. Token kinds
/// can remain provisional during a deferred parse; the owner republishes the
/// completed tree without callers having to interpolate ranges themselves.
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
                        layer.depth,
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
    /// Unlike [`Self::syntax_pair_at`], this is O(document), but it scans the
    /// rope's borrowed chunks directly and the editor holds a successful answer
    /// while the caret remains on any occurrence. It is available up to the
    /// same ceiling as the live syntax tree itself.
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
        // The rope lookup asks the tree for the small token under the caret
        // before starting its document scan, so punctuation and whitespace
        // remain one cheap descent.
        syntax_occurrences_in_rope(&inner.tree, &inner.rope, offset)
            .map(|found| found.ranges)
            .unwrap_or_default()
    }

    /// The name token the caret at `offset` is on, without searching for its
    /// other occurrences.
    ///
    /// This is the same lookup [`Self::occurrences_at`] starts with, exposed on
    /// its own so a caller holding a previous answer can ask "is the caret still
    /// on one of these?" for the price of a tree descent rather than another
    /// O(document) scan. Answering that by range arithmetic instead cannot be
    /// exact: two name tokens can abut, and then one offset sits at the end of
    /// the first and the start of the second.
    pub(in crate::view) fn name_token_at(&self, offset: usize) -> Option<Range<usize>> {
        let inner = self.0.as_ref();
        let offset = offset.min(inner.rope.len());
        name_token_at(&inner.tree, offset, |range| {
            Some(inner.rope.text_for_range(range))
        })
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
        // Injected grammars first, deepest first (layers are appended by depth):
        // a brace inside an interpolated region is the inner grammar's. Layer
        // trees are parsed with `included_ranges`, so their node offsets are
        // already document coordinates.
        for layer in inner.injections.iter().rev() {
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
mod tests;

/// Injected sub-grammars: a `<script>` body must be highlighted as JavaScript,
/// not left as opaque HTML raw text.
///
/// This is what the editable resolved output was missing relative to the
/// read-only diff panes above it, which have had injections all along.
#[cfg(test)]
mod injection_tests;
