//! The editable working-tree buffer.
//!
//! Structurally a smaller sibling of the merge tool's resolved-output pane: the
//! same [`components::TextInput`] over the same rope, fed by the same
//! [`rows::LiveSyntaxDocument`] — `tree.edit()` on the keystroke so the tree is
//! positionally correct for the very next frame, a budgeted foreground reparse,
//! and an off-thread one when that budget runs out. What it does *not* carry is
//! the conflict machinery: no placeholder mask, no protected rows, no overlay
//! for unresolved blocks. The one overlay it does add is the bracket pair the
//! caret sits in.
//!
//! Buffers are keyed by path and survive switching files, so an unsaved edit is
//! still there when the user comes back. Only the text and caret are stashed —
//! `TextInput`'s undo stack does not leave the widget.

use super::*;
use crate::kit::rope::Rope;
use crate::kit::text_model::TextModelSnapshot;
use crate::kit::{HighlightProvider, HighlightProviderResult};
use palette::IntoColor;
use std::path::{Path, PathBuf};

/// How long the buffer has to sit still before auto-save writes it.
///
/// Long enough that typing a word is one write rather than five, short enough
/// that "I stopped typing" and "it is on disk" feel like the same moment.
pub(in crate::view) const FILE_EDITOR_AUTOSAVE_DEBOUNCE_MS: u64 = 800;

/// A buffer the editor is holding for a file that is not on screen.
///
/// Carries the fingerprint the text was last known to agree with on disk, so
/// dirtiness survives the round trip: a single "last saved" slot on the pane
/// would be the *other* file's by the time this one comes back.
#[derive(Clone, Debug)]
pub(in crate::view) struct StashedFileEdit {
    pub(in crate::view) text: SharedString,
    pub(in crate::view) cursor: usize,
    /// Fingerprint of `text` itself, recorded when the entry was made rather
    /// than recomputed on read. A second entry point that hashed the flat
    /// string could not agree with the rope one anyway: `FxHasher` folds each
    /// `write` call separately, so N chunks and one slice of the same bytes
    /// give different results.
    pub(in crate::view) text_fingerprint: u64,
    pub(in crate::view) saved_fingerprint: u64,
    /// The blame watermark from when this buffer was on screen. Carried across
    /// the round trip for the same reason `saved_fingerprint` is: recomputing it
    /// on restore is impossible — the edits it summarizes are gone — and
    /// defaulting it to line 0 would blank the annotation column of every file
    /// the user has ever come back to.
    pub(in crate::view) first_dirty_line: Option<u32>,
}

impl StashedFileEdit {
    /// Whether this buffer still differs from what was last written.
    ///
    /// Clean entries are kept only so a write that fails leaves the text
    /// somewhere recoverable; they are dropped the next time the file is
    /// opened, which is also what stops them masking an external edit.
    pub(in crate::view) fn is_dirty(&self) -> bool {
        self.text_fingerprint != self.saved_fingerprint
    }
}

/// Identity of the text a save or a dirty check was computed over.
///
/// A hash rather than the text: the comparison runs on every keystroke, and
/// keeping a second copy of the file around to `==` against it would double the
/// buffer's memory for a question that a `u64` answers.
///
/// Fed the raw bytes rather than `str::hash`, which writes a `0xff` terminator
/// per call. Chunk boundaries are an artifact of the rope's edit history — the
/// same text reached by typing a character and deleting it again is split
/// differently — so hashing chunk-wise made a byte-identical buffer read as
/// permanently modified.
pub(in crate::view) fn file_editor_text_fingerprint(snapshot: &TextModelSnapshot) -> u64 {
    use std::hash::Hasher;

    let mut hasher = rustc_hash::FxHasher::default();
    hasher.write_usize(snapshot.len());
    // Chunk-wise, so a large file is never flattened into one string just to be
    // fingerprinted. `TextModelSnapshot::rope()` is an `Arc` bump.
    for chunk in snapshot.rope().chunks() {
        hasher.write(chunk.as_bytes());
    }
    hasher.finish()
}

/// The provider binding key for the editor's highlights.
///
/// Must be *stable* when nothing changed: installing a provider notifies the
/// input, which re-enters the `cx.observe` that installed it, so a key that
/// varied per call would rebind, notify, and spin forever.
/// `set_highlight_provider_with_key` early-returns on an unchanged key, and that
/// early return is what terminates the cycle.
pub(in crate::view) fn file_editor_provider_binding_key(
    document_version: u64,
    theme_epoch: u64,
    bracket_match: Option<&(Range<usize>, Range<usize>)>,
    search_overlay: &[Range<usize>],
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = rustc_hash::FxHasher::default();
    document_version.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    // Typing in the search box moves no text and touches no tree either, so
    // without this the provider stays bound to the pre-query closure and nothing
    // is ever painted.
    search_overlay.hash(&mut hasher);
    // Moving the caret between brackets moves no text and touches no tree, so
    // this is the only thing that tells the input its highlights changed.
    match bracket_match {
        Some((open, close)) => {
            1u8.hash(&mut hasher);
            open.hash(&mut hasher);
            close.hash(&mut hasher);
        }
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// Binding key for the editor's heuristic fallback provider.
///
/// The live key space is the tree's version; this one has no tree, so it keys
/// on the buffer revision the closure captured. Tagged so the two can never
/// collide on a buffer that switches arms.
pub(in crate::view) fn file_editor_heuristic_provider_binding_key(
    revision: (u64, u64),
    theme_epoch: u64,
    search_overlay: &[Range<usize>],
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = rustc_hash::FxHasher::default();
    "file-editor-heuristic".hash(&mut hasher);
    revision.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    search_overlay.hash(&mut hasher);
    hasher.finish()
}

/// Paint the matching bracket pair on top of the syntax runs.
///
/// The pair is two single-character ranges, so this splits at most two runs.
/// Mirrors the overlay composition the resolved output uses for its unresolved
/// rows: highlights arrive sorted and disjoint and leave that way.
pub(in crate::view) fn apply_file_editor_bracket_highlights(
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    bracket_match: Option<&(Range<usize>, Range<usize>)>,
    byte_range: Range<usize>,
    style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let Some((open, close)) = bracket_match else {
        return highlights;
    };
    let mut overlays: Vec<Range<usize>> = Vec::with_capacity(2);
    for span in [open, close] {
        let clipped = span.start.max(byte_range.start)..span.end.min(byte_range.end);
        if clipped.start < clipped.end {
            overlays.push(clipped);
        }
    }
    overlays.sort_by_key(|span| span.start);
    apply_file_editor_overlay_highlights(highlights, &overlays, style)
}

/// Wash `overlays` over `highlights`, keeping whatever colour the grammar gave
/// each run and replacing only its background.
///
/// `overlays` must be sorted, disjoint and already clipped to the window the
/// caller is answering for. Both the bracket pair and the search matches go
/// through here, search first, so the bracket affordance stays readable inside a
/// washed match.
pub(in crate::view) fn apply_file_editor_overlay_highlights(
    mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    overlays: &[Range<usize>],
    style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    if overlays.is_empty() {
        return highlights;
    }

    let mut out: Vec<(Range<usize>, gpui::HighlightStyle)> =
        Vec::with_capacity(highlights.len() + overlays.len() * 2);
    let mut overlay_ix = 0usize;
    for (range, run_style) in highlights.drain(..) {
        let mut cursor = range.start;
        while overlay_ix < overlays.len() && overlays[overlay_ix].end <= cursor {
            overlay_ix += 1;
        }
        let mut probe = overlay_ix;
        while cursor < range.end {
            let Some(overlay) = overlays.get(probe).filter(|o| o.start < range.end) else {
                break;
            };
            if overlay.start > cursor {
                out.push((cursor..overlay.start, run_style));
                cursor = overlay.start;
            }
            let end = overlay.end.min(range.end);
            let mut merged = run_style;
            merged.background_color = style.background_color;
            // Only when the overlay asks for one: the bracket wash leaves the
            // grammar's colour alone, while the search wash pins one on light
            // themes, where its background would drown a syntax colour.
            if style.color.is_some() {
                merged.color = style.color;
            }
            out.push((cursor..end, merged));
            cursor = end;
            if overlay.end <= end {
                probe += 1;
            }
        }
        if cursor < range.end {
            out.push((cursor..range.end, run_style));
        }
    }

    // An overlay landing in a stretch the grammar produced no run for (plain
    // punctuation in some grammars, or anything at all in a plain-text buffer)
    // still has to be painted, so add whatever the sweep above did not cover.
    for overlay in overlays {
        let covered = out
            .iter()
            .any(|(range, _)| range.start <= overlay.start && range.end >= overlay.end);
        if !covered {
            out.push((overlay.clone(), style));
        }
    }
    out.sort_by_key(|(range, _)| range.start);
    out
}

/// Wash the search matches that fall inside `byte_range` over the runs the
/// grammar produced.
///
/// `matches` is the whole document's list, so the window is found by binary
/// search — a provider is asked for one viewport at a time.
fn apply_file_editor_search_highlights(
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    matches: &[Range<usize>],
    byte_range: Range<usize>,
    style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    if matches.is_empty() || byte_range.is_empty() {
        return highlights;
    }

    let first = matches.partition_point(|range| range.end <= byte_range.start);
    let mut clipped: Vec<Range<usize>> = Vec::new();
    for range in &matches[first..] {
        if range.start >= byte_range.end {
            break;
        }
        let start = range.start.max(byte_range.start);
        let end = range.end.min(byte_range.end);
        if start < end {
            clipped.push(start..end);
        }
    }

    apply_file_editor_overlay_highlights(highlights, &clipped, style)
}

impl MainPaneView {
    /// Scroll the editor sideways so the caret the search reveal just placed is
    /// on screen.
    ///
    /// A multiline `TextInput` leaves horizontal scrolling to the container it
    /// sits in, and its own caret autoscroll is vertical only, so a match far
    /// along a long line otherwise scrolls into view still off the right edge.
    fn reveal_file_editor_search_match_horizontally(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(range) = self.file_editor_search_current_range() else {
            return;
        };
        let (Some(left), Some(right)) = self.file_editor_input.read_with(cx, |input, _| {
            (
                input.cursor_content_x(range.start),
                input.cursor_content_x(range.end),
            )
        }) else {
            return;
        };
        let scroll = &self.file_editor_scroll;
        let offset = scroll.offset();
        let Some(target_x) = super::helpers::reveal_scroll_x(
            left,
            right,
            scroll.bounds().size.width,
            scroll.max_offset().x,
            offset.x,
        ) else {
            return;
        };
        scroll.set_offset(point(target_x, offset.y));
    }

    /// Whether the pane is currently showing the editable buffer.
    pub(in crate::view) fn is_file_editor_active(&self) -> bool {
        self.active_repo()
            .is_some_and(|repo| repo.diff_state.edit_mode)
            && self.file_editor_path().is_some()
    }

    /// Seat the buffer the search scan reads, and re-scan if it moved.
    ///
    /// Every path that changes the editor's text goes through here, so the match
    /// list can never describe a revision the buffer has left behind. Gating the
    /// re-scan on the revision is also what keeps this off the install-provider →
    /// notify → observe cycle: a rebind moves no text, so the second lap stops.
    fn file_editor_search_source_changed(&mut self, snapshot: TextModelSnapshot) {
        let previous = self
            .file_editor_search_source
            .as_ref()
            .map(|snapshot| (snapshot.model_id(), snapshot.revision()));
        let current = (snapshot.model_id(), snapshot.revision());
        self.file_editor_search_source = Some(snapshot);
        if previous == Some(current) {
            return;
        }
        if self.diff_search_has_query() {
            self.diff_search_recompute_matches_preserving_current();
        }
    }

    /// The matches the highlight provider paints: every hit except the one the
    /// cursor is on.
    ///
    /// That one is left out deliberately — it is marked by the editor's
    /// *selection*, and selection quads are painted before highlight backgrounds,
    /// so a wash over the same span would cover it.
    fn file_editor_search_overlay_ranges(&self) -> Arc<[Range<usize>]> {
        if !self.diff_search_active || self.file_editor_search_matches.is_empty() {
            return Arc::from(Vec::new());
        }
        let current = self.diff_search_match_ix;
        self.file_editor_search_matches
            .iter()
            .enumerate()
            .filter(|(ix, _)| Some(*ix) != current)
            .map(|(_, range)| range.clone())
            .collect()
    }

    /// The working-tree file the editor is (or would be) editing.
    pub(in crate::view) fn file_editor_path(&self) -> Option<PathBuf> {
        let repo = self.active_repo()?;
        if !repo.diff_state.edit_mode {
            return None;
        }
        match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    /// The repo-relative path of the file on screen, if it has one on disk.
    ///
    /// This is what the Edit action needs: the editor always opens the
    /// workspace copy, so a commit's file resolves to the same path in the
    /// working tree.
    pub(in crate::view) fn editable_path_for_current_target(&self) -> Option<PathBuf> {
        let repo = self.active_repo()?;
        match repo.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(path.clone()),
            DiffTarget::Commit {
                path: Some(path), ..
            } => Some(path.clone()),
            _ => None,
        }
    }

    pub(in crate::view) fn file_editor_is_dirty(&self) -> bool {
        self.file_editor_dirty
    }

    pub(in crate::view) fn set_auto_save_file_edits(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.auto_save_file_edits == next {
            return;
        }
        self.auto_save_file_edits = next;
        // Turning it on adopts everything already pending — the buffer on
        // screen *and* the ones stashed behind it — rather than waiting for the
        // next keystroke to notice. Leaving the stash out made the unsaved-edits
        // dialog reachable with auto-save on, which it is documented not to be.
        if next {
            self.save_all_file_edits(cx);
        }
        cx.notify();
    }

    /// Load the file into the buffer when the target changed, or restore the
    /// stashed edit if there is one. Idempotent — called from render.
    pub(in crate::view) fn ensure_file_editor_loaded(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(path) = self.file_editor_path() else {
            return;
        };
        // A clean buffer follows the file: `git checkout`, a discard, or another
        // editor writing it all bump the repo's status revision, and re-reading
        // there is what stops a later save from putting the pre-change text
        // back. A *dirty* buffer is never re-read — that would be the edit loss
        // this whole path exists to avoid.
        let status_rev = self
            .active_repo()
            .map(|repo| repo.status_cache_rev())
            .unwrap_or(0);
        let same_file = self.file_editor_key.as_ref() == Some(&(repo_id, path.clone()));
        // Not while this repo has a command in flight. A save is dispatched, not
        // executed, so between the dispatch and the write the file on disk is
        // still the *old* one — re-reading there seats the pre-save text over
        // the buffer and marks it clean, and the next save writes that revert
        // back over what the first save committed.
        let writes_in_flight = self
            .active_repo()
            .is_some_and(|repo| repo.local_actions_in_flight > 0);
        let disk_may_have_moved = !self.file_editor_dirty
            && !writes_in_flight
            && self.file_editor_loaded_status_rev != status_rev;
        if same_file && !disk_may_have_moved {
            return;
        }
        self.file_editor_loaded_status_rev = status_rev;

        // Leaving one file for another: write it if auto-save is on, otherwise
        // keep the unsaved text so coming back does not silently drop it.
        self.flush_file_editor_buffer(cx);

        self.file_editor_key = Some((repo_id, path.clone()));
        // The buffer is about to be blanked and refilled asynchronously. Until
        // the read lands it holds nothing that belongs to this file, so it must
        // not read as unsaved content for it — carrying the *previous* file's
        // dirty flag across here let a save write the empty placeholder over the
        // file that was just opened.
        self.file_editor_dirty = false;
        self.file_editor_first_dirty_line = None;
        self.file_editor_saved_fingerprint = None;
        // Likewise the search: these offsets belong to the outgoing file and mean
        // nothing in the incoming one. The reload re-seats both.
        self.file_editor_search_source = None;
        self.file_editor_search_clear();
        // Markdown and SVG open rendered by default, and the editor edits the
        // *source* of those files. Flipping the toggle here (rather than in the
        // toolbar handler) covers every way in — the button, the shortcut and
        // the context menu, which dispatches the message directly.
        if let Some(kind) =
            crate::view::diff_target_rendered_preview_kind(self.rendered_diff_target())
        {
            self.rendered_preview_modes
                .set(kind, RenderedPreviewMode::Source);
        }
        self.file_editor_error = None;
        self.file_editor_language = rows::diff_syntax_language_for_path(&path);
        // Only a *different* file invalidates the tree. Tearing it down on a
        // same-file re-read — which a save triggers, via the status bump — threw
        // away the incremental document after every write: the next keystroke
        // paid a full-document parse instead of a `tree.edit()`, and any caret
        // move in between rebound through the no-tree branch and visibly
        // downgraded the file to heuristic highlighting.
        if !same_file {
            self.file_editor_bracket_match = None;
            self.file_editor_live_syntax = None;
            self.file_editor_live_syntax_source = None;
            self.file_editor_live_syntax_building = None;
            self.file_editor_live_syntax_build = None;
            self.file_editor_live_syntax_reparse = None;
            self.file_editor_autosave = None;
        }

        // A stashed buffer that still differs from disk is restored; a clean one
        // is only kept so a failed write leaves the text somewhere, and must not
        // shadow the file — drop it and re-read.
        match self
            .file_editor_stash
            .get(&(repo_id, path.clone()))
            .cloned()
        {
            Some(stashed) if stashed.is_dirty() => {
                // The stash holds only buffers that are *not* on screen, so
                // taking one back hands ownership to the input — leaving the
                // entry behind would report the file as unsaved even after the
                // user undid the edit by hand.
                self.file_editor_stash.remove(&(repo_id, path.clone()));
                self.file_editor_loading = false;
                self.apply_file_editor_text(
                    stashed.text,
                    Some(stashed.cursor),
                    Some(stashed.saved_fingerprint),
                    cx,
                );
                // After `apply_file_editor_text`, which resets it: the buffer
                // being seated is not a fresh read but the one the user left,
                // and its edits are still under the same lines.
                self.file_editor_first_dirty_line = stashed.first_dirty_line;
                return;
            }
            Some(_) => {
                self.file_editor_stash.remove(&(repo_id, path.clone()));
            }
            None => {}
        }

        let Some(absolute) = self.absolute_worktree_path(&path) else {
            self.file_editor_loading = false;
            self.file_editor_error = Some("Repository working directory is unavailable.".into());
            return;
        };

        // A *different* file blanks the buffer and shows the loading state; a
        // re-read of the file already on screen must not touch it. Blanking here
        // made the `unchanged` fast path below unreachable — the buffer it
        // compared against was always empty — so every save reset the caret and
        // cleared the undo stack, once per auto-save.
        if !same_file {
            self.file_editor_loading = true;
            self.file_editor_input.update(cx, |input, cx| {
                input.set_text("", cx);
            });
        }
        let load_key = (repo_id, path.clone());
        cx.spawn(async move |view: WeakEntity<MainPaneView>, cx| {
            let read = {
                let absolute = absolute.clone();
                move || super::preview::read_worktree_file_for_editing(&absolute)
            };
            let result = if crate::ui_runtime::current().uses_background_compute() {
                smol::unblock(read).await
            } else {
                read()
            };
            let _ = view.update(cx, |this, cx| {
                // The user may have moved on while this was in flight — to
                // another file, or to another repo tab holding the same relative
                // path, which is why this compares the whole key and not just
                // the path.
                if this.file_editor_key.as_ref() != Some(&load_key) {
                    return;
                }
                this.file_editor_loading = false;
                match result {
                    Ok(text) => {
                        // A save is followed by a status bump, so this re-read
                        // fires after every write. When the bytes match what the
                        // buffer already holds there is nothing to seat, and
                        // seating it anyway would reset the caret (and the undo
                        // stack) on every auto-save.
                        // Compared as text, not by fingerprint: the buffer's
                        // fingerprint is folded chunk-by-chunk off the rope and
                        // a flat string cannot reproduce it. The whole file was
                        // just read anyway, so one comparison costs nothing new.
                        let unchanged = this
                            .file_editor_input
                            .read_with(cx, |input, _| input.text() == text.as_ref());
                        if unchanged {
                            let fingerprint = this.file_editor_input.read_with(cx, |input, _| {
                                file_editor_text_fingerprint(&input.text_snapshot())
                            });
                            this.file_editor_saved_fingerprint = Some(fingerprint);
                            this.file_editor_dirty = false;
                            this.file_editor_first_dirty_line = None;
                            return;
                        }
                        // The file really did move under a clean buffer. Keep the
                        // caret where it was, clamped, rather than throwing the
                        // user back to the top of the file.
                        let cursor = this
                            .file_editor_input
                            .read_with(cx, |input, _| input.cursor_offset())
                            .min(text.len());
                        this.apply_file_editor_text(text, Some(cursor), None, cx);
                    }
                    Err(message) => {
                        this.file_editor_error = Some(message.into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Seat `text` in the input.
    ///
    /// `baseline` is the fingerprint the text is measured against for
    /// dirtiness: `None` when `text` *is* what was just read off disk, and the
    /// stashed buffer's own recorded baseline when one is being restored.
    fn apply_file_editor_text(
        &mut self,
        text: SharedString,
        cursor: Option<usize>,
        baseline: Option<u64>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Enter must insert what the file already uses. Without this the input
        // keeps its platform default and a CRLF file edited on Linux (or an LF
        // file on Windows) grows mixed endings, showing up as a diff on lines
        // nobody touched. The resolved-output buffer does the same.
        let line_ending = crate::kit::TextInput::detect_line_ending(text.as_ref());
        let snapshot = self.file_editor_input.update(cx, |input, cx| {
            input.set_line_ending(line_ending);
            input.set_text(text, cx);
            if let Some(cursor) = cursor {
                input.set_cursor_offset(cursor, cx);
            }
            // The seeding edit is not an edit the user made; draining keeps it
            // out of the incremental-parse path, which reparses from scratch
            // for a wholesale replacement anyway.
            let _ = input.drain_recent_utf8_edit_deltas();
            input.text_snapshot()
        });
        let fingerprint = file_editor_text_fingerprint(&snapshot);
        let baseline = baseline.unwrap_or(fingerprint);
        self.file_editor_saved_fingerprint = Some(baseline);
        self.file_editor_dirty = baseline != fingerprint;
        // Wholesale replacement: whatever edits the old watermark described are
        // not in this text. A restored stash puts its own back afterwards.
        self.file_editor_first_dirty_line = None;
        self.file_editor_search_source_changed(snapshot.clone());
        self.refresh_file_editor_syntax(&snapshot, None, cx);
        cx.notify();
    }

    /// Absolute path of a repo-relative working-tree path.
    pub(in crate::view) fn absolute_worktree_path(&self, path: &Path) -> Option<PathBuf> {
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let repo = self.active_repo()?;
        Some(repo.spec.workdir.join(path))
    }

    /// Called from the input's observer on every keystroke.
    pub(in crate::view) fn on_file_editor_edited(&mut self, cx: &mut gpui::Context<Self>) {
        if self.file_editor_key.is_none() {
            return;
        }
        let (snapshot, deltas) = self.file_editor_input.update(cx, |input, _| {
            (input.text_snapshot(), input.drain_recent_utf8_edit_deltas())
        });
        let edit = coalesce_resolved_output_edit_deltas(&deltas);
        let text_moved = !deltas.is_empty();
        // Ahead of the `text_moved` early return, and gated on the buffer's
        // revision rather than on the deltas: a wholesale `set_text` (a reload
        // from disk, a restored stash) records none, so the delta test would miss
        // it and leave the match list describing text that is gone.
        self.file_editor_search_source_changed(snapshot.clone());

        // Read before the edit is handed to the parser, which consumes it. The
        // coalesced start is the earliest byte the batch touched, and everything
        // before it is unchanged, so the *post*-edit rope maps it to the same
        // row the pre-edit one would.
        let edited_row = edit
            .as_ref()
            .map(|(replaced, _)| snapshot.rope().offset_to_point(replaced.start).row);

        self.refresh_file_editor_syntax(&snapshot, edit, cx);

        if !text_moved {
            return;
        }
        let dirty =
            self.file_editor_saved_fingerprint != Some(file_editor_text_fingerprint(&snapshot));
        if self.file_editor_dirty != dirty {
            self.file_editor_dirty = dirty;
            cx.notify();
        }
        if dirty {
            // Running minimum: blame below the *first* line the session touched
            // is the part whose attribution has moved, and undoing back to a
            // later edit does not restore the earlier one's line numbering.
            if let Some(row) = edited_row {
                self.file_editor_first_dirty_line = Some(
                    self.file_editor_first_dirty_line
                        .map_or(row, |current| current.min(row)),
                );
            }
        } else {
            // Back to what is on disk — every line is attributable again.
            self.file_editor_first_dirty_line = None;
        }
        if dirty && self.auto_save_file_edits {
            self.schedule_file_editor_autosave(cx);
        }
    }

    /// Restart the quiet-period timer that auto-save writes on.
    fn schedule_file_editor_autosave(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().uses_cursor_blink() {
            // Headless/test runtimes have no timer to wait on; the caller's
            // explicit save path covers them.
            return;
        }
        self.file_editor_autosave = Some(cx.spawn(async move |view: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(
                    FILE_EDITOR_AUTOSAVE_DEBOUNCE_MS,
                ))
                .await;
            let _ = view.update(cx, |this, cx| {
                this.file_editor_autosave = None;
                if this.auto_save_file_edits && this.file_editor_dirty {
                    this.save_file_editor_buffer(cx);
                }
            });
        }));
    }

    /// Write the buffer to the working tree.
    ///
    /// Reuses the same command the merge tool saves through, so the write goes
    /// through the workdir-escape check, lands in the command log, and raises
    /// the same "Saved → path" toast.
    pub(in crate::view) fn save_file_editor_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, path)) = self.file_editor_key.clone() else {
            return;
        };
        // Belt and braces against writing a buffer that is not the file's: while
        // a read is in flight the input holds a blank placeholder, and every
        // save entry point (the button, Ctrl+S, auto-save, Save all) can be
        // reached in that window.
        if self.file_editor_loading || !self.file_editor_dirty {
            return;
        }
        let snapshot = self
            .file_editor_input
            .read_with(cx, |input, _| input.text_snapshot());
        // One materialization: the dispatch needs an owned `String`, the stash a
        // `SharedString`, and building the shared one first lets the stash take a
        // handle instead of a second whole-file copy — which auto-save was
        // paying for every 800 ms of typing.
        let contents = SharedString::from(snapshot.as_str().to_string());
        let fingerprint = file_editor_text_fingerprint(&snapshot);

        let cursor = self
            .file_editor_input
            .read_with(cx, |input, _| input.cursor_offset());

        self.file_editor_autosave = None;
        self.store.dispatch(Msg::SaveWorktreeFile {
            repo_id,
            path: path.clone(),
            contents: contents.to_string(),
            stage: false,
        });
        // Optimistic, like every other command in the app: the write is what the
        // user asked for, and a failure raises its own error toast. Holding the
        // buffer dirty until the command landed would make the indicator flicker
        // on every auto-save.
        //
        // The stash entry is *updated* rather than removed, so a write that does
        // fail leaves the text somewhere the user can get back to instead of
        // having it dropped by the next navigation. It records the fingerprint
        // just written, so it reads as clean and is evicted when the file is
        // next opened.
        self.file_editor_saved_fingerprint = Some(fingerprint);
        self.file_editor_dirty = false;
        self.file_editor_first_dirty_line = None;
        // Only the newest clean entry is worth keeping — it exists solely so a
        // write that fails leaves the text somewhere recoverable. Without this
        // every file saved in a session held a full copy of its contents alive
        // for the lifetime of the window.
        self.file_editor_stash
            .retain(|_, stashed| stashed.is_dirty());
        self.file_editor_stash.insert(
            (repo_id, path.clone()),
            StashedFileEdit {
                text: contents,
                cursor,
                text_fingerprint: fingerprint,
                saved_fingerprint: fingerprint,
                // A recovery copy of text that is now on disk: clean, so nothing
                // below it is misattributed.
                first_dirty_line: None,
            },
        );
        // The read-only preview of the same path is now behind the file on
        // disk, and it is only invalidated when the *target* changes.
        self.invalidate_worktree_preview_for_saved_path(&path);
        cx.notify();
    }

    /// Keep an unsaved buffer around under its path.
    pub(in crate::view) fn stash_current_file_editor_buffer(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(key) = self.file_editor_key.clone() else {
            return;
        };
        if !self.file_editor_dirty {
            // A clean buffer is the truth for this file now — the user may have
            // undone their way back to disk. Drop any *dirty* entry left from an
            // earlier flush, or returning here would restore edits they threw
            // away. A clean entry is a save's recovery copy and stays.
            if self
                .file_editor_stash
                .get(&key)
                .is_some_and(StashedFileEdit::is_dirty)
            {
                self.file_editor_stash.remove(&key);
            }
            return;
        }
        let (text, cursor, text_fingerprint) = self.file_editor_input.read_with(cx, |input, _| {
            let snapshot = input.text_snapshot();
            (
                SharedString::from(input.text().to_string()),
                input.cursor_offset(),
                file_editor_text_fingerprint(&snapshot),
            )
        });
        let saved_fingerprint = self.file_editor_saved_fingerprint.unwrap_or_default();
        self.file_editor_stash.insert(
            key,
            StashedFileEdit {
                text,
                cursor,
                text_fingerprint,
                saved_fingerprint,
                first_dirty_line: self.file_editor_first_dirty_line,
            },
        );
    }

    /// Flush an unsaved buffer if auto-save is on, otherwise stash it.
    ///
    /// The two moments this covers are leaving the editor and losing focus,
    /// which is where "auto-save" has to mean more than "after a pause" — a
    /// pause that is interrupted by navigating away would otherwise lose the
    /// write it was about to make.
    pub(in crate::view) fn flush_file_editor_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        // `file_editor_loading` for the same reason `save_file_editor_buffer`
        // checks it: between blanking the buffer and the read landing, the
        // buffer holds a placeholder that reads as *dirty* — the saved
        // fingerprint was cleared just before the blanking, so the empty text
        // does not match it. Flushing there stashed that empty placeholder under
        // the file's own path, and the next open restored it over the file.
        if self.file_editor_loading || self.file_editor_key.is_none() || !self.file_editor_dirty {
            return;
        }
        if self.auto_save_file_edits {
            self.save_file_editor_buffer(cx);
        } else {
            self.stash_current_file_editor_buffer(cx);
        }
    }

    /// Drop stashed buffers whose repo tab has been closed.
    ///
    /// Their `RepoId` no longer resolves, so the store drops any save dispatched
    /// for them. Left in place they are unsavable *and* unclearable: the quit
    /// dialog lists them, Save all cannot write them, and the retry raises the
    /// dialog again — a window that can never be closed.
    ///
    /// Closing a repo tab therefore discards that repo's unsaved editor buffers
    /// without asking, which is a gap: the tab close should prompt the way the
    /// window close does.
    pub(in crate::view) fn prune_orphaned_file_editor_stash(&mut self) {
        // One definition of "still open", used by both the stash sweep and the
        // on-screen buffer below: two copies would let a future change to what
        // counts as open fix one and re-create the unclosable-window bug in the
        // other.
        let repos = self.state.repos.clone();
        let repo_exists = move |repo_id: RepoId| repos.iter().any(|repo| repo.id == repo_id);

        if !self.file_editor_stash.is_empty() {
            self.file_editor_stash
                .retain(|(repo_id, _), _| repo_exists(*repo_id));
        }

        // The buffer on screen has the same problem: its key still names the
        // closed repo, so a dirty flag left standing keeps reporting an unsaved
        // edit that nothing can write.
        if self
            .file_editor_key
            .as_ref()
            .is_some_and(|(repo_id, _)| !repo_exists(*repo_id))
        {
            self.file_editor_key = None;
            self.file_editor_dirty = false;
            self.file_editor_first_dirty_line = None;
            self.file_editor_saved_fingerprint = None;
            self.file_editor_loading = false;
            self.file_editor_error = None;
            self.file_editor_autosave = None;
            self.file_editor_live_syntax = None;
            self.file_editor_live_syntax_source = None;
            self.file_editor_live_syntax_building = None;
            self.file_editor_live_syntax_build = None;
            self.file_editor_live_syntax_reparse = None;
        }
    }

    /// Every `(repo, path)` with edits that are not on disk: the buffer on
    /// screen plus every buffer stashed behind it.
    ///
    /// Sorted and de-duplicated, so everything reading this — the quit dialog,
    /// the file explorer's unsaved section — agrees on both membership and order
    /// frame to frame.
    pub(in crate::view) fn unsaved_file_edit_keys(&self) -> Vec<(RepoId, PathBuf)> {
        // De-duplicated on the whole key, not on the path: two repo tabs each
        // holding an unsaved `README.md` are two files, and collapsing them
        // would under-report what the dialog is about to discard.
        let mut keys: Vec<(RepoId, PathBuf)> = self
            .file_editor_stash
            .iter()
            .filter(|(_, stashed)| stashed.is_dirty())
            .map(|(key, _)| key.clone())
            .collect();
        if self.file_editor_dirty
            && !self.file_editor_loading
            && let Some(key) = self.file_editor_key.as_ref()
            && !keys.contains(key)
        {
            keys.push(key.clone());
        }
        // `RepoId` is an opaque handle with no ordering; sorting on the path
        // and then the raw id keeps the list stable across frames.
        keys.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.0.cmp(&right.0.0)));
        keys
    }

    /// The unsaved paths belonging to one repo, in the same order.
    ///
    /// The file explorer shows a single repo's tree, so a path from another repo
    /// tab has nowhere to sit in it; those stay the quit dialog's business,
    /// where they are qualified by repo name.
    pub(in crate::view) fn unsaved_file_edit_paths(&self, repo_id: RepoId) -> Vec<PathBuf> {
        self.unsaved_file_edit_keys()
            .into_iter()
            .filter(|(id, _)| *id == repo_id)
            .map(|(_, path)| path)
            .collect()
    }

    /// Whether `path` in `repo_id` has edits that are not on disk.
    pub(in crate::view) fn file_edits_are_unsaved_for(&self, repo_id: RepoId, path: &Path) -> bool {
        if self
            .file_editor_stash
            .get(&(repo_id, path.to_path_buf()))
            .is_some_and(StashedFileEdit::is_dirty)
        {
            return true;
        }
        self.file_editor_dirty
            && !self.file_editor_loading
            && self
                .file_editor_key
                .as_ref()
                .is_some_and(|(id, editing)| *id == repo_id && editing.as_path() == path)
    }

    /// Throw away the unsaved edits for one file, wherever they are being held.
    ///
    /// The on-screen buffer and the stash are two different places, and a file
    /// can be in either; the explorer offers this for both without knowing which.
    pub(in crate::view) fn discard_file_edits_for(
        &mut self,
        repo_id: RepoId,
        path: &Path,
        cx: &mut gpui::Context<Self>,
    ) {
        let key = (repo_id, path.to_path_buf());
        if self.file_editor_key.as_ref() == Some(&key) {
            // On screen: re-reads from disk, which is what puts the text back.
            self.discard_file_editor_buffer(cx);
            return;
        }
        if self.file_editor_stash.remove(&key).is_some() {
            // Ahead of the next frame's sync so the row disappears on the click
            // that asked for it.
            self.sync_unsaved_file_edits_rev(cx);
            cx.notify();
        }
    }

    /// Recompute [`Self::unsaved_file_edits_rev`] and repaint the explorer when
    /// it moved.
    ///
    /// Derived once per frame rather than bumped at each mutation. The set
    /// changes from a dozen places — load, keystroke, undo back to clean, save,
    /// auto-save, stash, flush, discard, discard-all, orphan prune — and a
    /// missed bump is an explorer that keeps showing a pen on a file the user
    /// already saved, with nothing to point at. Hashing the keys is trivial on a
    /// set that is empty in the overwhelming majority of frames.
    pub(in crate::view) fn sync_unsaved_file_edits_rev(&mut self, cx: &mut gpui::Context<Self>) {
        use std::hash::{Hash, Hasher};

        let mut hasher = rustc_hash::FxHasher::default();
        for (repo_id, path) in self.unsaved_file_edit_keys() {
            repo_id.0.hash(&mut hasher);
            path.hash(&mut hasher);
        }
        let next = hasher.finish();
        if self.unsaved_file_edits_rev == next {
            return;
        }
        self.unsaved_file_edits_rev = next;
        // Deferred: the set also changes from handlers the root view is already
        // inside — the explorer's own discard button reaches this pane through
        // `root_view.update`, and updating an entity from within its own update
        // panics. Deferring runs the notify once that borrow is gone, on the
        // same flush, so the explorer still repaints before the next frame.
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.notify_unsaved_file_edits_changed(cx);
            });
        });
    }

    /// Every path with edits that are not on disk, labelled for display.
    ///
    /// Sorted and de-duplicated so the confirmation dialog lists each file once
    /// and in a stable order.
    pub(in crate::view) fn unsaved_file_edit_labels(&self) -> Vec<SharedString> {
        let keys = self.unsaved_file_edit_keys();

        // With two repo tabs open, "README.md" twice tells the user nothing
        // about what is about to be discarded — qualify by repo, but only when
        // there is more than one to tell apart.
        let spans_repos = keys.windows(2).any(|pair| pair[0].0 != pair[1].0);
        keys.into_iter()
            .map(|(repo_id, path)| {
                let repo_name = spans_repos
                    .then(|| {
                        self.state
                            .repos
                            .iter()
                            .find(|repo| repo.id == repo_id)
                            .and_then(|repo| repo.spec.workdir.file_name())
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .flatten();
                match repo_name {
                    Some(repo_name) => {
                        SharedString::from(format!("{repo_name} — {}", path.display()))
                    }
                    None => SharedString::from(path.display().to_string()),
                }
            })
            .collect()
    }

    /// Write every unsaved buffer, on screen or stashed.
    pub(in crate::view) fn save_all_file_edits(&mut self, cx: &mut gpui::Context<Self>) {
        // `mem::take` empties the map, so the clean recovery entry a previous
        // save left behind would be dropped along with the dirty ones. Put it
        // back afterwards — it is the only copy of text whose write may not have
        // landed, which is the whole reason saving keeps one.
        let current = self.file_editor_key.clone();
        let mut clean: Vec<((RepoId, PathBuf), StashedFileEdit)> = Vec::new();
        for ((repo_id, path), stashed) in std::mem::take(&mut self.file_editor_stash) {
            // The buffer on screen is saved below, from the live text rather
            // than from whatever was stashed for it earlier.
            if current.as_ref() == Some(&(repo_id, path.clone())) {
                continue;
            }
            if !stashed.is_dirty() {
                clean.push(((repo_id, path), stashed));
                continue;
            }
            // Entries for a closed repo tab are pruned when the tab goes (see
            // `prune_orphaned_file_editor_stash`), so anything still here has a
            // repo to save into. Asserting it rather than skipping is deliberate:
            // silently keeping an unsavable *dirty* entry made the quit dialog
            // reappear for ever — Save all could not clear it, so the retry
            // raised it again and the app could not be closed at all.
            debug_assert!(
                self.state.repos.iter().any(|repo| repo.id == repo_id),
                "stash entries for closed repos must be pruned, not carried"
            );
            self.store.dispatch(Msg::SaveWorktreeFile {
                repo_id,
                path: path.clone(),
                contents: stashed.text.to_string(),
                stage: false,
            });
            // Held as a recovery copy under the fingerprint just written, the
            // same bargain `save_file_editor_buffer` makes for the on-screen
            // buffer: if the command fails the text is still somewhere.
            let saved_fingerprint = stashed.text_fingerprint;
            clean.push((
                (repo_id, path),
                StashedFileEdit {
                    saved_fingerprint,
                    ..stashed
                },
            ));
        }
        // Saved *before* the recovery copies go back: `save_file_editor_buffer`
        // prunes clean entries to keep the stash bounded, and running it after
        // would delete the very copies this just made.
        self.save_file_editor_buffer(cx);
        self.file_editor_stash.extend(clean);
    }

    /// Throw away every unsaved buffer.
    pub(in crate::view) fn discard_all_file_edits(&mut self, cx: &mut gpui::Context<Self>) {
        self.file_editor_stash.clear();
        if self.file_editor_dirty {
            self.discard_file_editor_buffer(cx);
        }
    }

    /// Drop an unsaved buffer, reloading the file from disk.
    pub(in crate::view) fn discard_file_editor_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(key) = self.file_editor_key.clone() else {
            return;
        };
        self.file_editor_stash.remove(&key);
        self.file_editor_dirty = false;
        self.file_editor_first_dirty_line = None;
        self.file_editor_saved_fingerprint = None;
        // Forget which file is loaded so the next `ensure` re-reads it.
        self.file_editor_key = None;
        self.ensure_file_editor_loaded(cx);
    }

    /// Bring the live tree up to date with `snapshot` and rebind the provider.
    ///
    /// `edit` is the coalesced `(replaced, inserted)` span, or `None` when the
    /// text was replaced wholesale — which reparses from scratch.
    pub(in crate::view) fn refresh_file_editor_syntax(
        &mut self,
        snapshot: &TextModelSnapshot,
        edit: Option<(Range<usize>, Range<usize>)>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Nothing here belongs to the file yet. Opening a *different* file
        // blanks the buffer before the read lands, and that `set_text("")` is an
        // edit like any other as far as the pane's input observer is concerned —
        // so this used to run over the empty text and build a document whose
        // tree spans nothing. The file then arrived as a wholesale replacement,
        // and a budgeted parse that missed left that empty tree paired with the
        // full rope. `apply_file_editor_text` calls this itself once the real
        // text is seated, which is the only moment worth parsing.
        if self.file_editor_loading {
            return;
        }
        let revision = (snapshot.model_id(), snapshot.revision());
        let language = self.file_editor_language;
        let text_is_unchanged = self.file_editor_live_syntax_source == Some(revision);
        let language_is_unchanged = self
            .file_editor_live_syntax
            .as_ref()
            .is_some_and(|document| Some(document.language()) == language);

        if edit.is_none() && text_is_unchanged && language_is_unchanged {
            self.rebind_file_editor_highlight_provider(cx);
            return;
        }

        // Everything below reads through the rope; the whole document is never
        // flattened on the keystroke path.
        let rope = snapshot.rope();
        let budget = Some(self.full_document_syntax_budget().foreground_parse);
        let reusable = self
            .file_editor_live_syntax
            .as_ref()
            .is_some_and(|document| Some(document.language()) == language);
        if !reusable {
            self.file_editor_live_syntax = None;
            self.file_editor_live_syntax_source = None;
        }

        match self.file_editor_live_syntax.as_mut() {
            Some(_) if text_is_unchanged && edit.is_none() => {}
            Some(document) => {
                let outcome = document.sync(rope.clone(), Arc::default(), edit, budget);
                if outcome == rows::LiveSyntaxSyncOutcome::Abandoned {
                    self.file_editor_live_syntax = None;
                    self.file_editor_live_syntax_source = None;
                } else {
                    self.file_editor_live_syntax_source = Some(revision);
                }
            }
            None => {
                // Worth one budgeted attempt: a small file finishes inside it
                // and never shows a frame of unhighlighted text. Skipped when a
                // build for exactly this text is already off-thread — that
                // attempt has demonstrably failed once.
                // The two permanent reasons a build can fail — no wired grammar,
                // and text past the parse ceiling — are asked directly rather
                // than inferred from a failed attempt. Anything else that fails
                // is transient and must stay retryable, which is what a latch on
                // "the build returned None" got wrong: one unlucky parse left
                // the file unhighlighted for the rest of the session.
                let supported = language.is_some_and(|language| {
                    rows::live_syntax_document_supported(language, rope.len())
                });
                let already_building = self.file_editor_live_syntax_building == Some(revision);
                self.file_editor_live_syntax = language
                    .filter(|_| supported && !already_building)
                    .and_then(|language| {
                        rows::LiveSyntaxDocument::new(
                            language,
                            rope.clone(),
                            Arc::default(),
                            budget,
                        )
                    });
                self.file_editor_live_syntax_source =
                    self.file_editor_live_syntax.is_some().then_some(revision);

                // A first parse has no tree to fall back on, so a blown budget
                // leaves nothing at all and no incremental reparse can rescue
                // it. Finish it off-thread.
                if let Some(language) =
                    language.filter(|_| supported && self.file_editor_live_syntax.is_none())
                {
                    self.ensure_file_editor_live_syntax_build(language, rope, revision, cx);
                }
            }
        }

        self.ensure_file_editor_live_syntax_reparse(cx);
        self.rebind_file_editor_highlight_provider(cx);
    }

    fn ensure_file_editor_live_syntax_build(
        &mut self,
        language: rows::DiffSyntaxLanguage,
        rope: Rope,
        revision: (u64, u64),
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_editor_live_syntax_building == Some(revision) {
            return;
        }
        self.file_editor_live_syntax_building = Some(revision);
        self.file_editor_live_syntax_build =
            Some(cx.spawn(async move |view: WeakEntity<MainPaneView>, cx| {
                let build =
                    move || rows::LiveSyntaxDocument::new(language, rope, Arc::default(), None);
                let built = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(build).await
                } else {
                    build()
                };
                let _ = view.update(cx, |this, cx| {
                    if this.file_editor_live_syntax_building != Some(revision) {
                        return;
                    }
                    this.file_editor_live_syntax_building = None;
                    let Some(document) = built else {
                        // The caller already established that this file *can* be
                        // parsed, so reaching here is a transient failure. Leave
                        // everything retryable and let the next refresh try
                        // again rather than latching the file as unhighlightable.
                        return;
                    };
                    let snapshot = this
                        .file_editor_input
                        .read_with(cx, |input, _| input.text_snapshot());
                    if (snapshot.model_id(), snapshot.revision()) != revision {
                        // Zed's `parse_again`: a tree for text the buffer has
                        // moved past is useless, but so is waiting.
                        this.refresh_file_editor_syntax(&snapshot, None, cx);
                        return;
                    }
                    this.file_editor_live_syntax = Some(document);
                    this.file_editor_live_syntax_source = Some(revision);
                    this.rebind_file_editor_highlight_provider(cx);
                });
            }));
    }

    fn ensure_file_editor_live_syntax_reparse(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(request) = self
            .file_editor_live_syntax
            .as_ref()
            .and_then(rows::LiveSyntaxDocument::background_reparse_request)
        else {
            self.file_editor_live_syntax_reparse = None;
            return;
        };

        self.file_editor_live_syntax_reparse =
            Some(cx.spawn(async move |view: WeakEntity<MainPaneView>, cx| {
                let reparse = move || rows::live_syntax_reparse(request);
                let parsed = if crate::ui_runtime::current().uses_background_compute() {
                    smol::unblock(reparse).await
                } else {
                    reparse()
                };
                let Some((version, tree, injections)) = parsed else {
                    return;
                };
                let _ = view.update(cx, |this, cx| {
                    let adopted = this
                        .file_editor_live_syntax
                        .as_mut()
                        .is_some_and(|document| {
                            document.adopt_background_tree(version, tree, injections)
                        });
                    this.file_editor_live_syntax_reparse = None;
                    if !adopted {
                        // The buffer moved while this was in flight, so the tree
                        // describes text that no longer exists.
                        this.ensure_file_editor_live_syntax_reparse(cx);
                        return;
                    }
                    this.rebind_file_editor_highlight_provider(cx);
                });
            }));
    }

    /// Hand the input a provider over the document's current tree, with the
    /// caret's bracket pair painted on top.
    pub(in crate::view) fn rebind_file_editor_highlight_provider(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let live = self
            .file_editor_live_syntax
            .as_ref()
            .map(|document| (document.version(), document.snapshot(self.theme)));

        let (cursor, snapshot, has_selection) = self.file_editor_input.read_with(cx, |input, _| {
            let range = input.selected_range();
            (
                input.cursor_offset(),
                input.text_snapshot(),
                range.start != range.end,
            )
        });
        let source_len = snapshot.len();
        let search_overlay = self.file_editor_search_overlay_ranges();
        let (search_bg, search_fg) = rows::query_highlight_colors(self.theme);
        let search_style = gpui::HighlightStyle {
            color: search_fg.map(IntoColor::into_color),
            background_color: Some(search_bg.into_color()),
            ..Default::default()
        };

        let Some((version, snapshot)) = live else {
            // No wired grammar, or past the parse ceiling. Same fallback the
            // resolved output takes: line-local heuristic tokens, answered per
            // window so the cost stays proportional to the viewport. Bracket
            // matching needs a tree, so it stays off here.
            self.file_editor_bracket_match = None;
            let theme = self.theme;
            let language = self.file_editor_language;
            let rope = snapshot.rope();
            let binding_key = file_editor_heuristic_provider_binding_key(
                (snapshot.model_id(), snapshot.revision()),
                self.file_editor_provider_theme_epoch,
                &search_overlay,
            );
            let provider = HighlightProvider::with_pending(
                move |byte_range: Range<usize>| HighlightProviderResult {
                    highlights: apply_file_editor_search_highlights(
                        language
                            .map(|language| {
                                resolved_output_heuristic_highlights_for_range(
                                    theme,
                                    &rope,
                                    language,
                                    byte_range.clone(),
                                )
                            })
                            .unwrap_or_default(),
                        &search_overlay,
                        byte_range,
                        search_style,
                    ),
                    pending: false,
                },
                || 0,
                || false,
            );
            self.file_editor_input.update(cx, |input, cx| {
                input.set_highlight_provider_with_key(binding_key, provider, source_len, cx);
            });
            return;
        };
        // A selection is the user working on a span, not sitting in one; a pair
        // lit at each end of it reads as part of the selection.
        self.file_editor_bracket_match = (!has_selection)
            .then(|| snapshot.bracket_pair_at(cursor))
            .flatten();

        let bracket_match = self.file_editor_bracket_match.clone();
        let binding_key = file_editor_provider_binding_key(
            version,
            self.file_editor_provider_theme_epoch,
            bracket_match.as_ref(),
            &search_overlay,
        );
        let bracket_style = gpui::HighlightStyle {
            background_color: Some(
                self.theme
                    .colors
                    .editor
                    .bracket_match_background
                    .into_color(),
            ),
            ..Default::default()
        };
        let provider = HighlightProvider::with_pending(
            move |byte_range: Range<usize>| HighlightProviderResult {
                highlights: apply_file_editor_bracket_highlights(
                    apply_file_editor_search_highlights(
                        snapshot.highlights_for_byte_range(byte_range.clone()),
                        &search_overlay,
                        byte_range.clone(),
                        search_style,
                    ),
                    bracket_match.as_ref(),
                    byte_range,
                    bracket_style,
                ),
                pending: false,
            },
            || 0,
            || false,
        );
        self.file_editor_input.update(cx, |input, cx| {
            input.set_highlight_provider_with_key(binding_key, provider, source_len, cx);
        });
    }

    /// The editable body: a line-number gutter beside the buffer.
    ///
    /// Same arrangement as the resolved output — the input lays out at full
    /// content size inside an `overflow_scroll` container and reads that
    /// container's handle to window its shaping — minus the marker lane and the
    /// four-way scroll-sync group, which have no counterpart here.
    pub(in crate::view) fn render_file_editor(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        if let Some(message) = self.file_editor_error.clone() {
            return components::empty_state(theme, "Edit", message).into_any_element();
        }
        if self.file_editor_loading {
            return components::empty_state(theme, "Edit", "Loading").into_any_element();
        }

        // The halves of a search reveal that need a `cx`. The scan and the match
        // walk have none, so they bump a rev and leave these here. Guarded on the
        // rev rather than run every frame: rebinding builds a fresh closure.
        if self.file_editor_search_applied_rev != self.file_editor_search_rev {
            self.file_editor_search_applied_rev = self.file_editor_search_rev;
            self.rebind_file_editor_highlight_provider(cx);
        }
        if self.file_editor_search_reveal_applied_rev != self.file_editor_search_reveal_rev {
            self.file_editor_search_reveal_applied_rev = self.file_editor_search_reveal_rev;
            if let Some(range) = self.file_editor_search_current_range() {
                // Its autoscroll is also what covers a reveal computed before the
                // scroll handle had been laid out and had bounds to centre against.
                self.file_editor_input
                    .update(cx, |input, cx| input.set_selected_range(range, true, cx));
                // The caret has moved but not been laid out at its new place yet,
                // so the sideways half waits for the frame that paints it. The
                // input's own caret autoscroll only handles the vertical axis.
                self.file_editor_search_reveal_x_pending = true;
            }
        } else if self.file_editor_search_reveal_x_pending {
            self.file_editor_search_reveal_x_pending = false;
            self.reveal_file_editor_search_match_horizontally(cx);
        }

        // Word wrap is the same preference the diff and preview honour, applied
        // to the buffer here. It and content-width layout are exclusive: with
        // wrap on the input must be bounded by the viewport so it has a width to
        // wrap against; with it off the input lays out at its content width so
        // the surrounding container can scroll horizontally.
        let soft_wrap = self.diff_word_wrap;
        self.file_editor_input.update(cx, |input, cx| {
            input.set_soft_wrap(soft_wrap, cx);
            input.set_content_width_layout(!soft_wrap);
        });

        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let line_count = self
            .file_editor_input
            .read_with(cx, |input, _| input.text_snapshot().line_count())
            .max(1);
        let show_line_numbers = self.diff_show_line_numbers;

        // The input's line height is UI-scaled (`MainPaneView::new`), so the
        // gutter's rows and padding have to be too — a flat 20px row drifts a
        // whole line out of step every third line at 150%.
        let ui_scale_percent = ui_scale::current(cx).percent;
        let row_height =
            ui_scale::design_px_from_percent(RESOLVED_OUTPUT_ROW_HEIGHT_PX, ui_scale_percent);
        // Blame rides in the same gutter as the line numbers rather than in a
        // column of its own, so the editor keeps one scroll-synced strip.
        let blame_ctx = self.blame_render_ctx();
        let show_blame = blame_ctx.is_some();
        let blame_width = if show_blame {
            self.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        self.file_editor_blame = blame_ctx;
        self.file_editor_blame_width = blame_width;
        let gutter_width =
            file_editor_gutter_width(line_count, show_line_numbers, ui_scale_percent) + blame_width;
        self.file_editor_gutter_row_height = row_height;

        // A wrapped line owns several rows in the buffer and must own the same
        // several in the gutter, with its number on the first — the shape the
        // read-only text view already uses. The projection is a prefix sum over
        // the buffer's own per-line row counts, so the two cannot drift; it is
        // rebuilt each frame into a retained buffer rather than cached, because
        // the element rebuilds its y-offsets from the same array just as often.
        let gutter_rows = self.rebuild_file_editor_wrap_row_starts(line_count, cx);

        let annotate_handle = (blame_width > px(0.0))
            .then(|| self.annotate_resize_handle(ui_scale_percent, theme, cx));

        let gutter = uniform_list(
            "file_editor_gutter_list",
            gutter_rows,
            cx.processor(Self::render_file_editor_gutter_rows),
        )
        .h_full()
        .min_h(px(0.0))
        .track_scroll(&self.file_editor_gutter_scroll);

        let editor_scroll = self.file_editor_scroll.clone();
        let gutter_scroll = self.file_editor_gutter_scroll.clone();
        let scrollbar_gutter = components::Scrollbar::visible_gutter(
            editor_scroll.clone(),
            components::ScrollbarAxis::Vertical,
        );
        let editor_scrollbar =
            components::Scrollbar::new("file_editor_scrollbar", editor_scroll.clone());
        #[cfg(test)]
        let editor_scrollbar = editor_scrollbar.debug_selector("file_editor_scrollbar");

        // Copy the editor's offset into the gutter *before* the list lays out,
        // not only after its children prepaint. A wheel event updates the
        // editor's handle before this render runs, so reading it here puts the
        // gutter on the same offset in the same frame; the prepaint mirror below
        // is left as the catch-up for offsets the layout itself moves (a caret
        // autoscroll), which by definition are not known yet.
        {
            let target_y = editor_scroll.offset().y;
            let base = gutter_scroll.0.borrow().base_handle.clone();
            if base.offset().y != target_y {
                base.set_offset(point(px(0.0), target_y));
            }
        }

        div()
            // The gutter is virtualized, so it cannot simply share the editor's
            // scroll container. Copy the editor's offset into it after layout
            // instead: the editor is always the master, which is why there is no
            // reverse sync and no last-synced bookkeeping.
            .on_children_prepainted({
                let editor_scroll = editor_scroll.clone();
                let gutter_scroll = gutter_scroll.clone();
                move |_bounds, window, _cx| {
                    let target_y = editor_scroll.offset().y;
                    let base = gutter_scroll.0.borrow().base_handle.clone();
                    if base.offset().y != target_y {
                        base.set_offset(point(px(0.0), target_y));
                        window.refresh();
                    }
                }
            })
            .id("file_editor")
            .debug_selector(|| "file_editor".to_string())
            .relative()
            .flex()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .bg(theme.colors.editor.background)
            .font_family(editor_font_family)
            .when(show_line_numbers || show_blame, |row| {
                row.child(
                    div()
                        .id("file_editor_gutter")
                        .w(gutter_width)
                        .h_full()
                        .min_h(px(0.0))
                        .flex_shrink_0()
                        .bg(theme.colors.editor.gutter_background)
                        .border_r_1()
                        .border_color(theme.colors.editor.indent_guide)
                        // A wheel over the numbers has to move the code, not
                        // scroll the gutter out of step with it.
                        .on_scroll_wheel({
                            let editor_scroll = editor_scroll.clone();
                            move |event, window, cx| {
                                let delta = event.delta.pixel_delta(window.line_height());
                                let offset = editor_scroll.offset();
                                let max_y = editor_scroll.max_offset().y.max(px(0.0));
                                let next_y = (offset.y + delta.y).clamp(-max_y, px(0.0));
                                if next_y != offset.y {
                                    editor_scroll.set_offset(point(offset.x, next_y));
                                    window.refresh();
                                }
                                cx.stop_propagation();
                            }
                        })
                        .child(gutter),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .id("file_editor_scroll")
                            .debug_selector(|| "file_editor_scroll".to_string())
                            // flex-col so a content-width input overflows to the
                            // right instead of being shrunk to the viewport,
                            // which gives the container a horizontal range.
                            .flex()
                            .flex_col()
                            .when(soft_wrap, |d| d.items_stretch())
                            .when(!soft_wrap, |d| d.items_start())
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .min_h(px(0.0))
                            .pl_2()
                            .pr(scrollbar_gutter)
                            .when(soft_wrap, |d| d.overflow_y_scroll())
                            .when(!soft_wrap, |d| d.overflow_scroll())
                            .track_scroll(&self.file_editor_scroll)
                            .child(self.file_editor_input.clone()),
                    )
                    // The track must be outside the moving scroll surface or
                    // GPUI applies the content offset to the scrollbar itself.
                    .child(editor_scrollbar.render(theme)),
            )
            .when_some(annotate_handle, |row, handle| row.child(handle))
            .into_any_element()
    }

    fn render_file_editor_gutter_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let row_height = this.file_editor_gutter_row_height;
        let show_line_numbers = this.diff_show_line_numbers;
        let blame_width = this.file_editor_blame_width;
        let blame_ctx = this.file_editor_blame.clone();
        let ui_scale_percent = ui_scale::current(cx).percent;
        let first_dirty_line = this.file_editor_first_dirty_line;
        let annot_hover = this.blame_annot_hover;
        // How many lines the unsaved buffer has gained (or lost) against the
        // revision blame was computed for. Below the first edit, this is what
        // puts each row back on the line it came from.
        let blame_line_delta = blame_ctx.as_ref().map_or(0i64, |ctx| {
            let buffer_lines = this.file_editor_input.read(cx).text_snapshot().line_count() as i64;
            buffer_lines - ctx.line_count() as i64
        });
        let entity = cx.entity();

        let wrap_row_starts = std::mem::take(&mut this.file_editor_wrap_row_starts);

        let elements = range
            .map(|visual_ix| {
                // Without wrap the two spaces are the same; with it, the first
                // visual row of a line carries its number and blame, and the
                // continuations carry neither — which is what makes the block
                // read as one stretched row.
                let (ix, is_continuation) =
                    file_editor_line_for_visual_row(&wrap_row_starts, visual_ix);

                let blame_line =
                    file_editor_blame_line_for_editor_line(ix, first_dirty_line, blame_line_delta);
                let blame = blame_ctx
                    .as_ref()
                    .filter(|_| !is_continuation)
                    .and_then(|ctx| {
                        rows::build_row_blame_paint(
                            ctx,
                            false,
                            None,
                            blame_line,
                            // The previous *rendered* line, mapped the same way,
                            // so run starts are computed in the blamed
                            // revision's line space rather than the buffer's.
                            ix.checked_sub(1).and_then(|prev| {
                                file_editor_blame_line_for_editor_line(
                                    prev,
                                    first_dirty_line,
                                    blame_line_delta,
                                )
                            }),
                            theme,
                        )
                    });

                div()
                    .id(("file_editor_gutter_row", visual_ix))
                    .h(row_height)
                    .flex()
                    .flex_row()
                    .items_center()
                    // No text size here: a canvas child *does* inherit an
                    // ancestor div's text style (`Style::paint` never pushes one
                    // of its own), so `text_xs` on the row shaped the blame
                    // column a quarter smaller than the diff and preview columns,
                    // against sub-column widths sized for the larger font. The
                    // line-number cell sets its own below.
                    .when(blame_width > px(0.0), |row| {
                        row.child(rows::blame_gutter_row_canvas(
                            theme,
                            entity.clone(),
                            ui_scale_percent,
                            visual_ix,
                            row_height,
                            blame_width,
                            annot_hover,
                            blame,
                        ))
                    })
                    .when(show_line_numbers, |row| {
                        row.child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .px_2()
                                .flex()
                                .justify_end()
                                .text_xs()
                                .text_color(theme.colors.editor.line_number)
                                .child(if is_continuation {
                                    String::new()
                                } else {
                                    format!("{}", ix + 1)
                                }),
                        )
                    })
                    .into_any_element()
            })
            .collect();

        this.file_editor_wrap_row_starts = wrap_row_starts;
        elements
    }

    /// Refresh the visual-row projection and return how many rows the gutter
    /// has: one per line when the buffer is not wrapping, one per wrapped row
    /// when it is.
    ///
    /// Returns `line_count` unchanged whenever the buffer's row counts are not
    /// yet in step with the text — before the first prepaint, or in the frame
    /// between an edit and the wrap pass — so the gutter falls back to the
    /// unwrapped mapping rather than to a stale one.
    fn rebuild_file_editor_wrap_row_starts(
        &mut self,
        line_count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        // Taken out so the prefix sum can be written straight into the retained
        // buffer while the input is borrowed — no per-frame allocation, and no
        // copy of the row counts.
        let mut starts = std::mem::take(&mut self.file_editor_wrap_row_starts);
        starts.clear();

        let total = self.file_editor_input.read_with(cx, |input, _| {
            let rows_per_line = input.wrap_row_counts();
            if rows_per_line.len() != line_count {
                return None;
            }
            starts.reserve(line_count);
            let mut total = 0usize;
            for rows in rows_per_line {
                starts.push(total);
                total = total.saturating_add((*rows).max(1));
            }
            Some(total)
        });

        match total {
            Some(total) => {
                self.file_editor_wrap_row_starts = starts;
                total
            }
            None => {
                starts.clear();
                self.file_editor_wrap_row_starts = starts;
                line_count
            }
        }
    }
}

/// The 1-based line of the *blamed revision* that editor line `line_ix`
/// (0-based) came from, or `None` for a line the user has just typed.
///
/// Blame is indexed by committed line number, so an unsaved insertion or
/// deletion slides everything under it out of step. Rather than hide the column
/// while that is true — which blanked the whole strip on the first keystroke,
/// with auto-save off nothing clearing it until an explicit save — the lookup is
/// shifted back by `line_delta`, the number of lines the buffer has gained
/// against the blamed revision. Rows below an edit then keep the attribution
/// they actually have.
///
/// Exact for one contiguous edited region, which is what typing is; with edits
/// scattered above and below each other the shift is the net one, so the middle
/// can be off. That is a better answer than no answer, and it self-corrects on
/// save.
///
/// `None` for the lines inside the inserted text itself: they map to before the
/// first edit, i.e. to lines that already have their own row above, so there is
/// no revision line to attribute them to.
pub(in crate::view) fn file_editor_blame_line_for_editor_line(
    line_ix: usize,
    first_dirty_line: Option<u32>,
    line_delta: i64,
) -> Option<u32> {
    let one_based = i64::try_from(line_ix).ok()?.checked_add(1)?;
    let Some(first_dirty) = first_dirty_line.filter(|_| line_delta != 0) else {
        // Clean, or an edit that moved no line boundary: the buffer and the
        // blamed revision are still line-for-line.
        return u32::try_from(one_based).ok();
    };
    if line_ix < first_dirty as usize {
        return u32::try_from(one_based).ok();
    }
    let mapped = one_based.checked_sub(line_delta)?;
    // Anything landing at or above the first edited line is inside the freshly
    // typed text.
    if mapped <= i64::from(first_dirty) {
        return None;
    }
    u32::try_from(mapped).ok()
}

/// Map a gutter row to its logical line and whether it is a continuation of the
/// line above.
///
/// `starts[line]` is the first gutter row that line owns; an empty `starts`
/// means the buffer is not wrapping, where the two spaces coincide.
pub(in crate::view) fn file_editor_line_for_visual_row(
    starts: &[usize],
    visual_ix: usize,
) -> (usize, bool) {
    if starts.is_empty() {
        return (visual_ix, false);
    }
    // The last line whose first row is at or before this one.
    let line_ix = starts.partition_point(|&start| start <= visual_ix).max(1) - 1;
    (line_ix, starts[line_ix] != visual_ix)
}

/// Width of the editor's line-number gutter: the number cell plus the row's
/// `px_2` padding, so the container hugs its content and the divider sits right
/// against the code. The blame column, when shown, is added on top of this.
pub(in crate::view) fn file_editor_gutter_width(
    line_count: usize,
    show_line_numbers: bool,
    ui_scale_percent: u32,
) -> Pixels {
    if !show_line_numbers {
        return px(0.0);
    }
    // `px_2` on each side, scaled with everything else so the number cell is not
    // clipped once the digits grow with the UI. The digit width itself is already
    // scaled by `resolved_output_line_no_width`.
    let padding = ui_scale::design_px_from_percent(8.0 + 8.0, ui_scale_percent);
    rows::resolved_output_line_no_width(line_count, ui_scale_percent) + padding
}
