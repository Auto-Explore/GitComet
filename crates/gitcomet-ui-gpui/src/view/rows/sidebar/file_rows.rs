//! Changed-file rows for commit, worktree and range file lists. The lists
//! share their directory and file rows; each keeps its own data source,
//! selection, and click action.

use super::*;
use gitcomet_core::domain::{ApplyChangeSource, CommitFileChange};
use std::path::Path;

/// Which changed-file list a row belongs to: its element and selector ids,
/// and the list whose collapsed directories it toggles.
#[derive(Clone, Copy)]
struct ChangedFileList {
    dir_id: &'static str,
    group_id: &'static str,
    file_id: &'static str,
    row_group: &'static str,
    list: crate::view::rows::FileListId,
}

impl ChangedFileList {
    const COMMIT: Self = Self {
        dir_id: "commit_file_dir",
        group_id: "commit_file_group",
        file_id: "commit_file",
        row_group: "commit_file_row",
        list: crate::view::rows::FileListId::CommitFiles,
    };
    const WORKTREE: Self = Self {
        dir_id: "worktree_file_dir",
        group_id: "worktree_file_group",
        file_id: "worktree_file",
        row_group: "worktree_file_row",
        list: crate::view::rows::FileListId::WorktreeFiles,
    };
    const RANGE: Self = Self {
        dir_id: "range_file_dir",
        group_id: "range_file_group",
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
    /// A directory or group header row of `list`; `None` for a file row.
    /// Right-click on a folder opens its folder menu, whose "Apply changes"
    /// takes from `apply_source`.
    #[allow(clippy::too_many_arguments)]
    fn changed_file_directory_row(
        list: ChangedFileList,
        repo_id: RepoId,
        ix: usize,
        row: crate::view::rows::FileListRow,
        apply_source: Option<ApplyChangeSource>,
        active_menu: Option<&SharedString>,
        theme: AppTheme,
        ui_scale_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if let crate::view::rows::FileListRow::Group {
            group,
            label,
            count,
            collapsed,
        } = row
        {
            let pane = cx.weak_entity();
            return Some(crate::view::rows::group_header_row(
                crate::view::rows::GroupHeaderProps {
                    id: (list.group_id, group),
                    selector: format!("{}_{}_{label}", list.group_id, repo_id.0),
                    label,
                    count,
                    collapsed,
                },
                theme,
                crate::ui_scale::UiScale::current(cx),
                crate::view::rows::sidebar::sidebar_list_row_height(theme, ui_scale_percent),
                move |cx| {
                    let _ = pane.update(cx, |pane, cx| {
                        pane.toggle_file_list_group(repo_id, list.list, group, cx)
                    });
                },
            ));
        }
        // Only build the invoker while some menu is open.
        let menu_open = match (&row, active_menu) {
            (crate::view::rows::FileListRow::Directory { key, .. }, Some(active)) => {
                *active
                    == crate::view::rows::file_list_folder_menu_invoker(repo_id.0, list.list, key)
            }
            _ => false,
        };
        let (element, toggle) = crate::view::rows::changed_file_directory_row(
            (list.dir_id, ix).into(),
            move || format!("{}_{}_{}", list.dir_id, repo_id.0, ix),
            row,
            menu_open,
            theme,
            ui_scale_percent,
        )?;
        let crate::view::rows::DirectoryToggle {
            key,
            chain,
            collapsed,
        } = toggle;
        let menu_key = Arc::clone(&key);
        let menu_chain = chain.clone();
        Some(
            element
                .on_pointer_click(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let invoker = crate::view::rows::file_list_folder_menu_invoker(
                            repo_id.0, list.list, &menu_key,
                        );
                        this.open_popover_at(
                            PopoverKind::FileListFolderMenu {
                                repo_id,
                                list: list.list,
                                key: Arc::clone(&menu_key),
                                chain: menu_chain.clone(),
                                collapsed,
                                apply_source: apply_source.clone(),
                            }
                            .invoked_by(invoker),
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                )
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
        let active_menu = this.active_context_menu_invoker.clone();
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
        let multi_selected =
            multi_selected_paths(this, repo_id, crate::view::rows::FileListId::CommitFiles);

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
                            Some(ApplyChangeSource::Commit(details.id.clone())),
                            active_menu.as_ref(),
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
                let selected = match &multi_selected {
                    Some(paths) => paths.contains(&f.path),
                    None => repo
                        .diff_state
                        .diff_target
                        .as_ref()
                        .is_some_and(|t| match t {
                            DiffTarget::Commit {
                                commit_id: t_commit_id,
                                path: t_path,
                                ..
                            } => t_commit_id == &commit_id && t_path == &f.path,
                            _ => false,
                        }),
                };
                let display_position = plan.display_position(ordinal);

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
                            let list = crate::view::rows::FileListId::CommitFiles;
                            if select_file_list_row(
                                this,
                                repo_id,
                                list,
                                &path_for_click,
                                display_position,
                                e.modifiers(),
                            ) {
                                cx.notify();
                                return;
                            }
                            let target = DiffTarget::commit(
                                commit_id_for_click.clone(),
                                (*path_for_click).clone(),
                            )
                            .with_old_path(old_path_for_click.clone());
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref() == Some(&target)
                            });

                            if selected {
                                this.file_list_selection.remove(&(repo_id, list));
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
    /// Resolve against the current scan so sorting or a refresh cannot open a
    /// different file through a stale display index. Used by clicks and menus.
    pub(in crate::view) fn open_worktree_file_diff(
        &mut self,
        repo_id: RepoId,
        worktree_path: &Path,
        target: &DiffTarget,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return;
        };
        let Some(summary) = self
            .selected_worktree_summary()
            .filter(|summary| summary.path == worktree_path)
        else {
            return;
        };
        let inputs = self.cached_worktree_file_inputs(repo_id, repo.worktree_dirty_rev, summary);
        let Some(selected_ix) = inputs
            .entries
            .iter()
            .position(|entry| &entry.target == target)
        else {
            return;
        };
        let origin = gitcomet_state::model::ForeignDiffOrigin::Worktree {
            branch: summary.branch.clone(),
            detached: summary.detached,
        };
        self.focus_diff_panel(window, cx);
        self.store.dispatch(Msg::OpenInlineSubmoduleDiff {
            repo_id,
            origin,
            submodule_repo_path: worktree_path.to_path_buf(),
            parent_submodule_path: worktree_path.to_path_buf(),
            entries: Arc::clone(&inputs.entries),
            selected_ix,
        });
    }

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
        let active_menu = this.active_context_menu_invoker.clone();
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
                            None,
                            active_menu.as_ref(),
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
                let target_for_click = inputs.entries.get(source_ix)?.target.clone();
                let target_for_menu = target_for_click.clone();
                let worktree_path_for_menu = worktree_path.clone();
                let menu_invoker =
                    worktree_file_menu_invoker(repo_id, &worktree_path, &target_for_click);
                let context_menu_active = active_menu.as_ref() == Some(&menu_invoker);
                let worktree_path_for_click = worktree_path.clone();

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
                        context_menu_active,
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
                            this.open_worktree_file_diff(
                                repo_id,
                                &worktree_path_for_click,
                                &target_for_click,
                                window,
                                cx,
                            );
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone())
                    .on_pointer_click(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_popover_at(
                                PopoverKind::WorktreeFileMenu {
                                    repo_id,
                                    worktree_path: worktree_path_for_menu.clone(),
                                    target: target_for_menu.clone(),
                                }
                                .invoked_by(menu_invoker.clone()),
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
        let has_active_menu = this.active_context_menu_invoker.is_some();
        let active_menu = this.active_context_menu_invoker.clone();
        let apply_source = to.clone().map(|to| ApplyChangeSource::Range {
            from: from.clone(),
            to,
        });
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
        let multi_selected =
            multi_selected_paths(this, repo_id, crate::view::rows::FileListId::RangeFiles);

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
                            apply_source.clone(),
                            active_menu.as_ref(),
                            theme,
                            ui_scale_percent,
                            cx,
                        );
                    }
                };
                let source_ix = *projection.source_indices.get(ordinal.0)?;
                let (f, presentation) = files.get(source_ix).zip(file_rows.get(source_ix))?;
                let target = DiffTarget::commit_range(from.clone(), to.clone(), None).for_change(f);
                let selected = match &multi_selected {
                    Some(paths) => paths.contains(&f.path),
                    None => repo.diff_state.diff_target.as_ref() == Some(&target),
                };
                let display_position = plan.display_position(ordinal);
                let path_for_click = Arc::new(f.path.clone());
                let context_menu_active = has_active_menu
                    && this.active_context_menu_invoker.as_ref()
                        == Some(&range_file_menu_invoker(repo_id, &target));
                // One owned copy shared by both handlers instead of one each.
                let target_for_click = Arc::new(target);
                let target_for_menu = Arc::clone(&target_for_click);

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
                            let list = crate::view::rows::FileListId::RangeFiles;
                            if select_file_list_row(
                                this,
                                repo_id,
                                list,
                                &path_for_click,
                                display_position,
                                e.modifiers(),
                            ) {
                                cx.notify();
                                return;
                            }
                            let selected = this.active_repo().is_some_and(|repo| {
                                repo.id == repo_id
                                    && repo.diff_state.diff_target.as_ref()
                                        == Some(&*target_for_click)
                            });
                            if selected {
                                this.file_list_selection.remove(&(repo_id, list));
                                this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                            } else {
                                this.focus_diff_panel(window, cx);
                                this.store.dispatch(Msg::SelectDiff {
                                    repo_id,
                                    target: (*target_for_click).clone(),
                                });
                            }
                            cx.notify();
                        }),
                    )
                    .gitcomet_tooltip(theme, tooltip.clone());
                let row = row.on_pointer_click(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let DiffTarget::CommitRange {
                            from_commit_id,
                            to_commit_id,
                            path: Some(path),
                            ..
                        } = &*target_for_menu
                        else {
                            return;
                        };
                        let kind = PopoverKind::CommitRangeFileMenu {
                            repo_id,
                            from_commit_id: from_commit_id.clone(),
                            to_commit_id: to_commit_id.clone(),
                            path: path.clone(),
                        };
                        this.open_popover_at(
                            kind.invoked_by(range_file_menu_invoker(repo_id, &target_for_menu)),
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
}

/// The rows a commit or comparison list highlights for its selection; `None`
/// leaves the highlight to the previewed diff, as for a single selected row.
fn multi_selected_paths(
    this: &DetailsPaneView,
    repo_id: RepoId,
    list: crate::view::rows::FileListId,
) -> Option<rustc_hash::FxHashSet<std::path::PathBuf>> {
    let paths = this.commit_list_selected_paths(repo_id, list);
    (paths.len() > 1).then(|| paths.iter().cloned().collect())
}

/// Records a click in a commit or comparison list's selection. Returns true
/// for a Ctrl/Cmd or Shift click, which only changes the selection and leaves
/// the previewed diff alone.
fn select_file_list_row(
    this: &mut DetailsPaneView,
    repo_id: RepoId,
    list: crate::view::rows::FileListId,
    path: &std::path::Path,
    display_position: Option<usize>,
    modifiers: gpui::Modifiers,
) -> bool {
    this.commit_list_selection_apply_click(
        repo_id,
        list,
        path.to_path_buf(),
        display_position,
        modifiers,
    );
    modifiers.shift || modifiers.control || modifiers.platform
}

/// Identifies the comparison file row whose menu is open, so the row can show
/// it as active.
fn range_file_menu_invoker(repo_id: RepoId, target: &DiffTarget) -> SharedString {
    let DiffTarget::CommitRange {
        from_commit_id,
        to_commit_id,
        path,
        ..
    } = target
    else {
        return SharedString::default();
    };
    format!(
        "range_file_menu_{}_{}_{}_{}",
        repo_id.0,
        from_commit_id.as_ref(),
        to_commit_id.as_ref().map_or("worktree", |to| to.as_ref()),
        path.as_deref()
            .unwrap_or(std::path::Path::new(""))
            .display()
    )
    .into()
}

fn worktree_file_menu_invoker(
    repo_id: RepoId,
    worktree_path: &Path,
    target: &DiffTarget,
) -> SharedString {
    format!(
        "worktree_file_menu_{}_{:?}_{target:?}",
        repo_id.0, worktree_path
    )
    .into()
}
