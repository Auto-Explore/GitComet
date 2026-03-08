use super::util::{
    dedup_paths_in_order, diff_target_is_svg, diff_target_wants_image_preview,
    format_failure_summary, handle_session_persist_result, normalize_repo_path, push_diagnostic,
    push_notification, refresh_full_effects, refresh_primary_effects,
};
use crate::model::{
    AppNotificationKind, AppState, CloneOpState, CloneOpStatus, DiagnosticKind, Loadable, RepoId,
};
use crate::msg::Effect;
use crate::session;
use gitcomet_core::domain::{DiffTarget, RepoSpec};
use gitcomet_core::error::Error;
use gitcomet_core::services::{CommandOutput, GitRepository};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn open_repo(id_alloc: &AtomicU64, state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    let path = normalize_repo_path(path);
    if let Some(repo_id) = state
        .repos
        .iter()
        .find(|r| r.spec.workdir == path)
        .map(|r| r.id)
    {
        // Re-opening an already open repository should still refresh primary state, so stale
        // status/diff data gets reconciled immediately.
        return set_active_repo(state, repo_id);
    }

    let repo_id = RepoId(id_alloc.fetch_add(1, Ordering::Relaxed));
    let spec = RepoSpec { workdir: path };

    state.repos.push({
        let mut repo_state = crate::model::RepoState::new_opening(repo_id, spec.clone());
        if let Some(scope) = session::load_repo_history_scope(&spec.workdir) {
            repo_state.history_scope = scope;
        }
        if let Some(enabled) =
            session::load_repo_fetch_prune_deleted_remote_tracking_branches(&spec.workdir)
        {
            repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
        }
        repo_state
    });
    state.active_repo = Some(repo_id);
    let effects = vec![Effect::OpenRepo {
        repo_id,
        path: spec.workdir.clone(),
    }];
    let persist_result = session::persist_from_state(state);
    handle_session_persist_result(state, Some(repo_id), "opening a repository", persist_result);
    effects
}

pub(super) fn restore_session(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    open_repos: Vec<PathBuf>,
    active_repo: Option<PathBuf>,
) -> Vec<Effect> {
    repos.clear();
    state.repos.clear();
    state.active_repo = None;

    let repo_history_scopes = session::load_repo_history_scopes();
    let repo_fetch_prune_deleted_remote_tracking_branches =
        session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo();
    let active_repo = active_repo.map(normalize_repo_path);
    let mut active_repo_id: Option<RepoId> = None;

    let open_repos = dedup_paths_in_order(open_repos);
    let mut effects = Vec::with_capacity(open_repos.len());
    let mut seen_workdirs: HashSet<PathBuf> = HashSet::default();
    seen_workdirs.reserve(open_repos.len());

    for path in open_repos.into_iter().map(normalize_repo_path) {
        if !seen_workdirs.insert(path.clone()) {
            continue;
        }
        let repo_id = RepoId(id_alloc.fetch_add(1, Ordering::Relaxed));
        let spec = RepoSpec { workdir: path };
        if active_repo_id.is_none()
            && active_repo
                .as_ref()
                .is_some_and(|active| active == &spec.workdir)
        {
            active_repo_id = Some(repo_id);
        }

        state.repos.push({
            let mut repo_state = crate::model::RepoState::new_opening(repo_id, spec.clone());
            let workdir_key = spec.workdir.to_string_lossy();
            if let Some(scope) = repo_history_scopes.get(workdir_key.as_ref()).copied() {
                repo_state.history_scope = scope;
            }
            if let Some(enabled) = repo_fetch_prune_deleted_remote_tracking_branches
                .get(workdir_key.as_ref())
                .copied()
            {
                repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
            }
            repo_state
        });
        effects.push(Effect::OpenRepo {
            repo_id,
            path: spec.workdir.clone(),
        });
    }

    state.active_repo = if let Some(active_repo_id) = active_repo_id {
        Some(active_repo_id)
    } else {
        state.repos.last().map(|r| r.id)
    };

    let persist_result = session::persist_from_state(state);
    handle_session_persist_result(
        state,
        state.active_repo,
        "restoring repository session",
        persist_result,
    );
    effects
}

pub(super) fn close_repo(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    state.repos.retain(|r| r.id != repo_id);
    repos.remove(&repo_id);
    if state.active_repo == Some(repo_id) {
        state.active_repo = state.repos.first().map(|r| r.id);
    }
    let persist_result = session::persist_from_state(state);
    handle_session_persist_result(
        state,
        state.active_repo,
        "closing a repository",
        persist_result,
    );
    Vec::new()
}

pub(super) fn set_active_repo(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    if !state.repos.iter().any(|r| r.id == repo_id) {
        return Vec::new();
    }

    let changed = state.active_repo != Some(repo_id);
    state.active_repo = Some(repo_id);
    if changed {
        let persist_result = session::persist_from_state(state);
        handle_session_persist_result(
            state,
            Some(repo_id),
            "switching active repository",
            persist_result,
        );
    }

    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    // On focus events the UI can re-send SetActiveRepo for the already-active repo. Avoid
    // re-running the full refresh fan-out in that case: prioritize the minimum set that
    // keeps the UI correct and responsive.
    let mut effects = if changed {
        refresh_full_effects(repo_state)
    } else {
        refresh_primary_effects(repo_state)
    };

    // Reload the selected diff when switching repos; steady-state refreshes rely on the
    // filesystem watcher (`RepoExternallyChanged`) for diff invalidation.
    if changed && let Some(target) = repo_state.diff_target.clone() {
        let supports_file = matches!(
            &target,
            DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
        );
        let wants_image = diff_target_wants_image_preview(&target);
        let is_svg = diff_target_is_svg(&target);
        effects.push(Effect::LoadDiff {
            repo_id,
            target: target.clone(),
        });
        if supports_file {
            if wants_image {
                effects.push(Effect::LoadDiffFileImage {
                    repo_id,
                    target: target.clone(),
                });
            }
            if !wants_image || is_svg {
                effects.push(Effect::LoadDiffFile { repo_id, target });
            }
        }
    }
    effects
}

pub(super) fn set_fetch_prune_deleted_remote_tracking_branches(
    state: &mut AppState,
    repo_id: RepoId,
    enabled: bool,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let workdir = {
        let repo_state = &mut state.repos[repo_ix];
        if repo_state.fetch_prune_deleted_remote_tracking_branches == enabled {
            return Vec::new();
        }

        repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
        repo_state.spec.workdir.clone()
    };
    let persist_result =
        session::persist_repo_fetch_prune_deleted_remote_tracking_branches(&workdir, enabled);
    handle_session_persist_result(
        state,
        Some(repo_id),
        "updating fetch prune settings",
        persist_result,
    );
    Vec::new()
}

pub(super) fn reorder_repo_tabs(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
) -> Vec<Effect> {
    if state.repos.len() <= 1 {
        return Vec::new();
    }

    let Some(from_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };

    if let Some(before_repo_id) = insert_before {
        if before_repo_id == repo_id {
            return Vec::new();
        }
        if let Some(before_ix) = state.repos.iter().position(|r| r.id == before_repo_id)
            && from_ix + 1 == before_ix
        {
            // Already immediately before the target.
            return Vec::new();
        }
    } else if from_ix + 1 == state.repos.len() {
        // Already last.
        return Vec::new();
    }

    let moved = state.repos.remove(from_ix);
    let insert_ix = match insert_before {
        Some(before_repo_id) => state
            .repos
            .iter()
            .position(|r| r.id == before_repo_id)
            .unwrap_or(state.repos.len()),
        None => state.repos.len(),
    };
    state.repos.insert(insert_ix, moved);

    let persist_result = session::persist_from_state(state);
    handle_session_persist_result(
        state,
        state.active_repo,
        "reordering repository tabs",
        persist_result,
    );
    Vec::new()
}

pub(super) fn clone_repo(state: &mut AppState, url: String, dest: PathBuf) -> Vec<Effect> {
    state.clone = Some(CloneOpState {
        url: url.clone(),
        dest: dest.clone(),
        status: CloneOpStatus::Running,
        seq: 0,
        output_tail: Vec::new(),
    });
    vec![Effect::CloneRepo { url, dest }]
}

pub(super) fn clone_repo_progress(
    state: &mut AppState,
    dest: PathBuf,
    line: String,
) -> Vec<Effect> {
    if let Some(op) = state.clone.as_mut()
        && matches!(op.status, CloneOpStatus::Running)
        && op.dest == dest
    {
        op.seq = op.seq.wrapping_add(1);
        if !line.trim().is_empty() {
            op.output_tail.push(line);
            const MAX_LINES: usize = 80;
            if op.output_tail.len() > MAX_LINES {
                let drain = op.output_tail.len() - MAX_LINES;
                op.output_tail.drain(0..drain);
            }
        }
    }
    Vec::new()
}

pub(super) fn clone_repo_finished(
    state: &mut AppState,
    url: String,
    dest: PathBuf,
    result: std::result::Result<CommandOutput, Error>,
) -> Vec<Effect> {
    if let Some(op) = state.clone.as_mut()
        && op.dest == dest
    {
        op.url = url;
        op.status = match result {
            Ok(_) => CloneOpStatus::FinishedOk,
            Err(e) => CloneOpStatus::FinishedErr(format_failure_summary("Clone", &e)),
        };
        op.seq = op.seq.wrapping_add(1);
    } else {
        state.clone = Some(CloneOpState {
            url,
            dest,
            status: match result {
                Ok(_) => CloneOpStatus::FinishedOk,
                Err(e) => CloneOpStatus::FinishedErr(format_failure_summary("Clone", &e)),
            },
            seq: 1,
            output_tail: Vec::new(),
        });
    }
    Vec::new()
}

pub(super) fn repo_opened_ok(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    spec: RepoSpec,
    repo: Arc<dyn GitRepository>,
) -> Vec<Effect> {
    repos.insert(repo_id, repo);

    let spec = RepoSpec {
        workdir: normalize_repo_path(spec.workdir),
    };
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.spec = spec;
        repo_state.set_open(Loadable::Ready(()));
        repo_state.set_head_branch(Loadable::Loading);
        repo_state.set_upstream_divergence(Loadable::Loading);
        repo_state.set_branches(Loadable::Loading);
        repo_state.set_tags(Loadable::Loading);
        repo_state.set_remote_tags(Loadable::Loading);
        repo_state.set_remotes(Loadable::Loading);
        repo_state.set_remote_branches(Loadable::Loading);
        repo_state.set_status(Loadable::Loading);
        repo_state.set_log(Loadable::Loading);
        repo_state.log_loading_more = false;
        repo_state.set_stashes(Loadable::Loading);
        repo_state.reflog = Loadable::NotLoaded;
        repo_state.set_rebase_in_progress(Loadable::Loading);
        repo_state.set_merge_commit_message(Loadable::Loading);
        repo_state.file_history_path = None;
        repo_state.file_history = Loadable::NotLoaded;
        repo_state.blame_path = None;
        repo_state.blame_rev = None;
        repo_state.blame = Loadable::NotLoaded;
        repo_state.set_worktrees(Loadable::NotLoaded);
        repo_state.set_submodules(Loadable::NotLoaded);
        repo_state.set_selected_commit(None);
        repo_state.set_commit_details(Loadable::NotLoaded);
        repo_state.diff_target = None;
        repo_state.diff = Loadable::NotLoaded;
        repo_state.diff_file = Loadable::NotLoaded;
        repo_state.diff_file_image = Loadable::NotLoaded;
        repo_state.bump_diff_state_rev();
        repo_state.last_error = None;

        return refresh_full_effects(repo_state);
    }

    Vec::new()
}

pub(super) fn repo_opened_err(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    spec: RepoSpec,
    error: Error,
) -> Vec<Effect> {
    let spec = RepoSpec {
        workdir: normalize_repo_path(spec.workdir),
    };
    if matches!(
        error.kind(),
        gitcomet_core::error::ErrorKind::NotARepository
    ) {
        push_notification(
            state,
            AppNotificationKind::Error,
            format!("Folder is not a git repository: {}", spec.workdir.display()),
        );

        repos.remove(&repo_id);
        if let Some(ix) = state.repos.iter().position(|r| r.id == repo_id) {
            let was_active = state.active_repo == Some(repo_id);
            state.repos.remove(ix);
            if was_active {
                state.active_repo = if ix > 0 {
                    state.repos.get(ix - 1).map(|r| r.id)
                } else {
                    state.repos.get(ix).map(|r| r.id)
                };
            }
            let persist_result = session::persist_from_state(state);
            handle_session_persist_result(
                state,
                state.active_repo,
                "removing an invalid repository from session",
                persist_result,
            );
        }
        return Vec::new();
    }

    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.spec = spec;
        repo_state.set_open(Loadable::Error(error.to_string()));
        repo_state.last_error = Some(error.to_string());
        push_diagnostic(repo_state, DiagnosticKind::Error, error.to_string());
    }
    Vec::new()
}
