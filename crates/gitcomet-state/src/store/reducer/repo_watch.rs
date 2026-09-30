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
