use super::*;
use crate::view::review_navigation::{self, ReviewNavigation};
use crate::view::rows::{FileListId, FileListPlan, FileListRow, RowIx};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ReviewRow {
    File(PathBuf),
    Directory(Arc<std::path::Path>),
    Group(usize),
}

struct ReviewList {
    plan: Arc<FileListPlan>,
}

impl DetailsPaneView {
    pub(in crate::view) fn review_file_focus(&self, list: FileListId) -> &FocusHandle {
        match list {
            FileListId::CommitFiles => &self.review_file_focus_handles[0],
            FileListId::RangeFiles => &self.review_file_focus_handles[1],
            FileListId::WorktreeFiles => &self.review_file_focus_handles[2],
            FileListId::Status(section) => self.status_section_focus_handle(section),
        }
    }

    pub(super) fn visible_review_lists(&self) -> Vec<FileListId> {
        if !self.review_details_dismissed
            && let Some(repo) = self.active_repo()
        {
            if self.selected_worktree_summary().is_some() {
                return vec![FileListId::WorktreeFiles];
            }
            if repo.history_state.range_selection.is_some() {
                return vec![FileListId::RangeFiles];
            }
            if repo.history_state.selected_commit.is_some() {
                return vec![FileListId::CommitFiles];
            }
        }
        match self.change_tracking_view {
            ChangeTrackingView::Combined => vec![
                FileListId::Status(StatusSection::CombinedUnstaged),
                FileListId::Status(StatusSection::Staged),
            ],
            ChangeTrackingView::SplitUntracked => vec![
                FileListId::Status(StatusSection::Untracked),
                FileListId::Status(StatusSection::Unstaged),
                FileListId::Status(StatusSection::Staged),
            ],
        }
    }

    pub(in crate::view) fn visible_review_focus_handles(&self) -> Vec<FocusHandle> {
        self.visible_review_lists()
            .into_iter()
            .map(|list| self.review_file_focus(list).clone())
            .collect()
    }

    fn review_list(&self, list: FileListId) -> Option<ReviewList> {
        let repo = self.active_repo()?;
        let plan = match list {
            FileListId::CommitFiles => self.cached_commit_file_plan(
                repo.id,
                repo.history_state.commit_details_rev,
                &repo.history_state.commit_details.ready()?.files,
            ),
            FileListId::RangeFiles => self.cached_range_file_plan(
                repo.id,
                repo.history_state.range_files_rev,
                repo.history_state.range_files.ready()?,
            ),
            FileListId::WorktreeFiles => {
                let summary = self.selected_worktree_summary()?;
                let input =
                    self.cached_worktree_file_inputs(repo.id, repo.worktree_dirty_rev, summary);
                self.cached_worktree_file_plan(
                    repo.id,
                    repo.worktree_dirty_rev,
                    &summary.path,
                    &input.files,
                )
            }
            FileListId::Status(section) => self.status_file_plan(repo, section),
        };
        Some(ReviewList { plan })
    }

    // Resolve only the requested file from the existing cached projection.
    // Holding an arrow key must not clone every path in a large commit.
    fn review_target(&self, list: FileListId, ordinal: usize) -> Option<DiffTarget> {
        let repo = self.active_repo()?;
        Some(match list {
            FileListId::CommitFiles => {
                let details = repo.history_state.commit_details.ready()?;
                let projection = self.cached_commit_file_projection(
                    repo.id,
                    repo.history_state.commit_details_rev,
                    &details.files,
                );
                let file = details
                    .files
                    .get(*projection.source_indices.get(ordinal)?)?;
                DiffTarget::commit(details.id.clone(), file.path.clone())
                    .with_old_path(file.old_path.clone())
            }
            FileListId::RangeFiles => {
                let range = repo.history_state.range_selection.as_ref()?;
                let files = repo.history_state.range_files.ready()?;
                let projection = self.cached_range_file_projection(
                    repo.id,
                    repo.history_state.range_files_rev,
                    files,
                );
                DiffTarget::commit_range(range.from.clone(), range.to.clone(), None)
                    .for_change(files.get(*projection.source_indices.get(ordinal)?)?)
            }
            FileListId::WorktreeFiles => {
                let summary = self.selected_worktree_summary()?;
                let input =
                    self.cached_worktree_file_inputs(repo.id, repo.worktree_dirty_rev, summary);
                let projection = self.cached_worktree_file_projection(
                    repo.id,
                    repo.worktree_dirty_rev,
                    &summary.path,
                    &input.files,
                );
                input
                    .entries
                    .get(*projection.source_indices.get(ordinal)?)?
                    .target
                    .clone()
            }
            FileListId::Status(section) => {
                let entries = repo.status_entries_for_area(section.diff_area())?;
                let order = self.status_section_order(repo, section)?;
                DiffTarget::working_tree(
                    entries.get(*order.get(ordinal)?)?.path.clone(),
                    section.diff_area(),
                )
            }
        })
    }

    fn review_row_key(&self, list: FileListId, data: &ReviewList, row: usize) -> Option<ReviewRow> {
        Some(match data.plan.row_at(RowIx(row))? {
            FileListRow::File { ordinal, .. } => {
                ReviewRow::File(self.review_target(list, ordinal.0)?.file_path()?.to_owned())
            }
            FileListRow::Directory { key, .. } => ReviewRow::Directory(key),
            FileListRow::Group { group, .. } => ReviewRow::Group(group),
        })
    }

    fn review_scroll(&self, list: FileListId) -> &UniformListScrollHandle {
        match list {
            FileListId::CommitFiles => &self.commit_files_scroll,
            FileListId::RangeFiles => &self.range_files_scroll,
            FileListId::WorktreeFiles => &self.worktree_files_scroll,
            FileListId::Status(StatusSection::Staged) => &self.staged_scroll,
            FileListId::Status(StatusSection::Untracked) => &self.untracked_scroll,
            FileListId::Status(_) => &self.unstaged_scroll,
        }
    }

    fn review_current_row(&self, list: FileListId, data: &ReviewList) -> Option<usize> {
        let repo = self.active_repo()?;
        let key = self
            .review_cursor
            .as_ref()
            .filter(|(id, current, _, _)| *id == repo.id && *current == list)
            .map(|(_, _, key, _)| key.clone())
            .or_else(|| {
                repo.diff_state
                    .inline_submodule_diff
                    .as_ref()
                    .map(|inline| &inline.target)
                    .or(repo.diff_state.diff_target.as_ref())
                    .and_then(|target| target.file_path())
                    .map(|path| ReviewRow::File(path.to_owned()))
            })?;
        if let Some((id, current, _, row)) = &self.review_cursor
            && *id == repo.id
            && *current == list
            && self.review_row_key(list, data, *row).as_ref() == Some(&key)
        {
            return Some(*row);
        }
        (0..data.plan.row_len())
            .find(|&row| self.review_row_key(list, data, row).as_ref() == Some(&key))
    }

    pub(in crate::view) fn focus_review_files(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // File stepping in the diff can change the selection while this list
        // is unfocused. Return to that file, rather than an older list cursor.
        if let Some(repo) = self.active_repo()
            && repo.diff_state.diff_target.is_some()
        {
            self.review_cursor = None;
        }
        let lists = self.visible_review_lists();
        let list = lists
            .iter()
            .copied()
            .find(|list| {
                self.review_list(*list)
                    .is_some_and(|data| self.review_current_row(*list, &data).is_some())
            })
            .or_else(|| {
                lists.iter().copied().find(|list| {
                    self.review_list(*list)
                        .is_some_and(|data| data.plan.row_len() > 0)
                })
            })
            .or(lists.first().copied());
        let Some(list) = list else {
            return;
        };
        window.focus(self.review_file_focus(list), cx);
        if let Some(data) = self.review_list(list) {
            let row = self
                .review_current_row(list, &data)
                .or_else(|| {
                    (0..data.plan.row_len()).find(|&ix| {
                        matches!(data.plan.row_at(RowIx(ix)), Some(FileListRow::File { .. }))
                    })
                })
                .unwrap_or(0);
            self.select_review_row(list, &data, row, window, cx);
        }
    }

    fn select_review_row(
        &mut self,
        list: FileListId,
        data: &ReviewList,
        row: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(key) = self.review_row_key(list, data, row) else {
            return;
        };
        self.review_cursor = Some((repo_id, list, key, row));
        self.review_scroll(list)
            .scroll_to_item(row, gpui::ScrollStrategy::Center);
        if let Some(FileListRow::File { ordinal, .. }) = data.plan.row_at(RowIx(row))
            && let Some(target) = self.review_target(list, ordinal.0)
        {
            if list == FileListId::WorktreeFiles {
                if let Some(path) = self
                    .selected_worktree_summary()
                    .map(|summary| summary.path.clone())
                {
                    self.open_worktree_file_diff(repo_id, &path, &target, window, cx);
                    window.focus(self.review_file_focus(list), cx);
                }
            } else {
                let selected = self
                    .active_repo()
                    .is_some_and(|repo| repo.diff_state.diff_target.as_ref() == Some(&target));
                if !selected {
                    self.file_list_selection.remove(&(repo_id, list));
                    if matches!(list, FileListId::Status(_)) {
                        self.clear_status_multi_selection(repo_id);
                    }
                    let conflicted = matches!(&target, DiffTarget::WorkingTree { path, area: DiffArea::Unstaged, .. } if self.active_repo().and_then(|repo| repo.status_entry_for_path(DiffArea::Unstaged, path)).is_some_and(|entry| entry.kind == FileStatusKind::Conflicted));
                    self.store.dispatch(if conflicted {
                        Msg::SelectConflictDiff {
                            repo_id,
                            path: target.file_path().unwrap().to_owned(),
                        }
                    } else {
                        Msg::SelectDiff {
                            repo_id,
                            target: target.clone(),
                        }
                    });
                }
            }
        }
        cx.notify();
    }

    pub(in crate::view) fn review_file_container(
        &self,
        list: FileListId,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        div()
            .key_context("ChangedFiles")
            .track_focus(self.review_file_focus(list))
            .on_action(cx.listener(|this, _: &OpenActiveViewSearch, window, cx| {
                if this.open_review_search(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if this.handle_review_search_key(list, &event.keystroke, window, cx)
                        || this.handle_review_file_key(list, &event.keystroke, window, cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .children(self.review_search_bar(list, cx))
    }

    fn handle_review_file_key(
        &mut self,
        list: FileListId,
        key: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if !self.review_file_focus(list).is_focused(window) {
            return false;
        }
        if key.key == "f10" && key.modifiers.shift && key.modifiers.number_of_modifiers() == 1 {
            return self.open_review_file_menu(list, window, cx);
        }
        if !key.modifiers.modified() && key.key == "escape" {
            review_navigation::defer(self.root_view.clone(), ReviewNavigation::Graph, window, cx);
            return true;
        }
        if let FileListId::Status(section) = list
            && DiffFileStep::from_keystroke(key).is_none()
            && self.handle_status_section_shortcut(section, key, window, cx)
        {
            self.review_cursor = None;
            return true;
        }
        if key.modifiers.modified() {
            return false;
        }
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let Some(data) = self.review_list(list) else {
            return matches!(key.key.as_str(), "enter" | "up" | "down");
        };
        let len = data.plan.row_len();
        if len == 0 {
            return matches!(key.key.as_str(), "enter" | "up" | "down");
        }
        let current = self.review_current_row(list, &data);
        let row = current.unwrap_or(0);
        let step = self.file_list_page_rows(repo_id, list).unwrap_or(1);
        let next = match key.key.as_str() {
            "up" => Some(row.saturating_sub(1)),
            "down" => Some(if current.is_some() {
                (row + 1).min(len - 1)
            } else {
                0
            }),
            "pageup" => Some(row.saturating_sub(step)),
            "pagedown" => Some((row + step).min(len - 1)),
            "home" => Some(0),
            "end" => Some(len - 1),
            _ => None,
        };
        if let Some(next) = next {
            self.select_review_row(list, &data, next, window, cx);
            return true;
        }
        match (key.key.as_str(), data.plan.row_at(RowIx(row))) {
            ("enter", Some(FileListRow::File { .. })) => {
                self.select_review_row(list, &data, row, window, cx);
                review_navigation::defer(
                    self.root_view.clone(),
                    ReviewNavigation::Diff,
                    window,
                    cx,
                );
            }
            (
                "enter" | "left" | "right",
                Some(FileListRow::Directory {
                    key: path,
                    chain,
                    collapsed,
                    ..
                }),
            ) => {
                if key.key == "enter" || (key.key == "right") == collapsed {
                    self.toggle_file_list_dir(repo_id, list, path, chain, collapsed, cx);
                } else if key.key == "right" && row + 1 < len {
                    self.select_review_row(list, &data, row + 1, window, cx);
                } else {
                    self.review_parent_row(list, &data, row, window, cx);
                }
            }
            (
                "enter" | "left" | "right",
                Some(FileListRow::Group {
                    group, collapsed, ..
                }),
            ) => {
                if key.key == "enter" || (key.key == "right") == collapsed {
                    self.toggle_file_list_group(repo_id, list, group, cx);
                } else if key.key == "right" && row + 1 < len {
                    self.select_review_row(list, &data, row + 1, window, cx);
                }
            }
            ("left", Some(FileListRow::File { .. })) => {
                self.review_parent_row(list, &data, row, window, cx)
            }
            _ => return false,
        }
        true
    }

    fn review_parent_row(
        &mut self,
        list: FileListId,
        data: &ReviewList,
        row: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let depth = match data.plan.row_at(RowIx(row)) {
            Some(FileListRow::Directory { depth, .. } | FileListRow::File { depth, .. }) => depth,
            _ => 0,
        };
        if let Some(parent) = (0..row)
            .rev()
            .find(|&ix| match data.plan.row_at(RowIx(ix)) {
                Some(FileListRow::Directory { depth: d, .. }) => d < depth,
                Some(FileListRow::Group { .. }) => true,
                _ => false,
            })
        {
            self.select_review_row(list, data, parent, window, cx);
        }
    }

    fn open_review_file_menu(
        &mut self,
        list: FileListId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        let Some(data) = self.review_list(list) else {
            return false;
        };
        let Some(row) = self.review_current_row(list, &data) else {
            return false;
        };
        let Some(FileListRow::File { ordinal, .. }) = data.plan.row_at(RowIx(row)) else {
            return false;
        };
        let Some(target) = self.review_target(list, ordinal.0) else {
            return false;
        };
        let kind = match (list, &target) {
            (FileListId::WorktreeFiles, _) => {
                let Some(summary) = self.selected_worktree_summary() else {
                    return false;
                };
                PopoverKind::WorktreeFileMenu {
                    repo_id,
                    worktree_path: summary.path.clone(),
                    target: target.clone(),
                }
            }
            (
                _,
                DiffTarget::Commit {
                    commit_id, path, ..
                },
            ) => PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: commit_id.clone(),
                path: path.clone(),
            },
            (
                _,
                DiffTarget::CommitRange {
                    from_commit_id,
                    to_commit_id,
                    path: Some(path),
                    ..
                },
            ) => PopoverKind::CommitRangeFileMenu {
                repo_id,
                from_commit_id: from_commit_id.clone(),
                to_commit_id: to_commit_id.clone(),
                path: path.clone(),
            },
            (_, DiffTarget::WorkingTree { area, path, .. }) => PopoverKind::StatusFileMenu {
                repo_id,
                area: *area,
                path: path.clone(),
            },
            _ => return false,
        };
        let scroll = self.review_scroll(list).0.borrow();
        let bounds = scroll.base_handle.bounds();
        let height = match list {
            FileListId::Status(_) => crate::view::rows::status::status_row_height(self.ui_scale()),
            _ => crate::view::rows::sidebar::sidebar_list_row_height(
                self.theme,
                self.ui_scale_percent,
            ),
        };
        let y = (bounds.top() + scroll.base_handle.offset().y + height * (row + 1) as f32)
            .max(bounds.top())
            .min(bounds.bottom());
        let point = point(bounds.left() + self.ui_scale().px(16.0), y);
        drop(scroll);
        self.open_popover_at(kind, point, window, cx);
        true
    }

    // The preview stays selected after focus leaves the list; only its keyboard
    // cursor gets an outline. Folders/groups have their own cursor, since they
    // do not change the preview.
    pub(in crate::view) fn review_row_is_focused(
        &self,
        repo: RepoId,
        list: FileListId,
        row: &FileListRow,
        window: &Window,
    ) -> bool {
        if !self.review_file_focus(list).is_focused(window) {
            return false;
        }
        let cursor = self
            .review_cursor
            .as_ref()
            .filter(|(id, current, _, _)| *id == repo && *current == list)
            .map(|(_, _, key, _)| key);
        match (cursor, row) {
            (Some(ReviewRow::Directory(path)), FileListRow::Directory { key, .. }) => path == key,
            (Some(ReviewRow::Group(ix)), FileListRow::Group { group, .. }) => ix == group,
            (None | Some(ReviewRow::File(_)), FileListRow::File { ordinal, .. }) => {
                let selected = self.active_repo().and_then(|repo| {
                    repo.diff_state
                        .inline_submodule_diff
                        .as_ref()
                        .map(|inline| &inline.target)
                        .or(repo.diff_state.diff_target.as_ref())
                });
                selected.is_some_and(|selected| {
                    self.review_target(list, ordinal.0).as_ref() == Some(selected)
                })
            }
            _ => false,
        }
    }
}
