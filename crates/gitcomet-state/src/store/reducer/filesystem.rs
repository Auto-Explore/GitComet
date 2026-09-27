use crate::model::{AppState, Loadable};
use crate::msg::Effect;
use gitcomet_core::domain::{DiffTarget, FileSource};
use gitcomet_core::filesystem::PathChange;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn paths_changed(state: &mut AppState, changes: &[PathChange]) -> Vec<Effect> {
    let mut effects = vec![];
    for repo in &mut state.repos {
        let root = &repo.spec.workdir;
        if !changes.iter().any(|change| {
            change
                .old
                .iter()
                .chain(change.new.iter())
                .any(|p| p.starts_with(root))
        }) {
            continue;
        }
        for change in changes {
            let retarget = |path: &Path| -> Option<PathBuf> {
                let absolute = root.join(path);
                if !change
                    .old
                    .as_ref()
                    .is_some_and(|old| absolute.starts_with(old))
                {
                    return Some(path.to_path_buf());
                }
                change
                    .retarget(&absolute)
                    .and_then(|next| next.strip_prefix(root).ok().map(Path::to_path_buf))
            };
            repo.file_browser.selection.paths = repo
                .file_browser
                .selection
                .paths
                .iter()
                .filter_map(|p| retarget(p))
                .collect();
            repo.file_browser.selection.focused = repo
                .file_browser
                .selection
                .focused
                .as_deref()
                .and_then(retarget);
            repo.file_browser.selection.anchor = repo
                .file_browser
                .selection
                .anchor
                .as_deref()
                .and_then(retarget);
            repo.file_browser.expanded_dirs = repo
                .file_browser
                .expanded_dirs
                .iter()
                .filter_map(|p| retarget(p).map(Arc::new))
                .collect();
            if let Some(DiffTarget::WorkingTree { path, .. }) = &mut repo.diff_state.diff_target
                && let Some(next) = retarget(path)
            {
                *path = next;
            }
            for entry in &mut repo.navigation.view_history.entries {
                if entry.source == FileSource::WorkingDirectory
                    && let Some(next) = retarget(&entry.path)
                {
                    entry.path = next;
                }
            }
            for entry in &mut repo.navigation.main_history.entries {
                if let Some(DiffTarget::WorkingTree { path, .. }) = &mut entry.diff_target
                    && let Some(next) = retarget(path)
                {
                    *path = next;
                }
            }
        }
        repo.file_browser.stale = true;
        repo.file_browser.bump_rev();
        repo.diff_state.diff_target_rev = repo.diff_state.diff_target_rev.wrapping_add(1);
        if !matches!(repo.file_browser.entries, Loadable::NotLoaded) {
            effects.push(Effect::LoadFileBrowser {
                repo_id: repo.id,
                source: repo.file_browser.source.clone(),
            });
        }
        effects.push(Effect::LoadStatus { repo_id: repo.id });
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RepoId, RepoState};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::filesystem::{Filesystem, Operation, Request};

    fn selected_repo(root: &Path) -> AppState {
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: root.into(),
            },
        );
        repo.file_browser.selection.paths = ["folder", "folder/nested/file", "keep.txt"]
            .map(PathBuf::from)
            .into();
        repo.file_browser.selection.focused = Some("folder/nested/file".into());
        repo.file_browser.selection.anchor = Some("folder".into());
        repo.file_browser.expanded_dirs = ["folder", "folder/nested", "unrelated"]
            .into_iter()
            .map(|path| Arc::new(PathBuf::from(path)))
            .collect();
        state.repos.push(repo);
        state
    }

    #[test]
    fn removing_a_parent_clears_selection_before_the_next_toggle_and_operation() {
        for move_out in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("repo");
            std::fs::create_dir_all(root.join("folder/nested")).unwrap();
            for path in ["folder/nested/file", "keep.txt", "next.txt"] {
                std::fs::write(root.join(path), b"contents").unwrap();
            }
            let mut state = selected_repo(&root);
            let new = move_out.then(|| directory.path().join("moved"));
            if let Some(new) = &new {
                std::fs::rename(root.join("folder"), new).unwrap();
            } else {
                std::fs::remove_dir_all(root.join("folder")).unwrap();
            }
            paths_changed(
                &mut state,
                &[PathChange {
                    old: Some(root.join("folder")),
                    new,
                }],
            );
            let browser = &mut state.repos[0].file_browser;
            assert_eq!(browser.selection.paths, [PathBuf::from("keep.txt")].into());
            assert!(browser.selection.focused.is_none());
            assert!(browser.selection.anchor.is_none());
            assert_eq!(
                browser.expanded_dirs,
                [Arc::new(PathBuf::from("unrelated"))].into_iter().collect()
            );
            browser
                .selection
                .click("next.txt".into(), &[], true, false, false);
            let result = Filesystem::default().execute(
                Request::new(Operation::Duplicate {
                    sources: browser
                        .selection
                        .paths
                        .iter()
                        .map(|path| root.join(path))
                        .collect(),
                }),
                |_| {},
            );
            assert!(result.succeeded(), "{:?}", result.items);
            assert_eq!(result.items.len(), 2);
        }
    }

    #[test]
    fn renames_retarget_selection_focus_anchor_and_expansion_within_the_repository() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut state = selected_repo(root);
        paths_changed(
            &mut state,
            &[PathChange {
                old: Some(root.join("folder")),
                new: Some(root.join("renamed")),
            }],
        );
        let browser = &state.repos[0].file_browser;
        assert_eq!(
            browser.selection.paths,
            ["renamed", "renamed/nested/file", "keep.txt"]
                .map(PathBuf::from)
                .into()
        );
        assert_eq!(
            browser.selection.focused.as_deref(),
            Some(Path::new("renamed/nested/file"))
        );
        assert_eq!(
            browser.selection.anchor.as_deref(),
            Some(Path::new("renamed"))
        );
        assert_eq!(
            browser.expanded_dirs,
            ["renamed", "renamed/nested", "unrelated"]
                .into_iter()
                .map(|p| Arc::new(PathBuf::from(p)))
                .collect()
        );
    }
}
