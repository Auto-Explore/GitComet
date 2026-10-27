//! The editable working-tree buffer.
//!
//! Structurally a smaller sibling of the merge tool's resolved-output pane: the
//! same [`components::TextInput`] over the same rope, fed by the same
//! [`rows::LiveSyntaxDocument`] — `tree.edit()` on the keystroke so the tree is
//! positionally correct for the very next frame, a budgeted foreground reparse,
//! and an off-thread one when that budget runs out. What it does *not* carry is
//! the conflict machinery: no placeholder mask, no protected rows, no overlay
//! for unresolved blocks. The one overlay it does add is the delimiter pair the
//! caret sits in.
//!
//! Stable document buffers survive switching files. The document owner keeps
//! their lifecycle state; only text and caret move between presenters.
//! `TextInput`'s undo stack does not leave the widget.

use super::file_disk::{DiskCheckCause, DiskSurface};
use super::*;
use crate::kit::rope::Rope;
use crate::kit::text_model::TextModelSnapshot;
use crate::kit::{HighlightProvider, HighlightProviderResult};
use crate::view::documents::buffers::{DocumentBuffer, StashedFileEdit};
use crate::view::documents::disk::DiskIdentity;
use crate::view::documents::text::{
    SaveEncodeError, encode_for_save, file_editor_text_fingerprint,
};
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity};
use gitcomet_core::text_format::{FormatSource, SideKind, SideTextFormat};
use palette::IntoColor;
use rustc_hash::FxHasher;
use std::path::{Path, PathBuf};

impl DiffRenderer {
    pub(in crate::view) fn file_editor_key(&self) -> Option<DocumentIdentity> {
        self.editor_documents
            .active()
            .map(|buffer| buffer.read().identity().clone())
    }

    pub(in crate::view) fn file_editor_dirty(&self) -> bool {
        self.editor_documents
            .active()
            .is_some_and(|buffer| buffer.read().dirty())
    }

    pub(in crate::view) fn file_editor_loading(&self) -> bool {
        self.editor_documents
            .active()
            .is_some_and(|buffer| buffer.read().loading())
    }

    pub(in crate::view) fn file_editor_error(&self) -> Option<SharedString> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().error().cloned())
    }

    pub(in crate::view) fn file_editor_text_format(&self) -> Option<SideTextFormat> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().text_format())
    }

    pub(in crate::view) fn file_editor_source_text_format(&self) -> Option<SideTextFormat> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().source_text_format())
    }

    #[cfg(test)]
    pub(in crate::view) fn file_editor_saved_fingerprint(&self) -> Option<u64> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().saved_fingerprint())
    }

    pub(in crate::view) fn file_editor_first_dirty_line(&self) -> Option<u32> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().first_dirty_line())
    }

    pub(in crate::view) fn file_editor_save_error(&self) -> Option<SharedString> {
        self.editor_documents
            .active()
            .and_then(|buffer| buffer.read().save_error().clone())
    }

    pub(in crate::view) fn file_editor_disk(&self) -> DiskIdentity {
        self.editor_documents
            .active()
            .map(|buffer| buffer.read().disk().clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(in crate::view) fn file_editor_reread_seq(&self) -> u64 {
        self.editor_documents
            .active()
            .map(|buffer| buffer.read().generation())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(in crate::view) fn file_editor_stash(
        &self,
    ) -> std::collections::BTreeMap<DocumentIdentity, StashedFileEdit> {
        self.editor_documents.stashes().into_iter().collect()
    }

    #[cfg(test)]
    pub(in crate::view) fn file_editor_disk_versions(
        &self,
    ) -> std::collections::BTreeMap<DocumentIdentity, DiskVersion> {
        self.editor_documents
            .all()
            .into_iter()
            .filter_map(|buffer| {
                let state = buffer.read();
                Some((state.identity().clone(), state.version().clone()?))
            })
            .collect()
    }

    pub(in crate::view) fn document_identity(
        &self,
        repo_id: RepoId,
        path: &Path,
    ) -> Option<DocumentIdentity> {
        self.repositories()
            .iter()
            .find(|r| r.id == repo_id)
            .map(|r| DocumentIdentity(r.spec.workdir.join(path)))
    }

    pub(crate) fn filesystem_resolve_unsaved(
        &mut self,
        sources: &[PathBuf],
        save: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.stash_current_file_editor_buffer(cx);
        for key in self.unsaved_document_identities() {
            if !sources.iter().any(|source| key.0.starts_with(source)) {
                continue;
            }
            if save {
                if let Some(buffer) = self.editor_documents.stashed(&key) {
                    let _ = self.enqueue_file_editor_save(key, &buffer, false, true, cx);
                }
            } else if self.file_editor_key().as_ref() == Some(&key) {
                self.discard_file_editor_buffer(cx);
            } else {
                self.editor_documents.discard(&key, cx);
            }
        }
    }

    pub(crate) fn filesystem_saves_drained(&self) -> bool {
        self.repositories()
            .iter()
            .all(|r| r.local_actions_in_flight == 0)
    }

    pub(crate) fn filesystem_pause(
        &mut self,
        id: gitcomet_core::filesystem::OperationId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_editor_autosave = None;
        self.stash_current_file_editor_buffer(cx);
        self.editor_documents.pause(id, cx);
    }

    pub(crate) fn filesystem_finish(
        &mut self,
        id: gitcomet_core::filesystem::OperationId,
        changes: &[gitcomet_core::filesystem::PathChange],
        versions: &std::collections::BTreeMap<PathBuf, DiskVersion>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.stash_current_file_editor_buffer(cx);
        self.editor_documents.resume(id, cx);
        for buffer in self.editor_documents.retarget(changes, versions, cx) {
            self.detach_file_edit(buffer, cx);
        }
        let language = self
            .file_editor_key()
            .as_ref()
            .and_then(|key| rows::diff_syntax_language_for_path(&key.0));
        if self.file_editor_language != language {
            self.reset_file_editor_syntax(language);
            let snapshot = self.file_editor_input.read(cx).text_snapshot();
            self.refresh_file_editor_syntax(&snapshot, None, cx);
        }
        self.prune_orphaned_file_editor_stash(cx);
        if !self.editor_documents.paused() {
            if self.file_editor_loading() {
                // The old read belongs to its original path. Reissue it under
                // the current key once every operation has released its pause;
                // the read sequence also prevents the old result from landing.
                self.reread_file_editor_from_disk(cx);
            } else if self.file_editor_dirty() {
                self.schedule_file_editor_autosave(cx);
            }
            if self.auto_save_file_edits {
                // Navigation and save acknowledgments can leave newer edits
                // waiting behind a pause. The active buffer uses its debounce;
                // inactive buffers have no timer to resume them.
                for (key, stashed) in self.editor_documents.stashes() {
                    if self.file_editor_key().as_ref() != Some(&key) && stashed.is_dirty() {
                        let _ = self.enqueue_file_editor_save(key, &stashed, false, false, cx);
                    }
                }
            }
        }
        cx.notify();
    }
}

/// How long the buffer has to sit still before auto-save writes it.
///
/// Long enough that typing a word is one write rather than five, short enough
/// that "I stopped typing" and "it is on disk" feel like the same moment.
pub(in crate::view) const FILE_EDITOR_AUTOSAVE_DEBOUNCE_MS: u64 = 800;

/// The error the user sees for a save that wrote nothing. With `editor`, the
/// dialog offers to save as UTF-8 and to go to the character.
pub(in crate::view) fn save_error_report(
    repo_id: RepoId,
    path: &Path,
    error: &SaveEncodeError,
    editor: bool,
) -> ErrorReport {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut report = ErrorReport::message(
        Some(repo_id),
        format!(
            "Couldn't save {name}\n\n{error} Choose another encoding under “Save with encoding” in the encoding menu."
        ),
    );
    if editor {
        report = report.with_action(ErrorAction::SaveEditorAs {
            repo_id,
            path: path.to_path_buf(),
            format: gitcomet_core::text_format::TextFormat::UTF_8,
        });
        if let SaveEncodeError::Unmappable(unmappable) = error {
            report = report.with_action(ErrorAction::RevealInEditor {
                repo_id,
                path: path.to_path_buf(),
                line: unmappable.line,
                column: unmappable.column,
                ch: unmappable.ch,
            });
        }
    }
    report
}

/// The provider binding key for the editor's highlights.
///
/// Must be *stable* when nothing changed: installing a provider notifies the
/// input, which re-enters the `cx.observe` that installed it, so a key that
/// varied per call would rebind, notify, and spin forever.
/// `set_highlight_provider_with_key` early-returns on an unchanged key, and that
/// early return is what terminates the cycle.
/// Wash every occurrence of the caret's name that falls inside `byte_range`.
///
/// `occurrences` is whole-document and sorted; this clips it to the window the
/// provider was asked for, which is what the overlay composer requires.
pub(in crate::view) fn apply_file_editor_occurrence_highlights(
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    occurrences: &[Range<usize>],
    byte_range: Range<usize>,
    style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    if occurrences.is_empty() {
        return highlights;
    }
    let clipped: Vec<Range<usize>> = occurrences
        .iter()
        .filter_map(|span| {
            let start = span.start.max(byte_range.start);
            let end = span.end.min(byte_range.end);
            (start < end).then_some(start..end)
        })
        .collect();
    crate::text_runs::overlay_highlights(highlights, &clipped, style)
}

pub(in crate::view) fn file_editor_provider_binding_key(
    document_version: u64,
    theme_epoch: u64,
    pair: Option<&rows::SyntaxPair>,
    search_overlay: &[Range<usize>],
    occurrences: &[Range<usize>],
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    document_version.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    // Typing in the search box moves no text and touches no tree either, so
    // without this the provider stays bound to the pre-query closure and nothing
    // is ever painted.
    search_overlay.hash(&mut hasher);
    // Moving the caret between delimiters moves no text and touches no tree, so
    // this is the only thing that tells the input its highlights changed.
    match pair {
        Some(pair) => {
            1u8.hash(&mut hasher);
            pair.open.hash(&mut hasher);
            pair.close.hash(&mut hasher);
            // Two kinds can share a span -- an empty string `""` is both a quote
            // pair and, to a grammar that brackets it, nothing else -- so the
            // kind rides along rather than being assumed redundant.
            (pair.kind as u8).hash(&mut hasher);
        }
        None => 0u8.hash(&mut hasher),
    }
    // Moving the caret onto another name changes no text and touches no tree
    // either, so this is the only thing that tells the input to repaint.
    occurrences.hash(&mut hasher);
    hasher.finish()
}

/// Binding key for the editor's heuristic fallback provider.
///
/// The live key space is the tree's version; this one has no tree, so it keys
/// on the buffer revision the closure captured. Tagged so the two can never
/// collide on a buffer that switches arms.
pub(in crate::view) fn file_editor_heuristic_provider_binding_key(
    revision: (u64, u64),
    language: Option<rows::DiffSyntaxLanguage>,
    theme_epoch: u64,
    search_overlay: &[Range<usize>],
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    "file-editor-heuristic".hash(&mut hasher);
    revision.hash(&mut hasher);
    language.hash(&mut hasher);
    theme_epoch.hash(&mut hasher);
    search_overlay.hash(&mut hasher);
    hasher.finish()
}

/// Paint the matching pair on top of the syntax runs.
///
/// The two spans are usually one character each, but a tag pair covers whole
/// tags and so can span many runs; the overlay composer handles either.
/// Mirrors the overlay composition the resolved output uses for its unresolved
/// rows: highlights arrive sorted and disjoint and leave that way.
pub(in crate::view) fn apply_file_editor_pair_highlights(
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pair: Option<&rows::SyntaxPair>,
    byte_range: Range<usize>,
    style: gpui::HighlightStyle,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let Some(pair) = pair else {
        return highlights;
    };
    let mut overlays: Vec<Range<usize>> = Vec::with_capacity(2);
    for span in [&pair.open, &pair.close] {
        let clipped = span.start.max(byte_range.start)..span.end.min(byte_range.end);
        if clipped.start < clipped.end {
            overlays.push(clipped);
        }
    }
    overlays.sort_by_key(|span| span.start);
    crate::text_runs::overlay_highlights(highlights, &overlays, style)
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

    crate::text_runs::overlay_highlights(highlights, &clipped, style)
}

impl DiffRenderer {
    /// Scroll the editor sideways so the caret the search reveal just placed is
    /// on screen.
    ///
    /// A multiline `TextInput` leaves horizontal scrolling to the container it
    /// sits in, and its own caret autoscroll is vertical only, so a match far
    /// along a long line otherwise scrolls into view still off the right edge.
    fn reveal_file_editor_search_match_horizontally(&mut self, cx: &mut gpui::Context<Self>) {
        if self.diff_word_wrap {
            return;
        }
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
            .is_some_and(|repo| self.bound_diff_state(repo).edit_mode)
            && self.file_editor_path().is_some()
    }

    /// Seat the buffer the search scan reads, and re-scan if it moved.
    ///
    /// Every path that changes the editor's text goes through here, so the match
    /// list can never describe a revision the buffer has left behind. Gating the
    /// re-scan on the revision is also what keeps this off the install-provider →
    /// notify → observe cycle: a rebind moves no text, so the second lap stops.
    fn file_editor_search_source_changed(
        &mut self,
        snapshot: TextModelSnapshot,
        cx: &mut gpui::Context<Self>,
    ) {
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
            self.diff_search_schedule_preserving_current(cx);
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
        if !self.bound_diff_state(repo).edit_mode {
            return None;
        }
        match self.bound_diff_state(repo).diff_target.as_ref()? {
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
        if !self.store.policy.allow_edit {
            return None;
        }
        let repo = self.active_repo()?;
        match self.bound_diff_state(repo).diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(path.clone()),
            DiffTarget::Commit { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    pub(in crate::view) fn file_editor_is_dirty(&self) -> bool {
        self.file_editor_dirty()
    }

    /// Whether the action bar shows Cancel and Save for the editor: always
    /// without auto-save, and with it while the buffer cannot be written.
    pub(in crate::view) fn file_editor_shows_save_controls(&self) -> bool {
        !self.auto_save_file_edits
            || (self.file_editor_dirty() && self.file_editor_save_error().is_some())
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
        let Some(identity) = self.document_identity(repo_id, &path) else {
            return;
        };
        // Only a *different* file is loaded here. The buffer never follows the
        // disk on its own: an external write raises the "File changed on disk"
        // notice (`super::file_disk`) and the user chooses to reload.
        if self.file_editor_key().as_ref() == Some(&identity) {
            self.refresh_file_editor_decoding(cx);
            return;
        }

        // Leaving one file for another: write it if auto-save is on, otherwise
        // keep the unsaved text so coming back does not silently drop it.
        self.flush_file_editor_buffer(cx);

        // Invalidate outgoing reads even when a filesystem pause defers the
        // incoming read, including navigation away from and back to one path.
        self.editor_documents
            .activate(identity.clone(), repo_id, cx);
        let retained = self.editor_documents.stashed(&identity);
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
        self.file_editor_decode_key = None;
        self.file_editor_waiting_for_attributes = false;
        // The incremental tree belongs to the outgoing file. A same-file
        // re-read (`reread_file_editor_from_disk`) keeps it: tearing it down
        // there cost a full-document parse on the next keystroke.
        self.reset_file_editor_syntax(rows::diff_syntax_language_for_path(&path));
        self.file_editor_autosave = None;

        if let Some(stashed) = retained {
            self.file_editor_decode_key = self
                .selected_text_decode_request(SideKind::Worktree)
                .map(|(_, key)| key);
            self.present_file_editor_text(stashed.text, Some(stashed.cursor), cx);
            let revs = self.current_file_disk_revs();
            self.file_disk_read_landed(DiskSurface::Editor, revs, cx);
            self.spawn_file_disk_check(DiskCheckCause::CameIntoView, cx);
            return;
        }

        // Blank the buffer and show the loading state: nothing in it belongs
        // to the incoming file.
        if let Some(document) = self.editor_documents.active() {
            document.prepare_reload(cx);
        }
        self.file_editor_input.update(cx, |input, cx| {
            input.set_read_only(true, cx);
            input.set_text("", cx);
        });
        if !self.editor_documents.paused() {
            self.reread_file_editor_from_disk(cx);
        }
    }

    /// Read the file under `file_editor_key` and seat it.
    ///
    /// Shared by the first load and by a reload of the file already on screen:
    /// when the bytes match what the buffer holds nothing is seated (seating
    /// resets the caret and the undo stack), otherwise the caret is kept,
    /// clamped, rather than thrown back to the top.
    pub(in crate::view) fn reread_file_editor_from_disk(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(load_key) = self.file_editor_key().clone() else {
            return;
        };
        let absolute = load_key.0.clone();
        let Some((request, decode_key)) = self.selected_text_decode_request(SideKind::Worktree)
        else {
            self.file_editor_waiting_for_attributes = true;
            return;
        };
        self.file_editor_waiting_for_attributes = false;
        self.file_editor_decode_key = Some(decode_key);
        let document = self
            .editor_documents
            .active()
            .cloned()
            .expect("selected editor document");
        let seq = document.begin_read(self.file_editor_loading(), cx);
        let revs = self.current_file_disk_revs();
        // A re-read of the file on screen (not a first load into a blank
        // buffer) must not replace text typed while it ran.
        let text_at_start = (!self.file_editor_loading()).then(|| {
            self.file_editor_input.read_with(cx, |input, _| {
                let snapshot = input.text_snapshot();
                (snapshot.model_id(), snapshot.revision())
            })
        });
        let filesystem = self.store.filesystem();
        cx.spawn(async move |view: WeakEntity<DiffRenderer>, cx| {
            let read = {
                let absolute = absolute.clone();
                move || {
                    super::preview::preflight_worktree_file_for_editing(&absolute)?;
                    let _guard = filesystem.lock().unwrap_or_else(|e| e.into_inner());
                    let version =
                        super::preview::read_worktree_file_version_for_editing(&absolute)?;
                    let (text, stamp, hash, format) =
                        super::preview::read_worktree_file_for_editing(&absolute, &request)?;
                    if super::preview::read_worktree_file_version_for_editing(&absolute)? != version
                    {
                        return Err("File changed while reading. Open it again.".to_string());
                    }
                    Ok((text, stamp, hash, format, version))
                }
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
                // the path. A newer read supersedes this one outright.
                if this.file_editor_key().as_ref() != Some(&load_key)
                    || !document.read().accepts_read(seq)
                {
                    return;
                }
                let typed_since = text_at_start.is_some_and(|start| {
                    this.file_editor_input.read_with(cx, |input, _| {
                        let snapshot = input.text_snapshot();
                        (snapshot.model_id(), snapshot.revision()) != start
                    })
                });
                if typed_since {
                    // The keystrokes win. If the disk still disagrees, the
                    // check raises the notice again.
                    this.spawn_file_disk_check(DiskCheckCause::WorktreeChanged, cx);
                    return;
                }
                match result {
                    Ok((text, stamp, hash, format, version)) => {
                        document.finish_read(
                            seq,
                            version,
                            Some(format),
                            DiskIdentity::loaded(stamp, Some(hash)),
                            cx,
                        );
                        // Compared as text, not by fingerprint: the buffer's
                        // fingerprint is folded chunk-by-chunk off the rope and
                        // a flat string cannot reproduce it. The whole file was
                        // just read anyway, so one comparison costs nothing new.
                        let unchanged = this
                            .file_editor_input
                            .read_with(cx, |input, _| input.text() == text.as_ref());
                        if unchanged {
                            let read_only = this.file_editor_format_read_only();
                            let fingerprint = this.file_editor_input.update(cx, |input, cx| {
                                input.set_read_only(read_only, cx);
                                file_editor_text_fingerprint(&input.text_snapshot())
                            });
                            document.seat_text(fingerprint, None, cx);
                            cx.notify();
                        } else {
                            let cursor = this
                                .file_editor_input
                                .read_with(cx, |input, _| input.cursor_offset())
                                .min(text.len());
                            this.apply_file_editor_text(text, Some(cursor), cx);
                        }
                    }
                    Err(message) => {
                        document.fail_read(seq, message.into(), cx);
                        cx.notify();
                    }
                }
                this.file_disk_read_landed(DiskSurface::Editor, revs, cx);
            });
        })
        .detach();
    }

    /// Re-read the open file when its attributes arrive or change, or the user
    /// picks another encoding for it. Unsaved edits keep the encoding they
    /// were read in, and a save writes them back in it.
    fn refresh_file_editor_decoding(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((_, key)) = self.selected_text_decode_request(SideKind::Worktree) else {
            return;
        };
        if self.file_editor_waiting_for_attributes {
            self.reread_file_editor_from_disk(cx);
            return;
        }
        if self.file_editor_decode_key == Some(key) {
            return;
        }
        if self.file_editor_dirty() {
            self.file_editor_decode_key = Some(key);
        } else {
            self.reread_file_editor_from_disk(cx);
        }
    }

    /// Reload the file on screen from disk, dropping unsaved edits, keeping
    /// the caret. Not `discard_file_editor_buffer`: that blanks the buffer.
    pub(in crate::view) fn reload_file_editor_from_disk(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(key) = self.file_editor_key().clone() else {
            return;
        };
        self.editor_documents.discard(&key, cx);
        // A pending auto-save would write the discarded edits straight back.
        self.file_editor_autosave = None;

        self.reread_file_editor_from_disk(cx);
    }

    /// Take what is on disk now as the version the next save may replace.
    pub(super) fn refresh_file_editor_disk_version(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(key) = self.file_editor_key().clone() else {
            return;
        };
        let Some(document) = self.editor_documents.find(&key) else {
            return;
        };
        let filesystem = self.store.filesystem();
        cx.spawn(async move |view: WeakEntity<DiffRenderer>, cx| {
            let read = {
                let path = key.0.clone();
                move || {
                    let _guard = filesystem.lock().unwrap_or_else(|e| e.into_inner());
                    gitcomet_core::filesystem::DiskVersion::read_file(
                        &path,
                        super::preview::FILE_EDITOR_MAX_TEXT_BYTES as u64,
                    )
                }
            };
            let version = if crate::ui_runtime::current().uses_background_compute() {
                smol::unblock(read).await
            } else {
                read()
            };
            let _ = view.update(cx, |_, cx| {
                if document.read().identity() != &key {
                    return;
                }
                match version {
                    Ok(version) => document.set_disk_version(Some(version), cx),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        document.set_disk_version(None, cx);
                    }
                    Err(_) => {}
                }
            });
        })
        .detach();
    }

    /// Keep the buffer's format and editability together on reads, restores,
    /// and encoding changes.
    /// Text read in a format it cannot be written back in is view-only.
    fn file_editor_format_read_only(&self) -> bool {
        self.file_editor_text_format()
            .is_some_and(|format| !format.is_writable())
    }

    /// Seat freshly read text in the input and establish its disk baseline.
    fn apply_file_editor_text(
        &mut self,
        text: SharedString,
        cursor: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.present_file_editor_text(text, cursor, cx);
        let fingerprint =
            file_editor_text_fingerprint(&self.file_editor_input.read(cx).text_snapshot());
        if let Some(document) = self.editor_documents.active() {
            document.seat_text(fingerprint, None, cx);
        }
    }

    fn present_file_editor_text(
        &mut self,
        text: SharedString,
        cursor: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Enter must insert what the file already uses. Without this the input
        // keeps its platform default and a CRLF file edited on Linux (or an LF
        // file on Windows) grows mixed endings, showing up as a diff on lines
        // nobody touched. The resolved-output buffer does the same.
        let line_ending = crate::kit::TextInput::detect_line_ending(text.as_ref());
        let read_only = self.file_editor_format_read_only();
        let snapshot = self.file_editor_input.update(cx, |input, cx| {
            input.set_read_only(read_only, cx);
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
        self.file_editor_search_source_changed(snapshot.clone(), cx);
        self.refresh_file_editor_syntax(&snapshot, None, cx);
        cx.notify();
    }

    /// Absolute path of a repo-relative working-tree path.
    pub(in crate::view) fn absolute_worktree_path(&self, path: &Path) -> Option<PathBuf> {
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        Some(self.input.workdir()?.join(path))
    }

    /// Called from the input's observer on every keystroke.
    pub(in crate::view) fn on_file_editor_edited(&mut self, cx: &mut gpui::Context<Self>) {
        if self.file_editor_key().is_none() {
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
        self.file_editor_search_source_changed(snapshot.clone(), cx);

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
        if let Some(document) = self.editor_documents.active() {
            document.edit(file_editor_text_fingerprint(&snapshot), edited_row, cx);
        }
        let dirty = self.file_editor_dirty();
        if dirty && self.auto_save_file_edits {
            self.schedule_file_editor_autosave(cx);
        }
    }

    /// Restart the quiet-period timer that auto-save writes on.
    pub(super) fn schedule_file_editor_autosave(&mut self, cx: &mut gpui::Context<Self>) {
        if self.editor_documents.paused() {
            return;
        }
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
                // Not while "File changed on disk" is asking: writing now
                // would answer it for the user. Dismissing reschedules.
                if this.auto_save_file_edits
                    && this.file_editor_dirty()
                    && !this.file_disk_notice_awaits_editor()
                {
                    this.write_file_editor_buffer(false, cx);
                }
            });
        }));
    }

    /// Saves share the filesystem queue with moves. Keep the buffer dirty until
    /// the worker confirms both the expected disk identity and installation.
    /// Returns whether a write was dispatched.
    pub(in crate::view) fn save_file_editor_buffer(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.write_file_editor_buffer(true, cx)
    }

    /// `save_file_editor_buffer`; background writes pass `repeat_failure:
    /// false` so a failure already on screen is not reported again.
    fn write_file_editor_buffer(
        &mut self,
        repeat_failure: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.editor_documents.paused() || self.file_editor_loading() || !self.file_editor_dirty()
        {
            return false;
        }
        let Some(key) = self.file_editor_key().clone() else {
            return false;
        };
        // Every caller that gets here on purpose (button, Ctrl+S, Save all)
        // is the user keeping their edits over the other program's, so the
        // write replaces the version the notice described.
        let overwrite = self.file_disk_notice_awaits_editor();
        self.answer_file_disk_notice_by_saving();
        self.file_editor_autosave = None;
        self.stash_current_file_editor_buffer(cx);
        let Some(stashed) = self.editor_documents.stashed(&key) else {
            return false;
        };
        matches!(
            self.enqueue_file_editor_save(key, &stashed, overwrite, repeat_failure, cx),
            Ok(true)
        )
    }

    /// Queue `buffer`'s write, encoded as it was read. `Ok(false)` means its
    /// origin no longer permits saving or it waits for a pause or earlier write.
    fn enqueue_file_editor_save(
        &mut self,
        key: DocumentIdentity,
        buffer: &StashedFileEdit,
        overwrite: bool,
        repeat_failure: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Result<bool, SaveEncodeError> {
        if self.editor_documents.paused() || buffer.document.read().saving().is_some() {
            return Ok(false);
        }
        let Some((repo_id, lifetime, worktree)) = self
            .repositories()
            .iter()
            .filter(|repo| key.0.starts_with(&repo.spec.workdir))
            .max_by_key(|repo| repo.spec.workdir.components().count())
            .map(|repo| (repo.id, repo.lifetime(), repo.spec.workdir.clone()))
        else {
            self.prune_orphaned_file_editor_stash(cx);
            return Ok(false);
        };
        if self
            .repositories()
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(RepoState::is_bare)
        {
            buffer.document.set_save_error(
                Some(gitcomet_core::domain::WORKING_TREE_REQUIRED_MESSAGE.into()),
                cx,
            );
            return Ok(false);
        }
        let Some(origin) = self.store.document_save_origin(repo_id, lifetime) else {
            return Ok(false);
        };
        let current = self.file_editor_key().as_ref() == Some(&key);
        let text_format = buffer.document.read().text_format();
        let (bytes, pending_write) = match encode_for_save(buffer.text.clone(), text_format) {
            Ok(encoded) => encoded,
            Err(error) => {
                // Kept dirty: nothing was written.
                self.report_file_editor_encode_error(&key, &error, repeat_failure, cx);
                return Err(error);
            }
        };
        if current {
            if let Some(document) = self.editor_documents.active() {
                document.set_save_error(None, cx);
            }
            // A converted buffer, or one read in a chosen encoding, may need
            // that choice moved so the new bytes read back as written.
            if let Some(format) =
                text_format.filter(|format| format.source == FormatSource::Override)
                && let Some((_, path)) = self.document_repo_path(&key)
            {
                self.follow_saved_text_format(&path, bytes.as_bytes(), format.format, cx);
            }
        }
        let request =
            gitcomet_core::filesystem::Request::new(gitcomet_core::filesystem::Operation::Save {
                path: key.0.clone(),
                worktree: Some(worktree),
                contents: Arc::from(bytes.as_bytes()),
                expected: buffer.document.read().version().clone(),
                overwrite,
            });
        if !self.editor_documents.save(
            &buffer.document,
            origin,
            request,
            buffer.text_fingerprint,
            pending_write,
            cx,
        ) {
            return Ok(false);
        }
        cx.notify();
        Ok(true)
    }

    fn report_file_editor_encode_error(
        &mut self,
        key: &DocumentIdentity,
        error: &SaveEncodeError,
        repeat_failure: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_editor_key().as_ref() == Some(key) {
            self.file_editor_autosave = None;
        }
        let Some((repo_id, path)) = self.document_repo_path(key) else {
            return;
        };
        let report = save_error_report(repo_id, &path, error, true);
        let message = SharedString::from(report.message.clone());
        let mut repeat = false;
        if self.file_editor_key().as_ref() == Some(key) {
            repeat = self.file_editor_save_error().as_ref() == Some(&message);
            if let Some(document) = self.editor_documents.active() {
                document.set_save_error(Some(message), cx);
            }
        }
        if repeat_failure || !repeat {
            self.show_error_report(report, cx);
        }
        cx.notify();
    }

    /// The repository tab and repo-relative path `key` belongs to; the
    /// innermost repository wins for nested worktrees.
    pub(in crate::view) fn document_repo_path(
        &self,
        key: &DocumentIdentity,
    ) -> Option<(RepoId, PathBuf)> {
        self.repositories()
            .iter()
            .filter_map(|repo| {
                key.0
                    .strip_prefix(&repo.spec.workdir)
                    .ok()
                    .map(|path| (repo.id, path.to_path_buf()))
            })
            .min_by_key(|(_, path)| path.components().count())
    }

    pub(in crate::view) fn process_file_editor_saves(
        &mut self,
        state: &AppState,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gitcomet_core::filesystem::OperationId> {
        let mut completed = Vec::new();
        for result in &state.filesystem.completed {
            let Some(document) = self.editor_documents.pending().into_iter().find(|buffer| {
                buffer
                    .read()
                    .saving()
                    .as_ref()
                    .is_some_and(|save| save.operation == result.id)
            }) else {
                continue;
            };
            let key = document.read().identity().clone();
            let current = self
                .editor_documents
                .active()
                .is_some_and(|active| active.id() == document.id());
            if current {
                document.capture_fingerprint(
                    file_editor_text_fingerprint(&self.file_editor_input.read(cx).text_snapshot()),
                    cx,
                );
            }
            let outcome = if result.succeeded() {
                Ok(result.saved_version.clone())
            } else {
                Err(result
                    .items
                    .iter()
                    .filter_map(|item| match &item.outcome {
                        gitcomet_core::filesystem::ItemOutcome::Failed(message) => {
                            Some(message.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into())
            };
            document.finish_save(result.id, outcome.clone(), cx);
            completed.push(result.id);
            match outcome {
                Ok(_) => {
                    if current && self.file_editor_dirty() && self.auto_save_file_edits {
                        self.schedule_file_editor_autosave(cx);
                    } else if !current
                        && self.auto_save_file_edits
                        && let Some(stashed) = document.stashed()
                    {
                        let _ =
                            self.enqueue_file_editor_save(key.clone(), &stashed, false, false, cx);
                    }
                    if let Some(relative) = self
                        .active_repo()
                        .and_then(|repo| key.0.strip_prefix(&repo.spec.workdir).ok())
                        .map(Path::to_path_buf)
                    {
                        self.invalidate_worktree_preview_for_saved_path(&relative);
                    }
                }
                Err(message) => {
                    self.file_editor_autosave = None;
                    self.commands.defer(
                        DiffCommand::Toast(
                            components::ToastKind::Error,
                            message.to_string().into(),
                        ),
                        cx,
                    );
                }
            }
            self.editor_documents.release_inactive_clean();
            self.prune_orphaned_file_editor_stash(cx);
            cx.notify();
        }
        completed
    }

    /// Keep an unsaved buffer around under its path.
    pub(in crate::view) fn stash_current_file_editor_buffer(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_editor_loading() {
            return;
        }
        let Some(document) = self.editor_documents.active() else {
            return;
        };
        document.capture_fingerprint(
            file_editor_text_fingerprint(&self.file_editor_input.read(cx).text_snapshot()),
            cx,
        );
        let state = document.read();
        if !state.dirty() && state.saving().is_none() {
            drop(state);
            document.clear_retained();
            return;
        }
        drop(state);
        let input = self.file_editor_input.read(cx);
        document.retain_text(
            input.text().to_string().into(),
            input.cursor_offset(),
            file_editor_text_fingerprint(&input.text_snapshot()),
        );
    }

    /// Try auto-save, then stash any edits that could not be written.
    /// Returns whether a write was dispatched, so closing can wait for it.
    ///
    /// The two moments this covers are leaving the editor and losing focus,
    /// which is where "auto-save" has to mean more than "after a pause" — a
    /// pause that is interrupted by navigating away would otherwise lose the
    /// write it was about to make.
    pub(in crate::view) fn flush_file_editor_buffer(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        // A loading widget contains a placeholder. It cannot supply editable
        // text until the owner's read generation lands.
        if self.file_editor_loading() || self.file_editor_key().is_none() {
            return false;
        }
        // With "File changed on disk" open the buffer is kept, not written:
        // leaving the file is not an answer to the question.
        let dispatched = self.auto_save_file_edits
            && self.file_editor_error().is_none()
            && !self.file_disk_notice_awaits_editor()
            && self.write_file_editor_buffer(false, cx);
        self.stash_current_file_editor_buffer(cx);
        dispatched
    }

    fn detach_file_edit(&self, document: DocumentBuffer, cx: &mut gpui::Context<Self>) {
        self.editor_documents.adopt(document, cx);
    }

    pub(in crate::view) fn release_document_presenter(&mut self, cx: &mut gpui::App) {
        if let Some(document) = self.editor_documents.active()
            && !document.read().loading()
        {
            let input = self.file_editor_input.read(cx);
            let fingerprint = file_editor_text_fingerprint(&input.text_snapshot());
            let text = input.text().to_string().into();
            let cursor = input.cursor_offset();
            document.capture_fingerprint(fingerprint, cx);
            document.retain_text(text, cursor, fingerprint);
        }
        self.editor_documents.release(cx);
    }

    pub(in crate::view) fn prune_orphaned_file_editor_stash(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.editor_documents.pending().is_empty() || self.editor_documents.paused() {
            return;
        }
        self.stash_current_file_editor_buffer(cx);
        for document in self.editor_documents.all() {
            let repository = self
                .document_repo_path(document.read().identity())
                .map(|(repo, _)| repo);
            if let Some(repository) = repository {
                document.attach(repository, cx);
                continue;
            }
            self.editor_documents.detach(&document, cx);
            if document.read().dirty() {
                self.detach_file_edit(document, cx);
            }
        }
        self.editor_documents.release_inactive_clean();
        if self.editor_documents.active().is_none() {
            self.file_editor_autosave = None;
        }
    }

    fn unsaved_document_identities(&self) -> Vec<DocumentIdentity> {
        let mut keys: Vec<_> = self
            .editor_documents
            .all()
            .iter()
            .filter_map(|buffer| {
                let state = buffer.read();
                (state.dirty() && !state.loading()).then(|| state.identity().clone())
            })
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    pub(in crate::view) fn unsaved_file_edit_keys(&self) -> Vec<(RepoId, PathBuf)> {
        self.unsaved_document_identities()
            .into_iter()
            .filter_map(|key| {
                self.repositories()
                    .iter()
                    .filter_map(|repo| {
                        key.0
                            .strip_prefix(&repo.spec.workdir)
                            .ok()
                            .map(|path| (repo.id, path.to_path_buf()))
                    })
                    .min_by_key(|(_, p)| p.components().count())
            })
            .collect()
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
        let Some(key) = self.document_identity(repo_id, path) else {
            return false;
        };
        self.editor_documents.find(&key).is_some_and(|buffer| {
            let state = buffer.read();
            state.dirty() && !state.loading()
        })
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
        let Some(key) = self.document_identity(repo_id, path) else {
            return;
        };
        if self.file_editor_key().as_ref() == Some(&key) {
            // On screen: re-reads from disk, which is what puts the text back.
            self.discard_file_editor_buffer(cx);
            return;
        }
        self.editor_documents.discard(&key, cx);
        cx.notify();
    }

    /// Every path with edits that are not on disk, labelled for display.
    ///
    /// Sorted and de-duplicated so the confirmation dialog lists each file once
    /// and in a stable order.
    #[cfg(test)]
    pub(in crate::view) fn unsaved_file_edit_labels(&self) -> Vec<SharedString> {
        let identities = self.unsaved_document_identities();
        let multiple = self.repositories().len() > 1;
        identities
            .into_iter()
            .map(|key| {
                let relative = self
                    .repositories()
                    .iter()
                    .find_map(|r| key.0.strip_prefix(&r.spec.workdir).ok().map(|p| (r, p)));
                match relative {
                    Some((repo, path)) if multiple => format!(
                        "{} — {}",
                        repo.spec
                            .workdir
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        path.display()
                    )
                    .into(),
                    Some((_, path)) => path.display().to_string().into(),
                    None => key.0.display().to_string().into(),
                }
            })
            .collect()
    }

    /// Display labels for dirty buffers that belong to one repository. A move
    /// only detaches that repository, so unrelated tabs must neither block the
    /// move nor be discarded by its confirmation dialog.
    #[cfg(test)]
    pub(in crate::view) fn unsaved_file_edit_labels_for_repo(
        &self,
        repo_id: RepoId,
    ) -> Vec<SharedString> {
        self.unsaved_file_edit_paths(repo_id)
            .into_iter()
            .map(|path| SharedString::from(path.display().to_string()))
            .collect()
    }

    /// Write every unsaved buffer, retaining it until its save is confirmed.
    /// Returns false if any could not be encoded; those edits stay unsaved and
    /// a pending close must be canceled.
    pub(in crate::view) fn save_all_file_edits(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        self.save_file_edits_scoped_to(None, cx)
    }

    /// Write only the unsaved buffers owned by `repo_id`. Returns false like
    /// `save_all_file_edits`.
    pub(in crate::view) fn save_file_edits_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.save_file_edits_scoped_to(Some(repo_id), cx)
    }

    fn save_file_edits_scoped_to(
        &mut self,
        only_repo: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let in_scope = |this: &Self, key: &DocumentIdentity| {
            only_repo.is_none_or(|only_repo| {
                this.document_repo_path(key)
                    .is_some_and(|(repo_id, _)| repo_id == only_repo)
            })
        };
        // Saving on purpose answers "File changed on disk" for the buffer on
        // screen, as `save_file_editor_buffer` does.
        let current_in_scope = self
            .file_editor_key()
            .as_ref()
            .is_some_and(|key| in_scope(self, key));
        let overwrite_current = current_in_scope && self.file_disk_notice_awaits_editor();
        if current_in_scope {
            self.answer_file_disk_notice_by_saving();
        }
        self.stash_current_file_editor_buffer(cx);
        let mut failed = None;
        for (key, stashed) in self.editor_documents.stashes() {
            if !in_scope(self, &key) {
                continue;
            }
            let current = self.file_editor_key().as_ref() == Some(&key);
            if self.file_editor_loading() && current {
                continue;
            }
            if stashed.is_dirty()
                && self
                    .enqueue_file_editor_save(
                        key.clone(),
                        &stashed,
                        current && overwrite_current,
                        true,
                        cx,
                    )
                    .is_err()
            {
                failed.get_or_insert((key, current));
            }
        }
        let Some((key, current)) = failed else {
            return true;
        };
        // Bring the failed buffer back so its encoding can be changed.
        if !current && let Some((repo_id, path)) = self.document_repo_path(&key) {
            self.store.dispatch(Msg::OpenFileEditor { repo_id, path });
        }
        false
    }

    /// Throw away every unsaved buffer.
    pub(in crate::view) fn discard_all_file_edits(&mut self, cx: &mut gpui::Context<Self>) {
        for (key, _) in self.editor_documents.stashes() {
            if self.file_editor_key().as_ref() != Some(&key) {
                self.editor_documents.discard(&key, cx);
            }
        }
        if self.file_editor_dirty() {
            self.discard_file_editor_buffer(cx);
        }
    }

    /// Throw away only the dirty buffers owned by `repo_id`.
    pub(in crate::view) fn discard_file_edits_for_repo(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let paths = self.unsaved_file_edit_paths(repo_id);
        for path in paths {
            self.discard_file_edits_for(repo_id, &path, cx);
        }
    }

    /// Drop an unsaved buffer, reloading the file from disk.
    pub(in crate::view) fn discard_file_editor_buffer(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(key) = self.file_editor_key().clone() else {
            return;
        };
        self.editor_documents.discard(&key, cx);
        self.file_editor_autosave = None;
        if let Some(document) = self.editor_documents.active() {
            document.prepare_reload(cx);
        }
        self.file_editor_input.update(cx, |input, cx| {
            input.set_read_only(true, cx);
            input.set_text("", cx);
        });
        if !self.editor_documents.paused() {
            self.reread_file_editor_from_disk(cx);
        }
    }

    fn reset_file_editor_syntax(&mut self, language: Option<rows::DiffSyntaxLanguage>) {
        self.file_editor_language = language;
        self.file_editor_syntax_pair = None;
        self.file_editor_occurrences.clear();
        self.file_editor_occurrences_version = None;
        self.file_editor_live_syntax = None;
        self.file_editor_live_syntax_source = None;
        self.file_editor_live_syntax_building = None;
        self.file_editor_live_syntax_build = None;
        self.file_editor_live_syntax_reparse = None;
        self.file_editor_live_syntax_reparsing = None;
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
        if self.file_editor_loading() {
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
            self.ensure_file_editor_live_syntax_reparse(cx);
            self.rebind_file_editor_highlight_provider(cx);
            return;
        }

        // Everything below reads through the rope; the whole document is never
        // flattened on the keystroke path.
        let rope = snapshot.rope();
        let budget = self.live_syntax_foreground_budget();
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
                // background work is already in flight — that
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
                let already_building = self.file_editor_live_syntax_building.is_some()
                    || self.file_editor_live_syntax_reparsing.is_some();
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
        if self.file_editor_live_syntax_building.is_some()
            || self.file_editor_live_syntax_reparsing.is_some()
        {
            return;
        }
        let ownership = (Self::next_live_syntax_job_id(), revision);
        self.file_editor_live_syntax_building = Some(ownership);
        self.file_editor_live_syntax_build = Some(self.run_live_syntax_background_compute(
            cx,
            move || rows::LiveSyntaxDocument::new(language, rope, Arc::default(), None),
            move |this, cx, built| {
                if this.file_editor_live_syntax_building != Some(ownership) {
                    return;
                }
                this.file_editor_live_syntax_building = None;
                this.file_editor_live_syntax_build = None;
                let snapshot = this
                    .file_editor_input
                    .read_with(cx, |input, _| input.text_snapshot());
                if (snapshot.model_id(), snapshot.revision()) != revision
                    || this.file_editor_language != Some(language)
                {
                    // Zed's `parse_again`: a tree for text the buffer has
                    // moved past is useless, but so is waiting.
                    this.refresh_file_editor_syntax(&snapshot, None, cx);
                    return;
                }
                let Some(document) = built else {
                    // Leave a failed current build retryable on refresh.
                    return;
                };
                this.file_editor_live_syntax = Some(document);
                this.file_editor_live_syntax_source = Some(revision);
                this.rebind_file_editor_highlight_provider(cx);
            },
        ));
    }

    fn ensure_file_editor_live_syntax_reparse(&mut self, cx: &mut gpui::Context<Self>) {
        if self.file_editor_live_syntax_reparsing.is_some()
            || self.file_editor_live_syntax_building.is_some()
        {
            return;
        }
        let Some(request) = self
            .file_editor_live_syntax
            .as_ref()
            .and_then(rows::LiveSyntaxDocument::background_reparse_request)
        else {
            return;
        };
        let requested_version = request.version();
        self.file_editor_live_syntax_reparsing = Some(requested_version);
        self.file_editor_live_syntax_reparse = Some(self.run_live_syntax_background_compute(
            cx,
            move || rows::live_syntax_reparse(request),
            move |this, cx, parsed| {
                if this.file_editor_live_syntax_reparsing != Some(requested_version) {
                    return;
                }
                this.file_editor_live_syntax_reparsing = None;
                this.file_editor_live_syntax_reparse = None;
                let Some((version, tree, injections)) = parsed else {
                    if this.file_editor_live_syntax.is_none() {
                        let snapshot = this
                            .file_editor_input
                            .read_with(cx, |input, _| input.text_snapshot());
                        this.refresh_file_editor_syntax(&snapshot, None, cx);
                    } else if this
                        .file_editor_live_syntax
                        .as_ref()
                        .map(rows::LiveSyntaxDocument::version)
                        != Some(requested_version)
                    {
                        this.ensure_file_editor_live_syntax_reparse(cx);
                    }
                    return;
                };
                let adopted = this
                    .file_editor_live_syntax
                    .as_mut()
                    .is_some_and(|document| {
                        document.adopt_background_tree(version, tree, injections)
                    });
                if !adopted {
                    // The buffer moved while this was in flight, so the tree
                    // describes text that no longer exists.
                    if this.file_editor_live_syntax.is_none() {
                        let snapshot = this
                            .file_editor_input
                            .read_with(cx, |input, _| input.text_snapshot());
                        this.refresh_file_editor_syntax(&snapshot, None, cx);
                    } else {
                        this.ensure_file_editor_live_syntax_reparse(cx);
                    }
                    return;
                }
                this.rebind_file_editor_highlight_provider(cx);
            },
        ));
    }

    /// Hand the input a provider over the document's current tree, with the
    /// caret's delimiter pair painted on top.
    pub(in crate::view) fn rebind_file_editor_highlight_provider(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let live = self.file_editor_live_syntax.as_ref().map(|document| {
            (
                document.version(),
                document.snapshot(crate::kit::syntax_style::SyntaxHighlightPalette::new(
                    self.theme,
                )),
            )
        });

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
        let search_style = rows::query_highlight_style(self.theme);

        let Some((version, snapshot)) = live else {
            // No wired grammar, or past the parse ceiling. Same fallback the
            // resolved output takes: line-local heuristic tokens, answered per
            // window so the cost stays proportional to the viewport. Bracket
            // matching needs a tree, so it stays off here.
            self.file_editor_syntax_pair = None;
            self.file_editor_occurrences.clear();
            self.file_editor_occurrences_version = None;
            let theme = self.theme;
            let language = self.file_editor_language;
            let rope = snapshot.rope();
            let binding_key = file_editor_heuristic_provider_binding_key(
                (snapshot.model_id(), snapshot.revision()),
                language,
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
        self.file_editor_syntax_pair = (!has_selection)
            .then(|| snapshot.syntax_pair_at(cursor))
            .flatten();
        // A caret moving inside the name it is already on changes nothing, and
        // the scan is O(document) -- so hold the answer until the caret leaves
        // the set or the text changes under it.
        //
        // Asked of the tree rather than by comparing the caret against each
        // cached range: an inclusive range test also accepts the offset where
        // the *next* name begins, because two name tokens can abut -- Perl's
        // `$foo$bar` is `scalar_variable` 0..4 followed by 4..8. The caret
        // reaching 4 from the left then kept `$foo` lit while it sat on `$bar`,
        // and reaching the same offset from the right lit `$bar`. One tree
        // descent is both exact and cheaper than the scan it is guarding.
        let occurrences_still_apply = self.file_editor_occurrences_version == Some(version)
            && snapshot
                .name_token_at(cursor)
                .is_some_and(|token| self.file_editor_occurrences.contains(&token));
        if has_selection {
            self.file_editor_occurrences = Vec::new();
            self.file_editor_occurrences_version = None;
        } else if !occurrences_still_apply {
            self.file_editor_occurrences = snapshot.occurrences_at(cursor);
            self.file_editor_occurrences_version = Some(version);
        }

        let pair = self.file_editor_syntax_pair.clone();
        let occurrences = self.file_editor_occurrences.clone();
        let binding_key = file_editor_provider_binding_key(
            version,
            self.file_editor_provider_theme_epoch,
            pair.as_ref(),
            &search_overlay,
            &occurrences,
        );
        let occurrence_style = gpui::HighlightStyle {
            background_color: Some(
                self.theme
                    .colors
                    .editor
                    .occurrence_highlight_background
                    .into_color(),
            ),
            ..Default::default()
        };
        let pair_style = gpui::HighlightStyle {
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
                highlights: apply_file_editor_pair_highlights(
                    apply_file_editor_occurrence_highlights(
                        apply_file_editor_search_highlights(
                            snapshot.highlights_for_byte_range(byte_range.clone()),
                            &search_overlay,
                            byte_range.clone(),
                            search_style,
                        ),
                        &occurrences,
                        byte_range.clone(),
                        occurrence_style,
                    ),
                    pair.as_ref(),
                    byte_range,
                    pair_style,
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
        window: &gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        if let Some(message) = self.file_editor_error().clone() {
            return div().flex().flex_col().p_4().gap_2()
                .child(components::empty_state(theme, "Edit", message))
                .when(self.file_editor_dirty(), |d| d.child(div().flex().gap_2()
                    .child(components::Button::new("editor_keep_buffer", "Return to buffer").on_click(theme, cx, |this, _, _, cx| { if let Some(document) = this.editor_documents.active() { document.dismiss_error(cx); } cx.notify(); }))
                    .child(components::Button::new("editor_reload_disk", "Reload disk contents…").on_click(theme, cx, |_, _, window, cx| {
                        let answer = window.prompt(gpui::PromptLevel::Warning, "Discard this buffer and reload?", None, &[gpui::PromptButton::cancel("Cancel"), gpui::PromptButton::new("Reload")], cx);
                        cx.spawn_in(window, async move |view, cx| { if answer.await.ok() == Some(1) { let _ = view.update_in(cx, |this, _, cx| this.discard_file_editor_buffer(cx)); } }).detach();
                    }))
                    .child(components::Button::new("editor_replace_disk", "Replace disk contents…").on_click(theme, cx, |_, _, window, cx| {
                        let answer = window.prompt(gpui::PromptLevel::Warning, "Replace the current disk contents?", Some("This replaces the version saved by another editor."), &[gpui::PromptButton::cancel("Cancel"), gpui::PromptButton::new("Replace")], cx);
                        cx.spawn_in(window, async move |view, cx| { if answer.await.ok() == Some(1) { let _ = view.update_in(cx, |this, _, cx| {
                            this.stash_current_file_editor_buffer(cx);
                            if let Some(key) = this.file_editor_key().clone() && let Some(buffer) = this.editor_documents.stashed(&key) { let _ = this.enqueue_file_editor_save(key, &buffer, true, true, cx); }
                        }); } }).detach();
                    }))
                )).into_any_element();
        }
        if self.file_editor_loading() {
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
                self.file_editor_input.update(cx, |input, cx| {
                    input.set_selected_range(range, true, window, cx)
                });
                // The caret has moved but not been laid out at its new place yet,
                // so the sideways half waits for the frame that paints it. The
                // input's own caret autoscroll only handles the vertical axis.
                self.file_editor_search_reveal_x_pending = !self.diff_word_wrap;
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

        // The input's line height is UI-scaled (`DiffRenderer::new`), so the
        // gutter's rows and padding have to be too — a flat 20px row drifts a
        // whole line out of step every third line at 150%.
        let ui_scale_percent = ui_scale::current(cx).percent;
        let row_height = self.theme.editor_row_height(ui_scale_percent);
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
        let gutter_width = file_editor_gutter_width(
            line_count,
            show_line_numbers,
            ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
        ) + blame_width;
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
        let scrollbar_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
        let horizontal_gutter = if soft_wrap {
            px(0.0)
        } else {
            components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal)
        };
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
                        .pb(horizontal_gutter)
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
                    .pb(horizontal_gutter)
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
                            .when(soft_wrap, |d| {
                                restrict_scroll_to_vertical_axis(
                                    d.overflow_hidden().overflow_y_scroll(),
                                )
                            })
                            .when(!soft_wrap, |d| d.overflow_scroll())
                            .track_scroll(&self.file_editor_scroll)
                            .child(self.file_editor_input.clone()),
                    )
                    // The track must be outside the moving scroll surface or
                    // GPUI applies the content offset to the scrollbar itself.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom(horizontal_gutter)
                            .w(scrollbar_gutter)
                            .child(editor_scrollbar.render(theme)),
                    )
                    .when(!soft_wrap, |container| {
                        let scrollbar = components::Scrollbar::horizontal(
                            "file_editor_hscrollbar",
                            editor_scroll.clone(),
                        );
                        #[cfg(test)]
                        let scrollbar = scrollbar.debug_selector("file_editor_hscrollbar");
                        container.child(
                            div()
                                .absolute()
                                .left_0()
                                .right(scrollbar_gutter)
                                .bottom_0()
                                .h(horizontal_gutter)
                                .child(scrollbar.render(theme)),
                        )
                    }),
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
        let first_dirty_line = this.file_editor_first_dirty_line();
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
                                .text_size(theme.ui_text(12.0))
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
    scale: impl Into<ui_scale::UiScale>,
) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    if !show_line_numbers {
        return px(0.0);
    }
    // `px_2` on each side, scaled with everything else so the number cell is not
    // clipped once the digits grow with the UI. The digit width itself is already
    // scaled by `resolved_output_line_no_width`.
    let padding = ui_scale::design_px_from_percent(8.0 + 8.0, ui_scale_percent);
    rows::resolved_output_line_no_width(line_count, scale) + padding
}
