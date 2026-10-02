use gitcomet_core::domain::{DiffArea, FileSource, FileStatusKind};
use gitcomet_state::model::{RepoId, RepoState};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Default)]
pub(super) struct FileBrowserStatusCache {
    key: Option<(RepoId, u64, u64)>,
    badges: Rc<FileBrowserStatusBadges>,
}

#[derive(Default)]
pub(super) struct FileBrowserStatusBadges {
    files: FxHashMap<PathBuf, FileStatusKind>,
    directories: FxHashMap<PathBuf, FileStatusKind>,
}

impl FileBrowserStatusCache {
    pub(super) fn get(&mut self, repo: &RepoState) -> Option<Rc<FileBrowserStatusBadges>> {
        if repo.file_browser.source != FileSource::WorkingDirectory {
            return None;
        }
        let key = (
            repo.id,
            repo.worktree_status_cache_rev(),
            repo.staged_status_cache_rev(),
        );
        if self.key != Some(key) {
            let mut badges = FileBrowserStatusBadges::default();
            // Preserve the existing badge precedence: first unstaged change,
            // then first staged change, including directory descendants.
            for area in [DiffArea::Unstaged, DiffArea::Staged] {
                for status in repo.status_entries_for_area(area).unwrap_or(&[]) {
                    badges
                        .files
                        .entry(status.path.clone())
                        .or_insert(status.kind);
                    for ancestor in status
                        .path
                        .ancestors()
                        .take_while(|p| !p.as_os_str().is_empty())
                    {
                        let std::collections::hash_map::Entry::Vacant(slot) =
                            badges.directories.entry(ancestor.to_path_buf())
                        else {
                            // Its parents already have the earlier badge too.
                            break;
                        };
                        slot.insert(status.kind);
                    }
                }
            }
            self.key = Some(key);
            self.badges = Rc::new(badges);
        }
        Some(Rc::clone(&self.badges))
    }
}

impl FileBrowserStatusBadges {
    pub(super) fn get(&self, path: &Path, directory: bool) -> Option<FileStatusKind> {
        if directory {
            &self.directories
        } else {
            &self.files
        }
        .get(path)
        .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, FileStatus, RepoSpec, RepoStatus};
    use gitcomet_state::model::Loadable;
    use std::sync::Arc;

    fn status(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: path.into(),
            kind,
            conflict: None,
        }
    }

    fn repo() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/repo".into(),
            },
        )
    }

    #[test]
    fn badges_preserve_file_and_directory_precedence_and_path_boundaries() {
        use FileStatusKind::*;
        let mut repo = repo();
        repo.worktree_status = Loadable::Ready(Arc::new(vec![
            status("src/nested/a", Untracked),
            status("src/b", Modified),
            status("src", Deleted),
            status("submodule", Modified),
        ]));
        repo.staged_status = Loadable::Ready(Arc::new(vec![
            status("src/b", Added),
            status("docs/file", Renamed),
        ]));
        let badges = FileBrowserStatusCache::default().get(&repo).unwrap();
        for (path, directory, expected) in [
            ("src/nested/a", false, Some(Untracked)),
            ("src/b", false, Some(Modified)),
            ("src", false, Some(Deleted)),
            ("src", true, Some(Untracked)),
            ("src/nested", true, Some(Untracked)),
            ("docs", true, Some(Renamed)),
            ("docs/file", false, Some(Renamed)),
            ("submodule", true, Some(Modified)),
            ("sr", true, None),
            ("src/clean", false, None),
            ("docs", false, None),
        ] {
            assert_eq!(badges.get(Path::new(path), directory), expected, "{path}");
        }
    }

    #[test]
    fn badges_refresh_for_either_status_lane_and_repository_switches() {
        let mut repo = repo();
        let mut cache = FileBrowserStatusCache::default();
        let empty = cache.get(&repo).unwrap();
        repo.worktree_status =
            Loadable::Ready(Arc::new(vec![status("a", FileStatusKind::Modified)]));
        repo.worktree_status_rev += 1;
        let worktree = cache.get(&repo).unwrap();
        assert!(!Rc::ptr_eq(&empty, &worktree));
        assert_eq!(
            worktree.get(Path::new("a"), false),
            Some(FileStatusKind::Modified)
        );
        repo.staged_status = Loadable::Ready(Arc::new(vec![status("b", FileStatusKind::Added)]));
        repo.staged_status_rev += 1;
        let staged = cache.get(&repo).unwrap();
        assert!(!Rc::ptr_eq(&worktree, &staged));
        assert_eq!(
            staged.get(Path::new("b"), false),
            Some(FileStatusKind::Added)
        );
        repo.worktree_status = Loadable::Ready(Arc::new(vec![]));
        repo.worktree_status_rev += 1;
        assert_eq!(cache.get(&repo).unwrap().get(Path::new("a"), false), None);
        repo.file_browser.source = FileSource::Commit(CommitId("historical".into()));
        assert!(
            cache.get(&repo).is_none(),
            "historical trees do not show live badges"
        );
        repo.file_browser.source = FileSource::WorkingDirectory;
        assert_eq!(
            cache.get(&repo).unwrap().get(Path::new("b"), false),
            Some(FileStatusKind::Added)
        );
        repo.id = RepoId(2);
        repo.staged_status = Loadable::Ready(Arc::new(vec![]));
        assert_eq!(cache.get(&repo).unwrap().get(Path::new("b"), false), None);
    }

    #[test]
    fn badges_track_the_legacy_status_revision() {
        let mut repo = repo();
        let mut cache = FileBrowserStatusCache::default();
        let empty = cache.get(&repo).unwrap();
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            staged: Arc::new(vec![]),
            unstaged: Arc::new(vec![status("a", FileStatusKind::Modified)]),
        }));
        repo.status_rev += 1;
        let badges = cache.get(&repo).unwrap();
        assert!(!Rc::ptr_eq(&empty, &badges));
        assert_eq!(
            badges.get(Path::new("a"), false),
            Some(FileStatusKind::Modified)
        );
    }

    #[test]
    fn large_status_lists_share_one_index_across_visible_rows_and_redraws() {
        let mut repo = repo();
        repo.worktree_status = Loadable::Ready(Arc::new(
            (0..100_000)
                .map(|i| {
                    status(
                        &format!("folder/nested/file-{i}"),
                        FileStatusKind::Untracked,
                    )
                })
                .collect(),
        ));
        let mut cache = FileBrowserStatusCache::default();
        let first = cache.get(&repo).unwrap();
        for _ in 0..3 {
            // Search, expansion and selection redraws do not change Git badges.
            repo.file_browser.file_browser_rev += 1;
            let badges = cache.get(&repo).unwrap();
            assert!(
                Rc::ptr_eq(&first, &badges),
                "a redraw must reuse the status index"
            );
            for i in 99_950..100_000 {
                assert_eq!(
                    badges.get(Path::new(&format!("folder/nested/file-{i}")), false),
                    Some(FileStatusKind::Untracked)
                );
            }
            assert_eq!(
                badges.get(Path::new("folder"), true),
                Some(FileStatusKind::Untracked)
            );
        }
    }
}
