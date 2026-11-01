use super::DiffRenderer;
use super::services::DiffCommand;
use crate::view::{DiffTextRegion, DiffViewMode, components};
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::file_diff::FileDiffRowKind;
use gitcomet_core::patch_action::{
    PatchActionError, PatchOperation, PreparedDiffPatch, SourceLineSelection,
};
use gitcomet_extension_api::{HostedAction, HostedMenuItem};
use gitcomet_state::diff_session::DiffViewId;
use gitcomet_state::{model::RepoId, msg::Msg};
use gpui::{Pixels, Point, Window};
use std::sync::Arc;

#[cfg(test)]
#[path = "diff_patch_tests.rs"]
mod tests;

#[derive(Clone)]
struct PatchOrigin {
    repo_id: RepoId,
    lifetime: u64,
    view: Option<DiffViewId>,
    file_rev: u64,
    generation: u64,
    cache_seq: u64,
    target: DiffTarget,
    operation: PatchOperation,
}

impl DiffRenderer {
    pub(super) fn structural_source_lines(
        &self,
        rows: impl IntoIterator<Item = usize>,
        region: Option<DiffTextRegion>,
    ) -> SourceLineSelection {
        let mut selection = SourceLineSelection::default();
        for index in rows {
            let (old, new) = match self.diff_view {
                DiffViewMode::Split => self
                    .file_diff_split_row(index)
                    .map(|row| (row.old_line, row.new_line)),
                DiffViewMode::Inline => self
                    .file_diff_inline_row(index)
                    .map(|row| (row.old_line, row.new_line)),
            }
            .unwrap_or((None, None));
            if region != Some(DiffTextRegion::SplitRight) {
                selection.old.extend(old);
            }
            if region != Some(DiffTextRegion::SplitLeft) {
                selection.new.extend(new);
            }
        }
        selection
    }

    /// The source lines that `region` of `row` stands for when the user acts
    /// on the row itself (its gutter button, a right-click without a
    /// selection). Split draws a pair changed on one side as one change in the
    /// changed column, so it stands for both lines, as its one Inline row does.
    pub(super) fn structural_row_lines(
        &self,
        row: usize,
        region: DiffTextRegion,
    ) -> SourceLineSelection {
        let one_sided_pair = self.diff_view == DiffViewMode::Split
            && self
                .file_diff_split_row(row)
                .is_some_and(|row| row.kind == FileDiffRowKind::Context);
        self.structural_source_lines([row], (!one_sided_pair).then_some(region))
    }

    pub(super) fn open_structural_line_menu(
        &mut self,
        visible: usize,
        region: DiffTextRegion,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(selection) = self.structural_line_selection(visible, region) else {
            return;
        };
        let copy = self
            .selected_diff_text_string()
            .or_else(|| self.diff_text_string_for_region(visible, region));
        let mut scopes = vec![("selected lines", selection)];
        if let Some(block) = self
            .diff_mapped_ix_for_visible_ix(visible)
            .and_then(|row| self.structural_block_lines(row))
        {
            scopes.push(("block", block));
        }
        self.open_structural_menu(scopes, copy, anchor, window, cx);
    }

    pub(super) fn structural_line_selection(
        &self,
        visible: usize,
        region: DiffTextRegion,
    ) -> Option<SourceLineSelection> {
        let row = self.diff_source_visible_ix_for_visible_ix(visible)?;
        let selected = self.diff_text_selection_line_rows_at(self.diff_view, row, region);
        if selected.is_none()
            && let Some((a, b)) = self.diff_selection_range
            && (a.min(b)..=a.max(b)).contains(&visible)
        {
            return Some(self.structural_source_lines(
                (a.min(b)..=a.max(b)).filter_map(|row| self.diff_mapped_ix_for_visible_ix(row)),
                Some(region),
            ));
        }
        let Some((first, last)) = selected else {
            return Some(
                self.diff_source_mapped_ix_for_visible_ix(row)
                    .map(|row| self.structural_row_lines(row, region))
                    .unwrap_or_default(),
            );
        };
        let regions: &[DiffTextRegion] = match self.diff_view {
            DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
            DiffViewMode::Inline => &[DiffTextRegion::Inline],
        };
        let mut selection = SourceLineSelection::default();
        for row in first..=last {
            let Some(source) = self.diff_source_mapped_ix_for_visible_ix(row) else {
                continue;
            };
            for &side in regions {
                if self.diff_text_selection_takes_line(row, side) {
                    let lines = self.structural_source_lines([source], Some(side));
                    selection.old.extend(lines.old);
                    selection.new.extend(lines.new);
                }
            }
        }
        Some(selection)
    }

    pub(super) fn open_structural_block_menu(
        &mut self,
        source: usize,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(selection) = self.structural_block_lines(source) {
            let line = selection
                .new
                .first()
                .or_else(|| selection.old.first())
                .copied();
            if let Some(line) = line {
                self.open_structural_menu(
                    vec![(&format!("block at line {line}"), selection)],
                    None,
                    anchor,
                    window,
                    cx,
                );
            }
        }
    }

    pub(super) fn structural_block_lines(&self, row: usize) -> Option<SourceLineSelection> {
        let blocks = match self.diff_view {
            DiffViewMode::Split => self
                .file_diff_row_provider
                .as_ref()
                .map(|provider| provider.change_blocks()),
            DiffViewMode::Inline => self
                .file_diff_inline_row_provider
                .as_ref()
                .map(|provider| provider.change_blocks()),
        }
        .unwrap_or_default();
        blocks
            .into_iter()
            .find(|block| block.contains(&row))
            .map(|block| self.structural_source_lines(block, None))
    }

    fn open_structural_menu(
        &mut self,
        scopes: Vec<(&str, SourceLineSelection)>,
        copy: Option<String>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut items = vec![HostedMenuItem::Header(
            "Difftastic · whole source lines".into(),
        )];
        if let Some(copy) = copy {
            items.push(HostedMenuItem::action(HostedAction::new(
                "Copy",
                move |cx| {
                    gitcomet_ui_kit::clipboard::write_text(
                        cx,
                        copy.clone(),
                        gitcomet_ui_kit::clipboard::CopySource::DiffContextMenu,
                    )
                },
            )));
        }
        if let Some(DiffTarget::WorkingTree { area, .. }) = self.rendered_diff_target() {
            let operations: &[PatchOperation] = match area {
                DiffArea::Unstaged => &[PatchOperation::Stage, PatchOperation::Discard],
                DiffArea::Staged => &[PatchOperation::Unstage],
            };
            for (label, selection) in scopes {
                for &operation in operations {
                    let allowed = self.patch_operation_allowed(operation);
                    let weak = cx.entity().downgrade();
                    let selection = selection.clone();
                    let origin = self.patch_origin(operation);
                    let item = HostedMenuItem::action(HostedAction::new(
                        format!("{} {label}", operation.label()),
                        move |cx| {
                            if let Some(origin) = &origin {
                                let _ = weak.update(cx, |pane, cx| {
                                    pane.prepare_structural_patch_from_origin(
                                        selection.clone(),
                                        origin.clone(),
                                        cx,
                                    )
                                });
                            }
                        },
                    ));
                    items.push(item.disabled(!allowed));
                }
            }
        }
        self.commands.defer(
            DiffCommand::OpenHostedMenu {
                id: crate::view::extension_host::next_dialog_id(),
                items,
                anchor,
                window: window.window_handle(),
            },
            cx,
        );
    }

    fn patch_operation_allowed(&self, operation: PatchOperation) -> bool {
        match operation {
            PatchOperation::Stage | PatchOperation::Unstage => self.store.policy.allow_stage,
            PatchOperation::Discard => self.store.policy.allow_edit,
        }
    }

    fn patch_origin_current(&self, origin: &PatchOrigin) -> bool {
        self.structural_diff_active()
            && self.file_diff_cache_seq == origin.cache_seq
            && self.store.binding.map(|binding| binding.view) == origin.view
            && self.patch_generation() == Some(origin.generation)
            && self.patch_operation_allowed(origin.operation)
            && self.active_repo().is_some_and(|repo| {
                let diff = self.bound_diff_state(repo);
                repo.id == origin.repo_id
                    && repo.lifetime() == origin.lifetime
                    && repo.local_actions_in_flight == 0
                    && !diff.diff_reload_in_flight
                    && diff.diff_file_rev == origin.file_rev
                    && diff.diff_target.as_ref() == Some(&origin.target)
            })
    }

    fn patch_generation(&self) -> Option<u64> {
        let repo = self.active_repo()?;
        match self.store.binding {
            Some(binding) => repo
                .diff_sessions
                .get(&binding.view)
                .map(|session| session.generation),
            None => Some(repo.diff_state.diff_target_rev),
        }
    }

    pub(super) fn prepare_structural_patch(
        &mut self,
        selection: SourceLineSelection,
        operation: PatchOperation,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(origin) = self.patch_origin(operation) {
            self.prepare_structural_patch_from_origin(selection, origin, cx);
        }
    }

    fn patch_origin(&self, operation: PatchOperation) -> Option<PatchOrigin> {
        let repo = self.active_repo()?;
        let target = self.rendered_diff_target()?.clone();
        Some(PatchOrigin {
            repo_id: repo.id,
            lifetime: repo.lifetime(),
            view: self.store.binding.map(|binding| binding.view),
            file_rev: self.bound_diff_state(repo).diff_file_rev,
            generation: self.patch_generation()?,
            cache_seq: self.file_diff_cache_seq,
            target,
            operation,
        })
    }

    /// Prepares the exact Git patch for `selection` off the UI thread, then
    /// applies it, as the line engine applies its line and hunk patches.
    fn prepare_structural_patch_from_origin(
        &mut self,
        selection: SourceLineSelection,
        origin: PatchOrigin,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.patch_origin_current(&origin) {
            self.toast_stale_patch(cx);
            return;
        }
        let store = self.store.clone();
        let cancellation = self.patch_cancellation.clone();
        let old = self.file_diff_old_text.clone();
        let new = self.file_diff_new_text.clone();
        cx.spawn(async move |weak, cx| {
            let prepare_origin = origin.clone();
            let result = crate::ui_runtime::background_compute(move || {
                let repo = store
                    .repository(prepare_origin.repo_id, prepare_origin.lifetime)
                    .ok_or_else(|| {
                        "The repository was closed. Open a new comparison.".to_owned()
                    })?;
                repo.prepare_diff_patch(
                    &prepare_origin.target,
                    prepare_origin.operation,
                    &selection,
                    &old,
                    &new,
                    &cancellation,
                )
                .map_err(|error| error.to_string())
            })
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.apply_structural_patch(origin, result, cx)
            });
        })
        .detach();
    }

    fn apply_structural_patch(
        &mut self,
        origin: PatchOrigin,
        result: Result<PreparedDiffPatch, String>,
        cx: &mut gpui::Context<Self>,
    ) {
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(message) => {
                self.commands.send(
                    DiffCommand::Toast(components::ToastKind::Error, message.into()),
                    cx,
                );
                return;
            }
        };
        if !self.patch_origin_current(&origin) {
            self.toast_stale_patch(cx);
            return;
        }
        self.store.dispatch(Msg::ApplyPreparedDiffPatch {
            repo_id: origin.repo_id,
            lifetime: origin.lifetime,
            view: origin.view,
            file_rev: origin.file_rev,
            generation: origin.generation,
            prepared: Arc::new(prepared),
        });
    }

    fn toast_stale_patch(&self, cx: &mut gpui::Context<Self>) {
        self.commands.send(
            DiffCommand::Toast(
                components::ToastKind::Error,
                PatchActionError::Stale.to_string().into(),
            ),
            cx,
        );
    }
}
