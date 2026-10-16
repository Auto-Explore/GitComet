use super::*;
use crate::view::hosted::diff_pane::DiffPaneView;
use gitcomet_core::patch_action::{PatchOperation, PreparedDiffPatch, SourceLineSelection};
use gitcomet_core::services::CancellationToken;
use gitcomet_extension_api::{DiffSnapshot, HostedAction, HostedMenuItem};
use gitcomet_state::diff_session::DiffViewId;

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

enum ReviewState {
    Preparing,
    Ready {
        prepared: Arc<PreparedDiffPatch>,
        pane: Entity<DiffPaneView>,
        _subscription: gpui::Subscription,
    },
    Error(SharedString),
}

pub(super) struct PatchReview {
    sequence: u64,
    origin: PatchOrigin,
    cancellation: CancellationToken,
    state: ReviewState,
    focus: FocusHandle,
    focus_pending: bool,
}

impl PatchReview {
    pub(super) fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl MainPaneView {
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
        let selected = self
            .diff_text_normalized_selection()
            .filter(|(start, end)| {
                start != end
                    && (start.region != end.region || start.region == region)
                    && (start.source_visible_ix..=end.source_visible_ix).contains(&row)
            });
        if selected.is_none()
            && let Some((a, b)) = self.diff_selection_range
            && (a.min(b)..=a.max(b)).contains(&visible)
        {
            return Some(self.structural_source_lines(
                (a.min(b)..=a.max(b)).filter_map(|row| self.diff_mapped_ix_for_visible_ix(row)),
                Some(region),
            ));
        }
        let Some((start, end)) = selected else {
            return Some(self.structural_source_lines(
                self.diff_source_mapped_ix_for_visible_ix(row),
                Some(region),
            ));
        };
        let regions: &[DiffTextRegion] = match self.diff_view {
            DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
            DiffViewMode::Inline => &[DiffTextRegion::Inline],
        };
        let mut selection = SourceLineSelection::default();
        for row in start.source_visible_ix..=end.source_visible_ix {
            let Some(source) = self.diff_source_mapped_ix_for_visible_ix(row) else {
                continue;
            };
            for &side in regions {
                if row == end.source_visible_ix && side == end.region && end.offset == 0 {
                    continue;
                }
                let len = self.diff_text_full_line_for_region(row, side).len();
                if self
                    .diff_text_source_selection_range(row, side, len)
                    .is_some()
                {
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
                        format!("{} {label}…", operation.label()),
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
        let root = self.root_view.clone();
        let window = window.window_handle();
        let id = crate::view::extension_host::next_dialog_id();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = root.update(cx, |root, cx| {
                    root.popover_host.update(cx, |host, cx| {
                        host.open_hosted_menu(id, items, anchor, window, cx)
                    });
                });
            });
        });
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

    fn prepare_structural_patch_from_origin(
        &mut self,
        selection: SourceLineSelection,
        origin: PatchOrigin,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.patch_origin_current(&origin) {
            let _ = self.root_view.update(cx, |root, cx| {
                root.push_toast(
                    components::ToastKind::Error,
                    "The comparison changed. Select lines in the current diff and try again."
                        .into(),
                    cx,
                )
            });
            return;
        }
        let operation = origin.operation;
        let Ok(store) = self.root_view.read_with(cx, |root, _| root.store.clone()) else {
            return;
        };
        if let Some(review) = self.patch_review.take() {
            review.cancel();
        }
        self.patch_review_seq = self.patch_review_seq.wrapping_add(1);
        let sequence = self.patch_review_seq;
        let cancellation = CancellationToken::new();
        self.patch_review = Some(PatchReview {
            sequence,
            origin: origin.clone(),
            cancellation: cancellation.clone(),
            state: ReviewState::Preparing,
            focus: cx.focus_handle(),
            focus_pending: true,
        });
        let old = self.file_diff_old_text.clone();
        let new = self.file_diff_new_text.clone();
        cx.spawn(async move |weak, cx| {
            let result = smol::unblock(move || {
                let repo = store
                    .repository(origin.repo_id, origin.lifetime)
                    .ok_or_else(|| {
                        "The repository was closed. Open a new comparison.".to_owned()
                    })?;
                repo.prepare_diff_patch(
                    &origin.target,
                    operation,
                    &selection,
                    &old,
                    &new,
                    &cancellation,
                )
                .map_err(|error| error.to_string())
            })
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.complete_structural_patch(sequence, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn complete_structural_patch(
        &mut self,
        sequence: u64,
        result: Result<PreparedDiffPatch, String>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(review) = &self.patch_review else {
            return;
        };
        if review.sequence != sequence {
            return;
        }
        if !self.patch_origin_current(&review.origin) {
            self.patch_review.as_mut().unwrap().state = ReviewState::Error(
                "The comparison changed. Close this preview and review a new patch.".into(),
            );
            cx.notify();
            return;
        }
        let state = result
            .and_then(|prepared| {
                let path = match prepared.target() {
                    DiffTarget::WorkingTree { path, .. } => path.clone(),
                    _ => unreachable!(),
                };
                let diff = gitcomet_core::patch_action::parse_exact_diff(
                    prepared.target().clone(),
                    prepared.patch().to_vec(),
                )
                .map_err(|error| error.to_string())?;
                let root = self.root_view.clone();
                let pane = cx.new(|cx| {
                    DiffPaneView::exact_patch(
                        DiffSnapshot::from_patch(path, diff),
                        root,
                        self.theme,
                        cx,
                    )
                });
                let subscription = cx.observe(&pane, |_, _, cx| cx.notify());
                Ok(ReviewState::Ready {
                    prepared: Arc::new(prepared),
                    pane,
                    _subscription: subscription,
                })
            })
            .unwrap_or_else(|error| ReviewState::Error(error.into()));
        self.patch_review.as_mut().unwrap().state = state;
        cx.notify();
    }

    fn close_patch_review(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if let Some(review) = self.patch_review.take() {
            review.cancel();
        }
        self.restore_diff_panel_focus_after_toolbar_action(window, cx);
        cx.notify();
    }

    fn apply_patch_review(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(review) = &self.patch_review else {
            return;
        };
        if !self.patch_origin_current(&review.origin) {
            self.patch_review.as_mut().unwrap().state = ReviewState::Error(
                "The comparison changed. Close this preview and review a new patch.".into(),
            );
            cx.notify();
            return;
        }
        let ReviewState::Ready { prepared, pane, .. } = &review.state else {
            return;
        };
        if !pane.read(cx).exact_patch_ready(cx) {
            return;
        }
        let origin = &review.origin;
        self.store.dispatch(Msg::ApplyPreparedDiffPatch {
            repo_id: origin.repo_id,
            lifetime: origin.lifetime,
            view: origin.view,
            file_rev: origin.file_rev,
            generation: origin.generation,
            prepared: prepared.clone(),
        });
        self.close_patch_review(window, cx);
    }

    pub(super) fn render_patch_review(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let review = self.patch_review.as_ref()?;
        let current = self.patch_origin_current(&review.origin);
        let review = self.patch_review.as_mut().unwrap();
        if review.focus_pending {
            review.focus.focus(window, cx);
            review.focus_pending = false;
        }
        let theme = self.theme;
        let ready = current
            && match &review.state {
                ReviewState::Ready { pane, .. } => pane.read(cx).exact_patch_ready(cx),
                _ => false,
            };
        let body = if !current {
            div()
                .p_4()
                .child("The comparison changed. Close this preview and review a new patch.")
                .into_any_element()
        } else {
            match &review.state {
                ReviewState::Preparing => div()
                    .p_4()
                    .child("Preparing exact Git patch…")
                    .into_any_element(),
                ReviewState::Error(message) => {
                    div().p_4().child(message.clone()).into_any_element()
                }
                ReviewState::Ready { pane, .. } => pane.clone().into_any_element(),
            }
        };
        let operation = review.origin.operation;
        Some(div().id("structural_patch_review").debug_selector(|| "structural_patch_review".into())
            .absolute().inset_0().size_full().flex().flex_col().bg(theme.colors.surface.canvas).occlude()
            .track_focus(&review.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" { this.close_patch_review(window, cx); cx.stop_propagation(); }
            }))
            .child(div().p_3().flex_none().child(format!("Review {} · exact Git patch", operation.label().to_lowercase())))
            .child(div().px_3().pb_2().flex_none().text_color(theme.colors.foreground.secondary).child(if operation == PatchOperation::Stage {
                "Whole lines will be applied to the index. All whitespace changes are shown."
            } else if operation == PatchOperation::Unstage {
                "This patch will be reversed in the index. All whitespace changes are shown."
            } else { "This patch will be reversed in the working file. All whitespace changes are shown." }))
            .child(div().flex_1().min_h_0().child(body))
            .child(div().p_3().flex().gap_2().justify_end().flex_none()
                .child(components::Button::new("structural_patch_cancel", "Cancel").on_click(theme, cx, |this, _, window, cx| this.close_patch_review(window, cx)))
                .child(components::Button::new("structural_patch_apply", format!("{} reviewed patch", operation.label())).disabled(!ready)
                    .on_click(theme, cx, |this, _, window, cx| this.apply_patch_review(window, cx))))
            .into_any_element())
    }
}
