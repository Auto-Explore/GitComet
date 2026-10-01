use super::*;

#[derive(Debug)]
pub(super) struct StatusNavigationContext<'a> {
    section: StatusSection,
    entries: Vec<&'a gitcomet_core::domain::FileStatus>,
    current_ix: usize,
}

impl<'a> StatusNavigationContext<'a> {
    pub(super) fn prev_ix(&self) -> Option<usize> {
        self.current_ix.checked_sub(1)
    }

    pub(super) fn next_ix(&self) -> Option<usize> {
        (self.current_ix + 1 < self.entries.len()).then_some(self.current_ix + 1)
    }

    fn adjacent_ix(&self, direction: i8) -> Option<usize> {
        if direction < 0 {
            self.prev_ix()
        } else {
            self.next_ix()
        }
    }

    pub(super) fn next_or_prev_path(&self) -> Option<std::path::PathBuf> {
        self.next_ix()
            .or_else(|| self.prev_ix())
            .and_then(|ix| self.entries.get(ix).map(|entry| entry.path.clone()))
    }
}

#[cfg(test)]
fn status_navigation_section_for_target(
    status: &gitcomet_core::domain::RepoStatus,
    change_tracking_view: ChangeTrackingView,
    path: &std::path::Path,
    area: DiffArea,
) -> Option<StatusSection> {
    match area {
        DiffArea::Staged => Some(StatusSection::Staged),
        DiffArea::Unstaged => match change_tracking_view {
            ChangeTrackingView::Combined => Some(StatusSection::CombinedUnstaged),
            ChangeTrackingView::SplitUntracked => status
                .unstaged
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| {
                    if entry.kind == gitcomet_core::domain::FileStatusKind::Untracked {
                        StatusSection::Untracked
                    } else {
                        StatusSection::Unstaged
                    }
                }),
        },
    }
}

#[cfg(test)]
fn status_navigation_entries_for_section(
    status: &gitcomet_core::domain::RepoStatus,
    section: StatusSection,
) -> Vec<&gitcomet_core::domain::FileStatus> {
    match section {
        StatusSection::CombinedUnstaged => status.unstaged.iter().collect(),
        StatusSection::Untracked => status
            .unstaged
            .iter()
            .filter(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Untracked)
            .collect(),
        StatusSection::Unstaged => status
            .unstaged
            .iter()
            .filter(|entry| entry.kind != gitcomet_core::domain::FileStatusKind::Untracked)
            .collect(),
        StatusSection::Staged => status.staged.iter().collect(),
    }
}

#[cfg(test)]
pub(super) fn status_navigation_context<'a>(
    status: &'a gitcomet_core::domain::RepoStatus,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section =
        status_navigation_section_for_target(status, change_tracking_view, path.as_path(), *area)?;
    let entries = status_navigation_entries_for_section(status, section);
    let current_ix = entries.iter().position(|entry| entry.path == *path)?;
    Some(StatusNavigationContext {
        section,
        entries,
        current_ix,
    })
}

/// Which section a working-tree diff target belongs to. Shared so the order
/// looked up for navigation is the order navigation then walks.
pub(super) fn status_navigation_section(
    repo: &RepoState,
    path: &std::path::Path,
    area: DiffArea,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusSection> {
    Some(match area {
        DiffArea::Staged => StatusSection::Staged,
        DiffArea::Unstaged => match change_tracking_view {
            ChangeTrackingView::Combined => StatusSection::CombinedUnstaged,
            ChangeTrackingView::SplitUntracked => {
                let entry = repo.status_entry_for_path(DiffArea::Unstaged, path)?;
                if entry.kind == gitcomet_core::domain::FileStatusKind::Untracked {
                    StatusSection::Untracked
                } else {
                    StatusSection::Unstaged
                }
            }
        },
    })
}

/// `section_order` is the section's display order in the backing slice's index
/// space. `None` falls back to source order, which is only correct where the
/// caller does not care about position (it is what picks a *neighbouring* file).
pub(super) fn status_navigation_context_for_repo<'a>(
    repo: &'a RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    section_order: Option<&[usize]>,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section = status_navigation_section(repo, path.as_path(), *area, change_tracking_view)?;
    let entries: Vec<_> = match section_order {
        Some(order) => StatusSectionEntries::from_repo_with_order(repo, section, order.into())?
            .iter()
            .collect(),
        None => StatusSectionEntries::from_repo(repo, section)?
            .iter()
            .collect(),
    };
    let current_ix = entries.iter().position(|entry| entry.path == *path)?;
    Some(StatusNavigationContext {
        section,
        entries,
        current_ix,
    })
}

/// Whether the open working-tree target has a file before and after it in the
/// section's display order: what the toolbar arrows show. The pane renders on
/// every diff scroll step, so this walks the section once without copying it,
/// where the navigation context collects the whole section.
pub(super) fn status_navigation_neighbors(
    repo: &RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    section_order: Option<&[usize]>,
) -> Option<(bool, bool)> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section = status_navigation_section(repo, path.as_path(), *area, change_tracking_view)?;
    let mut len = 0usize;
    let mut current = None;
    let mut visit = |entry: &gitcomet_core::domain::FileStatus| {
        if current.is_none() && entry.path == *path {
            current = Some(len);
        }
        len += 1;
    };
    match section_order {
        Some(order) => {
            let entries = match section {
                StatusSection::Staged => repo.staged_status_entries()?,
                _ => repo.worktree_status_entries()?,
            };
            // Like the section iterator: an index past the end ends the list.
            order
                .iter()
                .map_while(|ix| entries.get(*ix))
                .for_each(&mut visit);
        }
        None => StatusSectionEntries::from_repo(repo, section)?
            .iter()
            .for_each(&mut visit),
    }
    let current = current?;
    Some((current > 0, current + 1 < len))
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum AdjacentDiffFileTarget {
    Range {
        target: DiffTarget,
        target_ix: usize,
    },
    WorkingTree {
        section: StatusSection,
        area: DiffArea,
        target_ix: usize,
        path: std::path::PathBuf,
        is_conflicted: bool,
    },
    Commit {
        commit_id: CommitId,
        target_ix: usize,
        path: std::path::PathBuf,
        old_path: Option<std::path::PathBuf>,
    },
}

/// The file `direction` steps to from `path` in a commit-diff file list, and
/// its position among the files as drawn. `drawn_source_indices` is the list's
/// display order; `None` means source order.
fn adjacent_file_in_list<'a>(
    files: &'a [gitcomet_core::domain::CommitFileChange],
    drawn_source_indices: Option<&[usize]>,
    path: &std::path::Path,
    direction: i8,
) -> Option<(usize, &'a gitcomet_core::domain::CommitFileChange)> {
    let source_indices;
    let drawn = if let Some(indices) = drawn_source_indices {
        indices
    } else {
        source_indices = (0..files.len()).collect::<Vec<_>>();
        source_indices.as_slice()
    };
    let current_ix = drawn
        .iter()
        .position(|source_ix| files.get(*source_ix).is_some_and(|file| file.path == path))?;
    let target_ix = if direction < 0 {
        current_ix.checked_sub(1)?
    } else {
        (current_ix + 1 < drawn.len()).then_some(current_ix + 1)?
    };
    Some((target_ix, files.get(*drawn.get(target_ix)?)?))
}

/// The entry an inline foreign diff should open when stepping `direction` from
/// `selected_ix`. Entries are in source order, so a sorted or tree-grouped list
/// passes `drawn_order` -- display position to entry index -- and navigation
/// follows the rows the user sees. An entry the list is not currently showing
/// has no neighbour, like the commit-file path above.
fn adjacent_inline_diff_ix(
    selected_ix: usize,
    entries_len: usize,
    drawn_order: Option<&[usize]>,
    direction: i8,
) -> Option<usize> {
    let step = |ix: usize, len: usize| match direction {
        d if d < 0 => ix.checked_sub(1),
        d if d > 0 => (ix + 1 < len).then_some(ix + 1),
        _ => None,
    };
    match drawn_order {
        Some(order) => {
            let display_ix = order.iter().position(|entry| *entry == selected_ix)?;
            order.get(step(display_ix, order.len())?).copied()
        }
        None => step(selected_ix, entries_len),
    }
}

pub(super) fn adjacent_diff_file_target_for_repo(
    repo: &RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    direction: i8,
    file_list_source_indices: Option<&[usize]>,
    status_section_order: Option<&[usize]>,
) -> Option<AdjacentDiffFileTarget> {
    if direction == 0 {
        return None;
    }

    match diff_target {
        DiffTarget::WorkingTree { .. } => {
            let navigation = status_navigation_context_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                status_section_order,
            )?;
            let target_ix = navigation.adjacent_ix(direction)?;
            let entry = navigation.entries.get(target_ix)?;
            let path = entry.path.clone();
            let area = navigation.section.diff_area();
            let is_conflicted = area == DiffArea::Unstaged
                && entry.kind == gitcomet_core::domain::FileStatusKind::Conflicted;

            Some(AdjacentDiffFileTarget::WorkingTree {
                section: navigation.section,
                area,
                target_ix,
                path,
                is_conflicted,
            })
        }
        DiffTarget::Commit {
            commit_id, path, ..
        } => {
            let Loadable::Ready(details) = &repo.history_state.commit_details else {
                return None;
            };
            if &details.id != commit_id {
                return None;
            }
            let (target_ix, file) =
                adjacent_file_in_list(&details.files, file_list_source_indices, path, direction)?;

            Some(AdjacentDiffFileTarget::Commit {
                commit_id: commit_id.clone(),
                target_ix,
                path: file.path.clone(),
                old_path: file.old_path.clone(),
            })
        }
        DiffTarget::CommitRange {
            from_commit_id,
            to_commit_id,
            path: Some(path),
            ..
        } => {
            // The file list on screen must be this comparison's.
            let range = repo.history_state.range_selection.as_ref()?;
            if range.diff_from() != from_commit_id || &range.to != to_commit_id {
                return None;
            }
            let Loadable::Ready(files) = &repo.history_state.range_files else {
                return None;
            };
            let (target_ix, file) =
                adjacent_file_in_list(files, file_list_source_indices, path, direction)?;
            Some(AdjacentDiffFileTarget::Range {
                target: DiffTarget::commit_range(
                    from_commit_id.clone(),
                    to_commit_id.clone(),
                    None,
                )
                .for_change(file),
                target_ix,
            })
        }
        DiffTarget::CommitRange { path: None, .. } => None,
    }
}

impl MainPaneView {
    /// The display order of the section the open working-tree diff belongs to.
    /// The pane owns the sort, so navigation has to ask it rather than re-derive
    /// an order of its own.
    pub(super) fn active_status_section_order(
        &self,
        repo_id: RepoId,
        change_tracking_view: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) -> Option<std::sync::Arc<[usize]>> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area, .. } =
            self.bound_diff_state(repo).diff_target.as_ref()?
        else {
            return None;
        };
        let section = status_navigation_section(repo, path.as_path(), *area, change_tracking_view)?;
        self.root_view
            .update(cx, |root, cx| {
                root.details_pane
                    .read(cx)
                    .active_status_section_order(repo_id, section)
            })
            .ok()
            .flatten()
    }

    /// The drawn order of the commit-diff file list the open diff belongs to:
    /// the commit details list, or the comparison list.
    pub(super) fn active_file_list_source_indices(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<std::sync::Arc<[usize]>> {
        let repo = self.active_repo()?;
        let is_range = matches!(
            self.bound_diff_state(repo).diff_target,
            Some(DiffTarget::CommitRange { .. })
        );
        self.root_view
            .update(cx, |root, cx| {
                let details = root.details_pane.read(cx);
                if is_range {
                    details.active_range_file_source_indices(repo_id)
                } else {
                    details.active_commit_file_source_indices(repo_id)
                }
            })
            .ok()
            .flatten()
    }

    /// Both the toolbar and its actions use these neighbors in display order.
    pub(super) fn inline_diff_file_neighbors(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<(Option<usize>, Option<usize>)> {
        let inline = self.active_inline_submodule_diff()?;
        let selected_ix = inline.selected_ix;
        let entries_len = inline.entries.len();
        // Worktree rows can be sorted or grouped into a tree; submodule rows
        // follow source order.
        let worktree_path = matches!(
            inline.origin,
            gitcomet_state::model::ForeignDiffOrigin::Worktree { .. }
        )
        .then(|| inline.submodule_repo_path.clone());
        let drawn_order = worktree_path.and_then(|path| {
            self.root_view
                .update(cx, |root, cx| {
                    root.details_pane
                        .read(cx)
                        .active_worktree_file_source_indices(repo_id, &path)
                })
                .ok()
                .flatten()
        });
        Some((
            adjacent_inline_diff_ix(selected_ix, entries_len, drawn_order.as_deref(), -1),
            adjacent_inline_diff_ix(selected_ix, entries_len, drawn_order.as_deref(), 1),
        ))
    }

    fn try_select_adjacent_diff_file_inner(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        focus_diff_panel: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.store.binding.is_some() {
            if !self.store.policy.file_navigation {
                return false;
            }
            let action = self.hosted_decor.as_ref().and_then(|decor| {
                if direction < 0 {
                    decor.options.file_navigation.previous.clone()
                } else {
                    decor.options.file_navigation.next.clone()
                }
            });
            if let Some(action) = action {
                action.invoke(cx);
                return true;
            }
            return false;
        }
        if let Some((prev_ix, next_ix)) = self.inline_diff_file_neighbors(repo_id, cx) {
            let Some(next_ix) = (match direction {
                d if d < 0 => prev_ix,
                d if d > 0 => next_ix,
                _ => None,
            }) else {
                return false;
            };
            if focus_diff_panel {
                window.focus(&self.diff_panel_focus_handle, cx);
            }
            self.store.dispatch(Msg::SelectInlineSubmoduleDiff {
                repo_id,
                selected_ix: next_ix,
            });
            return true;
        }

        let file_list_source_indices = self.active_file_list_source_indices(repo_id, cx);
        let change_tracking_view = self.active_change_tracking_view(cx);
        let status_section_order =
            self.active_status_section_order(repo_id, change_tracking_view, cx);
        let Some(target) = (|| {
            let repo = self.active_repo()?;
            let diff_target = self.bound_diff_state(repo).diff_target.as_ref()?;
            adjacent_diff_file_target_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                direction,
                file_list_source_indices.as_deref(),
                status_section_order.as_deref(),
            )
        })() else {
            return false;
        };

        if focus_diff_panel {
            window.focus(&self.diff_panel_focus_handle, cx);
        }
        match target {
            AdjacentDiffFileTarget::Range { target, target_ix } => {
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
                self.scroll_range_file_to_ix(target_ix, cx);
            }
            AdjacentDiffFileTarget::WorkingTree {
                section,
                area,
                target_ix: _,
                path,
                is_conflicted,
            } => {
                self.clear_status_multi_selection(repo_id, cx);
                self.scroll_status_section_to_path(section, &path, cx);
                if is_conflicted {
                    self.store
                        .dispatch(Msg::SelectConflictDiff { repo_id, path });
                } else {
                    self.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::working_tree(path, area),
                    });
                }
            }
            AdjacentDiffFileTarget::Commit {
                commit_id,
                target_ix,
                path,
                old_path,
            } => {
                self.store.dispatch(Msg::SelectDiff {
                    repo_id,
                    target: DiffTarget::commit(commit_id, path).with_old_path(old_path),
                });
                self.scroll_commit_details_file_to_ix(target_ix, cx);
            }
        }

        true
    }

    pub(in crate::view) fn try_select_adjacent_diff_file(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.try_select_adjacent_diff_file_inner(repo_id, direction, true, window, cx)
    }

    pub(in crate::view) fn try_select_adjacent_diff_file_preserving_focus(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.try_select_adjacent_diff_file_inner(repo_id, direction, false, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(path: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(path)
    }

    fn repo_state(id: RepoId, path: &str) -> RepoState {
        RepoState::new_opening(id, gitcomet_core::domain::RepoSpec { workdir: pb(path) })
    }

    fn file_status(
        path: &str,
        kind: gitcomet_core::domain::FileStatusKind,
    ) -> gitcomet_core::domain::FileStatus {
        gitcomet_core::domain::FileStatus {
            path: pb(path),
            kind,
            conflict: None,
        }
    }

    /// The toolbar's one-pass answer must match what the prev/next actions
    /// then do, for every entry, section order and change-tracking view.
    #[test]
    fn status_navigation_neighbors_match_adjacent_targets() {
        use gitcomet_core::domain::FileStatusKind::{Added, Modified, Untracked};
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        let unstaged = vec![
            file_status("a.txt", Modified),
            file_status("b.txt", Untracked),
            file_status("c.txt", Modified),
            file_status("d.txt", Untracked),
            file_status("e.txt", Modified),
        ];
        let staged = vec![
            file_status("s1.txt", Added),
            file_status("s2.txt", Modified),
        ];
        repo.status = Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: std::sync::Arc::new(staged.clone()),
                unstaged: std::sync::Arc::new(unstaged.clone()),
            }
            .into(),
        );
        let orders: [Option<Vec<usize>>; 4] = [
            None,
            Some(vec![4, 2, 0]),
            Some(vec![3, 1]),
            Some(vec![1, 9, 0]),
        ];
        for view in [
            ChangeTrackingView::Combined,
            ChangeTrackingView::SplitUntracked,
        ] {
            for (paths, area) in [(&unstaged, DiffArea::Unstaged), (&staged, DiffArea::Staged)] {
                for entry in paths {
                    let target = DiffTarget::working_tree(entry.path.clone(), area);
                    for order in &orders {
                        let order = order.as_deref();
                        let expected = (
                            adjacent_diff_file_target_for_repo(
                                &repo, &target, view, -1, None, order,
                            )
                            .is_some(),
                            adjacent_diff_file_target_for_repo(
                                &repo, &target, view, 1, None, order,
                            )
                            .is_some(),
                        );
                        assert_eq!(
                            status_navigation_neighbors(&repo, &target, view, order)
                                .unwrap_or((false, false)),
                            expected,
                            "{view:?} {target:?} {order:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn split_untracked_navigation_scopes_to_untracked_section() {
        let status = gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(Vec::new()),
            unstaged: std::sync::Arc::new(vec![
                file_status(
                    "new-a.txt",
                    gitcomet_core::domain::FileStatusKind::Untracked,
                ),
                file_status(
                    "src/lib.rs",
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                file_status(
                    "new-b.txt",
                    gitcomet_core::domain::FileStatusKind::Untracked,
                ),
            ]),
        };
        let target = DiffTarget::working_tree(pb("new-a.txt"), DiffArea::Unstaged);

        let navigation =
            status_navigation_context(&status, &target, ChangeTrackingView::SplitUntracked)
                .expect("split untracked navigation");

        assert_eq!(navigation.section, StatusSection::Untracked);
        assert_eq!(navigation.current_ix, 0);
        assert_eq!(
            navigation
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![pb("new-a.txt"), pb("new-b.txt")]
        );
        assert_eq!(navigation.next_or_prev_path(), Some(pb("new-b.txt")));
    }

    #[test]
    fn split_tracked_navigation_scopes_to_tracked_section() {
        let status = gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(Vec::new()),
            unstaged: std::sync::Arc::new(vec![
                file_status(
                    "new-a.txt",
                    gitcomet_core::domain::FileStatusKind::Untracked,
                ),
                file_status(
                    "src/lib.rs",
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                file_status(
                    "src/main.rs",
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
            ]),
        };
        let target = DiffTarget::working_tree(pb("src/lib.rs"), DiffArea::Unstaged);

        let navigation =
            status_navigation_context(&status, &target, ChangeTrackingView::SplitUntracked)
                .expect("split tracked navigation");

        assert_eq!(navigation.section, StatusSection::Unstaged);
        assert_eq!(navigation.current_ix, 0);
        assert_eq!(navigation.prev_ix(), None);
        assert_eq!(navigation.next_ix(), Some(1));
        assert_eq!(
            navigation
                .entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![pb("src/lib.rs"), pb("src/main.rs")]
        );
    }

    #[test]
    fn combined_navigation_keeps_untracked_and_tracked_together() {
        let status = gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(Vec::new()),
            unstaged: std::sync::Arc::new(vec![
                file_status(
                    "new-a.txt",
                    gitcomet_core::domain::FileStatusKind::Untracked,
                ),
                file_status(
                    "src/lib.rs",
                    gitcomet_core::domain::FileStatusKind::Modified,
                ),
                file_status(
                    "new-b.txt",
                    gitcomet_core::domain::FileStatusKind::Untracked,
                ),
            ]),
        };
        let target = DiffTarget::working_tree(pb("src/lib.rs"), DiffArea::Unstaged);

        let navigation = status_navigation_context(&status, &target, ChangeTrackingView::Combined)
            .expect("combined navigation");

        assert_eq!(navigation.section, StatusSection::CombinedUnstaged);
        assert_eq!(navigation.current_ix, 1);
        assert_eq!(navigation.prev_ix(), Some(0));
        assert_eq!(navigation.next_ix(), Some(2));
    }

    #[test]
    fn commit_details_file_navigation_selects_adjacent_commit_files() {
        let commit_id = CommitId("deadbeefdeadbeef".into());
        let file_a = pb("src/a.rs");
        let file_b = pb("src/b.rs");
        let file_c = pb("src/c.rs");

        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.history_state.commit_details =
            Loadable::Ready(std::sync::Arc::new(gitcomet_core::domain::CommitDetails {
                id: commit_id.clone(),
                message: "subject".into(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-04-14 12:00:00 +0300".into(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files: vec![
                    gitcomet_core::domain::CommitFileChange::new(
                        file_a.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                    gitcomet_core::domain::CommitFileChange::new(
                        file_b.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                    gitcomet_core::domain::CommitFileChange::new(
                        file_c.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                ],
            }));

        let target = DiffTarget::commit(commit_id.clone(), file_b.clone());

        assert_eq!(
            adjacent_diff_file_target_for_repo(
                &repo,
                &target,
                ChangeTrackingView::Combined,
                -1,
                None,
                None,
            ),
            Some(AdjacentDiffFileTarget::Commit {
                commit_id: commit_id.clone(),
                target_ix: 0,
                path: file_a,
                old_path: None,
            })
        );
        assert_eq!(
            adjacent_diff_file_target_for_repo(
                &repo,
                &target,
                ChangeTrackingView::Combined,
                1,
                None,
                None,
            ),
            Some(AdjacentDiffFileTarget::Commit {
                commit_id,
                target_ix: 2,
                path: file_c,
                old_path: None,
            })
        );
    }

    #[test]
    fn commit_details_file_navigation_uses_the_visible_sorted_projection() {
        let commit_id = CommitId("deadbeefdeadbeef".into());
        let file_a = pb("src/a.rs");
        let file_b = pb("src/b.rs");
        let file_c = pb("src/c.rs");

        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.history_state.commit_details =
            Loadable::Ready(std::sync::Arc::new(gitcomet_core::domain::CommitDetails {
                id: commit_id.clone(),
                message: "subject".into(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-04-14 12:00:00 +0300".into(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files: vec![
                    gitcomet_core::domain::CommitFileChange::new(
                        file_a.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                    gitcomet_core::domain::CommitFileChange::new(
                        file_b.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                    gitcomet_core::domain::CommitFileChange::new(
                        file_c.clone(),
                        gitcomet_core::domain::FileStatusKind::Modified,
                    ),
                ],
            }));

        let target = DiffTarget::commit(commit_id.clone(), file_b.clone());
        let visible_source_indices = [2, 1];

        assert_eq!(
            adjacent_diff_file_target_for_repo(
                &repo,
                &target,
                ChangeTrackingView::Combined,
                -1,
                Some(&visible_source_indices),
                None,
            ),
            Some(AdjacentDiffFileTarget::Commit {
                commit_id: commit_id.clone(),
                target_ix: 0,
                path: file_c,
                old_path: None,
            })
        );
        assert_eq!(
            adjacent_diff_file_target_for_repo(
                &repo,
                &target,
                ChangeTrackingView::Combined,
                1,
                Some(&visible_source_indices),
                None,
            ),
            None,
        );

        let hidden_target = DiffTarget::commit(commit_id, file_a);
        assert_eq!(
            adjacent_diff_file_target_for_repo(
                &repo,
                &hidden_target,
                ChangeTrackingView::Combined,
                1,
                Some(&visible_source_indices),
                None,
            ),
            None,
            "navigation is a no-op while the open diff is hidden by the filter"
        );
    }

    fn file_change(path: &std::path::Path) -> gitcomet_core::domain::CommitFileChange {
        gitcomet_core::domain::CommitFileChange::new(
            path.to_path_buf(),
            gitcomet_core::domain::FileStatusKind::Modified,
        )
    }

    #[test]
    fn comparison_file_navigation_steps_through_the_drawn_range_files() {
        let from = CommitId("1111111111111111".into());
        let to = CommitId("2222222222222222".into());
        let file_a = pb("src/a.rs");
        let file_b = pb("src/b.rs");
        let file_c = pb("src/c.rs");

        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection::new(
            from.clone(),
            Some(to.clone()),
            "from".into(),
            "to".into(),
        ));
        repo.history_state.range_files = Loadable::Ready(std::sync::Arc::new(vec![
            file_change(&file_a),
            file_change(&file_b),
            file_change(&file_c),
        ]));
        let target = DiffTarget::commit_range(from.clone(), Some(to.clone()), Some(file_b.clone()));
        let range_file = |path: &std::path::PathBuf, target_ix| AdjacentDiffFileTarget::Range {
            target: DiffTarget::commit_range(from.clone(), Some(to.clone()), Some(path.clone())),
            target_ix,
        };
        let adjacent = |repo: &RepoState, direction, drawn: Option<&[usize]>| {
            adjacent_diff_file_target_for_repo(
                repo,
                &target,
                ChangeTrackingView::Combined,
                direction,
                drawn,
                None,
            )
        };

        assert_eq!(adjacent(&repo, -1, None), Some(range_file(&file_a, 0)));
        assert_eq!(adjacent(&repo, 1, None), Some(range_file(&file_c, 2)));
        // A sorted list steps in display order, and its ends have no neighbour.
        assert_eq!(
            adjacent(&repo, -1, Some(&[2, 1, 0])),
            Some(range_file(&file_c, 0))
        );
        assert_eq!(adjacent(&repo, -1, Some(&[1, 0, 2])), None);

        // A diff left open from another comparison has no list on screen.
        repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection::new(
            from,
            None,
            "from".into(),
            "Working tree".into(),
        ));
        assert_eq!(adjacent(&repo, 1, None), None);
    }
}
