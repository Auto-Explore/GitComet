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
    crate::store::with_path_action_inline_for_bench(
        state,
        repo_id,
        Effect::StagePath { repo_id, path },
        f,
    )
}

#[inline]
pub fn with_stage_paths_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_path_action_inline_for_bench(
        state,
        repo_id,
        Effect::StagePaths { repo_id, paths },
        f,
    )
}

#[inline]
pub fn with_unstage_path_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_path_action_inline_for_bench(
        state,
        repo_id,
        Effect::UnstagePath { repo_id, path },
        f,
    )
}

#[inline]
pub fn with_unstage_paths_sync<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    f: impl FnOnce(&AppState, &[Effect]) -> T,
) -> T {
    crate::store::with_path_action_inline_for_bench(
        state,
        repo_id,
        Effect::UnstagePaths { repo_id, paths },
        f,
    )
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

/// Native setup, excluding fixture creation and watcher teardown.
pub fn watcher_startup(
    root: &std::path::Path,
    backend: &dyn gitcomet_core::services::GitBackend,
    cancel_during_setup: bool,
) -> (std::time::Duration, usize, bool) {
    crate::store::watcher_startup_for_bench(root, backend, cancel_during_setup)
}

#[cfg(all(test, feature = "benchmarks"))]
mod tests;
