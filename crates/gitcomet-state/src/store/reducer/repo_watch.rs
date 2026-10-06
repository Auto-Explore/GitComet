//! File watching: leases that keep an open repository watched, and the
//! warning shown when watching is degraded.

use super::util;
use crate::model::{AppState, RepoId};
use crate::msg::{Effect, RepoWatchDegradedReason};
use std::sync::Arc;

pub(super) fn acquire_lease(state: &mut AppState, repo_id: RepoId, lifetime: u64) -> Vec<Effect> {
    // Only an open repository can be watched; a lease on a closed
    // one is a no-op, and so is its release.
    if state
        .repos
        .iter()
        .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    {
        *Arc::make_mut(&mut state.watch_leases)
            .entry(repo_id)
            .or_default() += 1;
    }
    Vec::new()
}

pub(super) fn release_lease(state: &mut AppState, repo_id: RepoId, lifetime: u64) -> Vec<Effect> {
    if !state
        .repos
        .iter()
        .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    {
        return Vec::new();
    }
    if let Some(count) = state.watch_leases.get(&repo_id).copied() {
        let leases = Arc::make_mut(&mut state.watch_leases);
        if count <= 1 {
            leases.remove(&repo_id);
        } else {
            leases.insert(repo_id, count - 1);
        }
    }
    Vec::new()
}

pub(super) fn watch_degraded(state: &mut AppState, reason: RepoWatchDegradedReason) -> Vec<Effect> {
    let message = match reason {
        crate::msg::RepoWatchDegradedReason::IgnorePolicyFailed =>
            "Live file watching is limited because repository ignore rules could not be read. Changes refresh when the window regains focus; watching will retry automatically.".into(),
        crate::msg::RepoWatchDegradedReason::TooManyFolders { dir_count } => format!(
            "This repository has at least {dir_count} folders outside its ignore rules. \
             Live watching of subfolders is limited. Add generated folders to .gitignore \
             to reduce coverage. Changes also refresh when the window regains focus."
        ),
        crate::msg::RepoWatchDegradedReason::WatchLimitReached { unwatched_dirs } => {
            format!(
                "Live file watching is partial: {unwatched_dirs} locations could not be watched \
             because a native watch could not be registered. Changes in them refresh when the window \
             regains focus. Watching will retry automatically."
            )
        }
    };
    util::push_notification(state, crate::model::AppNotificationKind::Warning, message);
    Vec::new()
}

pub(super) fn watch_worktree(
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: u64,
    path: std::path::PathBuf,
    watch: bool,
) -> Vec<Effect> {
    if !state
        .repos
        .iter()
        .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    {
        return Vec::new();
    }
    let leases = Arc::make_mut(&mut state.worktree_watch_leases);
    let key = (repo_id, lifetime, path);
    if watch {
        *leases.entry(key).or_default() += 1;
    } else if let Some(count) = leases.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            leases.remove(&key);
        }
    }
    Vec::new()
}

pub(super) fn worktree_changed(
    state: &mut AppState,
    repo_id: RepoId,
    lifetime: u64,
    path: std::path::PathBuf,
    change: crate::msg::RepoExternalChange,
) -> Vec<Effect> {
    let Some(repo) = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
    else {
        return Vec::new();
    };
    let refresh_diff = repo
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .is_some_and(|inline| {
            inline.submodule_repo_path == path
                && (change.index
                    || change.git_state
                    || change.text_attributes
                    || change.worktree
                        && inline
                            .target
                            .file_path()
                            .is_none_or(|file| change.paths.may_contain(file)))
        });
    let mut effects: Vec<_> =
        super::effects::request_worktree_dirty_path_effect(repo, path.clone())
            .into_iter()
            .collect();
    if refresh_diff {
        effects
            .extend(super::diff_selection::refresh_inline_submodule_selected_diff(state, repo_id));
    }
    // Hosted sessions and lists reading this linked worktree.
    let linked = gitcomet_core::domain::normalize_worktree_path(&path);
    if let Some(repo) = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
        && (change.worktree || change.index || change.git_state || change.text_attributes)
    {
        effects.extend(super::diff_session::reload_worktree_sessions(
            repo,
            &change,
            Some(&linked),
        ));
    }
    effects
}
