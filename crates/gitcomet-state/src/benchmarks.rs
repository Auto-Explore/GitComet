use crate::model::{AppState, RepoId};
use crate::msg::{ConflictRegionChoice, Effect, Msg, RepoPath, RepoPathList};
use gitcomet_core::domain::DiffTarget;

pub fn dispatch_sync(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    crate::store::dispatch_sync_for_bench(state, msg)
}

pub fn with_set_active_repo_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_set_active_repo_inline_for_bench(state, repo_id, f)
}

pub fn with_reorder_repo_tabs_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_reorder_repo_tabs_inline_for_bench(state, repo_id, insert_before, f)
}

pub fn with_select_diff_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    target: DiffTarget,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_select_diff_inline_for_bench(state, repo_id, target, f)
}

#[inline]
pub fn with_stage_path_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_stage_path_inline_for_bench(state, repo_id, path, f)
}

#[inline]
pub fn with_stage_paths_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_stage_paths_inline_for_bench(state, repo_id, paths, f)
}

#[inline]
pub fn with_unstage_path_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_unstage_path_inline_for_bench(state, repo_id, path, f)
}

#[inline]
pub fn with_unstage_paths_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_unstage_paths_inline_for_bench(state, repo_id, paths, f)
}

#[inline]
pub fn set_conflict_region_choice_sync(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) {
    crate::store::set_conflict_region_choice_inline_for_bench(
        state,
        repo_id,
        path,
        region_index,
        choice,
    )
}

#[inline]
pub fn reset_conflict_resolutions_sync(state: &mut AppState, repo_id: RepoId, path: RepoPath) {
    crate::store::reset_conflict_resolutions_inline_for_bench(state, repo_id, path)
}

#[cfg(all(test, feature = "benchmarks"))]
mod tests {
    use super::*;
    use crate::model::{Loadable, RepoState};
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegion, ConflictRegionResolution, ConflictRegionText,
        ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::domain::{DiffArea, DiffTarget, RepoStatus};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn add_conflict_repo(
        state: &mut AppState,
        resolutions: &[ConflictRegionResolution],
    ) -> RepoPath {
        let path_buf = PathBuf::from("src/conflict.rs");
        let path = RepoPath::from(path_buf.clone());

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-conflict"),
            },
        );
        repo.open = Loadable::Ready(());
        repo.set_conflict_file_path(Some(path_buf.clone()));

        let mut session = ConflictSession::new(
            path_buf,
            FileConflictKind::BothModified,
            ConflictPayload::Text(Arc::from("base\n")),
            ConflictPayload::Text(Arc::from("ours\n")),
            ConflictPayload::Text(Arc::from("theirs\n")),
        );
        for resolution in resolutions {
            session.regions.push(ConflictRegion {
                base: Some(ConflictRegionText::from("base region\n")),
                ours: ConflictRegionText::from("ours region\n"),
                theirs: ConflictRegionText::from("theirs region\n"),
                resolution: resolution.clone(),
            });
        }
        repo.set_conflict_session(Some(session));

        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));
        path
    }

    #[test]
    fn set_active_repo_sync_uses_reducer_path() {
        let mut state = AppState::default();

        let mut repo1 = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-1"),
            },
        );
        repo1.open = Loadable::Ready(());

        let mut repo2 = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-2"),
            },
        );
        repo2.open = Loadable::Ready(());

        state.repos.push(repo1);
        state.repos.push(repo2);
        state.active_repo = Some(RepoId(1));

        with_set_active_repo_sync(&mut state, RepoId(2), |_state, effects| {
            assert!(
                effects.iter().any(|effect| matches!(
                    effect,
                    Effect::LoadStatus { repo_id } if *repo_id == RepoId(2)
                )),
                "expected reducer refresh effects for the target repo"
            );
        });
        assert_eq!(state.active_repo, Some(RepoId(2)));
    }

    #[test]
    fn reorder_repo_tabs_sync_uses_reducer_path() {
        let mut state = AppState::default();

        let mut repo1 = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-tab-1"),
            },
        );
        repo1.open = Loadable::Ready(());

        let mut repo2 = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-tab-2"),
            },
        );
        repo2.open = Loadable::Ready(());

        state.repos.push(repo1);
        state.repos.push(repo2);
        state.active_repo = Some(RepoId(1));

        with_reorder_repo_tabs_sync(&mut state, RepoId(1), None, |_state, effects| {
            assert!(matches!(
                effects,
                [Effect::PersistSession {
                    repo_id: Some(RepoId(1)),
                    action: "reordering repository tabs",
                }]
            ));
        });
        assert_eq!(state.repos[0].id, RepoId(2));
        assert_eq!(state.repos[1].id, RepoId(1));
    }

    #[test]
    fn select_diff_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-select"),
            },
        );
        repo.open = Loadable::Ready(());
        repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));

        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: DiffArea::Staged,
        };

        with_select_diff_sync(&mut state, RepoId(1), target.clone(), |state, effects| {
            assert!(matches!(
                effects,
                [Effect::LoadSelectedDiff {
                    repo_id: RepoId(1),
                    load_patch_diff: true,
                    load_file_text: true,
                    load_file_image: false,
                    preview_text_side: None,
                }]
            ));
            assert_eq!(state.repos[0].diff_state.diff_target, Some(target.clone()));
            assert!(state.repos[0].diff_state.diff.is_loading());
            assert!(state.repos[0].diff_state.diff_file.is_loading());
        });
    }

    #[test]
    fn stage_path_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-stage"),
            },
        );
        repo.open = Loadable::Ready(());
        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));

        let path = PathBuf::from("src/lib.rs");
        with_stage_path_sync(&mut state, RepoId(1), path.clone(), |state, effects| {
            assert!(matches!(
                effects,
                [Effect::StagePath { repo_id: RepoId(1), path: effect_path }]
                    if effect_path == &path
            ));
            assert_eq!(state.repos[0].ops_rev, 1);
            assert_eq!(state.repos[0].local_actions_in_flight, 1);
        });
    }

    #[test]
    fn stage_paths_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-stage-batch"),
            },
        );
        repo.open = Loadable::Ready(());
        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));

        let paths = RepoPathList::from(vec![
            PathBuf::from("src/lib.rs"),
            PathBuf::from("src/main.rs"),
        ]);
        with_stage_paths_sync(&mut state, RepoId(1), paths.clone(), |state, effects| {
            assert!(matches!(
                effects,
                [Effect::StagePaths {
                    repo_id: RepoId(1),
                    paths: effect_paths
                }] if effect_paths == &paths
            ));
            assert_eq!(state.repos[0].ops_rev, 1);
            assert_eq!(state.repos[0].local_actions_in_flight, 1);
        });
    }

    #[test]
    fn unstage_path_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-unstage"),
            },
        );
        repo.open = Loadable::Ready(());
        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));

        let path = PathBuf::from("src/lib.rs");
        with_unstage_path_sync(&mut state, RepoId(1), path.clone(), |state, effects| {
            assert!(matches!(
                effects,
                [Effect::UnstagePath { repo_id: RepoId(1), path: effect_path }]
                    if effect_path == &path
            ));
            assert_eq!(state.repos[0].ops_rev, 1);
            assert_eq!(state.repos[0].local_actions_in_flight, 1);
        });
    }

    #[test]
    fn unstage_paths_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/bench-repo-unstage-batch"),
            },
        );
        repo.open = Loadable::Ready(());
        state.repos.push(repo);
        state.active_repo = Some(RepoId(1));

        let paths = RepoPathList::from(vec![
            PathBuf::from("src/lib.rs"),
            PathBuf::from("src/main.rs"),
        ]);
        with_unstage_paths_sync(&mut state, RepoId(1), paths.clone(), |state, effects| {
            assert!(matches!(
                effects,
                [Effect::UnstagePaths {
                    repo_id: RepoId(1),
                    paths: effect_paths
                }] if effect_paths == &paths
            ));
            assert_eq!(state.repos[0].ops_rev, 1);
            assert_eq!(state.repos[0].local_actions_in_flight, 1);
        });
    }

    #[test]
    fn conflict_set_region_choice_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();
        let path = add_conflict_repo(&mut state, &[ConflictRegionResolution::Unresolved]);
        let before_rev = state.repos[0].conflict_state.conflict_rev;

        set_conflict_region_choice_sync(&mut state, RepoId(1), path, 0, ConflictRegionChoice::Ours);

        let session = state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("conflict session");
        assert_eq!(session.regions.len(), 1);
        assert_eq!(
            session.regions[0].resolution,
            ConflictRegionResolution::PickOurs
        );
        assert_eq!(state.repos[0].conflict_state.conflict_rev, before_rev + 1);
    }

    #[test]
    fn conflict_reset_resolutions_sync_uses_inline_reducer_path() {
        let mut state = AppState::default();
        let path = add_conflict_repo(
            &mut state,
            &[
                ConflictRegionResolution::PickOurs,
                ConflictRegionResolution::PickTheirs,
            ],
        );
        let before_rev = state.repos[0].conflict_state.conflict_rev;

        reset_conflict_resolutions_sync(&mut state, RepoId(1), path);

        let session = state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("conflict session");
        assert_eq!(session.regions.len(), 2);
        assert!(
            session
                .regions
                .iter()
                .all(|region| region.resolution == ConflictRegionResolution::Unresolved)
        );
        assert_eq!(state.repos[0].conflict_state.conflict_rev, before_rev + 1);
    }
}
