//! Stage/unstage of a single diff line from the hover button painted in a diff
//! row's left gutter. The button is only an entry point: it reuses the same
//! patch builder and messages as the diff context menu's "Stage line(s)".

use super::*;
use gitcomet_core::domain::DiffLineKind;

impl DiffRenderer {
    /// Which side of the index the rendered diff is on, when its lines can be
    /// staged one by one at all. `None` disables the gutter button: commit and
    /// commit-range diffs have no index to move lines to, and the preview,
    /// conflict and submodule modes render something other than a patch.
    pub(in crate::view) fn diff_stage_gutter_area(&self) -> Option<DiffArea> {
        if !self.store.policy.allow_stage || !self.store.policy.line_action {
            return None;
        }
        // Cheap check first: this runs once per rendered frame, and the preview
        // probes below stat the filesystem.
        let area = match self.rendered_diff_target()? {
            DiffTarget::WorkingTree { area, .. } => *area,
            DiffTarget::Commit { .. } | DiffTarget::CommitRange { .. } => return None,
        };
        if self.is_file_preview_active()
            || self.is_conflict_resolver_active()
            || self.is_inline_submodule_diff_active()
            || self.is_markdown_preview_active()
            // The displayed payload lines do not address the stored pointer.
            || self.has_large_file_text_diff()
        {
            return None;
        }
        Some(area)
    }

    /// Patch source index of the change line a row's gutter button acts on.
    /// A split row can map to both an added and a removed line, so `kind`
    /// selects the one belonging to the button's own column.
    fn diff_stage_gutter_src_ix(&self, visible_ix: usize, kind: DiffLineKind) -> Option<usize> {
        self.diff_src_ixs_for_visible_ix(visible_ix)
            .into_iter()
            .find(|src_ix| {
                self.patch_diff_row(*src_ix)
                    .is_some_and(|line| line.kind == kind)
            })
    }

    /// Unified patch for exactly the one line the gutter button belongs to,
    /// built for the direction it will be applied in: an unstaged diff stages
    /// the line by applying it forward to the index, a staged diff unstages it
    /// by applying it in reverse. The two need different treatment of the
    /// neighbouring changes they leave behind, so the direction has to be
    /// decided here rather than at dispatch.
    pub(in crate::view) fn diff_stage_gutter_patch(
        &self,
        visible_ix: usize,
        kind: DiffLineKind,
    ) -> Option<gitcomet_state::msg::ContentBytes> {
        let area = self.diff_stage_gutter_area()?;
        let src_ix = self.diff_stage_gutter_src_ix(visible_ix, kind)?;
        let rows = self.patch_diff_rows_slice(0, self.patch_diff_row_len());
        let mut selected = FxHashSet::default();
        selected.insert(src_ix);
        match area {
            DiffArea::Unstaged => {
                crate::view::diff_utils::build_unified_patch_for_selected_lines_across_hunks(
                    &rows, &selected,
                )
            }
            DiffArea::Staged => {
                crate::view::diff_utils::build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                    &rows, &selected,
                )
            }
        }
        .map(Into::into)
    }

    /// Whether an earlier stage or discard is still applying or reloading the
    /// diff. The rows stay on screen meanwhile (so the pane does not flash) and
    /// still describe the index as it was, so a gutter click is dropped rather
    /// than built into a patch that no longer applies.
    ///
    /// Both halves matter: the command finishing is what clears
    /// `local_actions_in_flight`, and the reload it asks for only starts
    /// there, so between them the rows are at their most stale.
    pub(in crate::view::panes::diff) fn diff_actions_busy(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            repo.local_actions_in_flight > 0 || self.bound_diff_state(repo).diff_reload_in_flight
        })
    }

    /// Apply the gutter button: stage the line in an unstaged diff, unstage it in
    /// a staged one. The reducer reloads the diff once the command finishes, so
    /// there is nothing to refresh here.
    pub(in crate::view) fn stage_or_unstage_diff_line(
        &mut self,
        visible_ix: usize,
        kind: DiffLineKind,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(area) = self.diff_stage_gutter_area() else {
            return;
        };
        if self.diff_actions_busy() {
            return;
        }
        if self.structural_diff_active() {
            let region = match (self.diff_view, kind) {
                (DiffViewMode::Inline, _) => DiffTextRegion::Inline,
                (_, DiffLineKind::Remove) => DiffTextRegion::SplitLeft,
                _ => DiffTextRegion::SplitRight,
            };
            if let Some(row) = self.diff_mapped_ix_for_visible_ix(visible_ix) {
                let selection = self.structural_row_lines(row, region);
                let operation = match area {
                    DiffArea::Unstaged => gitcomet_core::patch_action::PatchOperation::Stage,
                    DiffArea::Staged => gitcomet_core::patch_action::PatchOperation::Unstage,
                };
                self.prepare_structural_patch(selection, operation, cx);
            }
            return;
        }
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(patch) = self.diff_stage_gutter_patch(visible_ix, kind) else {
            let message = match area {
                DiffArea::Unstaged => "Couldn't build patch to stage this line",
                DiffArea::Staged => "Couldn't build patch to unstage this line",
            };
            self.commands.send(
                DiffCommand::Toast(components::ToastKind::Error, message.into()),
                cx,
            );
            return;
        };

        self.store.dispatch(match area {
            DiffArea::Unstaged => Msg::StageHunk { repo_id, patch },
            DiffArea::Staged => Msg::UnstageHunk { repo_id, patch },
        });
    }
}
