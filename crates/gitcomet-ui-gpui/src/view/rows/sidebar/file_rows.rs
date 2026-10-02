//! Changed-file rows for commit, worktree and range file lists. The lists
//! share their directory and file rows; each keeps its own data source,
//! selection, and click action.

use super::*;
use gitcomet_core::domain::CommitFileChange;

/// Which changed-file list a row belongs to: its element and selector ids,
/// and the list whose collapsed directories it toggles.
#[derive(Clone, Copy)]
struct ChangedFileList {
    dir_id: &'static str,
    file_id: &'static str,
    row_group: &'static str,
    list: crate::view::rows::FileListId,
}

impl ChangedFileList {
    const COMMIT: Self = Self {
        dir_id: "commit_file_dir",
        file_id: "commit_file",
        row_group: "commit_file_row",
        list: crate::view::rows::FileListId::CommitFiles,
    };
    const WORKTREE: Self = Self {
        dir_id: "worktree_file_dir",
        file_id: "worktree_file",
        row_group: "worktree_file_row",
        list: crate::view::rows::FileListId::WorktreeFiles,
    };
    const RANGE: Self = Self {
        dir_id: "range_file_dir",
        file_id: "range_file",
        row_group: "range_file_row",
        list: crate::view::rows::FileListId::RangeFiles,
    };
}

/// One file row's inputs in a details-pane list.
struct ChangedFileRow<'a> {
    list: ChangedFileList,
    repo_id: RepoId,
    ix: usize,
    file: &'a CommitFileChange,
    presentation: &'a crate::view::rows::CommitFileRowPresentation,
    is_tree: bool,
    depth: usize,
    selected: bool,
    context_menu_active: bool,
    path_alignment_group: Option<components::PathTruncationAlignmentGroup>,
    diff_stat: bool,
}

impl DetailsPaneView {
    /// A directory row of `list`; `None` for a file row.
    fn changed_file_directory_row(
        list: ChangedFileList,
        repo_id: RepoId,
        ix: usize,
        row: crate::view::rows::FileListRow,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let (element, toggle) = crate::view::rows::changed_file_directory_row(
            (list.dir_id, ix).into(),
            move || format!("{}_{}_{}", list.dir_id, repo_id.0, ix),
            row,
            theme,
            ui_scale_percent,
        )?;
        let crate::view::rows::DirectoryToggle {
            key,
            chain,
            collapsed,
        } = toggle;
        Some(
            element
                .on_activate(
                    false,
                    controls::ControlActivation::Composite,
                    cx.listener(move |this, e: &ClickEvent, _window, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        this.toggle_file_list_dir(
                            repo_id,
                            list.list,
                            Arc::clone(&key),
                            Arc::clone(&chain),
                            collapsed,
                            cx,
                        );
                    }),
                )
                .into_any_element(),
        )
    }

    /// A file row's body; the caller adds its click action and tooltip (the
    /// returned label).
    fn changed_file_row(
        row: ChangedFileRow<'_>,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) -> (gpui::Stateful<gpui::Div>, SharedString) {
        let ChangedFileRow {
            list,
            repo_id,
            ix,
            file,
            presentation,
            is_tree,
            depth,
            selected,
            context_menu_active,
            path_alignment_group,
            diff_stat,
        } = row;
        crate::view::rows::changed_file_row(
            crate::view::rows::ChangedFileRow {
                element_id: (list.file_id, ix).into(),
                row_group: format!("{}_{ix}", list.row_group).into(),
                selector: move || format!("{}_{}_{}", list.file_id, repo_id.0, ix),
                file,
                presentation,
                is_tree,
                depth,
                selected,
                context_menu_active,
                path_alignment_group,
                diff_stat,
                leading: None,
            },
            theme,
            ui_scale_percent,
            cx,
        )
    }

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
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                    directory => {
                        return Self::changed_file_directory_row(
                            ChangedFileList::COMMIT,
                            repo_id,
                            ix,
                            directory,
                            theme,
                            ui_scale_percent,
                            cx,
                        );
                    }
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) =
                    details.files.get(source_ix).zip(file_rows.get(source_ix))?;
                let commit_id = details.id.clone();

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

                let commit_id_for_click = commit_id.clone();
                let commit_id_for_menu = commit_id.clone();
                // One owned copy shared by both handlers instead of one each.
                let path_for_click: Arc<std::path::PathBuf> = Arc::new(f.path.clone());
                let path_for_menu = Arc::clone(&path_for_click);
                // A rename's old side loads from where the file came from.
                let old_path_for_click = f.old_path.clone();

                let (row, tooltip) = Self::changed_file_row(
                    ChangedFileRow {
                        list: ChangedFileList::COMMIT,
                        repo_id,
                        ix,
                        file: f,
                        presentation,
                        is_tree,
                        depth,
                        selected,
                        context_menu_active,
                        path_alignment_group: path_alignment_group.clone(),
                        diff_stat: true,
                    },
                    theme,
                    ui_scale_percent,
                    cx,
                );
                let row = row
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
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                    directory => {
                        return Self::changed_file_directory_row(
                            ChangedFileList::WORKTREE,
                            repo_id,
                            ix,
                            directory,
                            theme,
                            ui_scale_percent,
                            cx,
                        );
                    }
                };
                // `source_ix` indexes `inputs.entries`, which the reducer
                // re-derives independently. Sorting the display must not change
                // the index a click sends.
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let selected = selected_ix_now == Some(source_ix);
                let ix_for_click = source_ix;
                let inputs_for_click = Arc::clone(&inputs);
                let worktree_path_for_click = worktree_path.clone();
                let origin_for_click = origin.clone();

                let (row, tooltip) = Self::changed_file_row(
                    ChangedFileRow {
                        list: ChangedFileList::WORKTREE,
                        repo_id,
                        ix,
                        file: f,
                        presentation,
                        is_tree,
                        depth,
                        selected,
                        context_menu_active: false,
                        path_alignment_group: path_alignment_group.clone(),
                        // Worktree rows show no line counts.
                        diff_stat: false,
                    },
                    theme,
                    ui_scale_percent,
                    cx,
                );
                let row = row
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

    /// Render the changed-file rows for an active two-point comparison. Mirrors
    /// [`Self::render_commit_file_rows`] but sources the file list from
    /// `history_state.range_files` and builds `DiffTarget::CommitRange` targets,
    /// so clicking a file loads its diff through the normal diff pipeline.
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
                    crate::view::rows::FileListRow::File { ordinal, depth } => (ordinal, depth),
                    directory => {
                        return Self::changed_file_directory_row(
                            ChangedFileList::RANGE,
                            repo_id,
                            ix,
                            directory,
                            theme,
                            ui_scale_percent,
                            cx,
                        );
                    }
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let target = DiffTarget::commit_range(from.clone(), to.clone(), None).for_change(f);
                let selected = repo.diff_state.diff_target.as_ref() == Some(&target);
                let target_for_click = target.clone();

                let (row, tooltip) = Self::changed_file_row(
                    ChangedFileRow {
                        list: ChangedFileList::RANGE,
                        repo_id,
                        ix,
                        file: f,
                        presentation,
                        is_tree,
                        depth,
                        selected,
                        context_menu_active: false,
                        path_alignment_group: path_alignment_group.clone(),
                        diff_stat: true,
                    },
                    theme,
                    ui_scale_percent,
                    cx,
                );
                let row = row
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
