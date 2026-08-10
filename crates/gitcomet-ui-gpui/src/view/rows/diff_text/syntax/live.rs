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
//! Used by the merge tool's editable resolved output.

use super::super::{SyntaxHighlightPalette, syntax_highlight_palette};
use super::*;
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
    text: &'a [u8],
    mask: &'a [Range<usize>],
) -> impl FnMut(usize, tree_sitter::Point) -> &'a [u8] {
    move |offset, _position| {
        if offset >= text.len() {
            return &[];
        }
        // `mask` is sorted and disjoint, so the first span ending past `offset`
        // is the only one that can contain or follow it.
        let ix = mask.partition_point(|span| span.end <= offset);
        match mask.get(ix) {
            Some(span) if span.start <= offset => {
                let masked_end = span.end.min(text.len());
                &BLANKS[..(masked_end - offset).min(BLANKS.len())]
            }
            Some(span) => &text[offset..span.start.min(text.len())],
            None => &text[offset..],
        }
    }
}

fn parse_masked_tree(
    spec: &TreesitterHighlightSpec,
    text: &str,
    mask: &[Range<usize>],
    old_tree: Option<&tree_sitter::Tree>,
    budget: Option<Duration>,
) -> Option<tree_sitter::Tree> {
    let bytes = text.as_bytes();
    with_ts_parser_parse_result(&spec.ts_language, |parser| {
        let mut read = masked_read(bytes, mask);
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
/// Ported from Zed's `BufferChunks::next`. `next_capture` yields captures in
/// ascending start order; the stack holds those still open at the cursor, and
/// the innermost — the last pushed — wins. That is what makes a `self` inside a
/// parameter list read as a keyword rather than inheriting the enclosing
/// function-signature capture.
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

/// A live tree-sitter document, owned by the view that edits it.
pub(in crate::view) struct LiveSyntaxDocument {
    language: DiffSyntaxLanguage,
    spec: &'static TreesitterHighlightSpec,
    text: Arc<str>,
    line_starts: Arc<[usize]>,
    mask: Arc<[Range<usize>]>,
    tree: tree_sitter::Tree,
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
        text: Arc<str>,
        line_starts: Arc<[usize]>,
        mask: Arc<[Range<usize>]>,
        budget: Option<Duration>,
    ) -> Option<Self> {
        if text.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
            return None;
        }
        let spec = tree_sitter_highlight_spec(language)?;
        let tree = parse_masked_tree(spec, text.as_ref(), mask.as_ref(), None, budget)?;
        Some(Self {
            language,
            spec,
            text,
            line_starts,
            mask,
            tree,
            stale: false,
            version: next_live_syntax_version(),
        })
    }

    pub(in crate::view) fn language(&self) -> DiffSyntaxLanguage {
        self.language
    }

    /// Fold one coalesced edit into the tree and reparse.
    ///
    /// `text` and `line_starts` must already reflect the edit. `edit` is
    /// `(replaced, inserted)` — the replaced span in the *old* text's
    /// coordinates and the inserted span in the new text's, sharing a start.
    /// `None` means the text was replaced wholesale, which reparses from
    /// scratch: a conflict resolution rewrites structure, and there is no
    /// keystroke latency to protect on that path.
    ///
    /// The version advances either way, so a caller keying a highlight provider
    /// on it always rebinds.
    ///
    /// Returns [`LiveSyntaxSyncOutcome::Abandoned`] without touching the
    /// document when the edit takes the buffer past the size ceiling; the
    /// caller drops it, exactly as it would never have been built at that size.
    pub(in crate::view) fn sync(
        &mut self,
        text: Arc<str>,
        line_starts: Arc<[usize]>,
        mask: Arc<[Range<usize>]>,
        edit: Option<(Range<usize>, Range<usize>)>,
        budget: Option<Duration>,
    ) -> LiveSyntaxSyncOutcome {
        // The ceiling bounds the *document*, not just the incremental step, so
        // it has to be rechecked on every edit. Parsing past it here would let
        // a single paste buy an unbounded background reparse for the rest of
        // the session.
        if text.len() > PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES {
            return LiveSyntaxSyncOutcome::Abandoned;
        }

        let seed = match edit {
            Some((replaced, inserted)) => {
                let old_bytes = self.text.as_bytes();
                let replaced = clamp_to_len(replaced, old_bytes.len());
                let inserted = clamp_to_len(inserted, text.len());
                self.tree.edit(&tree_sitter::InputEdit {
                    start_byte: replaced.start,
                    old_end_byte: replaced.end,
                    new_end_byte: inserted.end,
                    start_position: treesitter_point_for_byte(
                        &self.line_starts,
                        old_bytes,
                        replaced.start,
                    ),
                    old_end_position: treesitter_point_for_byte(
                        &self.line_starts,
                        old_bytes,
                        replaced.end,
                    ),
                    new_end_position: treesitter_point_for_byte(
                        &line_starts,
                        text.as_bytes(),
                        inserted.end,
                    ),
                });
                true
            }
            None => false,
        };

        self.text = text;
        self.line_starts = line_starts;
        self.mask = mask;
        self.version = next_live_syntax_version();

        let old_tree = seed.then_some(&self.tree);
        match parse_masked_tree(
            self.spec,
            self.text.as_ref(),
            self.mask.as_ref(),
            old_tree,
            budget,
        ) {
            Some(tree) => {
                self.tree = tree;
                self.stale = false;
                LiveSyntaxSyncOutcome::Reparsed
            }
            None => {
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
            text: Arc::clone(&self.text),
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
    ) -> bool {
        if self.version != for_version {
            return false;
        }
        self.tree = tree;
        self.stale = false;
        self.version = next_live_syntax_version();
        true
    }

    pub(in crate::view) fn snapshot(&self, theme: AppTheme) -> LiveSyntaxSnapshot {
        LiveSyntaxSnapshot(Arc::new(LiveSyntaxSnapshotInner {
            spec: self.spec,
            text: Arc::clone(&self.text),
            tree: self.tree.clone(),
            palette: syntax_highlight_palette(theme),
        }))
    }
}

/// A deferred reparse, detached from the document so it can run off-thread.
pub(in crate::view) struct LiveSyntaxReparseRequest {
    spec: &'static TreesitterHighlightSpec,
    text: Arc<str>,
    mask: Arc<[Range<usize>]>,
    old_tree: tree_sitter::Tree,
    version: u64,
}

/// Run a deferred reparse to completion. Safe to call under `smol::unblock`.
///
/// Returns the version it was parsed for, so the caller can tell whether the
/// document moved on in the meantime.
pub(in crate::view) fn live_syntax_reparse(
    request: LiveSyntaxReparseRequest,
) -> Option<(u64, tree_sitter::Tree)> {
    parse_masked_tree(
        request.spec,
        request.text.as_ref(),
        request.mask.as_ref(),
        Some(&request.old_tree),
        None,
    )
    .map(|tree| (request.version, tree))
}

struct LiveSyntaxSnapshotInner {
    spec: &'static TreesitterHighlightSpec,
    text: Arc<str>,
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
        let text = inner.text.as_bytes();
        let range = clamp_to_len(byte_range, text.len());
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

            catch_treesitter_query_panic(|| {
                TS_CURSOR.with(|cursor| {
                    let mut cursor = cursor.borrow_mut();
                    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
                    cursor.set_byte_range(pass.clone());
                    cursor.set_containing_byte_range(0..usize::MAX);
                    // `set_byte_range` yields every capture *intersecting* the
                    // window, so a string or block comment opened far above it
                    // still arrives — no look-behind needed.
                    let mut captures =
                        cursor.captures(&inner.spec.query, inner.tree.root_node(), text);
                    tree_sitter::StreamingIterator::advance(&mut captures);

                    let text_len = text.len();
                    let capture_kinds = inner.spec.capture_kinds.as_slice();
                    let next_capture = || loop {
                        let (m, capture_ix) = captures.get()?;
                        let hit = m.captures.get(*capture_ix).and_then(|capture| {
                            let kind = capture_kinds
                                .get(capture.index as usize)
                                .copied()
                                .flatten()?;
                            let range = clamp_to_len(capture.node.byte_range(), text_len);
                            (!range.is_empty()).then_some((range, kind))
                        });
                        tree_sitter::StreamingIterator::advance(&mut captures);
                        if hit.is_some() {
                            return hit;
                        }
                    };

                    sweep_runs(next_capture, &inner.palette, pass.clone(), &mut out);
                });
            });

            pass_start = pass_end;
        }
        out
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
        if starts.last() == Some(&text.len()) && text.len() > 0 {
            starts.pop();
        }
        starts.into()
    }

    fn document(text: &str, mask: Vec<Range<usize>>) -> LiveSyntaxDocument {
        let text: Arc<str> = Arc::from(text);
        LiveSyntaxDocument::new(
            DiffSyntaxLanguage::Rust,
            Arc::clone(&text),
            line_starts_for(text.as_ref()),
            mask.into(),
            None,
        )
        .expect("rust live document should build")
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
        let masked = document(text, vec![10..26]).snapshot(AppTheme::gitcomet_dark());
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
            Arc::from(after),
            line_starts_for(after),
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
                Arc::from(after.as_str()),
                line_starts_for(&after),
                Vec::new().into(),
                None,
            )
            .is_none(),
            "the fixture must be past the ceiling for this test to mean anything"
        );

        let outcome = doc.sync(
            Arc::from(after.as_str()),
            line_starts_for(&after),
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
            Arc::from(after.as_str()),
            line_starts_for(&after),
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
        let (version, tree) = live_syntax_reparse(request).expect("unbudgeted reparse succeeds");
        assert!(
            doc.adopt_background_tree(version, tree),
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

    #[test]
    fn a_background_tree_for_a_superseded_version_is_rejected() {
        let before = "fn main() {}\n";
        let mut doc = document(before, Vec::new());
        let stale_version = doc.version();
        let tree = doc.snapshot(AppTheme::gitcomet_dark()).0.tree.clone();

        let after = "fn main() { let x = 1; }\n";
        doc.sync(
            Arc::from(after),
            line_starts_for(after),
            Vec::new().into(),
            None,
            None,
        );

        assert!(
            !doc.adopt_background_tree(stale_version, tree),
            "a tree parsed for text that has since changed must be discarded"
        );
    }

    #[test]
    fn wholesale_replacement_reparses_from_scratch() {
        let mut doc = document("fn main() {}\n", Vec::new());
        let after = "struct Point { x: u32, y: u32 }\n";

        let outcome = doc.sync(
            Arc::from(after),
            line_starts_for(after),
            Vec::new().into(),
            None,
            None,
        );
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
        let text = b"abcdefghij";
        let mask = [2..5usize];
        let mut read = masked_read(text, &mask);

        assert_eq!(read(0, tree_sitter::Point::new(0, 0)), b"ab");
        assert_eq!(read(2, tree_sitter::Point::new(0, 2)), b"   ");
        assert_eq!(read(5, tree_sitter::Point::new(0, 5)), b"fghij");
        assert_eq!(read(10, tree_sitter::Point::new(0, 10)), b"");
    }

    #[test]
    fn masked_read_spans_longer_than_the_blank_buffer_are_served_in_pieces() {
        let text = vec![b'x'; 200];
        let mask = [0..200usize];
        let mut read = masked_read(&text, &mask);

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
                Arc::clone(&huge),
                line_starts_for(huge.as_ref()),
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
                Arc::clone(&text),
                line_starts_for(text.as_ref()),
                Vec::new().into(),
                None,
            )
            .is_none(),
            "a language with no wired grammar should fall back rather than build"
        );
    }

    /// The merge tool's resolved output and the diff panes above it must colour
    /// the same code the same way, and they do not share an engine: this one
    /// sweeps a `QueryCursor` over the viewport ([`sweep_runs`]), the diff panes
    /// materialize per-line tokens and resolve overlaps with
    /// `normalize_non_overlapping_tokens`. Hold the tree constant and check the
    /// two derivations agree byte for byte.
    ///
    /// Deliberately macro-free: [`super::prepared`] also applies
    /// `*_injections.scm` and this engine does not yet, so a `println!` body is
    /// a known divergence rather than a regression. See the note on
    /// [`sweep_runs`] about merging injected layers into its capture stream.
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

        let theme = AppTheme::gitcomet_dark();
        let palette = syntax_highlight_palette(theme);
        let snapshot = document(text, Vec::new()).snapshot(theme);

        let mut live_by_byte = vec![None; text.len()];
        for (range, style) in snapshot.highlights_for_byte_range(0..text.len()) {
            for byte in range {
                live_by_byte[byte] = Some(style);
            }
        }

        // Same tree, so any disagreement below is in how the captures are turned
        // into styles -- which is exactly what differs between the two engines.
        let line_starts = line_starts_for(text);
        let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Rust)
            .expect("rust should have a wired grammar");
        let per_line = collect_treesitter_document_line_tokens_for_line_window(
            &snapshot.0.tree,
            spec,
            text.as_bytes(),
            line_starts.as_ref(),
            0,
            line_starts.len(),
        );
        let mut prepared_by_byte = vec![None; text.len()];
        for (line_ix, tokens) in per_line.iter().enumerate() {
            let line_start = line_starts[line_ix];
            for token in tokens {
                let Some(style) = palette.style(token.kind) else {
                    continue;
                };
                for byte in (line_start + token.range.start)..(line_start + token.range.end) {
                    prepared_by_byte[byte] = Some(style);
                }
            }
        }

        // Guards against the comparison passing because both sides are empty.
        for (needle, kind) in [
            ("Stage<'a>", SyntaxTokenKind::Type),
            ("usize", SyntaxTokenKind::TypeBuiltin),
            ("retries: usize", SyntaxTokenKind::Property),
            ("wrapping_add", SyntaxTokenKind::FunctionMethod),
        ] {
            let at = text.find(needle).expect("fixture should contain the probe");
            assert_eq!(
                live_by_byte[at],
                palette.style(kind),
                "{needle:?} at {at} should carry {kind:?}; without these classes the \
                 comparison below cannot tell tree-sitter from the heuristic tokenizer"
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
            "the resolved output and the diff panes must colour identical text \
             identically; diverging bytes:\n  {}",
            mismatched.join("\n  ")
        );
    }
}
