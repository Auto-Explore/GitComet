use crate::model::{AppState, Loadable};
use crate::msg::Effect;
use gitcomet_core::domain::{DiffArea, DiffTarget, FileSource};
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
        let mut diff_target = repo.diff_state.diff_target.clone();
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
            for paths in [
                &mut repo.file_browser.revealed_paths,
                &mut repo.file_browser.pending_recursive_expansions,
            ] {
                *paths = paths.iter().filter_map(|p| retarget(p)).collect();
            }
            for entry in &mut repo.navigation.view_history.entries {
                if entry.source == FileSource::WorkingDirectory
                    && let Some(next) = retarget(&entry.path)
                {
                    entry.path = next;
                }
            }
            for target in diff_target
                .iter_mut()
                .chain(
                    repo.diff_state
                        .edit_return_view
                        .iter_mut()
                        .map(|view| &mut view.target),
                )
                .chain(
                    repo.navigation
                        .main_history
                        .entries
                        .iter_mut()
                        .filter_map(|entry| entry.diff_target.as_mut()),
                )
            {
                // Filesystem moves change the checkout, not the index or history.
                if let DiffTarget::WorkingTree {
                    path,
                    area: DiffArea::Unstaged,
                    ..
                } = target
                    && let Some(next) = retarget(path)
                {
                    *path = next;
                }
            }
        }
        repo.file_browser.stale = true;
        repo.file_browser.bump_rev();
        if diff_target != repo.diff_state.diff_target {
            repo.set_diff_target(diff_target.clone());
            if let Some(target) = diff_target {
                let plan = super::util::selected_diff_load_plan(repo, &target);
                super::util::apply_selected_diff_load_plan_state(repo, plan);
                repo.diff_state.inline_submodule_diff = None;
                repo.bump_diff_state_rev();
                effects.extend(super::util::diff_reload_effects(repo, repo.id, target));
            }
        }
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
        repo.file_browser.revealed_paths = ["folder/nested/file", "keep.txt"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        repo.file_browser.pending_recursive_expansions = ["folder", "unrelated"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        state.repos.push(repo);
        state
    }

    #[test]
    fn filesystem_moves_retarget_only_live_views_in_history_and_editor_returns() {
        use crate::model::ViewHistoryEntry;
        use gitcomet_core::domain::CommitId;
        let old = PathBuf::from("folder/nested/file");
        let live = DiffTarget::working_tree(old.clone(), DiffArea::Unstaged);
        let targets = [
            live.clone(),
            DiffTarget::working_tree(old.clone(), DiffArea::Staged),
            DiffTarget::commit(CommitId("abc".into()), old.clone()),
            DiffTarget::commit_range(
                CommitId("abc".into()),
                Some(CommitId("def".into())),
                Some(old.clone()),
            ),
        ];
        for original in &targets {
            for edit in [false, true] {
                for parent in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path();
                    let new = PathBuf::from(if parent {
                        "renamed/nested/file"
                    } else {
                        "folder/nested/new"
                    });
                    let new_live = DiffTarget::working_tree(new.clone(), DiffArea::Unstaged);
                    let mut state = selected_repo(root);
                    let repo = &mut state.repos[0];
                    repo.diff_state.content_preview = original == &live;
                    for target in &targets {
                        repo.diff_state.diff_target = Some(target.clone());
                        repo.navigation
                            .main_history
                            .record(repo.main_view_snapshot());
                    }
                    for source in [
                        FileSource::WorkingDirectory,
                        FileSource::Commit(CommitId("abc".into())),
                    ] {
                        repo.navigation.view_history.record(ViewHistoryEntry {
                            source,
                            path: old.clone(),
                            old_path: None,
                        });
                    }
                    repo.diff_state.diff_target = Some(original.clone());
                    let repos = Default::default();
                    if edit {
                        super::super::diff_selection::open_file_editor(
                            &repos,
                            &mut state,
                            RepoId(1),
                            old.clone(),
                        );
                    }
                    paths_changed(
                        &mut state,
                        &[PathChange {
                            old: Some(root.join(if parent { Path::new("folder") } else { &old })),
                            new: Some(root.join(if parent { Path::new("renamed") } else { &new })),
                        }],
                    );
                    let repo = &state.repos[0];
                    assert_eq!(repo.navigation.view_history.entries[0].path, new);
                    assert_eq!(repo.navigation.view_history.entries[1].path, old);
                    for (entry, target) in repo.navigation.main_history.entries.iter().zip(&targets)
                    {
                        assert_eq!(
                            entry.diff_target.as_ref(),
                            Some(if target == &live { &new_live } else { target })
                        );
                    }
                    if edit {
                        assert_eq!(repo.diff_state.diff_target, Some(new_live.clone()));
                        super::super::diff_selection::exit_diff_edit_mode(
                            &repos,
                            &mut state,
                            RepoId(1),
                        );
                    }
                    let repo = &state.repos[0];
                    assert_eq!(
                        repo.diff_state.diff_target.as_ref(),
                        Some(if original == &live {
                            &new_live
                        } else {
                            original
                        })
                    );
                    assert_eq!(repo.diff_state.content_preview, original == &live);
                    assert!(!repo.diff_state.edit_mode);
                }
            }
        }
    }

    #[test]
    fn filesystem_move_mappings_match_repository_paths_and_retarget_selections() {
        let directory = tempfile::tempdir().unwrap();
        let root =
            gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
        std::fs::create_dir_all(root.join("folder/nested")).unwrap();
        std::fs::write(root.join("folder/nested/file"), b"contents").unwrap();
        let mut state = selected_repo(&root);
        let result = Filesystem::default().execute(
            Request::new(Operation::Rename {
                // On Windows, this input has a verbatim prefix and RepoSpec does not.
                source: std::fs::canonicalize(root.join("folder")).unwrap(),
                name: "renamed".into(),
            }),
            |_| {},
        );
        assert!(result.succeeded(), "{:?}", result.items);
        paths_changed(&mut state, &result.changes);
        let selection = &state.repos[0].file_browser.selection;
        assert_eq!(
            selection.paths,
            ["renamed", "renamed/nested/file", "keep.txt"]
                .map(PathBuf::from)
                .into()
        );
        assert_eq!(
            selection.focused.as_deref(),
            Some(Path::new("renamed/nested/file"))
        );
        assert_eq!(selection.anchor.as_deref(), Some(Path::new("renamed")));
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
                browser.revealed_paths,
                [PathBuf::from("keep.txt")].into_iter().collect()
            );
            assert_eq!(
                browser.pending_recursive_expansions,
                [PathBuf::from("unrelated")].into_iter().collect()
            );
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
            browser.revealed_paths,
            ["renamed/nested/file", "keep.txt"]
                .into_iter()
                .map(PathBuf::from)
                .collect()
        );
        assert_eq!(
            browser.pending_recursive_expansions,
            ["renamed", "unrelated"]
                .into_iter()
                .map(PathBuf::from)
                .collect()
        );
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
