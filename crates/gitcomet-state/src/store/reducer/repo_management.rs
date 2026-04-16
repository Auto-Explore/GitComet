use super::effects::append_ensure_sidebar_data_effects;
use super::util::{
    SelectedConflictTarget, append_refresh_full_effects, append_refresh_primary_effects,
    append_start_conflict_target_reload, append_start_current_conflict_target_reload,
    clear_banner_error_for_repo, dedup_paths_in_order, format_failure_summary,
    handle_session_persist_result, normalize_repo_path, push_diagnostic, push_notification,
    refresh_full_effect_capacity, refresh_full_effects, refresh_primary_effect_capacity,
    selected_conflict_target, selected_diff_load_plan,
};
use crate::model::{
    AppNotificationKind, AppState, CloneOpState, CloneOpStatus, CloneProgressMeter,
    CloneProgressStage, DiagnosticKind, GitLogSettings, Loadable, RepoId, RepoLoadsInFlight,
    RepoState,
};
use crate::msg::Effect;
use crate::session;
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, GitRepository};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const HOT_REPO_SWITCH_SECONDARY_REFRESH_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY: usize = 16;
pub(crate) type SetActiveRepoEffects = SmallVec<[Effect; SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY]>;
pub(crate) const REORDER_REPO_TABS_INLINE_EFFECT_CAPACITY: usize = 1;
pub(crate) type ReorderRepoTabsEffects =
    SmallVec<[Effect; REORDER_REPO_TABS_INLINE_EFFECT_CAPACITY]>;

fn repo_switch_secondary_metadata_ready(
    repo_state: &RepoState,
    git_log_settings: GitLogSettings,
) -> bool {
    matches!(repo_state.branches, Loadable::Ready(_))
        && (!git_log_settings.show_history_tags
            || !git_log_settings.auto_fetch_tags_on_repo_activation()
            || matches!(repo_state.tags, Loadable::Ready(_)))
        && matches!(repo_state.remotes, Loadable::Ready(_))
        && matches!(repo_state.remote_branches, Loadable::Ready(_))
        && matches!(repo_state.stashes, Loadable::Ready(_))
        && matches!(repo_state.rebase_in_progress, Loadable::Ready(_))
        && matches!(repo_state.merge_commit_message, Loadable::Ready(_))
}

fn repo_switch_can_use_primary_refresh(
    repo_state: &RepoState,
    git_log_settings: GitLogSettings,
    now: SystemTime,
) -> bool {
    repo_switch_secondary_metadata_ready(repo_state, git_log_settings)
        && repo_state
            .last_active_at
            .and_then(|last_active_at| now.duration_since(last_active_at).ok())
            .is_some_and(|elapsed| elapsed <= HOT_REPO_SWITCH_SECONDARY_REFRESH_WINDOW)
}

fn is_missing_repo_error(error: &Error) -> bool {
    matches!(
        error.kind(),
        gitcomet_core::error::ErrorKind::Io(std::io::ErrorKind::NotFound)
    )
}

fn is_plain_clone_abort_error(error: &Error) -> bool {
    matches!(error.kind(), ErrorKind::Backend(message) if message == "clone aborted")
}

fn persist_session_effect(
    _state: &AppState,
    repo_id: Option<RepoId>,
    action: &'static str,
) -> Effect {
    Effect::PersistSession { repo_id, action }
}

fn append_repo_switch_worktree_refresh_effect(
    repo_state: &mut RepoState,
    effects: &mut SetActiveRepoEffects,
) {
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREES)
    {
        if !matches!(repo_state.worktrees, Loadable::Ready(_)) {
            repo_state.set_worktrees(Loadable::Loading);
        }
        effects.push(Effect::LoadWorktrees {
            repo_id: repo_state.id,
        });
    }
}

pub(super) fn open_repo(id_alloc: &AtomicU64, state: &mut AppState, path: PathBuf) -> Vec<Effect> {
    let now = SystemTime::now();
    let path = normalize_repo_path(path);
    if let Some(repo_id) = state
        .repos
        .iter()
        .find(|r| r.spec.workdir == path)
        .map(|r| r.id)
    {
        // Re-opening an already open repository should still refresh primary state, so stale
        // status/diff data gets reconciled immediately.
        let effects = set_active_repo(state, repo_id);
        let persist_result = session::persist_recent_repo(&path);
        handle_session_persist_result(
            state,
            Some(repo_id),
            "updating recent repositories",
            persist_result,
        );
        return effects;
    }

    let repo_id = RepoId(id_alloc.fetch_add(1, Ordering::Relaxed));
    let spec = RepoSpec { workdir: path };

    state.repos.push({
        let mut repo_state = crate::model::RepoState::new_opening(repo_id, spec.clone());
        if let Some(scope) = session::load_repo_history_scope(&spec.workdir) {
            repo_state.history_state.history_scope = scope;
        }
        if let Some(enabled) =
            session::load_repo_fetch_prune_deleted_remote_tracking_branches(&spec.workdir)
        {
            repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
        }
        repo_state.last_active_at = Some(now);
        repo_state
    });
    state.active_repo = Some(repo_id);
    let persist_recent_result = session::persist_recent_repo(&spec.workdir);
    let mut effects = vec![Effect::OpenRepo {
        repo_id,
        path: spec.workdir.clone(),
    }];
    effects.push(persist_session_effect(
        state,
        Some(repo_id),
        "opening a repository",
    ));
    handle_session_persist_result(
        state,
        Some(repo_id),
        "updating recent repositories",
        persist_recent_result,
    );
    effects
}

pub(super) fn restore_session(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    open_repos: Vec<PathBuf>,
    active_repo: Option<PathBuf>,
) -> Vec<Effect> {
    let now = SystemTime::now();
    repos.clear();
    state.repos.clear();
    state.active_repo = None;

    let repo_history_scopes = session::load_repo_history_scopes();
    let repo_fetch_prune_deleted_remote_tracking_branches =
        session::load_repo_fetch_prune_deleted_remote_tracking_branches_by_repo();
    let active_repo = active_repo.map(normalize_repo_path);
    let mut active_repo_id: Option<RepoId> = None;

    let open_repos = dedup_paths_in_order(open_repos);
    let mut effects = Vec::with_capacity(open_repos.len() + 1);
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
            let workdir_key = session::path_storage_key(&spec.workdir);
            if let Some(scope) = repo_history_scopes.get(&workdir_key).copied() {
                repo_state.history_state.history_scope = scope;
            }
            if let Some(enabled) = repo_fetch_prune_deleted_remote_tracking_branches
                .get(&workdir_key)
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
    if let Some(active_repo_id) = state.active_repo
        && let Some(repo_state) = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == active_repo_id)
    {
        repo_state.last_active_at = Some(now);
    }

    effects.push(persist_session_effect(
        state,
        state.active_repo,
        "restoring repository session",
    ));
    effects
}

pub(super) fn close_repo(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    clear_banner_error_for_repo(state, repo_id);
    let removed_repo_ix = state.repos.iter().position(|repo| repo.id == repo_id);
    state.repos.retain(|r| r.id != repo_id);
    repos.remove(&repo_id);
    if state.active_repo == Some(repo_id) {
        state.active_repo = removed_repo_ix.and_then(|repo_ix| {
            if state.repos.is_empty() {
                None
            } else if repo_ix > 0 {
                state.repos.get(repo_ix - 1).map(|repo| repo.id)
            } else {
                state.repos.first().map(|repo| repo.id)
            }
        });
    }
    vec![persist_session_effect(
        state,
        state.active_repo,
        "closing a repository",
    )]
}

pub(super) fn set_active_repo(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let mut effects = SetActiveRepoEffects::new();
    fill_set_active_repo_inline(state, repo_id, &mut effects);
    effects.into_vec()
}

pub(super) fn fill_set_active_repo_inline(
    state: &mut AppState,
    repo_id: RepoId,
    effects: &mut SetActiveRepoEffects,
) {
    enum SelectedDiffReload {
        Conflict(PathBuf),
        ConflictCurrent,
        Diff(super::util::SelectedDiffLoadPlan),
    }

    effects.clear();

    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return;
    };

    let now = SystemTime::now();
    let changed = state.active_repo != Some(repo_id);
    state.active_repo = Some(repo_id);
    let persist_effect = changed
        .then(|| persist_session_effect(state, Some(repo_id), "switching active repository"));
    let git_log_settings = state.git_log_settings;

    let repo_state = &mut state.repos[repo_ix];

    // Session-restore placeholders and repos still opening do not have a backend handle yet.
    // Defer handle-dependent refreshes until RepoOpenedOk installs the handle and schedules the
    // initial refresh for the active repo.
    if !matches!(repo_state.open, Loadable::Ready(())) {
        repo_state.last_active_at = Some(now);
        if let Some(effect) = persist_effect {
            effects.push(effect);
        }
        return;
    }

    let use_full_refresh =
        changed && !repo_switch_can_use_primary_refresh(repo_state, git_log_settings, now);
    repo_state.last_active_at = Some(now);

    // Reload the selected diff when switching repos; steady-state refreshes rely on the
    // filesystem watcher (`RepoExternallyChanged`) for diff invalidation.
    let selected_diff_reload = if changed {
        repo_state.diff_state.diff_target.as_ref().map(|target| {
            if let Some(conflict_target) = selected_conflict_target(repo_state, target) {
                match conflict_target {
                    SelectedConflictTarget::Current => SelectedDiffReload::ConflictCurrent,
                    SelectedConflictTarget::Path(path) => {
                        SelectedDiffReload::Conflict(path.to_path_buf())
                    }
                }
            } else {
                SelectedDiffReload::Diff(selected_diff_load_plan(repo_state, target))
            }
        })
    } else {
        None
    };

    // On focus events the UI can re-send SetActiveRepo for the already-active repo. Avoid
    // re-running the full refresh fan-out in that case: prioritize the minimum set that
    // keeps the UI correct and responsive.
    let extra_effect_capacity = usize::from(selected_diff_reload.is_some())
        + usize::from(persist_effect.is_some())
        + usize::from(changed)
        + usize::from(changed && !use_full_refresh)
        + usize::from(repo_state.sidebar_data_request.worktrees)
        + usize::from(repo_state.sidebar_data_request.submodules)
        + usize::from(repo_state.sidebar_data_request.stashes);
    let base_effect_capacity = if use_full_refresh {
        refresh_full_effect_capacity()
    } else {
        refresh_primary_effect_capacity()
    };
    debug_assert!(
        base_effect_capacity + extra_effect_capacity <= SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY
    );
    if use_full_refresh {
        append_refresh_full_effects(repo_state, git_log_settings, effects);
    } else {
        append_refresh_primary_effects(repo_state, effects);
    }
    if changed
        && !use_full_refresh
        && repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::BRANCHES)
    {
        effects.push(Effect::LoadBranches { repo_id });
    }
    if changed {
        append_repo_switch_worktree_refresh_effect(repo_state, effects);
    }
    append_ensure_sidebar_data_effects(repo_state, effects);

    if let Some(selected_diff_reload) = selected_diff_reload {
        match selected_diff_reload {
            SelectedDiffReload::ConflictCurrent => {
                append_start_current_conflict_target_reload(effects, repo_state);
            }
            SelectedDiffReload::Conflict(conflict_path) => {
                append_start_conflict_target_reload(effects, repo_state, &conflict_path);
            }
            SelectedDiffReload::Diff(load_plan) => {
                effects.push(Effect::LoadSelectedDiff {
                    repo_id,
                    load_patch_diff: load_plan.load_patch_diff,
                    load_file_text: load_plan.load_file_text,
                    preview_text_side: load_plan.preview_text_side,
                    load_file_image: load_plan.load_file_image,
                });
            }
        }
    }
    if let Some(effect) = persist_effect {
        effects.push(effect);
    }
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
    let mut effects = ReorderRepoTabsEffects::new();
    fill_reorder_repo_tabs_inline(state, repo_id, insert_before, &mut effects);
    effects.into_vec()
}

pub(super) fn fill_reorder_repo_tabs_inline(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
    effects: &mut ReorderRepoTabsEffects,
) {
    if state.repos.len() <= 1 {
        return;
    }

    if insert_before == Some(repo_id) {
        return;
    }

    let mut from_ix = None;
    let mut before_ix = None;
    for (ix, repo) in state.repos.iter().enumerate() {
        if repo.id == repo_id {
            from_ix = Some(ix);
        }
        if insert_before == Some(repo.id) {
            before_ix = Some(ix);
        }
        if from_ix.is_some() && (insert_before.is_none() || before_ix.is_some()) {
            break;
        }
    }

    let Some(from_ix) = from_ix else {
        return;
    };

    match before_ix {
        Some(before_ix) if from_ix + 1 == before_ix => {
            // Already immediately before the target.
            return;
        }
        Some(before_ix) if from_ix < before_ix => {
            state.repos[from_ix..before_ix].rotate_left(1);
        }
        Some(before_ix) => {
            state.repos[before_ix..=from_ix].rotate_right(1);
        }
        None if from_ix + 1 == state.repos.len() => {
            // Already last.
            return;
        }
        None => {
            state.repos[from_ix..].rotate_left(1);
        }
    };

    effects.push(persist_session_effect(
        state,
        state.active_repo,
        "reordering repository tabs",
    ));
}

pub(super) fn clone_repo(state: &mut AppState, url: String, dest: PathBuf) -> Vec<Effect> {
    state.clone = Some(CloneOpState {
        url: Arc::<str>::from(url.as_str()),
        dest: Arc::new(dest.clone()),
        status: CloneOpStatus::Running,
        progress: CloneProgressMeter::default(),
        seq: 0,
        output_tail: VecDeque::new(),
    });
    vec![Effect::CloneRepo {
        url,
        dest,
        auth: None,
    }]
}

fn parse_clone_progress_percent(line: &str) -> Option<u8> {
    let percent_ix = line.find('%')?;
    let digits = line[..percent_ix]
        .chars()
        .rev()
        .skip_while(|ch| ch.is_ascii_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse::<u8>()
        .ok()
        .map(|percent| percent.min(100))
}

fn parse_clone_progress_meter(line: &str) -> Option<CloneProgressMeter> {
    let stage = if line.starts_with("Resolving deltas:") || line.starts_with("Updating files:") {
        CloneProgressStage::RemoteObjects
    } else if line.starts_with("Receiving objects:")
        || line.starts_with("remote: Counting objects:")
        || line.starts_with("remote: Compressing objects:")
    {
        CloneProgressStage::Loading
    } else {
        return None;
    };
    let percent = parse_clone_progress_percent(line)?;
    Some(CloneProgressMeter { stage, percent })
}

pub(super) fn abort_clone_repo(state: &mut AppState, dest: PathBuf) -> Vec<Effect> {
    let Some(op) = state.clone.as_mut() else {
        return Vec::new();
    };
    if op.dest.as_ref() != &dest || !matches!(op.status, CloneOpStatus::Running) {
        return Vec::new();
    }

    op.status = CloneOpStatus::Cancelling;
    op.seq = op.seq.wrapping_add(1);
    vec![Effect::AbortCloneRepo { dest }]
}

pub(super) fn clone_repo_progress(
    state: &mut AppState,
    dest: Arc<PathBuf>,
    line: String,
) -> Vec<Effect> {
    const MAX_LINES: usize = 80;

    if let Some(op) = state.clone.as_mut()
        && matches!(op.status, CloneOpStatus::Running)
        && op.dest.as_ref() == dest.as_ref()
    {
        op.seq = op.seq.wrapping_add(1);
        if let Some(progress) = parse_clone_progress_meter(&line) {
            op.progress = progress;
        }
        if !line.trim().is_empty() {
            if op.output_tail.capacity() < MAX_LINES {
                op.output_tail
                    .reserve(MAX_LINES.saturating_sub(op.output_tail.capacity()));
            }
            if op.output_tail.len() == MAX_LINES {
                op.output_tail.pop_front();
            }
            op.output_tail.push_back(line);
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
        && op.dest.as_ref() == &dest
    {
        op.url = Arc::<str>::from(url.as_str());
        op.status = match result {
            Ok(_) => CloneOpStatus::FinishedOk,
            Err(ref error)
                if matches!(op.status, CloneOpStatus::Cancelling)
                    && is_plain_clone_abort_error(error) =>
            {
                CloneOpStatus::Cancelled
            }
            Err(e) => CloneOpStatus::FinishedErr(format_failure_summary("Clone", &e)),
        };
        op.seq = op.seq.wrapping_add(1);
    } else {
        state.clone = Some(CloneOpState {
            url: Arc::<str>::from(url.as_str()),
            dest: Arc::new(dest),
            status: match result {
                Ok(_) => CloneOpStatus::FinishedOk,
                Err(e) => CloneOpStatus::FinishedErr(format_failure_summary("Clone", &e)),
            },
            progress: CloneProgressMeter::default(),
            seq: 1,
            output_tail: VecDeque::new(),
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
    let git_log_settings = state.git_log_settings;

    let spec = RepoSpec {
        workdir: normalize_repo_path(spec.workdir),
    };
    let mut clear_banner = false;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.set_spec(spec);
        repo_state.set_open(Loadable::Ready(()));
        repo_state.missing_on_disk = false;
        repo_state.set_head_branch(Loadable::Loading);
        repo_state.set_detached_head_commit(None);
        repo_state.set_upstream_divergence(Loadable::Loading);
        repo_state.set_branches(Loadable::Loading);
        if git_log_settings.show_history_tags
            && git_log_settings.auto_fetch_tags_on_repo_activation()
        {
            repo_state.set_tags(Loadable::Loading);
        } else {
            repo_state.set_tags(Loadable::NotLoaded);
        }
        repo_state.set_remote_tags(Loadable::NotLoaded);
        repo_state.set_remotes(Loadable::Loading);
        repo_state.set_remote_branches(Loadable::Loading);
        repo_state.set_status(Loadable::Loading);
        repo_state.set_log(Loadable::Loading);
        repo_state.set_log_loading_more(false);
        repo_state.set_stashes(Loadable::NotLoaded);
        repo_state.reflog = Loadable::NotLoaded;
        repo_state.set_rebase_in_progress(Loadable::Loading);
        repo_state.set_merge_commit_message(Loadable::Loading);
        repo_state.history_state.file_history_path = None;
        repo_state.history_state.file_history = Loadable::NotLoaded;
        repo_state.history_state.blame_path = None;
        repo_state.history_state.blame_rev = None;
        repo_state.history_state.blame = Loadable::NotLoaded;
        repo_state.set_worktrees(Loadable::NotLoaded);
        repo_state.set_submodules(Loadable::NotLoaded);
        repo_state.set_selected_commit(None);
        repo_state.set_commit_details(Loadable::NotLoaded);
        repo_state.diff_state.diff_target = None;
        repo_state.diff_state.diff = Loadable::NotLoaded;
        repo_state.diff_state.diff_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
        repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
        repo_state.bump_diff_state_rev();
        repo_state.last_error = None;
        clear_banner = true;
    }

    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }

    let should_refresh_worktrees = state.active_repo == Some(repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let mut effects = refresh_full_effects(repo_state, git_log_settings);
        if should_refresh_worktrees
            && repo_state
                .loads_in_flight
                .request(RepoLoadsInFlight::WORKTREES)
        {
            repo_state.set_worktrees(Loadable::Loading);
            effects.push(Effect::LoadWorktrees { repo_id });
        }
        if should_refresh_worktrees {
            append_ensure_sidebar_data_effects(repo_state, &mut effects);
        }
        return effects;
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
        clear_banner_error_for_repo(state, repo_id);
        push_notification(
            state,
            AppNotificationKind::Error,
            format!("Folder is not a git repository: {}", spec.workdir.display()),
        );

        let remove_recent_result = session::remove_recent_repo(&spec.workdir);
        handle_session_persist_result(
            state,
            Some(repo_id),
            "removing an invalid repository from recent repositories",
            remove_recent_result,
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

    let mut clear_banner = false;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.set_spec(spec);
        repo_state.set_open(Loadable::Error(error.to_string()));
        repo_state.missing_on_disk = is_missing_repo_error(&error);
        if repo_state.missing_on_disk {
            repo_state.last_error = None;
            clear_banner = true;
        } else {
            repo_state.last_error = Some(error.to_string());
            push_diagnostic(repo_state, DiagnosticKind::Error, error.to_string());
        }
    }
    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }
    Vec::new()
}
