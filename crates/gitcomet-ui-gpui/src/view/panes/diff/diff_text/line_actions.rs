//! Stage, unstage and discard the selected diff lines.
//!
//! The diff editor menu lists its line entries with a bare letter beside them
//! (S, U, D). Those letters work with the menu closed too: they build the same
//! patch from the same selection and apply it through the same messages.

use super::{
    DiffArea, DiffEditorMenuClick, DiffLineSelectionState, DiffRenderer, DiffTarget,
    DiffTextRegion, DiffViewMode, FxHashMap, FxHashSet,
    build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply,
};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;
use gitcomet_core::patch_action::{PatchOperation, SourceLineSelection};
use gitcomet_state::msg::{ContentBytes, Msg};
use std::path::{Path, PathBuf};

/// A diff editor menu line entry that answers to a bare letter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiffLineAction {
    /// "Stage line(s)", S, in an unstaged diff.
    Stage,
    /// "Unstage line(s)", U, in a staged diff.
    Unstage,
    /// "Discard line(s)", D, in an unstaged diff.
    Discard,
}

impl DiffLineAction {
    /// The entry a key names. Only the bare letter, as the menu shows it:
    /// Ctrl/Cmd+S and Ctrl/Cmd+D act on the whole file.
    pub(in crate::view) fn from_keystroke(keystroke: &gpui::Keystroke) -> Option<Self> {
        let mods = keystroke.modifiers;
        if mods.control || mods.alt || mods.platform || mods.function || mods.shift {
            return None;
        }
        match keystroke.key.as_str() {
            "s" => Some(Self::Stage),
            "u" => Some(Self::Unstage),
            "d" => Some(Self::Discard),
            _ => None,
        }
    }

    /// The side of the index whose diff lists this entry.
    fn area(self) -> DiffArea {
        match self {
            Self::Stage | Self::Discard => DiffArea::Unstaged,
            Self::Unstage => DiffArea::Staged,
        }
    }

    fn operation(self) -> PatchOperation {
        match self {
            Self::Stage => PatchOperation::Stage,
            Self::Unstage => PatchOperation::Unstage,
            Self::Discard => PatchOperation::Discard,
        }
    }
}

/// Finds the patch lines behind visible rows. The whole-file view draws the
/// file's own lines, not the patch's, so its rows are matched back to their
/// patch lines by line number.
pub(super) struct DiffLineSources {
    file_diff: Option<FileDiffSources>,
}

struct FileDiffSources {
    file_rel: PathBuf,
    add_by_new_line: FxHashMap<u32, usize>,
    remove_by_old_line: FxHashMap<u32, usize>,
    context_by_old_line: FxHashMap<u32, usize>,
}

impl DiffLineSources {
    /// The file a whole-file view shows, relative to the working directory.
    pub(super) fn file_rel(&self) -> Option<&Path> {
        self.file_diff.as_ref().map(|file| file.file_rel.as_path())
    }
}

/// The patch lines a selection of rows covers.
pub(super) struct SelectedDiffLines {
    /// Every patch line under the selection, context included.
    pub(super) src_ixs: FxHashSet<usize>,
    /// The added and removed lines among them: the ones a line action moves.
    pub(super) change_src_ixs: FxHashSet<usize>,
}

impl DiffRenderer {
    pub(super) fn diff_line_sources(&self, workdir: &Path) -> DiffLineSources {
        // Collapsed hunks list the same file rows as the full file view, so
        // they need the lookup too: without it every selected row would scan
        // the whole patch for its line.
        if !self.lists_file_diff_rows() {
            return DiffLineSources { file_diff: None };
        }
        let file_diff = self.file_diff_cache_path.as_ref().map(|abs| {
            let file_rel = abs.strip_prefix(workdir).unwrap_or(abs).to_path_buf();
            // Git diffs use forward slashes even on Windows.
            let rel_str = file_rel.to_str().map(|text| text.replace('\\', "/"));

            // The maps only hold patch lines, which in a working-tree diff are
            // the changes and their context: far fewer than the file's rows.
            let approx_map_len = self.patch_diff_row_len();
            let map = || FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());
            let mut add_by_new_line: FxHashMap<u32, usize> = map();
            let mut remove_by_old_line: FxHashMap<u32, usize> = map();
            let mut context_by_old_line: FxHashMap<u32, usize> = map();

            for ix in 0..self.patch_diff_row_len() {
                let Some(line) = self.patch_diff_row(ix) else {
                    continue;
                };
                if self.diff_file_for_src_ix.get(ix).and_then(|p| p.as_deref())
                    != rel_str.as_deref()
                {
                    continue;
                }
                match line.kind {
                    DiffLineKind::Add => {
                        if let Some(n) = line.new_line {
                            add_by_new_line.insert(n, ix);
                        }
                    }
                    DiffLineKind::Remove => {
                        if let Some(o) = line.old_line {
                            remove_by_old_line.insert(o, ix);
                        }
                    }
                    DiffLineKind::Context => {
                        if let Some(o) = line.old_line {
                            context_by_old_line.insert(o, ix);
                        }
                    }
                    DiffLineKind::Header | DiffLineKind::Hunk => {}
                }
            }

            FileDiffSources {
                file_rel,
                add_by_new_line,
                remove_by_old_line,
                context_by_old_line,
            }
        });
        DiffLineSources { file_diff }
    }

    /// The patch lines a visible row draws: two for a split row pairing a
    /// removal with its replacement.
    pub(super) fn diff_line_src_ixs(
        &self,
        sources: &DiffLineSources,
        visible_ix: usize,
    ) -> Vec<usize> {
        let Some(lookup) = sources.file_diff.as_ref() else {
            return self.diff_src_ixs_for_visible_ix(visible_ix);
        };
        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            // A collapsed hunk header maps to no file row but acts on its hunk.
            return self.diff_src_ixs_for_visible_ix(visible_ix);
        };
        match self.diff_view {
            DiffViewMode::Inline => {
                let Some(line) = self.file_diff_inline_render_data(mapped_ix) else {
                    return Vec::new();
                };
                match line.kind {
                    DiffLineKind::Add => line
                        .new_line
                        .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                        .into_iter()
                        .collect(),
                    DiffLineKind::Remove => line
                        .old_line
                        .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                        .into_iter()
                        .collect(),
                    DiffLineKind::Context => line
                        .old_line
                        .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                        .into_iter()
                        .collect(),
                    DiffLineKind::Header | DiffLineKind::Hunk => Vec::new(),
                }
            }
            DiffViewMode::Split => {
                let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                    return Vec::new();
                };
                match row.kind {
                    FileDiffRowKind::Context => row
                        .old_line
                        .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                        .into_iter()
                        .collect(),
                    FileDiffRowKind::Add => row
                        .new_line
                        .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                        .into_iter()
                        .collect(),
                    FileDiffRowKind::Remove => row
                        .old_line
                        .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                        .into_iter()
                        .collect(),
                    FileDiffRowKind::Modify => {
                        let mut out = Vec::with_capacity(2);
                        if let Some(o) = row.old_line
                            && let Some(ix) = lookup.remove_by_old_line.get(&o).copied()
                        {
                            out.push(ix);
                        }
                        if let Some(n) = row.new_line
                            && let Some(ix) = lookup.add_by_new_line.get(&n).copied()
                            && !out.contains(&ix)
                        {
                            out.push(ix);
                        }
                        out
                    }
                }
            }
        }
    }

    /// The patch lines under visible rows `start..=end`.
    pub(super) fn selected_diff_lines(
        &self,
        sources: &DiffLineSources,
        (start, end): (usize, usize),
    ) -> SelectedDiffLines {
        let approx_selected = end
            .saturating_sub(start)
            .saturating_add(1)
            .saturating_mul(2);
        let mut src_ixs: FxHashSet<usize> =
            FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());
        let mut change_src_ixs: FxHashSet<usize> =
            FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());
        for visible_ix in start..=end {
            for src_ix in self.diff_line_src_ixs(sources, visible_ix) {
                // The kind table avoids cloning each line just to read its kind.
                let Some(kind) = self
                    .diff_line_kind_for_src_ix
                    .get(src_ix)
                    .copied()
                    .or_else(|| self.patch_diff_row(src_ix).map(|line| line.kind))
                else {
                    continue;
                };
                src_ixs.insert(src_ix);
                if matches!(kind, DiffLineKind::Add | DiffLineKind::Remove) {
                    change_src_ixs.insert(src_ix);
                }
            }
        }
        SelectedDiffLines {
            src_ixs,
            change_src_ixs,
        }
    }

    /// Whether this view lists `action` in its diff editor menu at all.
    fn diff_line_action_available(&self, action: DiffLineAction) -> bool {
        let policy = &self.store.policy;
        if !policy.allow_stage || (action == DiffLineAction::Discard && !policy.allow_edit) {
            return false;
        }
        let Some(repo) = self.active_repo() else {
            return false;
        };
        matches!(
            self.rendered_diff_target(),
            Some(DiffTarget::WorkingTree { area, .. }) if *area == action.area()
        ) && !self.bound_diff_state(repo).content_preview
            && !self.is_file_preview_active()
            && !self.is_file_editor_active()
            && !self.is_conflict_resolver_active()
            && !self.is_inline_submodule_diff_active()
            && !self.is_markdown_preview_active()
            // The displayed payload lines do not address the stored pointer.
            && !self.has_large_file_text_diff()
    }

    /// The visible rows of the change block navigation outlined, while it is
    /// on screen. A mouse selection removes the outline.
    fn focused_block_visible_rows(&self) -> Option<(usize, usize)> {
        let rows = &self.live_focused_change_block()?.rows;
        let last = rows.end.checked_sub(1).filter(|last| *last >= rows.start)?;
        self.diff_text_visible_range_for_source_range(rows.start, last)
    }

    /// What chooses the lines a line action takes, for telling whether it
    /// changed since the diff editor menu opened.
    fn diff_line_selection_state(&self) -> DiffLineSelectionState {
        DiffLineSelectionState {
            target: self.rendered_diff_target().cloned(),
            layout: self.diff_visible_layout_key(),
            row_anchor: self.diff_selection_anchor,
            rows: self.diff_selection_range,
            text: (self.diff_text_anchor, self.diff_text_head),
            block: self.diff_focused_change_block.clone(),
        }
    }

    /// Remembers the press that opened the diff editor menu, once it has
    /// settled the selection the menu acts on.
    pub(in crate::view::panes::diff) fn remember_diff_editor_menu_click(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) {
        self.diff_editor_menu_click = Some(DiffEditorMenuClick {
            visible_ix,
            region,
            seen: self.diff_line_selection_state(),
        });
    }

    /// The press that opened the diff editor menu, while nothing has changed
    /// the lines that menu acted on.
    fn live_diff_editor_menu_click(&self) -> Option<(usize, DiffTextRegion)> {
        self.diff_editor_menu_click
            .as_ref()
            .filter(|click| click.seen == self.diff_line_selection_state())
            .map(|click| (click.visible_ix, click.region))
    }

    /// The visible rows a line key acts on: the lines the diff editor menu
    /// acted on while its selection stands, else the lines the text selection
    /// takes, else the selected rows, else the change block navigation
    /// outlined. A bare caret selects nothing.
    fn diff_line_action_rows(&self) -> Option<(usize, usize)> {
        let len = self.diff_visible_len();
        if len == 0 {
            return None;
        }
        if let Some((visible_ix, region)) = self.live_diff_editor_menu_click() {
            return self.diff_editor_menu_rows(self.diff_view, visible_ix, region, len);
        }
        let (start, end) = self
            .diff_text_selection_line_rows(self.diff_view)
            .and_then(|(first, last)| self.diff_text_visible_range_for_source_range(first, last))
            .or_else(|| self.diff_selection_range.map(|(a, b)| (a.min(b), a.max(b))))
            .or_else(|| self.focused_block_visible_rows())?;
        Some((start.min(len - 1), end.min(len - 1)))
    }

    /// The patch `action` applies to the selected lines, built exactly as the
    /// diff editor menu builds that entry's. `None` when nothing is selected
    /// or no added or removed line is.
    #[cfg(test)]
    pub(in crate::view) fn selected_diff_lines_patch(
        &self,
        action: DiffLineAction,
    ) -> Option<ContentBytes> {
        self.diff_lines_patch_for_rows(action, self.diff_line_action_rows()?)
    }

    fn diff_lines_patch_for_rows(
        &self,
        action: DiffLineAction,
        rows: (usize, usize),
    ) -> Option<ContentBytes> {
        let sources = self.diff_line_sources(self.rendered_diff_workdir()?);
        let selected = self.selected_diff_lines(&sources, rows);
        if selected.change_src_ixs.is_empty() {
            return None;
        }
        let diff = self.patch_diff_rows_slice(0, self.patch_diff_row_len());
        // Staging applies the lines forward to the index. Unstaging applies
        // them in reverse to the index and discarding in reverse to the
        // working tree, which needs the opposite treatment of the unselected
        // changes around them or git rejects the patch.
        match action {
            DiffLineAction::Stage => {
                build_unified_patch_for_selected_lines_across_hunks(&diff, &selected.change_src_ixs)
            }
            DiffLineAction::Unstage | DiffLineAction::Discard => {
                build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                    &diff,
                    &selected.change_src_ixs,
                )
            }
        }
        .map(Into::into)
    }

    /// The source lines a line key acts on in a structural diff: the lines
    /// the diff editor menu acted on while its selection stands, else the
    /// lines the text selection takes, else both sides of the selected rows or
    /// of the outlined change block.
    pub(in crate::view::panes::diff) fn structural_selected_lines(
        &self,
    ) -> Option<SourceLineSelection> {
        if let Some((visible_ix, region)) = self.live_diff_editor_menu_click() {
            return self.structural_line_selection(visible_ix, region);
        }
        if let Some((first, last)) = self.diff_text_selection_line_rows(self.diff_view) {
            return Some(self.structural_text_selection_lines(first, last));
        }
        let (a, b) = self
            .diff_selection_range
            .map(|(a, b)| (a.min(b), a.max(b)))
            .or_else(|| self.focused_block_visible_rows())?;
        Some(self.structural_source_lines(
            (a..=b).filter_map(|row| self.diff_mapped_ix_for_visible_ix(row)),
            None,
        ))
    }

    /// A line key pressed outside the diff acts only on the change block
    /// opening the diff or change navigation outlined. Focus inside the
    /// diff goes through `handle_diff_shortcut` instead.
    pub(in crate::view) fn apply_outlined_block_line_action(
        &mut self,
        action: DiffLineAction,
        window: &gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.is_conflict_resolver_active()
            || self.diff_panel_focus_handle.contains_focused(window, cx)
            || self.live_focused_change_block().is_none()
        {
            return false;
        }
        self.apply_diff_line_action(action, cx)
    }

    /// Stages, unstages or discards the selected lines, as the diff editor
    /// menu entry showing the key does. Returns whether the key was taken: a
    /// view without that entry, or nothing selected to act on, leaves it.
    pub(in crate::view) fn apply_diff_line_action(
        &mut self,
        action: DiffLineAction,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !self.diff_line_action_available(action) {
            return false;
        }
        if self.structural_diff_active() {
            let Some(selection) = self
                .structural_selected_lines()
                .filter(|selection| !selection.old.is_empty() || !selection.new.is_empty())
            else {
                return false;
            };
            // While an earlier action applies, the rows still show the diff
            // before it: take the key, but build nothing from them.
            if !self.diff_actions_busy() {
                self.prepare_structural_patch(selection, action.operation(), cx);
            }
            return true;
        }
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let Some(rows) = self.diff_line_action_rows() else {
            return false;
        };
        // As above: a patch built from stale rows would not apply.
        if self.diff_actions_busy() {
            return true;
        }
        let Some(patch) = self.diff_lines_patch_for_rows(action, rows) else {
            return false;
        };
        self.store.dispatch(match action {
            DiffLineAction::Stage => Msg::StageHunk { repo_id, patch },
            DiffLineAction::Unstage => Msg::UnstageHunk { repo_id, patch },
            DiffLineAction::Discard => Msg::ApplyWorktreePatch {
                repo_id,
                patch,
                reverse: true,
            },
        });
        true
    }
}
