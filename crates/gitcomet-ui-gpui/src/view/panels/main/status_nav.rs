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

pub(super) fn status_navigation_context<'a>(
    status: &'a gitcomet_core::domain::RepoStatus,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area } = diff_target else {
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

impl MainPaneView {
    pub(in crate::view) fn try_select_adjacent_status_file(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let change_tracking_view = self.active_change_tracking_view(cx);
        let Some((section, area, target_ix, target_path, is_conflicted)) = (|| {
            let repo = self.active_repo()?;
            let Loadable::Ready(status) = &repo.status else {
                return None;
            };
            let diff_target = repo.diff_state.diff_target.as_ref()?;
            let navigation = status_navigation_context(status, diff_target, change_tracking_view)?;
            let target_ix = navigation.adjacent_ix(direction)?;
            let entry = navigation.entries.get(target_ix)?;
            let target_path = entry.path.clone();
            let area = navigation.section.diff_area();
            let is_conflicted = area == DiffArea::Unstaged
                && entry.kind == gitcomet_core::domain::FileStatusKind::Conflicted;

            Some((
                navigation.section,
                area,
                target_ix,
                target_path,
                is_conflicted,
            ))
        })() else {
            return false;
        };

        window.focus(&self.diff_panel_focus_handle);
        self.clear_status_multi_selection(repo_id, cx);
        if is_conflicted {
            self.store.dispatch(Msg::SelectConflictDiff {
                repo_id,
                path: target_path,
            });
        } else {
            self.store.dispatch(Msg::SelectDiff {
                repo_id,
                target: DiffTarget::WorkingTree {
                    path: target_path,
                    area,
                },
            });
        }
        self.scroll_status_section_to_ix(section, target_ix, cx);

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(path: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(path)
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

    #[test]
    fn split_untracked_navigation_scopes_to_untracked_section() {
        let status = gitcomet_core::domain::RepoStatus {
            staged: Vec::new(),
            unstaged: vec![
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
            ],
        };
        let target = DiffTarget::WorkingTree {
            path: pb("new-a.txt"),
            area: DiffArea::Unstaged,
        };

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
            staged: Vec::new(),
            unstaged: vec![
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
            ],
        };
        let target = DiffTarget::WorkingTree {
            path: pb("src/lib.rs"),
            area: DiffArea::Unstaged,
        };

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
            staged: Vec::new(),
            unstaged: vec![
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
            ],
        };
        let target = DiffTarget::WorkingTree {
            path: pb("src/lib.rs"),
            area: DiffArea::Unstaged,
        };

        let navigation = status_navigation_context(&status, &target, ChangeTrackingView::Combined)
            .expect("combined navigation");

        assert_eq!(navigation.section, StatusSection::CombinedUnstaged);
        assert_eq!(navigation.current_ix, 1);
        assert_eq!(navigation.prev_ix(), Some(0));
        assert_eq!(navigation.next_ix(), Some(2));
    }
}
