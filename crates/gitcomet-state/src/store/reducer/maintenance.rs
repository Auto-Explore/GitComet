//! Git's maintenance recommendation, and the runs the user starts from it.
use crate::model::{AppState, MaintenanceSettings, RepoId, RepoState};
use crate::msg::{Effect, RepoExternalChange};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Focus and tab switches ask for a check at most this often; the daily limit
/// is the effect's, kept in the session file.
const CHECK_REQUEST_INTERVAL: Duration = Duration::from_secs(60 * 60);

fn common_dir(state: &AppState, repo_id: RepoId) -> Option<Arc<Path>> {
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)?
        .common_dir
        .clone()
}

/// Tabs of the same repository (its worktrees) share one object store, so
/// they share its maintenance too.
fn same_repository<'a>(
    state: &'a mut AppState,
    common_dir: &'a Path,
) -> impl Iterator<Item = &'a mut RepoState> + 'a {
    state
        .repos
        .iter_mut()
        .filter(move |repo| repo.common_dir.as_deref() == Some(common_dir))
}

fn running_on(state: &AppState, common_dir: &Path) -> bool {
    state
        .repos
        .iter()
        .any(|repo| repo.common_dir.as_deref() == Some(common_dir) && repo.maintenance.running)
}

pub(in crate::store) fn request_check(state: &mut AppState, repo_id: RepoId) -> Option<Effect> {
    let settings = state.maintenance_settings;
    request_check_for(
        settings,
        state.repos.iter_mut().find(|repo| repo.id == repo_id)?,
    )
}

pub(super) fn request_check_for(
    settings: MaintenanceSettings,
    repo: &mut RepoState,
) -> Option<Effect> {
    if !settings.recommend {
        return None;
    }
    let now = SystemTime::now();
    let repo_id = repo.id;
    let maintenance = &mut repo.maintenance;
    let asked_recently = maintenance.check_requested_at.is_some_and(|at| {
        now.duration_since(at)
            .is_ok_and(|elapsed| elapsed < CHECK_REQUEST_INTERVAL)
    });
    if repo.common_dir.is_none() || maintenance.recommended || maintenance.running || asked_recently
    {
        return None;
    }
    maintenance.check_requested_at = Some(now);
    Some(Effect::CheckRepoMaintenance { repo_id })
}

pub(super) fn checked(state: &mut AppState, repo_id: RepoId, needed: bool) -> Vec<Effect> {
    // A check already running when the user turned recommendations off.
    if needed
        && state.maintenance_settings.recommend
        && let Some(common_dir) = common_dir(state, repo_id)
        && !running_on(state, &common_dir)
        && let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
    {
        repo.maintenance.recommended = true;
    }
    Vec::new()
}

/// Turning recommendations off withdraws every card already shown.
pub(super) fn set_settings(state: &mut AppState, settings: MaintenanceSettings) -> Vec<Effect> {
    state.maintenance_settings = settings;
    if !settings.recommend {
        for repo in &mut state.repos {
            repo.maintenance.recommended = false;
        }
    }
    Vec::new()
}

pub(super) fn snooze(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(common_dir) = common_dir(state, repo_id) else {
        return Vec::new();
    };
    for repo in same_repository(state, &common_dir) {
        repo.maintenance.recommended = false;
    }
    vec![Effect::PersistRepoMaintenanceSnooze {
        common_dir: common_dir.to_path_buf(),
    }]
}

pub(super) fn start(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(common_dir) = common_dir(state, repo_id) else {
        return Vec::new();
    };
    if running_on(state, &common_dir) {
        return Vec::new();
    }
    for repo in same_repository(state, &common_dir) {
        repo.maintenance.recommended = false;
    }
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    repo.maintenance.running = true;
    repo.bump_ops_rev();
    vec![Effect::RunMaintenance { repo_id }]
}

/// While maintenance rewrites a repository's packs, watcher refreshes of its
/// tabs wait: each would map the old packs again, and Windows cannot delete a
/// mapped file.
pub(super) fn defer_external_change(
    state: &mut AppState,
    repo_id: RepoId,
    change: &RepoExternalChange,
) -> bool {
    let Some(common_dir) = common_dir(state, repo_id) else {
        return false;
    };
    if !running_on(state, &common_dir) {
        return false;
    }
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return false;
    };
    let deferred = &mut repo.maintenance.deferred_change;
    *deferred = Some(match deferred.take() {
        Some(deferred) => deferred.union(change.clone()),
        None => change.clone(),
    });
    true
}

/// Ends the run and returns the refreshes its tabs held back meanwhile.
pub(super) fn finished(state: &mut AppState, repo_id: RepoId) -> Vec<(RepoId, RepoExternalChange)> {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo.maintenance.running = false;
        repo.bump_ops_rev();
    }
    let Some(common_dir) = common_dir(state, repo_id) else {
        return Vec::new();
    };
    same_repository(state, &common_dir)
        .filter_map(|repo| Some((repo.id, repo.maintenance.deferred_change.take()?)))
        .collect()
}
