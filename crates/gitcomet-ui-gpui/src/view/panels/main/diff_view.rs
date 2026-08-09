use super::*;
use crate::view::panes::main::DiffHorizontalScrollColumn;
use crate::view::panes::main::diff_search::DiffSearchOptions;
use gitcomet_core::domain::{
    SubmoduleDiffRangeKind, SubmoduleDiffSummary, SubmoduleDiffSummaryMode, SubmoduleInnerChange,
    SubmoduleStatus,
};
use gitcomet_state::model::{InlineSubmoduleDiffEntry, InlineSubmoduleDiffSection};
use gpui::Focusable;

struct DiffSearchOverlayLayer {
    child: AnyElement,
}

impl IntoElement for DiffSearchOverlayLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DiffSearchOverlayLayer {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let _ = self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Diff text rows paint in their own layers, so layer the search UI as a unit.
        window.paint_layer(bounds, |window| self.child.paint(window, cx));
    }
}

fn short_submodule_hash(commit_id: &CommitId) -> String {
    let raw = commit_id.as_ref();
    raw.chars().take(12).collect()
}

fn short_submodule_hash_opt(commit_id: Option<&CommitId>) -> String {
    commit_id
        .map(short_submodule_hash)
        .unwrap_or_else(|| "missing".to_string())
}

fn full_submodule_hash_opt(commit_id: Option<&CommitId>) -> String {
    commit_id
        .map(|commit_id| commit_id.as_ref().to_string())
        .unwrap_or_else(|| "missing".to_string())
}

fn submodule_range_label(kind: SubmoduleDiffRangeKind) -> &'static str {
    match kind {
        SubmoduleDiffRangeKind::StagedPointer => "Committed -> Index",
        SubmoduleDiffRangeKind::UnstagedPointer => "Index -> Checked out",
        SubmoduleDiffRangeKind::CommitHistory => "Parent -> Commit",
    }
}

fn inline_submodule_entries(summary: &SubmoduleDiffSummary) -> Vec<InlineSubmoduleDiffEntry> {
    let mut entries = Vec::new();
    for range in &summary.ranges {
        let Some((from_commit_id, to_commit_id)) = range.from.clone().zip(range.to.clone()) else {
            continue;
        };
        entries.extend(range.changes.iter().map(|change| InlineSubmoduleDiffEntry {
            path: change.path.clone(),
            kind: change.kind,
            target: DiffTarget::CommitRange {
                from_commit_id: from_commit_id.clone(),
                to_commit_id: Some(to_commit_id.clone()),
                path: Some(change.path.clone()),
            },
            section: InlineSubmoduleDiffSection::Range(range.kind),
        }));
    }
    entries.extend(
        summary
            .live_staged
            .iter()
            .map(|change| InlineSubmoduleDiffEntry {
                path: change.path.clone(),
                kind: change.kind,
                target: DiffTarget::WorkingTree {
                    path: change.path.clone(),
                    area: DiffArea::Staged,
                },
                section: InlineSubmoduleDiffSection::LiveStaged,
            }),
    );
    entries.extend(
        summary
            .live_unstaged
            .iter()
            .map(|change| InlineSubmoduleDiffEntry {
                path: change.path.clone(),
                kind: change.kind,
                target: DiffTarget::WorkingTree {
                    path: change.path.clone(),
                    area: DiffArea::Unstaged,
                },
                section: InlineSubmoduleDiffSection::LiveUnstaged,
            }),
    );
    entries
}

impl Focusable for MainPaneView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.diff_panel_focus_handle.clone()
    }
}

impl MainPaneView {
    /// Labels for the split-diff column headers, matched to what is actually
    /// being compared — conflict views keep their own local/remote wording.
    pub(in crate::view) fn split_diff_pane_labels(&self) -> (&'static str, &'static str) {
        let repo = self.active_repo();
        let target = repo.and_then(|repo| match &repo.diff_state.diff {
            Loadable::Ready(diff) => Some(&diff.target),
            _ => repo.diff_state.diff_target.as_ref(),
        });
        match target {
            Some(DiffTarget::Commit { .. }) => ("Parent", "This commit"),
            Some(DiffTarget::CommitRange { .. }) => ("From commit", "To commit"),
            Some(DiffTarget::WorkingTree {
                area: DiffArea::Staged,
                ..
            }) => ("HEAD", "Staged"),
            Some(DiffTarget::WorkingTree { .. }) | None => ("Index", "Working tree"),
        }
    }

    /// A thin vertical drag handle at the annotation column's right edge that
    /// resizes the column. Positioned absolutely; the caller's container must
    /// be `relative()`.
    pub(in crate::view) fn annotate_resize_handle(
        &self,
        ui_scale_percent: u32,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        let annot_w = self.annotate_column_width_px(ui_scale_percent);
        let handle_w = px(7.0);
        div()
            .id("annotate_resize_handle")
            .group("annotate_resize_handle")
            .absolute()
            .left((annot_w - handle_w / 2.0).max(px(0.0)))
            .top(px(0.0))
            .h_full()
            .w(handle_w)
            .cursor(CursorStyle::ResizeLeftRight)
            .child(components::resize_grip(
                theme,
                ui_scale_percent,
                "annotate_resize_handle",
                components::ResizeGripAxis::Vertical,
                self.annotate_resize.is_some(),
                None,
            ))
            .on_drag(AnnotateResizeHandle::Divider, |_h, _o, _w, cx| {
                cx.new(|_cx| AnnotateResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    crate::press_gesture::claim_press(cx);
                    this.annotate_resize = Some(AnnotateResizeState {
                        start_x: e.position.x,
                        start_width: this.annotate_column_width,
                    });
                }),
            )
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<AnnotateResizeHandle>, _w, cx| {
                    let Some(state) = this.annotate_resize else {
                        return;
                    };
                    if *e.drag(cx) != AnnotateResizeHandle::Divider {
                        return;
                    }
                    let per_unit = f32::from(crate::ui_scale::design_px_from_percent(
                        1.0,
                        ui_scale_percent,
                    ))
                    .max(0.01);
                    let dx_design = f32::from(e.event.position.x - state.start_x) / per_unit;
                    this.annotate_column_width = (state.start_width + dx_design).clamp(
                        crate::view::rows::DIFF_ANNOTATION_MIN_WIDTH_PX,
                        crate::view::rows::DIFF_ANNOTATION_MAX_WIDTH_PX,
                    );
                    this.invalidate_diff_wrap_visible_cache();
                    cx.notify();
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.annotate_resize = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    this.annotate_resize = None;
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    pub(crate) fn handle_diff_shortcut(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let key = keystroke.key.as_str();
        let mods = keystroke.modifiers;

        let mut handled = false;

        // When the editable resolved-output pane is focused the user is typing
        // free text: every text-producing keystroke (space, a/b/c/d, etc.)
        // belongs to that editor, not to the diff/conflict shortcut table.
        // Letting them through here staged the conflict file on the first space
        // typed (StagePath → the file leaves Conflicted → the resolver closes
        // mid-edit). Two deliberate carve-outs:
        //   * Ctrl+1/2/3 pick aliases are chords with no text-input collision,
        //     so they stay live while editing (kdiff3 parity).
        //   * Ctrl+Home/End/PgUp/PgDn are intentionally NOT handled here, so the
        //     editor keeps them for cursor movement (kdiff3 parity).
        if self
            .conflict_resolver_input
            .read(cx)
            .focus_handle()
            .is_focused(window)
        {
            if self.is_conflict_resolver_active()
                && (mods.control || mods.platform)
                && !mods.alt
                && !mods.function
                && !mods.shift
                && self.conflict_resolver_conflict_count() > 0
                && let Some(choice) = conflict_resolver::conflict_ctrl_pick_choice_for_key(
                    key,
                    self.conflict_resolver.view_mode,
                )
            {
                self.conflict_resolver_pick_active_conflict(choice, cx);
                return true;
            }
            return false;
        }

        if key == "escape" && !mods.control && !mods.alt && !mods.platform && !mods.function {
            if self.diff_search_active {
                self.deactivate_diff_search(window, cx);
                handled = true;
            }
            if !handled
                && self.is_inline_submodule_diff_active()
                && let Some(repo_id) = self.active_repo_id()
            {
                self.store
                    .dispatch(Msg::CloseInlineSubmoduleDiff { repo_id });
                handled = true;
            }
            if !handled && let Some(repo_id) = self.active_repo_id() {
                self.clear_status_multi_selection(repo_id, cx);
                self.clear_diff_selection_or_exit(repo_id, cx);
                handled = true;
            }
        }

        if !handled && mods.secondary() && mods.number_of_modifiers() == 1 && key == "f" {
            handled = self.open_search_for_active_view(window, cx);
        }

        if !handled
            && self.diff_search_active
            && matches!(key, "f2" | "f3")
            && !mods.control
            && !mods.alt
            && !mods.platform
            && !mods.function
        {
            if key == "f2" {
                self.diff_search_prev_match();
            } else {
                self.diff_search_next_match();
            }
            handled = true;
        }

        if !handled
            && key == "space"
            && !mods.control
            && !mods.alt
            && !mods.platform
            && !mods.function
            && !self.is_inline_submodule_diff_active()
            && !self
                .diff_raw_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && !self
                .diff_search_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && let Some(repo_id) = self.active_repo_id()
            && let Some(repo) = self.active_repo()
            && let Some(diff_target) = repo.diff_state.diff_target.clone()
            && let DiffTarget::WorkingTree { path, area } = &diff_target
        {
            let path = path.clone();
            let area = *area;
            let change_tracking_view = self.active_change_tracking_view(cx);
            let next_path_in_section = status_nav::status_navigation_context_for_repo(
                repo,
                &diff_target,
                change_tracking_view,
            )
            .and_then(|navigation| navigation.next_or_prev_path());
            let status_ready = repo.status_entries_for_area(area).is_some();

            // A multi-file status selection wins over the single shown file, so
            // the shortcut matches what the status row button and context menu
            // already do with the same selection.
            if let Some(paths) = self.status_selection_for_shortcut(repo_id, area, &path, cx) {
                if self.confirm_stage_conflict_markers(
                    repo_id,
                    area,
                    paths.clone(),
                    true,
                    window,
                    cx,
                ) {
                    return true;
                }
                self.clear_status_selection_for_shortcut(repo_id, cx);
                self.stage_or_unstage_status_paths(repo_id, area, paths);
                self.rebuild_diff_cache(cx);
                return true;
            }

            if self.confirm_stage_conflict_markers(
                repo_id,
                area,
                vec![path.clone()],
                false,
                window,
                cx,
            ) {
                return true;
            }

            match (status_ready, area) {
                (true, DiffArea::Unstaged) => {
                    self.store.dispatch(Msg::StagePath {
                        repo_id,
                        path: path.clone(),
                    });
                    if let Some(next_path) = next_path_in_section {
                        self.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::WorkingTree {
                                path: next_path,
                                area: DiffArea::Unstaged,
                            },
                        });
                    } else {
                        self.clear_diff_selection_or_exit(repo_id, cx);
                    }
                }
                (true, DiffArea::Staged) => {
                    self.store.dispatch(Msg::UnstagePath {
                        repo_id,
                        path: path.clone(),
                    });
                    if let Some(next_path) = next_path_in_section {
                        self.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::WorkingTree {
                                path: next_path,
                                area: DiffArea::Staged,
                            },
                        });
                    } else {
                        self.clear_diff_selection_or_exit(repo_id, cx);
                    }
                }
                (false, DiffArea::Unstaged) => {
                    self.store.dispatch(Msg::StagePath {
                        repo_id,
                        path: path.clone(),
                    });
                }
                (false, DiffArea::Staged) => {
                    self.store.dispatch(Msg::UnstagePath {
                        repo_id,
                        path: path.clone(),
                    });
                }
            }
            self.rebuild_diff_cache(cx);
            handled = true;
        }

        if !handled
            && (key == "f1" || key == "f4")
            && !mods.control
            && !mods.alt
            && !mods.platform
            && !mods.function
            && let Some(repo_id) = self.active_repo_id()
        {
            let direction = if key == "f1" { -1 } else { 1 };
            handled = self.try_select_adjacent_diff_file(repo_id, direction, window, cx);
        }

        if !handled
            && !self.is_inline_submodule_diff_active()
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && !self
                .diff_raw_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && !self
                .diff_search_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && let Some(repo_id) = self.active_repo_id()
            && let Some(repo) = self.active_repo()
            && let Some(diff_target) = repo.diff_state.diff_target.clone()
            && let DiffTarget::WorkingTree { path, area } = &diff_target
        {
            let path = path.clone();
            let area = *area;
            let status_ready = repo.status_entries_for_area(area).is_some();

            match key {
                "s" if area == DiffArea::Unstaged && !mods.shift => {
                    let change_tracking_view = self.active_change_tracking_view(cx);
                    let next_path_in_section = status_nav::status_navigation_context_for_repo(
                        repo,
                        &diff_target,
                        change_tracking_view,
                    )
                    .and_then(|navigation| navigation.next_or_prev_path());

                    // A multi-file status selection wins over the single shown
                    // file, matching the status row button and context menu.
                    // Resolved before confirming, or the dialog would describe —
                    // and then stage — only the shown file out of the selection.
                    if let Some(paths) =
                        self.status_selection_for_shortcut(repo_id, area, &path, cx)
                    {
                        if self.confirm_stage_conflict_markers(
                            repo_id,
                            area,
                            paths.clone(),
                            true,
                            window,
                            cx,
                        ) {
                            return true;
                        }
                        self.clear_status_selection_for_shortcut(repo_id, cx);
                        self.stage_or_unstage_status_paths(repo_id, area, paths);
                        self.rebuild_diff_cache(cx);
                        return true;
                    }

                    if self.confirm_stage_conflict_markers(
                        repo_id,
                        area,
                        vec![path.clone()],
                        false,
                        window,
                        cx,
                    ) {
                        return true;
                    }

                    if status_ready {
                        self.store.dispatch(Msg::StagePath {
                            repo_id,
                            path: path.clone(),
                        });
                        if let Some(next_path) = next_path_in_section {
                            self.store.dispatch(Msg::SelectDiff {
                                repo_id,
                                target: DiffTarget::WorkingTree {
                                    path: next_path,
                                    area: DiffArea::Unstaged,
                                },
                            });
                        } else {
                            self.clear_diff_selection_or_exit(repo_id, cx);
                        }
                    } else {
                        self.store.dispatch(Msg::StagePath {
                            repo_id,
                            path: path.clone(),
                        });
                    }
                    self.rebuild_diff_cache(cx);
                    handled = true;
                }
                "u" if area == DiffArea::Staged && !mods.shift => {
                    let change_tracking_view = self.active_change_tracking_view(cx);
                    let next_path_in_section = status_nav::status_navigation_context_for_repo(
                        repo,
                        &diff_target,
                        change_tracking_view,
                    )
                    .and_then(|navigation| navigation.next_or_prev_path());

                    // A multi-file status selection wins over the single shown
                    // file, matching the status row button and context menu.
                    if let Some(paths) =
                        self.status_selection_for_shortcut(repo_id, area, &path, cx)
                    {
                        if self.confirm_stage_conflict_markers(
                            repo_id,
                            area,
                            paths.clone(),
                            true,
                            window,
                            cx,
                        ) {
                            return true;
                        }
                        self.clear_status_selection_for_shortcut(repo_id, cx);
                        self.stage_or_unstage_status_paths(repo_id, area, paths);
                        self.rebuild_diff_cache(cx);
                        return true;
                    }

                    if status_ready {
                        self.store.dispatch(Msg::UnstagePath {
                            repo_id,
                            path: path.clone(),
                        });
                        if let Some(next_path) = next_path_in_section {
                            self.store.dispatch(Msg::SelectDiff {
                                repo_id,
                                target: DiffTarget::WorkingTree {
                                    path: next_path,
                                    area: DiffArea::Staged,
                                },
                            });
                        } else {
                            self.clear_diff_selection_or_exit(repo_id, cx);
                        }
                    } else {
                        self.store.dispatch(Msg::UnstagePath {
                            repo_id,
                            path: path.clone(),
                        });
                    }
                    self.rebuild_diff_cache(cx);
                    handled = true;
                }
                "d" if !mods.shift => {
                    let bounds = window.window_bounds().get_bounds();
                    let anchor = point(
                        (bounds.size.width * 0.5).max(px(64.0)),
                        (bounds.size.height * 0.25).max(px(24.0)),
                    );
                    self.open_popover_at(
                        PopoverKind::DiscardChangesConfirm {
                            repo_id,
                            area,
                            path: Some(path),
                        },
                        anchor,
                        window,
                        cx,
                    );
                    handled = true;
                }
                "h" if !mods.shift => {
                    let bounds = window.window_bounds().get_bounds();
                    let anchor = point(
                        (bounds.size.width * 0.5).max(px(64.0)),
                        (bounds.size.height * 0.25).max(px(24.0)),
                    );
                    self.open_popover_at(
                        PopoverKind::FileHistory {
                            repo_id,
                            path: path.clone(),
                        },
                        anchor,
                        window,
                        cx,
                    );
                    handled = true;
                }
                "e" if !mods.shift && crate::external_editor::configured_setting().is_some() => {
                    let full_path = repo.spec.workdir.join(&path);
                    let root_view = self.root_view.clone();
                    let p = full_path;
                    cx.defer(move |cx| {
                        if let Some(root) = root_view.upgrade() {
                            root.update(cx, |root, cx| {
                                root.open_path_in_external_code_editor(p, cx);
                            });
                        }
                    });
                    handled = true;
                }
                "c" if mods.shift => {
                    crate::clipboard::write_text(
                        cx,
                        path.display().to_string(),
                        crate::clipboard::CopySource::FilePathShortcut,
                    );
                    handled = true;
                }
                _ => {}
            }
        }

        if !handled
            && !self.is_inline_submodule_diff_active()
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && !self
                .diff_raw_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && !self
                .diff_search_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && let Some(_repo_id) = self.active_repo_id()
            && let Some(repo) = self.active_repo()
            && let Some(diff_target) = repo.diff_state.diff_target.clone()
        {
            let path = match &diff_target {
                DiffTarget::WorkingTree { path, .. } => Some(path.clone()),
                DiffTarget::Commit { path, .. } => path.clone(),
                DiffTarget::CommitRange { path, .. } => path.clone(),
            };
            if let Some(path) = path {
                match key {
                    "e" if !mods.shift
                        && crate::external_editor::configured_setting().is_some() =>
                    {
                        let full_path = repo.spec.workdir.join(&path);
                        let root_view = self.root_view.clone();
                        let p = full_path;
                        cx.defer(move |cx| {
                            if let Some(root) = root_view.upgrade() {
                                root.update(cx, |root, cx| {
                                    root.open_path_in_external_code_editor(p, cx);
                                });
                            }
                        });
                        handled = true;
                    }
                    _ => {}
                }
            }
        }

        let copy_target_is_focused = self
            .diff_raw_input
            .read(cx)
            .focus_handle()
            .is_focused(window);
        let is_file_preview = self.is_file_preview_active();
        if is_file_preview {
            if !handled
                && !copy_target_is_focused
                && (mods.control || mods.platform)
                && !mods.alt
                && !mods.function
                && !mods.shift
                && key == "c"
                && self.diff_text_has_selection()
            {
                self.copy_selected_diff_text_to_clipboard(cx);
                handled = true;
            }

            if !handled
                && !copy_target_is_focused
                && (mods.control || mods.platform)
                && !mods.alt
                && !mods.function
                && key == "a"
            {
                self.select_all_diff_text();
                handled = true;
            }

            return handled;
        }

        let conflict_resolver_active = self.is_conflict_resolver_active();
        let markdown_preview_active = self.is_markdown_preview_active();
        let conflict_preview_active = self.is_conflict_rendered_preview_active();

        if mods.alt && !mods.control && !mods.platform && !mods.function {
            match key {
                "i" | "s" => {
                    if conflict_resolver_active {
                        handled = false;
                    } else if self.active_conflict_target().is_some() {
                        self.set_diff_view_mode(DiffViewMode::Split, cx);
                        handled = true;
                        let root_view = self.root_view.clone();
                        cx.defer(move |cx| {
                            if let Some(root) = root_view.upgrade() {
                                root.update(cx, |root, cx| {
                                    root.set_diff_view_mode(DiffViewMode::Split, cx);
                                });
                            }
                        });
                    // The markdown diff preview renders both layouts (see
                    // `render_markdown_diff_preview`), so these switch it just
                    // like the text diff. The single-pane file preview has no
                    // old/new pair to split, so it stays excluded.
                    } else if !self.is_file_preview_active() {
                        let new_mode = if key == "i" {
                            DiffViewMode::Inline
                        } else {
                            DiffViewMode::Split
                        };
                        self.set_diff_view_mode(new_mode, cx);
                        handled = true;
                        let root_view = self.root_view.clone();
                        let mode = new_mode;
                        cx.defer(move |cx| {
                            if let Some(root) = root_view.upgrade() {
                                root.update(cx, |root, cx| {
                                    root.set_diff_view_mode(mode, cx);
                                });
                            }
                        });
                    }
                }
                "w" if !markdown_preview_active && !conflict_preview_active => {
                    self.toggle_reveal_whitespace_chars(cx);
                    handled = true;
                }
                "b" if !markdown_preview_active && !conflict_preview_active => {
                    let next = !self.annotate_enabled;
                    handled = true;
                    let root_view = self.root_view.clone();
                    cx.defer(move |cx| {
                        if let Some(root) = root_view.upgrade() {
                            root.update(cx, |root, cx| {
                                root.set_annotate_enabled(next, cx);
                            });
                        }
                    });
                }
                "up" => {
                    handled = self.navigate_prev_diff_change(cx);
                }
                "down" => {
                    handled = self.navigate_next_diff_change(cx);
                }
                "left" => {
                    if let Some(repo_id) = self.active_repo_id() {
                        self.store.dispatch(Msg::GlobalNavBack { repo_id });
                        handled = true;
                    }
                }
                "right" => {
                    if let Some(repo_id) = self.active_repo_id() {
                        self.store.dispatch(Msg::GlobalNavForward { repo_id });
                        handled = true;
                    }
                }
                _ => {}
            }
        }

        if !handled
            && matches!(key, "f2" | "f3" | "f7")
            && !mods.control
            && !mods.alt
            && !mods.platform
            && !mods.function
        {
            match key {
                "f2" => {
                    let _ = self.navigate_prev_search_match_or_diff_change(cx);
                }
                "f3" => {
                    let _ = self.navigate_next_search_match_or_diff_change(cx);
                }
                "f7" if mods.shift => {
                    let _ = self.navigate_prev_diff_change(cx);
                }
                "f7" => {
                    let _ = self.navigate_next_diff_change(cx);
                }
                _ => {}
            }
            handled = true;
        }

        if !handled
            && conflict_resolver_active
            && !mods.control
            && !mods.alt
            && !mods.platform
            && !mods.function
            && !copy_target_is_focused
            && !self
                .conflict_resolver_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            // Single-letter picks must not swallow characters typed into the
            // search box (e.g. "d" would otherwise pick Both).
            && !self
                .diff_search_input
                .read(cx)
                .focus_handle()
                .is_focused(window)
            && self.conflict_resolver_conflict_count() > 0
        {
            if let Some(choice) = conflict_resolver::conflict_quick_pick_choice_for_key(
                key,
                self.conflict_resolver.view_mode,
            ) {
                self.conflict_resolver_pick_active_conflict(choice, cx);
                handled = true;
            } else if key == "u" {
                // section 30: U un-resolves the active conflict (pick or auto-solve).
                self.conflict_resolver_unresolve_active_conflict(cx);
                handled = true;
            }
        }

        // section 30: kdiff3-compatible Ctrl+1/2/3 pick aliases. When the output
        // editor is focused these are handled by the carve-out at the top of this
        // fn; this block covers the case where focus is elsewhere.
        if !handled
            && conflict_resolver_active
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && !mods.shift
            && self.conflict_resolver_conflict_count() > 0
            && let Some(choice) = conflict_resolver::conflict_ctrl_pick_choice_for_key(
                key,
                self.conflict_resolver.view_mode,
            )
        {
            self.conflict_resolver_pick_active_conflict(choice, cx);
            handled = true;
        }

        // section 30: kdiff3-compatible delta navigation — Ctrl+Home/End jump to the
        // first/last delta, Ctrl+PgUp/PgDn to the previous/next unresolved
        // conflict.
        if !handled
            && conflict_resolver_active
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && !mods.shift
            && self.conflict_resolver_conflict_count() > 0
        {
            match key {
                "home" => {
                    self.conflict_jump_first(cx);
                    handled = true;
                }
                "end" => {
                    self.conflict_jump_last(cx);
                    handled = true;
                }
                "pageup" => {
                    self.conflict_jump_prev_unresolved(cx);
                    handled = true;
                }
                "pagedown" => {
                    self.conflict_jump_next_unresolved(cx);
                    handled = true;
                }
                _ => {}
            }
        }

        if !handled
            && !copy_target_is_focused
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && !mods.shift
            && key == "c"
            && self.diff_text_has_selection()
        {
            self.copy_selected_diff_text_to_clipboard(cx);
            handled = true;
        }

        if !handled
            && !copy_target_is_focused
            && (mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            && key == "a"
        {
            self.select_all_diff_text();
            handled = true;
        }

        handled
    }

    fn toggle_reveal_whitespace_chars(&mut self, cx: &mut gpui::Context<Self>) {
        self.set_diff_reveal_whitespace_chars_and_persist(!self.reveal_whitespace_chars, cx);
    }

    fn prepare_source_mode_for_diff_search(&mut self, cx: &mut gpui::Context<Self>) {
        if self.is_markdown_preview_active() {
            self.rendered_preview_modes
                .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
            let wants_file_diff = self.wants_file_diff_view(self.is_file_preview_active());
            if wants_file_diff {
                self.ensure_file_diff_cache(cx);
            }
        }
        if self.is_conflict_rendered_preview_active() {
            self.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Text;
        }
    }

    fn activate_diff_search(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.prepare_source_mode_for_diff_search(cx);
        let was_search_active = self.diff_search_active;
        self.diff_search_active = true;
        self.clear_diff_text_query_overlay_cache();
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_segments_cache.clear();
        self.clear_conflict_diff_query_overlay_caches();
        self.diff_search_cancel_pending_query_recompute();
        if was_search_active {
            self.diff_search_recompute_matches();
        } else {
            self.diff_search_recompute_matches_and_scroll_to_first();
        }
        let focus = self.diff_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
        self.diff_search_input
            .update(cx, |input, cx| input.select_all_text(cx));
    }

    fn deactivate_diff_search(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.diff_search_cancel_pending_query_recompute();
        self.diff_search_active = false;
        self.diff_search_query = SharedString::default();
        self.diff_search_regex_error = None;
        self.diff_search_matches.clear();
        self.diff_search_match_ix = None;
        self.diff_search_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.diff_search_scroll.set_offset(point(px(0.0), px(0.0)));
        self.clear_diff_text_query_overlay_cache();
        self.clear_worktree_preview_segments_cache();
        self.clear_conflict_diff_query_overlay_caches();
        window.focus(&self.diff_panel_focus_handle, cx);
    }

    fn focus_diff_search_input(&self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let focus = self.diff_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
    }

    fn refresh_diff_search_after_option_change(&mut self) {
        let query = self.diff_search_query.clone();
        self.invalidate_diff_text_query_overlay_cache(query.as_ref(), self.diff_search_options);
        self.clear_worktree_preview_segments_cache();
        self.clear_conflict_diff_query_overlay_caches();
        self.diff_search_cancel_pending_query_recompute();
        self.diff_search_recompute_matches_and_scroll_to_first();
    }

    fn set_diff_search_options(
        &mut self,
        next: DiffSearchOptions,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_search_options != next {
            self.diff_search_options = next;
            self.refresh_diff_search_after_option_change();
        }
        self.focus_diff_search_input(window, cx);
        cx.notify();
    }

    fn insert_diff_search_line_break(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.diff_search_input.update(cx, |input, cx| {
            input.replace_selection_utf8("\n", cx);
        });
        self.focus_diff_search_input(window, cx);
        cx.notify();
    }

    fn restore_diff_panel_focus_after_toolbar_action(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let focus = self.diff_panel_focus_handle.clone();
        window.focus(&focus, cx);
        cx.focus_self(window);
        let focus = self.diff_panel_focus_handle.clone();
        window.on_next_frame(move |window, cx| {
            window.focus(&focus, cx);
        });
    }

    /// The "Blame" toggle button, shared by the diff toolbar and the file
    /// content view so annotations can be toggled in either.
    fn diff_annotate_toggle_button(
        &self,
        theme: AppTheme,
        selected_bg: gpui::Rgba,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gitcomet_state::model::Loadable;
        // Reflect blame load status on the toggle so a failed or slow blame is not
        // a silently blank annotation column: the column is only drawn from `blame`
        // Ready, so when it is Loading/Error this control is the user-facing
        // feedback. Toggling off then on retries (see request_blame_for_current_target).
        let blame_status = self
            .annotate_enabled
            .then(|| self.active_repo().map(|repo| &repo.history_state.blame))
            .flatten();
        // A rendered preview has no annotation gutter to draw into, so the
        // toggle greys out there rather than silently doing nothing — matching
        // Alt+B, which is inert for the same reason. Text mode still annotates.
        let preview_blocks_blame = self.is_markdown_preview_active();
        let (tooltip, errored): (SharedString, bool) = if preview_blocks_blame {
            (
                "Blame is unavailable in the rendered preview\nSwitch to Text to annotate".into(),
                false,
            )
        } else {
            match blame_status {
                Some(Loadable::Loading) => ("Loading blame…".into(), false),
                Some(Loadable::Error(message)) => (
                    format!("Blame failed: {message}\nToggle off and on to retry").into(),
                    true,
                ),
                _ => (
                    format!(
                        "Toggle blame annotations ({})",
                        crate::view::shortcut_labels::alt_shortcut("B")
                    )
                    .into(),
                    false,
                ),
            }
        };
        let selected_bg = if errored {
            with_alpha(theme.colors.danger, if theme.is_dark { 0.30 } else { 0.20 })
        } else {
            selected_bg
        };
        components::Button::new("diff_annotate", "Blame")
            .borderless()
            .style(components::ButtonStyle::Subtle)
            .disabled(preview_blocks_blame)
            .selected(self.annotate_enabled)
            .selected_bg(selected_bg)
            .on_click(theme, cx, |this, _e, window, cx| {
                let next = !this.annotate_enabled;
                this.restore_diff_panel_focus_after_toolbar_action(window, cx);
                let root_view = this.root_view.clone();
                cx.defer(move |cx| {
                    if let Some(root) = root_view.upgrade() {
                        root.update(cx, |root, cx| {
                            root.set_annotate_enabled(next, cx);
                        });
                    }
                });
                cx.notify();
            })
            .debug_selector(|| "diff_annotate".to_string())
            .gitcomet_tooltip(theme, tooltip)
    }

    pub(in crate::view) fn open_search_for_active_view(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let diff_visible = self
            .active_repo()
            .and_then(|repo| repo.diff_state.diff_target.as_ref())
            .is_some();
        if !diff_visible {
            return false;
        }

        self.activate_diff_search(window, cx);
        true
    }

    fn render_diff_search_overlay(
        &mut self,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if !self.diff_search_active {
            return None;
        }

        let query = self.diff_search_query.as_ref();
        let regex_invalid = self.diff_search_regex_error.is_some();
        let match_label: SharedString = if query.is_empty() {
            "Type to search".into()
        } else if regex_invalid {
            "Invalid regex".into()
        } else if self.diff_search_matches.is_empty() {
            "No matches".into()
        } else {
            let ix = self
                .diff_search_match_ix
                .unwrap_or(0)
                .min(self.diff_search_matches.len().saturating_sub(1));
            format!("{}/{}", ix + 1, self.diff_search_matches.len()).into()
        };
        let match_label_color = if regex_invalid && !query.is_empty() {
            theme.colors.danger
        } else {
            theme.colors.text_muted
        };
        let option_selected_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.34 } else { 0.24 });
        let options = self.diff_search_options;
        let compact_control_height = px(26.0);
        let compact_icon_button_width = px(22.0);
        let compact_option_button_width = px(24.0);
        let max_search_input_height = px(super::super::COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX);

        let panel = div()
            .flex()
            .items_start()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(2.0))
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.surface_bg_elevated)
            .shadow(crate::theme::shadow_surface(theme))
            .child(
                div()
                    .relative()
                    .w(px(220.0))
                    .min_w(px(140.0))
                    .debug_selector(|| "diff_search_input_slot".to_string())
                    .child(
                        div()
                            .id("diff_search_input_scroll")
                            .relative()
                            .w_full()
                            .min_w(px(0.0))
                            .max_h(max_search_input_height)
                            .pr(components::Scrollbar::visible_gutter(
                                self.diff_search_scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            ))
                            .overflow_y_scroll()
                            .track_scroll(&self.diff_search_scroll)
                            .child(self.diff_search_input.clone()),
                    )
                    .child(
                        components::Scrollbar::new(
                            "diff_search_scrollbar",
                            self.diff_search_scroll.clone(),
                        )
                        .render(theme),
                    ),
            )
            .child(
                components::Button::new("diff_search_newline", "")
                    .start_slot(svg_icon(
                        "icons/line_break.svg",
                        theme.colors.text,
                        px(14.0),
                    ))
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.insert_diff_search_line_break(window, cx);
                    })
                    .w(compact_icon_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Insert newline (Shift+Enter)".into())
                    .debug_selector(|| "diff_search_newline".to_string()),
            )
            .child(
                components::Button::new("diff_search_match_case", "Aa")
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(options.match_case)
                    .selected_bg(option_selected_bg)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        let mut next = this.diff_search_options;
                        next.match_case = !next.match_case;
                        this.set_diff_search_options(next, window, cx);
                    })
                    .w(compact_option_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Match case".into())
                    .debug_selector(|| "diff_search_match_case".to_string()),
            )
            .child(
                components::Button::new("diff_search_whole_word", "W")
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(options.whole_word)
                    .selected_bg(option_selected_bg)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        let mut next = this.diff_search_options;
                        next.whole_word = !next.whole_word;
                        this.set_diff_search_options(next, window, cx);
                    })
                    .w(compact_option_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Match whole word".into())
                    .debug_selector(|| "diff_search_whole_word".to_string()),
            )
            .child(
                components::Button::new("diff_search_regex", ".*")
                    .borderless()
                    .style(components::ButtonStyle::Subtle)
                    .selected(options.regex)
                    .selected_bg(option_selected_bg)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        let mut next = this.diff_search_options;
                        next.regex = !next.regex;
                        this.set_diff_search_options(next, window, cx);
                    })
                    .w(compact_option_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Use regular expression".into())
                    .debug_selector(|| "diff_search_regex".to_string()),
            )
            .child(
                div()
                    .w(px(104.0))
                    .min_w(px(104.0))
                    .max_w(px(104.0))
                    .h(compact_control_height)
                    .flex()
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(match_label_color)
                    .debug_selector(|| "diff_search_match_label".to_string())
                    .child(match_label),
            )
            .child(
                components::Button::new("diff_search_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.text_muted,
                        px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.deactivate_diff_search(window, cx);
                        cx.notify();
                    })
                    .w(compact_icon_button_width)
                    .h(compact_control_height)
                    .debug_selector(|| "diff_search_close".to_string()),
            )
            .occlude()
            .with_animation(
                "diff_search_overlay_mount",
                Animation::new(Duration::from_millis(120)).with_easing(gpui::quadratic),
                |panel, delta| {
                    let slide_y = (1.0 - delta) * -8.0;
                    panel.opacity(delta).relative().top(px(slide_y))
                },
            );

        let overlay_panel = div()
            .id("diff_search_overlay_panel")
            .debug_selector(|| "diff_search_overlay".to_string())
            .absolute()
            .top(components::control_height_md(ui_scale_percent))
            .right(px(8.0))
            .child(panel)
            .into_any_element();

        let overlay = div()
            .id("diff_search_overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(overlay_panel)
            .into_any_element();

        Some(DiffSearchOverlayLayer { child: overlay }.into_any_element())
    }

    fn prepare_submodule_hash_input(
        &mut self,
        slot: usize,
        value: String,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        let Some(input) = self
            .submodule_hash_inputs
            .get(slot % self.submodule_hash_inputs.len().max(1))
            .cloned()
        else {
            return self.diff_raw_input.clone();
        };
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(value, cx);
            input.set_read_only(true, cx);
        });
        input
    }

    fn render_submodule_summary(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let Some(repo) = self.active_repo() else {
            return components::empty_state(theme, "Submodule", "No repository.")
                .into_any_element();
        };
        let Some(repo_id) = self.active_repo_id() else {
            return components::empty_state(theme, "Submodule", "No repository.")
                .into_any_element();
        };
        let Some(selected_target) = repo.diff_state.diff_target.as_ref().cloned() else {
            return components::empty_state(theme, "Submodule", "No submodule selected.")
                .into_any_element();
        };
        let (submodule_path, selected_area) = match &selected_target {
            DiffTarget::WorkingTree { path, area } => (path.clone(), Some(*area)),
            DiffTarget::Commit {
                path: Some(path), ..
            } => (path.clone(), None),
            _ => {
                return components::empty_state(theme, "Submodule", "No submodule selected.")
                    .into_any_element();
            }
        };

        let repo_workdir = repo.spec.workdir.clone();
        let open_path = repo_workdir.join(&submodule_path);
        let fallback_status = match &repo.submodules {
            Loadable::Ready(submodules) => submodules
                .iter()
                .find(|submodule| submodule.path == submodule_path)
                .map(|submodule| submodule.status),
            _ => None,
        };
        let fallback_initialized = open_path.join(".git").exists();

        match &repo.diff_state.submodule_summary {
            Loadable::NotLoaded | Loadable::Loading => {
                components::empty_state(theme, "Submodule", "Loading submodule summary…")
                    .into_any_element()
            }
            Loadable::Error(error) => {
                components::empty_state(theme, "Submodule", error.clone()).into_any_element()
            }
            Loadable::Ready(summary) => {
                let summary = (**summary).clone();
                let inline_entries = inline_submodule_entries(&summary);
                let summary_status = summary.status.or(fallback_status);
                let initialized = match summary_status {
                    Some(SubmoduleStatus::NotInitialized) => false,
                    Some(SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping) => false,
                    Some(_) => true,
                    None => fallback_initialized,
                };
                let can_open = initialized;
                let can_change_pointer = summary.mode == SubmoduleDiffSummaryMode::Worktree
                    && can_open
                    && !matches!(
                        summary_status,
                        Some(SubmoduleStatus::MergeConflict | SubmoduleStatus::MissingMapping)
                    );
                let show_load = summary.mode == SubmoduleDiffSummaryMode::Worktree
                    && (matches!(summary_status, Some(SubmoduleStatus::NotInitialized))
                        || (summary_status.is_none() && !fallback_initialized));
                let submodule_repo_path = repo_workdir.join(&summary.path);
                let summary_path = summary.path.clone();

                let status_badge = |status: SubmoduleStatus| {
                    let (label, color) = match status {
                        SubmoduleStatus::UpToDate => ("Loaded", theme.colors.success),
                        SubmoduleStatus::NotInitialized => (
                            "Not loaded",
                            with_alpha(
                                theme.colors.text_muted,
                                if theme.is_dark { 0.86 } else { 0.94 },
                            ),
                        ),
                        SubmoduleStatus::HeadMismatch => ("Head mismatch", theme.colors.warning),
                        SubmoduleStatus::MergeConflict => ("Conflict", theme.colors.danger),
                        SubmoduleStatus::MissingMapping => ("Missing mapping", theme.colors.danger),
                        SubmoduleStatus::Unknown(_) => ("Unknown", theme.colors.text_muted),
                    };

                    div()
                        .px_1p5()
                        .h(px(20.0))
                        .rounded(px(theme.radii.row))
                        .border_1()
                        .border_color(with_alpha(color, if theme.is_dark { 0.45 } else { 0.32 }))
                        .bg(with_alpha(color, if theme.is_dark { 0.14 } else { 0.10 }))
                        .text_xs()
                        .text_color(color)
                        .child(label)
                };

                let change_row_icon = |kind: FileStatusKind| match kind {
                    FileStatusKind::Untracked | FileStatusKind::Added => {
                        ("icons/plus.svg", theme.colors.success)
                    }
                    FileStatusKind::Modified => ("icons/pencil.svg", theme.colors.warning),
                    FileStatusKind::Deleted => ("icons/minus.svg", theme.colors.danger),
                    FileStatusKind::Renamed => ("icons/swap.svg", theme.colors.accent),
                    FileStatusKind::Conflicted => ("icons/warning.svg", theme.colors.danger),
                };

                let render_change_rows =
                    |section_key: &str,
                     changes: &[SubmoduleInnerChange],
                     range_commits: Option<(CommitId, CommitId)>,
                     live_area: Option<DiffArea>,
                     _this: &mut MainPaneView,
                     cx: &mut gpui::Context<MainPaneView>| {
                        if changes.is_empty() {
                            return vec![
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_sm()
                                    .text_color(theme.colors.text_muted)
                                    .child("No inner changes.")
                                    .into_any_element(),
                            ];
                        }

                        changes
                            .iter()
                            .map(|change| {
                                let (icon, icon_color) = change_row_icon(change.kind);
                                let additions = change
                                    .additions
                                    .map(|value| format!("+{value}"))
                                    .unwrap_or_else(|| "—".to_string());
                                let deletions = change
                                    .deletions
                                    .map(|value| format!("-{value}"))
                                    .unwrap_or_else(|| "—".to_string());
                                let change_path = change.path.clone();
                                let target = range_commits.as_ref().map_or_else(
                                    || {
                                        live_area.map(|area| DiffTarget::WorkingTree {
                                            path: change_path.clone(),
                                            area,
                                        })
                                    },
                                    |(from_commit_id, to_commit_id)| {
                                        Some(DiffTarget::CommitRange {
                                            from_commit_id: from_commit_id.clone(),
                                            to_commit_id: Some(to_commit_id.clone()),
                                            path: Some(change_path.clone()),
                                        })
                                    },
                                );
                                let inline_selected_ix = target.as_ref().and_then(|target| {
                                    inline_entries
                                        .iter()
                                        .position(|entry| &entry.target == target)
                                });
                                let repo_path_for_click = submodule_repo_path.clone();
                                let repo_path_for_menu = submodule_repo_path.clone();
                                let summary_path_for_inline = summary.path.clone();
                                let inline_entries_for_click = inline_entries.clone();
                                let context_menu_path = change_path.clone();

                                let mut row = div()
                                .id(format!("{}_{}", section_key, change_path.display()))
                                .px_2()
                                .py_1()
                                .rounded(px(theme.radii.row))
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(super::super::icons::svg_icon(
                                    icon,
                                    icon_color,
                                    px(12.0),
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .line_clamp(1)
                                        .child(change_path.display().to_string()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(
                                            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                        )
                                        .text_color(theme.colors.success)
                                        .child(additions),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(
                                            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                        )
                                        .text_color(theme.colors.danger)
                                        .child(deletions),
                                );

                                if let Some(target) = target {
                                    row = row
                                        .cursor(CursorStyle::PointingHand)
                                        .hover(move |row| row.bg(theme.colors.hover))
                                        .on_click(cx.listener(
                                            move |this, _e: &ClickEvent, _window, cx| {
                                                let selected_ix = inline_selected_ix.unwrap_or(0);
                                                this.store.dispatch(Msg::OpenInlineSubmoduleDiff {
                                                    repo_id,
                                                    submodule_repo_path: repo_path_for_click
                                                        .clone(),
                                                    parent_submodule_path: summary_path_for_inline
                                                        .clone(),
                                                    entries: inline_entries_for_click.clone(),
                                                    selected_ix,
                                                });
                                                cx.notify();
                                            },
                                        ))
                                        .on_mouse_down(
                                            MouseButton::Right,
                                            cx.listener(
                                                move |this, e: &MouseDownEvent, window, cx| {
                                                    cx.stop_propagation();
                                                    this.activate_context_menu_invoker(
                                                        format!(
                                                            "submodule_inner_diff_menu_{}_{}",
                                                            repo_id.0,
                                                            context_menu_path.display()
                                                        )
                                                        .into(),
                                                        cx,
                                                    );
                                                    this.open_popover_at(
                                                        PopoverKind::SubmoduleInnerDiffMenu {
                                                            repo_id,
                                                            submodule_repo_path: repo_path_for_menu
                                                                .clone(),
                                                            target: target.clone(),
                                                        },
                                                        e.position,
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            ),
                                        );
                                }

                                row.into_any_element()
                            })
                            .collect::<Vec<_>>()
                    };

                let render_change_section =
                    |title: &'static str,
                     section_key: &str,
                     changes: &[SubmoduleInnerChange],
                     range_commits: Option<(CommitId, CommitId)>,
                     live_area: Option<DiffArea>,
                     this: &mut MainPaneView,
                     cx: &mut gpui::Context<MainPaneView>| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .px_2()
                                    .pt_1()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(title),
                            )
                            .children(render_change_rows(
                                section_key,
                                changes,
                                range_commits,
                                live_area,
                                this,
                                cx,
                            ))
                            .into_any_element()
                    };

                let mut range_sections = Vec::new();
                for (slot, range) in summary.ranges.iter().enumerate() {
                    let emphasized = match range.kind {
                        SubmoduleDiffRangeKind::StagedPointer => {
                            selected_area == Some(DiffArea::Staged)
                        }
                        SubmoduleDiffRangeKind::UnstagedPointer => {
                            selected_area == Some(DiffArea::Unstaged)
                        }
                        SubmoduleDiffRangeKind::CommitHistory => true,
                    };
                    let changed = range.from != range.to;
                    let range_hash_input = self.prepare_submodule_hash_input(
                        slot,
                        format!(
                            "{} -> {}",
                            full_submodule_hash_opt(range.from.as_ref()),
                            full_submodule_hash_opt(range.to.as_ref())
                        ),
                        theme,
                        cx,
                    );
                    let range_commits = match (range.from.as_ref(), range.to.as_ref()) {
                        (Some(from), Some(to)) => Some((from.clone(), to.clone())),
                        _ => None,
                    };

                    let mut section = div()
                        .id(format!("submodule_range_{:?}", range.kind))
                        .px_2()
                        .py_2()
                        .rounded(px(theme.radii.row))
                        .border_1()
                        .border_color(if emphasized {
                            theme.colors.active
                        } else {
                            theme.colors.border
                        })
                        .bg(if emphasized {
                            with_alpha(theme.colors.hover, if theme.is_dark { 0.28 } else { 0.48 })
                        } else {
                            gpui::rgba(0x00000000)
                        })
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.colors.text_muted)
                                        .child(submodule_range_label(range.kind)),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_family(
                                            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                        )
                                        .text_color(if changed {
                                            theme.colors.text
                                        } else {
                                            theme.colors.text_muted
                                        })
                                        .child(format!(
                                            "{} -> {}",
                                            short_submodule_hash_opt(range.from.as_ref()),
                                            short_submodule_hash_opt(range.to.as_ref())
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.text_muted)
                                        .child("Hashes"),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w(px(0.0))
                                        .font_family(
                                            crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                        )
                                        .child(range_hash_input),
                                ),
                        );
                    if let Some(reason) = range.unavailable_reason.as_ref() {
                        section = section.child(
                            div()
                                .px_2()
                                .text_sm()
                                .text_color(theme.colors.text_muted)
                                .child(reason.clone()),
                        );
                    }
                    section = section.child(render_change_section(
                        "Changes between hashes",
                        &format!("submodule_range_{:?}", range.kind),
                        &range.changes,
                        range_commits,
                        None,
                        self,
                        cx,
                    ));
                    range_sections.push(section.into_any_element());
                }

                div()
                    .id("submodule_summary_scroll")
                    .flex()
                    .flex_col()
                    .h_full()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .gap_2()
                    .bg(theme.colors.window_bg)
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(super::super::icons::svg_icon(
                                        "icons/box.svg",
                                        match summary_status.unwrap_or(SubmoduleStatus::UpToDate) {
                                            SubmoduleStatus::NotInitialized => with_alpha(
                                                theme.colors.text_muted,
                                                if theme.is_dark { 0.82 } else { 0.94 },
                                            ),
                                            SubmoduleStatus::HeadMismatch => theme.colors.warning,
                                            SubmoduleStatus::MergeConflict
                                            | SubmoduleStatus::MissingMapping => {
                                                theme.colors.danger
                                            }
                                            SubmoduleStatus::UpToDate
                                            | SubmoduleStatus::Unknown(_) => theme.colors.accent,
                                        },
                                        px(14.0),
                                    ))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .child(summary.path.display().to_string()),
                                    )
                                    .when_some(summary_status, |this, status| {
                                        this.child(status_badge(status))
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        components::Button::new(
                                            "submodule_summary_open",
                                            "Open submodule",
                                        )
                                        .style(components::ButtonStyle::Outlined)
                                        .disabled(!can_open)
                                        .on_click(
                                            theme,
                                            cx,
                                            move |this, _e, _w, cx| {
                                                if can_open {
                                                    this.store
                                                        .dispatch(Msg::OpenRepo(open_path.clone()));
                                                    cx.notify();
                                                }
                                            },
                                        ),
                                    )
                                    .when(show_load, |row| {
                                        let load_path = summary.path.clone();
                                        row.child(
                                            components::Button::new(
                                                "submodule_summary_load",
                                                "Load submodule",
                                            )
                                            .style(components::ButtonStyle::Outlined)
                                            .on_click(
                                                theme,
                                                cx,
                                                move |this, _e, _w, cx| {
                                                    this.store.dispatch(Msg::LoadSubmodule {
                                                        repo_id,
                                                        path: load_path.clone(),
                                                    });
                                                    cx.notify();
                                                },
                                            ),
                                        )
                                    })
                                    .child(
                                        components::Button::new(
                                            "submodule_summary_change_pointer",
                                            "Change pointer…",
                                        )
                                        .style(components::ButtonStyle::Outlined)
                                        .disabled(!can_change_pointer)
                                        .on_click(
                                            theme,
                                            cx,
                                            move |this, e, window, cx| {
                                                if !can_change_pointer {
                                                    return;
                                                }
                                                this.open_popover_at(
                                                    PopoverKind::submodule(
                                                        repo_id,
                                                        SubmodulePopoverKind::ChangePointerPrompt {
                                                            path: summary_path.clone(),
                                                        },
                                                    ),
                                                    e.position(),
                                                    window,
                                                    cx,
                                                );
                                                cx.notify();
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .children(range_sections)
                    .when(
                        summary.mode == SubmoduleDiffSummaryMode::Worktree
                            && !summary.live_staged.is_empty(),
                        |this| {
                            this.child(render_change_section(
                                "Uncommitted inner staged",
                                "submodule_live_staged",
                                &summary.live_staged,
                                None,
                                Some(DiffArea::Staged),
                                self,
                                cx,
                            ))
                        },
                    )
                    .when(
                        summary.mode == SubmoduleDiffSummaryMode::Worktree
                            && !summary.live_unstaged.is_empty(),
                        |this| {
                            this.child(render_change_section(
                                "Uncommitted inner unstaged",
                                "submodule_live_unstaged",
                                &summary.live_unstaged,
                                None,
                                Some(DiffArea::Unstaged),
                                self,
                                cx,
                            ))
                        },
                    )
                    .into_any_element()
            }
        }
    }

    pub(in crate::view) fn diff_view(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let repo_id = self.active_repo_id();
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);

        // Intentionally no outer panel header; keep diff controls in the inner header.

        let title = self.diff_panel_title(theme, cx);
        let viewer_nav = self.diff_viewer_nav_cluster(theme, cx);
        let inline_submodule_diff_active = self.is_inline_submodule_diff_active();

        let has_submodule_summary = self
            .active_repo()
            .is_some_and(|repo| !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded));
        let untracked_directory_notice = if has_submodule_summary || inline_submodule_diff_active {
            None
        } else {
            self.untracked_directory_notice()
        };

        let is_file_preview = self.is_file_preview_active()
            && untracked_directory_notice.is_none()
            && !has_submodule_summary
            && !inline_submodule_diff_active;
        let supports_diff_content_toggle = (inline_submodule_diff_active || !has_submodule_summary)
            && self.supports_diff_content_mode_toggle(is_file_preview);

        if is_file_preview {
            self.ensure_selected_file_preview_loaded(cx);
        } else if (has_submodule_summary
            || inline_submodule_diff_active
            || untracked_directory_notice.is_some())
            && matches!(self.worktree_preview, Loadable::Loading)
        {
            self.worktree_preview_path = None;
            self.worktree_preview = Loadable::NotLoaded;
            self.reset_worktree_preview_source_state();
            self.reset_diff_horizontal_scroll_state();
        }
        let wants_file_diff =
            supports_diff_content_toggle && self.wants_file_diff_view(is_file_preview);
        let wants_collapsed_diff =
            supports_diff_content_toggle && self.wants_collapsed_diff_view(is_file_preview);

        let repo = self.active_repo();
        let conflict_target = (!inline_submodule_diff_active)
            .then_some(())
            .and(repo)
            .and_then(|repo| {
                let DiffTarget::WorkingTree { path, area } =
                    repo.diff_state.diff_target.as_ref()?
                else {
                    return None;
                };
                if *area != DiffArea::Unstaged {
                    return None;
                }
                let conflict = repo
                    .status_entry_for_path(DiffArea::Unstaged, path.as_path())
                    .filter(|entry| entry.kind == FileStatusKind::Conflicted)?;
                Some((path.clone(), conflict.conflict))
            });
        let (conflict_target_path, conflict_kind) = conflict_target
            .map(|(path, kind)| (Some(path), kind))
            .unwrap_or((None, None));
        let conflict_file_state = match (repo, conflict_target_path.as_deref()) {
            (Some(repo), Some(path)) => Some(renderable_conflict_file(
                repo,
                &self.conflict_resolver,
                path,
            )),
            _ => None,
        };
        // Detect binary from the renderable conflict file, including the
        // same-target cached snapshot we keep during transient reloads.
        let is_binary_conflict = conflict_file_state
            .and_then(|state| match state {
                RenderableConflictFile::File(file) => Some(conflict_file_is_binary(&file)),
                _ => None,
            })
            .unwrap_or(false);
        let conflict_strategy = Self::conflict_resolver_strategy(conflict_kind, is_binary_conflict);
        let is_conflict_resolver = conflict_strategy.is_some();
        let is_conflict_compare = conflict_target_path.is_some() && conflict_strategy.is_none();
        let conflict_rendered_preview_active = self.is_conflict_rendered_preview_active();

        let rendered_preview_kind =
            super::super::diff_target_rendered_preview_kind(self.rendered_diff_target());
        let rendered_view_toggle_kind = super::super::main_diff_rendered_preview_toggle_kind(
            wants_file_diff,
            wants_collapsed_diff,
            is_file_preview,
            rendered_preview_kind,
        );
        let is_markdown_preview_view = rendered_view_toggle_kind
            == Some(RenderedPreviewKind::Markdown)
            && self
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Rendered;
        let is_image_diff_loaded = wants_file_diff
            && self
                .rendered_file_image_diff_loadable()
                .is_some_and(|file| !matches!(file, Loadable::NotLoaded));
        let is_image_diff_view = wants_file_diff
            && is_image_diff_loaded
            && (!matches!(rendered_preview_kind, Some(RenderedPreviewKind::Svg))
                || self.rendered_preview_modes.get(RenderedPreviewKind::Svg)
                    == RenderedPreviewMode::Rendered);

        let (prev_file_btn, next_file_btn) = if show_diff_file_navigation(self.view_mode) {
            self.diff_prev_next_file_buttons(repo_id, is_conflict_resolver, theme, cx)
        } else {
            (None, None)
        };

        let mut controls = div().flex().items_center().gap_1();
        if self.is_inline_submodule_diff_active()
            && let Some(repo_id) = repo_id
        {
            controls = controls.child(
                components::Button::new("inline_submodule_back", "Back")
                    .separated_end_slot(Self::diff_nav_hotkey_hint(theme, "Esc"))
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.store
                            .dispatch(Msg::CloseInlineSubmoduleDiff { repo_id });
                        cx.notify();
                    }),
            );
        }
        let is_simple_conflict_strategy = matches!(
            self.conflict_resolver.strategy,
            Some(
                gitcomet_core::conflict_session::ConflictResolverStrategy::BinarySidePick
                    | gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete
                    | gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly
            )
        );
        if is_conflict_resolver && is_simple_conflict_strategy {
            controls = self.conflict_toolbar_simple_controls(
                controls,
                prev_file_btn,
                next_file_btn,
                theme,
            );
        } else if is_conflict_resolver {
            controls = self.conflict_toolbar_full_controls(
                controls,
                prev_file_btn,
                next_file_btn,
                conflict_rendered_preview_active,
                repo_id,
                &conflict_target_path,
                theme,
                cx,
            );
        } else if !is_file_preview {
            let view_toggle_selected_bg =
                with_alpha(theme.colors.accent, if theme.is_dark { 0.26 } else { 0.20 });
            let view_toggle_border = with_alpha(
                theme.colors.text_muted,
                if theme.is_dark { 0.38 } else { 0.28 },
            );
            let view_toggle_divider = with_alpha(view_toggle_border, 0.90);

            if supports_diff_content_toggle {
                let diff_mode_invoker: SharedString = "diff_content_mode_header".into();
                let diff_mode_active = self
                    .active_context_menu_invoker
                    .as_ref()
                    .is_some_and(|id| id == &diff_mode_invoker);
                let diff_mode_label = self.diff_content_mode.label();

                controls = controls.child(
                    div()
                        .id("diff_content_mode_header")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_1()
                        .h(components::control_height(ui_scale_percent))
                        .rounded(px(theme.radii.row))
                        .when(diff_mode_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if diff_mode_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(with_alpha(theme.colors.hover, 0.55))
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .cursor(CursorStyle::PointingHand)
                        .child(
                            div()
                                .min_w(px(0.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .text_sm()
                                .child(diff_mode_label),
                        )
                        .child(svg_icon(
                            "icons/chevron_down.svg",
                            theme.colors.text_muted,
                            px(12.0),
                        ))
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            this.activate_context_menu_invoker(diff_mode_invoker.clone(), cx);
                            this.open_popover_at(
                                PopoverKind::DiffContentModeSettings,
                                e.position(),
                                window,
                                cx,
                            );
                        })),
                );
            }

            controls = controls.when_some(prev_file_btn, |d, btn| d.child(btn));

            if !is_image_diff_view {
                let nav_entries = self.diff_nav_entries();
                let can_nav_prev = diff_navigation::diff_nav_prev_target(
                    &nav_entries,
                    self.diff_nav_prev_current_ix(),
                )
                .is_some();
                let can_nav_next = diff_navigation::diff_nav_next_target(
                    &nav_entries,
                    self.diff_nav_next_current_ix(),
                )
                .is_some();

                let prev_hunk_btn = components::Button::new("diff_prev_hunk", "")
                    .start_slot(svg_icon("icons/arrow_up.svg", theme.colors.text, px(14.0)))
                    .style(components::ButtonStyle::Outlined)
                    .disabled(!can_nav_prev)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_jump_prev();
                        cx.notify();
                    })
                    .gitcomet_tooltip(
                        theme,
                        format!(
                            "Previous change (F2 / Shift+F7 / {})",
                            crate::view::shortcut_labels::alt_shortcut("Up")
                        )
                        .into(),
                    );

                let next_hunk_btn = components::Button::new("diff_next_hunk", "")
                    .start_slot(svg_icon(
                        "icons/arrow_down.svg",
                        theme.colors.text,
                        px(14.0),
                    ))
                    .style(components::ButtonStyle::Outlined)
                    .disabled(!can_nav_next)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.diff_jump_next();
                        cx.notify();
                    })
                    .gitcomet_tooltip(
                        theme,
                        format!(
                            "Next change (F3 / F7 / {})",
                            crate::view::shortcut_labels::alt_shortcut("Down")
                        )
                        .into(),
                    );

                let diff_inline_btn = components::Button::new("diff_inline", "Inline")
                    .borderless()
                    .rounded_left()
                    .style(components::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Inline)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.set_diff_view_mode(DiffViewMode::Inline, cx);
                        this.restore_diff_panel_focus_after_toolbar_action(window, cx);
                        let root_view = this.root_view.clone();
                        cx.defer(move |cx| {
                            if let Some(root) = root_view.upgrade() {
                                root.update(cx, |root, cx| {
                                    root.set_diff_view_mode(DiffViewMode::Inline, cx);
                                });
                            }
                        });
                        cx.notify();
                    })
                    .debug_selector(|| "diff_inline".to_string())
                    .gitcomet_tooltip(
                        theme,
                        format!(
                            "Inline diff view ({})",
                            crate::view::shortcut_labels::alt_shortcut("I")
                        )
                        .into(),
                    );

                let diff_split_btn = components::Button::new("diff_split", "Split")
                    .borderless()
                    .rounded_right()
                    .style(components::ButtonStyle::Subtle)
                    .selected(self.diff_view == DiffViewMode::Split)
                    .selected_bg(view_toggle_selected_bg)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.set_diff_view_mode(DiffViewMode::Split, cx);
                        this.restore_diff_panel_focus_after_toolbar_action(window, cx);
                        let root_view = this.root_view.clone();
                        cx.defer(move |cx| {
                            if let Some(root) = root_view.upgrade() {
                                root.update(cx, |root, cx| {
                                    root.set_diff_view_mode(DiffViewMode::Split, cx);
                                });
                            }
                        });
                        cx.notify();
                    })
                    .debug_selector(|| "diff_split".to_string())
                    .gitcomet_tooltip(
                        theme,
                        format!(
                            "Split diff view ({})",
                            crate::view::shortcut_labels::alt_shortcut("S")
                        )
                        .into(),
                    );

                let diff_annotate_btn =
                    self.diff_annotate_toggle_button(theme, view_toggle_selected_bg, cx);

                let view_toggle = div()
                    .id("diff_view_toggle")
                    .debug_selector(|| "diff_view_toggle".to_string())
                    .flex()
                    .items_center()
                    .h(components::control_height(ui_scale_percent))
                    .rounded(px(theme.radii.row))
                    .border_1()
                    .border_color(view_toggle_border)
                    .bg(gpui::rgba(0x00000000))
                    .overflow_hidden()
                    .child(diff_inline_btn)
                    .child(div().h_full().w(px(1.0)).bg(view_toggle_divider))
                    .child(diff_split_btn);

                controls = controls
                    .child(prev_hunk_btn)
                    .child(next_hunk_btn)
                    .when_some(next_file_btn, |d, btn| d.child(btn))
                    .child(view_toggle)
                    .child(diff_annotate_btn);
            } else {
                controls = controls.when_some(next_file_btn, |d, btn| d.child(btn));
            }
        } else {
            // File content view (e.g. a file shown at a commit): expose the
            // Blame toggle here too so annotations can be walked through history.
            let annotate_selected_bg =
                with_alpha(theme.colors.accent, if theme.is_dark { 0.26 } else { 0.20 });
            controls = controls
                .when_some(prev_file_btn, |d, btn| d.child(btn))
                .when_some(next_file_btn, |d, btn| d.child(btn))
                .child(self.diff_annotate_toggle_button(theme, annotate_selected_bg, cx));
        }

        if !is_conflict_resolver && let Some(preview_kind) = rendered_view_toggle_kind {
            let preview_mode = self.rendered_preview_modes.get(preview_kind);
            controls = controls.child(
                div()
                    .id(preview_kind.toggle_id())
                    .debug_selector(move || preview_kind.toggle_id().to_string())
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        components::Button::new(
                            preview_kind.rendered_button_id(),
                            preview_kind.rendered_label(),
                        )
                        .style(if preview_mode == RenderedPreviewMode::Rendered {
                            components::ButtonStyle::Filled
                        } else {
                            components::ButtonStyle::Outlined
                        })
                        .on_click(
                            theme,
                            cx,
                            move |this, _e, window, cx| {
                                this.rendered_preview_modes
                                    .set(preview_kind, RenderedPreviewMode::Rendered);
                                this.restore_diff_panel_focus_after_toolbar_action(window, cx);
                                cx.notify();
                            },
                        ),
                    )
                    .child(
                        components::Button::new(
                            preview_kind.source_button_id(),
                            preview_kind.source_label(),
                        )
                        .style(if preview_mode == RenderedPreviewMode::Source {
                            components::ButtonStyle::Filled
                        } else {
                            components::ButtonStyle::Outlined
                        })
                        .on_click(
                            theme,
                            cx,
                            move |this, _e, window, cx| {
                                this.rendered_preview_modes
                                    .set(preview_kind, RenderedPreviewMode::Source);
                                this.restore_diff_panel_focus_after_toolbar_action(window, cx);
                                cx.notify();
                            },
                        ),
                    ),
            );
        }

        if let Some(repo_id) = repo_id {
            // The full text resolver gets its own settings menu under the cog
            // (section 30); everything else keeps the diff actions menu.
            let resolver_settings_active = is_conflict_resolver && !is_simple_conflict_strategy;
            let (cog_id, cog_kind, cog_tooltip): (&'static str, PopoverKind, &'static str) =
                if resolver_settings_active {
                    (
                        "mergetool_settings_menu",
                        PopoverKind::MergetoolSettingsMenu,
                        "Merge tool settings",
                    )
                } else {
                    (
                        "diff_action_menu",
                        PopoverKind::DiffActionMenu,
                        "Diff actions",
                    )
                };
            let diff_action_invoker: SharedString = cog_id.into();
            let diff_action_active = self
                .active_context_menu_invoker
                .as_ref()
                .is_some_and(|id| id == &diff_action_invoker);
            controls = controls.child(
                components::Button::new(cog_id, "")
                    .start_slot(svg_icon("icons/cog.svg", theme.colors.text_muted, px(14.0)))
                    .style(components::ButtonStyle::Transparent)
                    .selected(diff_action_active)
                    .selected_bg(theme.colors.active)
                    .on_click(theme, cx, move |this, e, window, cx| {
                        this.activate_context_menu_invoker(diff_action_invoker.clone(), cx);
                        this.open_popover_at(cog_kind.clone(), e.position(), window, cx);
                    })
                    .debug_selector(move || cog_id.to_string())
                    .gitcomet_tooltip(theme, cog_tooltip.into()),
            );
            controls = controls.child(
                components::Button::new("diff_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.text_muted,
                        px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.clear_status_multi_selection(repo_id, cx);
                        this.clear_diff_selection_or_exit(repo_id, cx);
                        cx.notify();
                    })
                    .debug_selector(|| "diff_close".to_string())
                    .gitcomet_tooltip(theme, "Close diff".into()),
            );
        }

        let header = div()
            .debug_selector(|| "diff_file_header".to_string())
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .h(components::control_height_md(ui_scale_percent))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().min_w(px(0.0)).overflow_hidden().child(title))
                    .when_some(viewer_nav, |d, cluster| d.child(cluster)),
            )
            .child(
                // Right-anchor the controls and clip from the leading edge so a
                // narrow pane hides lower-priority buttons instead of pushing
                // the action menu / close button past the pane clip, where they
                // still paint but can no longer be clicked.
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .overflow_hidden()
                    .child(controls),
            );

        let body: AnyElement = if has_submodule_summary && !inline_submodule_diff_active {
            self.render_submodule_summary(theme, cx)
        } else if let Some(message) = untracked_directory_notice {
            components::empty_state(theme, "Directory", message).into_any_element()
        } else if is_file_preview {
            if is_markdown_preview_view {
                match &self.worktree_preview {
                    Loadable::NotLoaded | Loadable::Loading => {
                        components::empty_state(theme, "Preview", "Loading").into_any_element()
                    }
                    Loadable::Error(e) => {
                        components::empty_state(theme, "Preview", e.clone()).into_any_element()
                    }
                    Loadable::Ready(_) => {
                        self.ensure_single_markdown_preview_cache(cx);
                        self.watch_pending_markdown_preview_images(cx);
                        match &self.worktree_markdown_preview {
                            Loadable::NotLoaded | Loadable::Loading => {
                                components::empty_state(theme, "Preview", "Loading")
                                    .into_any_element()
                            }
                            Loadable::Error(e) => {
                                components::empty_state(theme, "Preview", e.clone())
                                    .into_any_element()
                            }
                            Loadable::Ready(document) => {
                                if document.rows.is_empty() {
                                    let message = if self.worktree_preview_line_count() == Some(0) {
                                        "Empty file."
                                    } else {
                                        "Nothing to render."
                                    };
                                    components::empty_state(theme, "Preview", message)
                                        .into_any_element()
                                } else {
                                    // A single document lays out as one flowing
                                    // element tree rather than a uniform row
                                    // list: text wraps by itself, images sit at
                                    // their own size, and the gaps around
                                    // headings are margins.
                                    self.markdown_preview_wrap
                                        .clear_list(MarkdownPreviewList::Worktree);
                                    let document = std::sync::Arc::clone(document);
                                    let image_base_dir = self
                                        .markdown_preview_image_base_dir()
                                        .map(|dir| std::sync::Arc::from(dir.as_path()));
                                    let body = rows::render_markdown_document(
                                        document.as_ref(),
                                        &rows::MarkdownDocumentContext {
                                            theme,
                                            ui_scale_percent,
                                            editor_font_family: editor_font_family.clone().into(),
                                            image_base_dir,
                                            picture_sizes: std::sync::Arc::clone(
                                                &self.worktree_markdown_preview_picture_sizes,
                                            ),
                                            block_scrolls: self
                                                .worktree_markdown_preview_block_scrolls
                                                .clone(),
                                            view: Some(cx.entity()),
                                            text_region: DiffTextRegion::Inline,
                                            change_bar_color:
                                                rows::worktree_markdown_preview_bar_color(
                                                    self, theme,
                                                ),
                                        },
                                    );

                                    let scroll_handle =
                                        self.worktree_preview_scroll.0.borrow().base_handle.clone();
                                    let scrollbar_gutter = components::Scrollbar::visible_gutter(
                                        scroll_handle.clone(),
                                        components::ScrollbarAxis::Vertical,
                                    );
                                    let edge_gap = crate::ui_scale::design_px_from_percent(
                                        super::diff::MARKDOWN_PREVIEW_DOCUMENT_EDGE_GAP_PX,
                                        ui_scale_percent,
                                    );
                                    div()
                                        .id("worktree_markdown_preview_scroll_container")
                                        .debug_selector(|| {
                                            "worktree_markdown_preview_scroll_container".to_string()
                                        })
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .bg(theme.colors.window_bg)
                                        .child(
                                            div()
                                                .id("worktree_markdown_preview_document")
                                                .debug_selector(|| {
                                                    "worktree_markdown_preview_document".to_string()
                                                })
                                                .size_full()
                                                .min_h(px(0.0))
                                                .overflow_y_scroll()
                                                .track_scroll(&scroll_handle)
                                                .pt(edge_gap)
                                                .pb(edge_gap)
                                                .pr(scrollbar_gutter)
                                                .child(body),
                                        )
                                        .child(
                                            components::Scrollbar::new(
                                                "worktree_markdown_preview_scrollbar",
                                                scroll_handle,
                                            )
                                            .render(theme),
                                        )
                                        .into_any_element()
                                }
                            }
                        }
                    }
                }
            } else {
                match &self.worktree_preview {
                    Loadable::NotLoaded | Loadable::Loading => {
                        components::empty_state(theme, "File", "Loading").into_any_element()
                    }
                    Loadable::Error(e) => {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(e.clone(), cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("worktree_preview_error_scroll")
                            .bg(theme.colors.window_bg)
                            .font_family(editor_font_family.clone())
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    }
                    Loadable::Ready(line_count) => {
                        let line_count = *line_count;
                        if line_count == 0 {
                            components::empty_state(theme, "File", "Empty file.").into_any_element()
                        } else {
                            // Word wrap turns one line into several rows, so the
                            // projection has to be built before the list is
                            // asked how long it is.
                            self.ensure_diff_wrap_visible_rows(window, cx);
                            let row_count = self
                                .worktree_preview_visible_len()
                                .unwrap_or(line_count)
                                .max(1);
                            let wrapped = self.worktree_preview_wrap_active();
                            let list = uniform_list(
                                "worktree_preview_list",
                                row_count,
                                cx.processor(Self::render_worktree_preview_rows),
                            )
                            .h_full()
                            .min_h(px(0.0))
                            .track_scroll(&self.worktree_preview_scroll)
                            .with_horizontal_sizing_behavior(
                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                            );

                            let scroll_handle =
                                self.worktree_preview_scroll.0.borrow().base_handle.clone();
                            let scrollbar_gutter = components::Scrollbar::visible_gutter(
                                scroll_handle.clone(),
                                components::ScrollbarAxis::Vertical,
                            );
                            div()
                                .id("worktree_preview_scroll_container")
                                .debug_selector(|| "worktree_preview_scroll_container".to_string())
                                .relative()
                                .h_full()
                                .min_h(px(0.0))
                                .bg(theme.colors.window_bg)
                                .font_family(editor_font_family.clone())
                                .child(
                                    div()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .pr(scrollbar_gutter)
                                        .child(list),
                                )
                                .child(
                                    components::Scrollbar::new(
                                        "worktree_preview_scrollbar",
                                        scroll_handle.clone(),
                                    )
                                    .render(theme),
                                )
                                // Wrapped rows end at the pane, so there is
                                // nothing left of the line to scroll to.
                                .when(!wrapped, |container| {
                                    container.child(
                                        components::Scrollbar::horizontal(
                                            "worktree_preview_hscrollbar",
                                            scroll_handle,
                                        )
                                        .always_visible()
                                        .render(theme),
                                    )
                                })
                                .into_any_element()
                        }
                    }
                }
            }
        } else if is_conflict_resolver {
            self.render_conflict_resolver_pane(
                conflict_target_path,
                repo_id,
                theme,
                ui_scale_percent,
                editor_font_family.clone(),
                cx,
            )
        } else if is_conflict_compare {
            match (repo, conflict_target_path) {
                (None, _) => {
                    components::empty_state(theme, "Resolve", "No repository.").into_any_element()
                }
                (_, None) => {
                    components::empty_state(theme, "Resolve", "No conflicted file selected.")
                        .into_any_element()
                }
                (Some(repo), Some(path)) => {
                    let title: SharedString =
                        format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

                    match renderable_conflict_file(repo, &self.conflict_resolver, &path) {
                        RenderableConflictFile::Loading => {
                            components::empty_state(theme, title, "Loading conflict data…")
                                .into_any_element()
                        }
                        RenderableConflictFile::Error(error) => {
                            components::empty_state(theme, title, error).into_any_element()
                        }
                        RenderableConflictFile::Missing => {
                            components::empty_state(theme, title, "No conflict data.")
                                .into_any_element()
                        }
                        RenderableConflictFile::File(file) => {
                            let ours_label: SharedString = if file.ours.is_some() {
                                "Ours".into()
                            } else {
                                "Ours (deleted)".into()
                            };
                            let theirs_label: SharedString = if file.theirs.is_some() {
                                "Theirs".into()
                            } else {
                                "Theirs (deleted)".into()
                            };

                            let columns_header = components::split_columns_header(
                                theme,
                                ui_scale_percent,
                                ours_label,
                                theirs_label,
                            );

                            let diff_len = self.conflict_resolver.two_way_split_visible_len();

                            let diff_body: AnyElement = if diff_len == 0 {
                                components::empty_state(theme, "Diff", "No conflict diff to show.")
                                    .into_any_element()
                            } else {
                                let scroll_handle = self.diff_scroll.0.borrow().base_handle.clone();
                                let list = uniform_list(
                                    "conflict_compare_diff",
                                    diff_len,
                                    cx.processor(Self::render_conflict_compare_diff_rows),
                                )
                                .h_full()
                                .min_h(px(0.0))
                                .track_scroll(&self.diff_scroll)
                                .with_horizontal_sizing_behavior(
                                    gpui::ListHorizontalSizingBehavior::Unconstrained,
                                );

                                div()
                                    .id("conflict_compare_container")
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .h_full()
                                    .min_h(px(0.0))
                                    .bg(theme.colors.window_bg)
                                    .font_family(editor_font_family.clone())
                                    .child(columns_header)
                                    .child(
                                        div()
                                            .id("conflict_compare_scroll_container")
                                            .relative()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .child(
                                                div()
                                                    .h_full()
                                                    .min_h(px(0.0))
                                                    .pr(components::Scrollbar::visible_gutter(
                                                        self.diff_scroll.clone(),
                                                        components::ScrollbarAxis::Vertical,
                                                    ))
                                                    .child(list),
                                            )
                                            .child(
                                                components::Scrollbar::new(
                                                    "conflict_compare_scrollbar",
                                                    self.diff_scroll.clone(),
                                                )
                                                .always_visible()
                                                .render(theme),
                                            )
                                            .child(
                                                components::Scrollbar::horizontal(
                                                    "conflict_compare_hscrollbar",
                                                    scroll_handle,
                                                )
                                                .always_visible()
                                                .render(theme),
                                            ),
                                    )
                                    .into_any_element()
                            };

                            diff_body
                        }
                    }
                }
            }
        } else if wants_file_diff || wants_collapsed_diff {
            self.render_selected_file_diff(theme, window, cx)
        } else {
            match repo {
                None => components::empty_state(theme, "Diff", "No repository.").into_any_element(),
                Some(_repo) => match self.rendered_patch_diff_loadable() {
                    Some(Loadable::NotLoaded) | None => {
                        components::empty_state(theme, "Diff", "Select a file.").into_any_element()
                    }
                    Some(Loadable::Loading) => {
                        components::empty_state(theme, "Diff", "Loading").into_any_element()
                    }
                    Some(Loadable::Error(e)) => {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(e.clone(), cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("diff_error_scroll")
                            .font_family(editor_font_family.clone())
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    }
                    Some(Loadable::Ready(_diff)) => {
                        if wants_file_diff || wants_collapsed_diff {
                            self.render_selected_file_diff(theme, window, cx)
                        } else {
                            self.ensure_diff_visible_indices();
                            self.ensure_diff_wrap_visible_rows(window, cx);
                            self.maybe_autoscroll_diff_to_first_change();

                            {
                                if self.patch_diff_row_len() == 0 {
                                    components::empty_state(theme, "Diff", "No differences.")
                                        .into_any_element()
                                } else if self.diff_visible_len() == 0 {
                                    components::empty_state(theme, "Diff", "Nothing to render.")
                                        .into_any_element()
                                } else {
                                    let markers = self.diff_scrollbar_markers_cache.clone();
                                    match self.diff_view {
                                        DiffViewMode::Inline => {
                                            let horizontal_scrollbar_gutter =
                                                components::Scrollbar::gutter(
                                                    components::ScrollbarAxis::Horizontal,
                                                );
                                            let scrollbar_gutter = self
                                                .diff_vertical_scrollbar_gutter_for_column(
                                                    DiffHorizontalScrollColumn::Primary,
                                                    self.diff_scroll.clone(),
                                                );
                                            let list = uniform_list(
                                                "diff",
                                                self.diff_visible_len(),
                                                cx.processor(Self::render_diff_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .pb(if self.diff_word_wrap {
                                                px(0.0)
                                            } else {
                                                horizontal_scrollbar_gutter
                                            })
                                            .track_scroll(&self.diff_scroll)
                                            .when(!self.diff_word_wrap, |list| {
                                                list.with_horizontal_sizing_behavior(
                                                    gpui::ListHorizontalSizingBehavior::Unconstrained,
                                                )
                                            });
                                            div()
                                                .id("diff_scroll_container")
                                                .relative()
                                                .h_full()
                                                .min_h(px(0.0))
                                                .bg(theme.colors.window_bg)
                                                .font_family(editor_font_family.clone())
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .min_h(px(0.0))
                                                        .pr(scrollbar_gutter)
                                                        .child(list),
                                                )
                                                .child(
                                                    components::Scrollbar::new(
                                                        "diff_scrollbar",
                                                        self.diff_scroll.clone(),
                                                    )
                                                    .markers(markers)
                                                    .always_visible()
                                                    .render(theme),
                                                )
                                                .when(!self.diff_word_wrap, |d| {
                                                    d.child(Self::render_diff_horizontal_scrollbar(
                                                        theme,
                                                        "diff_hscrollbar",
                                                        self.diff_scroll.clone(),
                                                        scrollbar_gutter,
                                                        "diff_hscrollbar",
                                                    ))
                                                })
                                                .into_any_element()
                                        }
                                        DiffViewMode::Split => {
                                            self.sync_diff_split_scroll();
                                            let vertical_sync_enabled =
                                                self.diff_scroll_sync.includes_vertical();
                                            let count = self.diff_visible_len();
                                            let horizontal_scrollbar_gutter =
                                                components::Scrollbar::gutter(
                                                    components::ScrollbarAxis::Horizontal,
                                                );
                                            let left_scrollbar_gutter = self
                                                .diff_vertical_scrollbar_gutter_for_column(
                                                    DiffHorizontalScrollColumn::Primary,
                                                    self.diff_scroll.clone(),
                                                );
                                            let right_scrollbar_gutter = self
                                                .diff_vertical_scrollbar_gutter_for_column(
                                                    DiffHorizontalScrollColumn::SplitRight,
                                                    self.diff_split_right_scroll.clone(),
                                                );
                                            let shared_scrollbar_gutter = if vertical_sync_enabled {
                                                left_scrollbar_gutter
                                            } else {
                                                px(0.0)
                                            };
                                            let handle_w = px(PANE_RESIZE_HANDLE_PX);
                                            let main_w = (self.main_pane_content_width(cx)
                                                - shared_scrollbar_gutter)
                                                .max(px(0.0));
                                            let (_, min_col_w) = diff_split_drag_params(main_w);
                                            let (left_w, right_w) = diff_split_column_widths(
                                                main_w,
                                                self.diff_split_ratio,
                                            );
                                            let left = uniform_list(
                                                "diff_split_left",
                                                count,
                                                cx.processor(Self::render_diff_split_left_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .pb(if self.diff_word_wrap {
                                                px(0.0)
                                            } else {
                                                horizontal_scrollbar_gutter
                                            })
                                            .track_scroll(&self.diff_scroll)
                                            .when(!self.diff_word_wrap, |list| {
                                                list.with_horizontal_sizing_behavior(
                                                    gpui::ListHorizontalSizingBehavior::Unconstrained,
                                                )
                                            });
                                            let right = uniform_list(
                                                "diff_split_right",
                                                count,
                                                cx.processor(Self::render_diff_split_right_rows),
                                            )
                                            .h_full()
                                            .min_h(px(0.0))
                                            .pb(if self.diff_word_wrap {
                                                px(0.0)
                                            } else {
                                                horizontal_scrollbar_gutter
                                            })
                                            .track_scroll(&self.diff_split_right_scroll)
                                            .when(!self.diff_word_wrap, |list| {
                                                list.with_horizontal_sizing_behavior(
                                                    gpui::ListHorizontalSizingBehavior::Unconstrained,
                                                )
                                            });
                                            let collapsed_file_stat = self
                                                .is_collapsed_diff_projection_active()
                                                .then(|| self.collapsed_diff_total_file_stat())
                                                .flatten();
                                            let (left_label, right_label) =
                                                self.split_diff_pane_labels();
                                            let left_header = Self::split_column_header_label(
                                                left_label,
                                                collapsed_file_stat.map(|(_, removed)| removed),
                                                '-',
                                                theme.colors.diff_remove_text,
                                            );
                                            let right_header = Self::split_column_header_label(
                                                right_label,
                                                collapsed_file_stat.map(|(added, _)| added),
                                                '+',
                                                theme.colors.diff_add_text,
                                            );

                                            let split_dragging = self.diff_split_resize.is_some();
                                            let resize_handle = |id: &'static str| {
                                                div()
                                                    .id(id)
                                                    .group(id)
                                                    .w(handle_w)
                                                    .h_full()
                                                    .cursor(CursorStyle::ResizeLeftRight)
                                                    .child(components::resize_grip(
                                                        theme,
                                                        ui_scale_percent,
                                                        id,
                                                        components::ResizeGripAxis::Vertical,
                                                        split_dragging,
                                                        Some(theme.colors.border),
                                                    ))
                                                    .on_drag(
                                                        DiffSplitResizeHandle::Divider,
                                                        |_handle, _offset, _window, cx| {
                                                            cx.new(|_cx| DiffSplitResizeDragGhost)
                                                        },
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  e: &MouseDownEvent,
                                                                  _w,
                                                                  cx| {
                                                                cx.stop_propagation();
                                                                crate::press_gesture::claim_press(
                                                                    cx,
                                                                );
                                                                this.diff_split_resize = Some(
                                                                    DiffSplitResizeState {
                                                                        handle:
                                                                            DiffSplitResizeHandle::Divider,
                                                                        start_x: e.position.x,
                                                                        start_ratio: this
                                                                            .diff_split_ratio,
                                                                    },
                                                                );
                                                                cx.notify();
                                                            },
                                                        ),
                                                    )
                                                    .on_drag_move(cx.listener(
                                                        move |this,
                                                              e: &gpui::DragMoveEvent<
                                                            DiffSplitResizeHandle,
                                                        >,
                                                              _w,
                                                              cx| {
                                                            let Some(state) = this.diff_split_resize
                                                            else {
                                                                return;
                                                            };
                                                            if state.handle != *e.drag(cx) {
                                                                return;
                                                            }

                                                            let scrollbar_gutter = if this
                                                                .diff_scroll_sync
                                                                .includes_vertical()
                                                            {
                                                                components::Scrollbar::visible_gutter(
                                                                    this.diff_scroll.clone(),
                                                                    components::ScrollbarAxis::Vertical,
                                                                )
                                                            } else {
                                                                px(0.0)
                                                            };
                                                            let main_w = (this
                                                                .main_pane_content_width(cx)
                                                                - scrollbar_gutter)
                                                                .max(px(0.0));
                                                            let available =
                                                                (main_w - handle_w).max(px(0.0));
                                                            let dx =
                                                                e.event.position.x - state.start_x;
                                                            match next_diff_split_drag_ratio(
                                                                available,
                                                                min_col_w,
                                                                state.start_ratio,
                                                                dx,
                                                            ) {
                                                                None => {
                                                                    this.diff_split_ratio = 0.5;
                                                                }
                                                                Some(next_ratio) => {
                                                                    this.diff_split_ratio =
                                                                        next_ratio;
                                                                }
                                                            }
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .on_mouse_up(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _e, _w, cx| {
                                                            this.diff_split_resize = None;
                                                            cx.notify();
                                                        }),
                                                    )
                                                    .on_mouse_up_out(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _e, _w, cx| {
                                                            this.diff_split_resize = None;
                                                            cx.notify();
                                                        }),
                                                    )
                                            };

                                            let columns_header = div()
                                                .id("diff_split_columns_header")
                                                .debug_selector(|| {
                                                    "diff_split_columns_header".to_string()
                                                })
                                                .w_full()
                                                .h(components::control_height(ui_scale_percent))
                                                .flex()
                                                .items_center()
                                                .text_xs()
                                                .text_color(theme.colors.text_muted)
                                                .bg(theme.colors.surface_bg_elevated)
                                                .border_b_1()
                                                .border_color(theme.colors.border)
                                                .child(
                                                    div()
                                                        .w(left_w)
                                                        .min_w(px(0.0))
                                                        .px_2()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .child(left_header),
                                                )
                                                .child(resize_handle(
                                                    "diff_split_resize_handle_header",
                                                ))
                                                .child(
                                                    div()
                                                        .w(right_w)
                                                        .min_w(px(0.0))
                                                        .px_2()
                                                        .overflow_hidden()
                                                        .whitespace_nowrap()
                                                        .child(right_header),
                                                );

                                            div()
                                                .id("diff_split_scroll_container")
                                                .relative()
                                                .h_full()
                                                .min_h(px(0.0))
                                                .flex()
                                                .flex_col()
                                                .bg(theme.colors.window_bg)
                                                .font_family(editor_font_family.clone())
                                                .child(columns_header)
                                                .child(
                                                    div()
                                                        .relative()
                                                        .pr(shared_scrollbar_gutter)
                                                        .flex()
                                                        .flex_col()
                                                        .flex_1()
                                                        .min_h(px(0.0))
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_h(px(0.0))
                                                                .flex()
                                                                .child(
                                                                    div()
                                                                        .relative()
                                                                        .w(left_w)
                                                                        .min_w(px(0.0))
                                                                        .h_full()
                                                                        .child(
                                                                            div()
                                                                                .h_full()
                                                                                .min_h(px(0.0))
                                                                                .pr(
                                                                                    if vertical_sync_enabled {
                                                                                        px(0.0)
                                                                                    } else {
                                                                                        left_scrollbar_gutter
                                                                                    },
                                                                                )
                                                                                .child(left),
                                                                        )
                                                                        .when(
                                                                            !vertical_sync_enabled,
                                                                            |d| {
                                                                                d.child(
                                                                                    components::Scrollbar::new(
                                                                                        "diff_split_left_scrollbar",
                                                                                        self.diff_scroll.clone(),
                                                                                    )
                                                                                    .markers(
                                                                                        markers
                                                                                            .clone(),
                                                                                    )
                                                                                    .always_visible()
                                                                                    .render(theme),
                                                                                )
                                                                            },
                                                                        )
                                                                        .when(
                                                                            !self.diff_word_wrap,
                                                                            |d| {
                                                                                d.child(
                                                                                    Self::render_diff_horizontal_scrollbar(
                                                                                        theme,
                                                                                        "diff_split_left_hscrollbar",
                                                                                        self.diff_scroll.clone(),
                                                                                        if vertical_sync_enabled {
                                                                                            px(0.0)
                                                                                        } else {
                                                                                            left_scrollbar_gutter
                                                                                        },
                                                                                        "diff_split_left_hscrollbar",
                                                                                    ),
                                                                                )
                                                                            },
                                                                        ),
                                                                )
                                                                .child(resize_handle(
                                                                    "diff_split_resize_handle_body",
                                                                ))
                                                                .child(
                                                                    div()
                                                                        .relative()
                                                                        .w(right_w)
                                                                        .min_w(px(0.0))
                                                                        .h_full()
                                                                        .child(
                                                                            div()
                                                                                .h_full()
                                                                                .min_h(px(0.0))
                                                                                .pr(
                                                                                    if vertical_sync_enabled {
                                                                                        px(0.0)
                                                                                    } else {
                                                                                        right_scrollbar_gutter
                                                                                    },
                                                                                )
                                                                                .child(right),
                                                                        )
                                                                        .when(
                                                                            !vertical_sync_enabled,
                                                                            |d| {
                                                                                d.child(
                                                                                    components::Scrollbar::new(
                                                                                        "diff_split_right_scrollbar",
                                                                                        self.diff_split_right_scroll.clone(),
                                                                                    )
                                                                                    .markers(
                                                                                        markers
                                                                                            .clone(),
                                                                                    )
                                                                                    .always_visible()
                                                                                    .render(theme),
                                                                                )
                                                                            },
                                                                        )
                                                                        .when(
                                                                            !self.diff_word_wrap,
                                                                            |d| {
                                                                                d.child(
                                                                                    Self::render_diff_horizontal_scrollbar(
                                                                                        theme,
                                                                                        "diff_split_right_hscrollbar",
                                                                                        self.diff_split_right_scroll.clone(),
                                                                                        if vertical_sync_enabled {
                                                                                            px(0.0)
                                                                                        } else {
                                                                                            right_scrollbar_gutter
                                                                                        },
                                                                                        "diff_split_right_hscrollbar",
                                                                                    ),
                                                                                )
                                                                            },
                                                                        ),
                                                                ),
                                                        ),
                                                )
                                                .when(vertical_sync_enabled, |d| {
                                                    d.child(
                                                        components::Scrollbar::new(
                                                            "diff_scrollbar",
                                                            self.diff_scroll.clone(),
                                                        )
                                                        .markers(markers)
                                                        .always_visible()
                                                        .render(theme),
                                                    )
                                                })
                                                .into_any_element()
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        };
        self.diff_text_layout_cache_epoch = self.diff_text_layout_cache_epoch.wrapping_add(1);
        self.prune_diff_text_layout_cache();
        self.diff_text_hitboxes.clear();
        // The map still holds last frame's buttons, so it is the one place that
        // knows a hovered button has stopped being painted — the row itself
        // clears its hover on the next mouse move, but a row that scrolled away
        // or stopped being a change line paints no handler to do it, and a wheel
        // scroll delivers no mouse move at all.
        self.clear_diff_stage_gutter_hover_if_unpainted(cx);
        self.diff_stage_gutter_cells.clear();
        let diff_editor_menu_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == "diff_editor_menu");
        let diff_search_overlay = self.render_diff_search_overlay(theme, ui_scale_percent, cx);

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .bg(theme.colors.surface_bg_elevated)
            .when(diff_editor_menu_active, |d| d.bg(theme.colors.active))
            .track_focus(&self.diff_panel_focus_handle)
            .on_action(
                cx.listener(|this, _: &crate::view::TextInputDiffPrevFile, window, cx| {
                    if let Some(repo_id) = this.active_repo_id()
                        && this
                            .try_select_adjacent_diff_file_preserving_focus(repo_id, -1, window, cx)
                    {
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::view::TextInputDiffNextFile, window, cx| {
                    if let Some(repo_id) = this.active_repo_id()
                        && this
                            .try_select_adjacent_diff_file_preserving_focus(repo_id, 1, window, cx)
                    {
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(
                |this, _: &crate::view::TextInputDiffPrevSearchMatchOrChange, _window, cx| {
                    if this.navigate_prev_search_match_or_diff_change(cx) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(
                |this, _: &crate::view::TextInputDiffNextSearchMatchOrChange, _window, cx| {
                    if this.navigate_next_search_match_or_diff_change(cx) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(
                |this, _: &crate::view::TextInputDiffPrevChange, _window, cx| {
                    if this.navigate_prev_diff_change(cx) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(
                |this, _: &crate::view::TextInputDiffNextChange, _window, cx| {
                    if this.navigate_next_diff_change(cx) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseDownEvent, window, cx| {
                    window.focus(&this.diff_panel_focus_handle, cx);
                }),
            )
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                if this.handle_diff_shortcut(&e.keystroke, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                header
                    .h(components::control_height_md(ui_scale_percent))
                    .px_2()
                    .bg(theme.colors.surface_bg_elevated)
                    .border_b_1()
                    .border_color(theme.colors.border),
            )
            .child(
                div()
                    .id("diff_body_container")
                    .debug_selector(|| "diff_body_container".to_string())
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .h_full()
                    .child(body),
            )
            .when_some(diff_search_overlay, |d, overlay| d.child(overlay))
            .child(DiffTextSelectionTracker { view: cx.entity() })
    }
}
