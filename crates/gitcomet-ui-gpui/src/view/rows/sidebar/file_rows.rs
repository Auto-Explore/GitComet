//! Changed-file rows for commit, worktree and range file lists.

use super::*;

impl DetailsPaneView {
    pub(in crate::view) fn render_commit_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return Vec::new();
        };

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let repo_id = repo.id;
        let has_active_menu = this.active_context_menu_invoker.is_some();
        let file_rows = this.cached_commit_file_rows(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let projection = this.cached_commit_file_projection(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let plan = this.cached_commit_file_plan(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let is_tree = plan.is_tree();
        let visible_signature = this.commit_files_visible_signature(
            repo_id,
            repo.history_state.commit_details_rev,
            &range,
            projection.source_indices.len(),
        );
        // A tree shows leaf names, which have no shared prefix to align, and a
        // row that reported into the group would anchor it on the shortest one.
        let path_alignment_group = (!is_tree).then(|| {
            this.commit_files_path_alignment_group
                .visible_rows(visible_signature)
        });

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("commit_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || format!("commit_file_dir_{}_{}", repo_id.0, ix))
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::CommitFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) =
                    details.files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let commit_id = details.id.clone();
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);

                let context_menu_active = has_active_menu && {
                    let invoker: SharedString = format!(
                        "commit_file_menu_{}_{}_{}",
                        repo_id.0,
                        commit_id.as_ref(),
                        f.path.display()
                    )
                    .into();
                    this.active_context_menu_invoker.as_ref() == Some(&invoker)
                };
                let selected = repo
                    .diff_state
                    .diff_target
                    .as_ref()
                    .is_some_and(|t| match t {
                        DiffTarget::Commit {
                            commit_id: t_commit_id,
                            path: Some(t_path),
                            ..
                        } => t_commit_id == &commit_id && t_path == &f.path,
                        _ => false,
                    });
                let row_group: SharedString = format!("commit_file_row_{ix}").into();
                let interaction = crate::view::rows::FileRowInteraction::new(
                    theme,
                    tint,
                    selected,
                    context_menu_active,
                );
                let badge_disc = interaction.badge_disc(row_group.clone());

                let commit_id_for_click = commit_id.clone();
                let commit_id_for_menu = commit_id.clone();
                // One owned copy shared by both handlers instead of one each.
                let path_for_click: Arc<std::path::PathBuf> = Arc::new(f.path.clone());
                let path_for_menu = Arc::clone(&path_for_click);
                // A rename's old side loads from where the file came from.
                let old_path_for_click = f.old_path.clone();
                let tooltip = path_label.clone();

                let row = div()
                    .id(("commit_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("commit_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .when(f.additions.is_some() || f.deletions.is_some(), |row| {
                        row.child(div().flex_none().child(components::diff_stat(
                            theme,
                            ui_scale_percent,
                            f.additions.unwrap_or(0) as usize,
                            f.deletions.unwrap_or(0) as usize,
                        )))
                    })
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            let target = DiffTarget::commit(
                                commit_id_for_click.clone(),
                                Some((*path_for_click).clone()),
                            )
                            .with_old_path(old_path_for_click.clone());
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref() == Some(&target)
                            });

                            if selected {
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            } else {
                                this.focus_diff_panel(window, cx);
                                this.store.dispatch(Msg::SelectDiff { repo_id, target });
                            }
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());
                let row = row.on_pointer_click(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let invoker: SharedString = format!(
                            "commit_file_menu_{}_{}_{}",
                            repo_id.0,
                            commit_id_for_menu.as_ref(),
                            path_for_menu.display()
                        )
                        .into();
                        this.open_popover_at(
                            (PopoverKind::CommitFileMenu {
                                repo_id,
                                commit_id: commit_id_for_menu.clone(),
                                path: (*path_for_menu).clone(),
                            })
                            .invoked_by(invoker),
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                );

                Some(row.into_any_element())
            })
            .collect()
    }

    /// Render the changed-file rows for an active two-point comparison. Mirrors
    /// [`Self::render_commit_file_rows`] but sources the file list from
    /// `history_state.range_files` and builds `DiffTarget::CommitRange` targets,
    /// so clicking a file loads its diff through the normal diff pipeline.
    /// Changed files of a linked worktree that is not this tab.
    ///
    /// Clicking one opens it through the inline foreign-diff machinery — the
    /// same path submodule diffs take — so the diff renders here rather than
    /// forcing a tab switch.
    pub(in crate::view) fn render_worktree_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let repo_id = repo.id;
        let worktree_dirty_rev = repo.worktree_dirty_rev;
        let Some(summary) = this.selected_worktree_summary() else {
            return Vec::new();
        };
        // Derived once per scan, not per frame: this list is virtualized, but the
        // inputs behind it are one entry per changed file.
        let inputs = this.cached_worktree_file_inputs(repo_id, worktree_dirty_rev, summary);
        let files = &inputs.files;

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let file_rows =
            this.cached_worktree_file_rows(repo_id, worktree_dirty_rev, &summary.path, files);
        let projection =
            this.cached_worktree_file_projection(repo_id, worktree_dirty_rev, &summary.path, files);
        let plan =
            this.cached_worktree_file_plan(repo_id, worktree_dirty_rev, &summary.path, files);
        let is_tree = plan.is_tree();
        let selected_ix_now = repo
            .diff_state
            .inline_submodule_diff
            .as_ref()
            .filter(|inline| inline.submodule_repo_path == summary.path)
            .map(|inline| inline.selected_ix);
        let visible_signature = this.worktree_files_visible_signature(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &range,
            files.len(),
        );
        let path_alignment_group = (!is_tree).then(|| {
            this.worktree_files_path_alignment_group
                .visible_rows(visible_signature)
        });
        let worktree_path = summary.path.clone();
        let origin = gitcomet_state::model::ForeignDiffOrigin::Worktree {
            branch: summary.branch.clone(),
            detached: summary.detached,
        };

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("worktree_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || {
                                format!("worktree_file_dir_{}_{}", repo_id.0, ix)
                            })
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::WorktreeFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                // `source_ix` indexes `inputs.entries`, which the reducer
                // re-derives independently. Sorting the display must not change
                // the index a click sends.
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);
                let selected = selected_ix_now == Some(source_ix);
                let tooltip = path_label.clone();
                let ix_for_click = source_ix;
                let inputs_for_click = Arc::clone(&inputs);
                let worktree_path_for_click = worktree_path.clone();
                let origin_for_click = origin.clone();

                let row_group: SharedString = format!("worktree_file_row_{ix}").into();
                let interaction =
                    crate::view::rows::FileRowInteraction::new(theme, tint, selected, false);
                let badge_disc = interaction.badge_disc(row_group.clone());

                let row = div()
                    .id(("worktree_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("worktree_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            this.focus_diff_panel(window, cx);
                            this.store.dispatch(Msg::OpenInlineSubmoduleDiff {
                                repo_id,
                                origin: origin_for_click.clone(),
                                submodule_repo_path: worktree_path_for_click.clone(),
                                parent_submodule_path: worktree_path_for_click.clone(),
                                entries: Arc::clone(&inputs_for_click.entries),
                                selected_ix: ix_for_click,
                            });
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());

                Some(row.into_any_element())
            })
            .collect()
    }

    pub(in crate::view) fn render_range_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Some(range_selection) = repo.history_state.range_selection.clone() else {
            return Vec::new();
        };
        let Loadable::Ready(files) = &repo.history_state.range_files else {
            return Vec::new();
        };
        let files = files.clone();

        let theme = this.theme;
        let ui_scale_percent = this.ui_scale_percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let repo_id = repo.id;
        // A merge-base comparison's file diffs start at the resolved base.
        let from = range_selection.diff_from().clone();
        let to = range_selection.to.clone();
        let file_rows =
            this.cached_range_file_rows(repo_id, repo.history_state.range_files_rev, &files);
        let projection =
            this.cached_range_file_projection(repo_id, repo.history_state.range_files_rev, &files);
        let plan = this.cached_range_file_plan(repo_id, repo.history_state.range_files_rev, &files);
        let is_tree = plan.is_tree();
        let visible_signature = this.range_files_visible_signature(
            repo_id,
            repo.history_state.range_files_rev,
            &range,
            files.len(),
        );
        let path_alignment_group = (!is_tree).then(|| {
            this.range_files_path_alignment_group
                .visible_rows(visible_signature)
        });

        let rows: Vec<(usize, crate::view::rows::FileListRow)> = range
            .filter_map(|row_ix| {
                plan.row_at(crate::view::rows::RowIx(row_ix))
                    .map(|row| (row_ix, row))
            })
            .collect();

        rows.into_iter()
            .filter_map(|(ix, row)| {
                let (ordinal, depth) = match row {
                    crate::view::rows::FileListRow::Directory {
                        key,
                        label,
                        depth,
                        collapsed,
                        chain,
                        subtree: _,
                        additions,
                        deletions,
                    } => {
                        return Some(
                            crate::view::rows::directory_row(
                                crate::view::rows::DirectoryRowProps {
                                    theme,
                                    ui_scale_percent,
                                    id: ("range_file_dir", ix).into(),
                                    label: &label,
                                    depth,
                                    collapsed,
                                    additions,
                                    deletions,
                                    row_height: sidebar_list_row_height(theme, ui_scale_percent),
                                    row_group: None,
                                    detail: crate::view::rows::directory_row_detail_for_width(
                                        // No width probe on this list.
                                        gpui::Pixels::MAX,
                                        depth,
                                        additions.is_some() || deletions.is_some(),
                                        ui_scale_percent,
                                    ),
                                },
                            )
                            .debug_selector(move || format!("range_file_dir_{}_{}", repo_id.0, ix))
                            .on_activate(
                                false,
                                controls::ControlActivation::Composite,
                                cx.listener(move |this, e: &ClickEvent, _window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    this.toggle_file_list_dir(
                                        repo_id,
                                        crate::view::rows::FileListId::RangeFiles,
                                        Arc::clone(&key),
                                        Arc::clone(&chain),
                                        collapsed,
                                        cx,
                                    );
                                }),
                            )
                            .into_any_element(),
                        );
                    }
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let visuals = presentation.visuals;
                let path_label = if is_tree {
                    SharedString::from(
                        f.path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| presentation.label.to_string()),
                    )
                } else {
                    presentation.label.clone()
                };
                let (icon, color) = if f.is_submodule {
                    (visuals.icon, visuals.color(&theme))
                } else {
                    crate::view::rows::file_row_icon(&f.path, f.kind, &theme)
                };
                // The change kind rides the row wash and a badge on the icon's corner.
                let tint = crate::view::rows::file_kind_row_tint(f.kind, &theme);
                let badge = crate::view::rows::file_row_kind_badge(f.kind, &theme);
                let target = DiffTarget::commit_range(from.clone(), to.clone(), None).for_change(f);
                let selected = repo.diff_state.diff_target.as_ref() == Some(&target);
                let target_for_click = target.clone();
                let tooltip = path_label.clone();

                let row_group: SharedString = format!("range_file_row_{ix}").into();
                let interaction =
                    crate::view::rows::FileRowInteraction::new(theme, tint, selected, false);
                let badge_disc = interaction.badge_disc(row_group.clone());

                let row = div()
                    .id(("range_file", ix))
                    // Only so the badge disc can follow the row's hover fill.
                    .group(row_group.clone())
                    .debug_selector(move || format!("range_file_{}_{}", repo_id.0, ix))
                    .h(sidebar_list_row_height(theme, ui_scale_percent))
                    .flex()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .pl(if is_tree {
                        crate::view::rows::file_row_indent_px(depth, ui_scale_percent)
                    } else {
                        scaled_px(8.0)
                    })
                    .pr(scaled_px(8.0))
                    .w_full()
                    .map(|row| interaction.apply(row))
                    .child(crate::view::rows::file_row_icon_slot(
                        icon,
                        color,
                        badge,
                        badge_disc,
                        14.0,
                        16.0,
                        ui_scale_percent,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .line_height(theme.ui_text(18.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                match path_alignment_group.clone() {
                                    Some(group) => components::TruncatedText::aligned_path(
                                        path_label,
                                        theme.ui_text(14.0),
                                        group,
                                    ),
                                    // A tree row's label is a bare file name, so
                                    // there is no path to align against.
                                    None => components::TruncatedText::new(
                                        path_label,
                                        theme.ui_text(14.0),
                                    ),
                                }
                                .render(cx),
                            ),
                    )
                    .when(f.additions.is_some() || f.deletions.is_some(), |row| {
                        row.child(div().flex_none().child(components::diff_stat(
                            theme,
                            ui_scale_percent,
                            f.additions.unwrap_or(0) as usize,
                            f.deletions.unwrap_or(0) as usize,
                        )))
                    })
                    .on_activate(
                        false,
                        controls::ControlActivation::Composite,
                        cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref()
                                        == Some(&target_for_click)
                            });
                            if selected {
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            } else {
                                this.focus_diff_panel(window, cx);
                                this.store.dispatch(Msg::SelectDiff {
                                    repo_id,
                                    target: target_for_click.clone(),
                                });
                            }
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());

                Some(row.into_any_element())
            })
            .collect()
    }
}
