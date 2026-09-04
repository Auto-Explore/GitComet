use super::effects::append_ensure_sidebar_data_effects;
use super::util::{
    EffectAccumulator, SelectedConflictTarget, append_auto_background_metadata_effects,
    append_refresh_full_effects, append_refresh_primary_effects,
    append_start_conflict_target_reload, append_start_current_conflict_target_reload,
    background_metadata_effect_capacity, clear_banner_error_for_repo, dedup_paths_in_order,
    format_failure_summary, handle_session_persist_result, normalize_repo_path, push_diagnostic,
    push_notification, refresh_full_effect_capacity, refresh_full_effects,
    refresh_primary_effect_capacity, selected_conflict_target, selected_diff_load_plan,
};
use crate::model::{
    AppNotificationKind, AppState, CloneOpState, CloneOpStatus, CloneProgressMeter,
    CloneProgressStage, DiagnosticKind, GitLogSettings, Loadable, RepoId, RepoLoadsInFlight,
    RepoState, SidebarMode,
};
use crate::msg::Effect;
use crate::session;
use crate::store::repo_load_trace;
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, GitRepository};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const HOT_REPO_SWITCH_SECONDARY_REFRESH_WINDOW: Duration = Duration::from_secs(5);
const REACTIVATED_FILE_HISTORY_LIMIT: usize = 200;
pub(crate) const SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY: usize = 32;
pub(crate) type SetActiveRepoEffects = SmallVec<[Effect; SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY]>;
pub(crate) const REORDER_REPO_TABS_INLINE_EFFECT_CAPACITY: usize = 1;
pub(crate) type ReorderRepoTabsEffects =
    SmallVec<[Effect; REORDER_REPO_TABS_INLINE_EFFECT_CAPACITY]>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenRepoMode {
    Standard,
    ProvisionalExternalDrop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailedOpenActivation {
    AdjacentTab,
    PreviousActive(Option<RepoId>),
}

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

fn persist_recent_repo_effect(repo_id: Option<RepoId>, workdir: PathBuf) -> Effect {
    Effect::PersistRecentRepo {
        repo_id,
        workdir,
        action: "updating recent repositories",
    }
}

fn persist_repo_history_mode_effect(
    repo_id: Option<RepoId>,
    workdir: PathBuf,
    mode: gitcomet_core::domain::HistoryMode,
) -> Effect {
    Effect::PersistRepoHistoryMode {
        repo_id,
        workdir,
        mode,
        action: "updating history mode",
    }
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
    if let Some(effect) = super::effects::request_worktree_dirty_effect(repo_state) {
        effects.push(effect);
    }
}

fn clear_loading<T>(loadable: &mut Loadable<T>) -> bool {
    if matches!(loadable, Loadable::Loading) {
        *loadable = Loadable::NotLoaded;
        true
    } else {
        false
    }
}

fn clear_cancelled_repo_loading(repo_state: &mut RepoState) {
    repo_state.loads_in_flight.clear();
    // The cancelled walk's reply is dropped by the repo-load guard, so nothing
    // downstream will ever clear the count it left on screen.
    repo_state.set_log_scan_progress(None);
    if matches!(repo_state.open, Loadable::Loading) {
        repo_state.set_open(Loadable::NotLoaded);
    }
    if repo_state.head_branch.is_loading() {
        repo_state.set_head_branch(Loadable::NotLoaded);
    }
    if repo_state.upstream_divergence.is_loading() {
        repo_state.set_upstream_divergence(Loadable::NotLoaded);
    }
    if repo_state.branches.is_loading() {
        repo_state.set_branches(Loadable::NotLoaded);
    }
    if repo_state.tags.is_loading() {
        repo_state.set_tags(Loadable::NotLoaded);
    }
    if repo_state.remote_tags.is_loading() {
        repo_state.set_remote_tags(Loadable::NotLoaded);
    }
    if repo_state.remotes.is_loading() {
        repo_state.set_remotes(Loadable::NotLoaded);
    }
    if repo_state.remote_branches.is_loading() {
        repo_state.set_remote_branches(Loadable::NotLoaded);
    }
    if repo_state.worktree_status.is_loading() {
        repo_state.set_worktree_status(Loadable::NotLoaded);
    }
    if repo_state.staged_status.is_loading() {
        repo_state.set_staged_status(Loadable::NotLoaded);
    }
    if repo_state.status.is_loading() {
        repo_state.set_status(Loadable::NotLoaded);
    }
    if repo_state.log.is_loading() {
        repo_state.set_log(Loadable::NotLoaded);
    }
    repo_state.set_log_loading_more(false);
    if repo_state.stashes.is_loading() {
        repo_state.set_stashes(Loadable::NotLoaded);
    }
    // `clear_loading` cannot be used here: it mutates the field in place, which
    // would leave `reflog_rev` stale for the panel that keys its row cache on it.
    if repo_state.reflog.is_loading() {
        repo_state.set_reflog(Loadable::NotLoaded);
    }
    if repo_state.rebase_in_progress.is_loading() {
        repo_state.set_rebase_in_progress(Loadable::NotLoaded);
    }
    if repo_state.sequencer_state.is_loading() {
        repo_state.set_sequencer_state(Loadable::NotLoaded);
    }
    if repo_state.merge_commit_message.is_loading() {
        repo_state.set_merge_commit_message(Loadable::NotLoaded);
    }
    if repo_state.worktrees.is_loading() {
        repo_state.set_worktrees(Loadable::NotLoaded);
    }
    if repo_state.submodules.is_loading() {
        repo_state.set_submodules(Loadable::NotLoaded);
    }
    clear_loading(&mut repo_state.history_state.file_history);
    clear_loading(&mut repo_state.history_state.blame);
    if repo_state.history_state.commit_details.is_loading() {
        repo_state.set_commit_details(Loadable::NotLoaded);
    }

    let mut diff_changed = false;
    diff_changed |= clear_loading(&mut repo_state.diff_state.diff);
    diff_changed |= clear_loading(&mut repo_state.diff_state.diff_file);
    diff_changed |= clear_loading(&mut repo_state.diff_state.diff_preview_text_file);
    diff_changed |= clear_loading(&mut repo_state.diff_state.submodule_summary);
    diff_changed |= clear_loading(&mut repo_state.diff_state.diff_file_image);
    if let Some(inline) = repo_state.diff_state.inline_submodule_diff.as_mut() {
        let mut inline_changed = false;
        inline_changed |= clear_loading(&mut inline.diff);
        inline_changed |= clear_loading(&mut inline.diff_file);
        inline_changed |= clear_loading(&mut inline.diff_file_image);
        if inline_changed {
            inline.diff_rev = inline.diff_rev.wrapping_add(1);
            inline.diff_file_rev = inline.diff_file_rev.wrapping_add(1);
            repo_state.diff_state.inline_submodule_diff_rev = repo_state
                .diff_state
                .inline_submodule_diff_rev
                .wrapping_add(1);
            diff_changed = true;
        }
    }
    if diff_changed {
        repo_state.bump_diff_state_rev();
    }
    if repo_state.conflict_state.conflict_file.is_loading() {
        repo_state.set_conflict_file(Loadable::NotLoaded);
    }
}

pub(in crate::store::reducer) fn append_cancel_repo_loads_effect_for_repo(
    state: &mut AppState,
    repo_id: Option<RepoId>,
    effects: &mut impl Extend<Effect>,
) {
    let Some(repo_id) = repo_id else {
        return;
    };
    let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return;
    };
    let load_epoch = repo_state.bump_load_epoch();
    repo_load_trace::trace!(
        "reducer_cancel_repo_loads repo_id={:?} cancel_epoch={} next_epoch={} workdir={}",
        repo_id,
        load_epoch,
        repo_state.load_epoch,
        repo_state.spec.workdir.display()
    );
    clear_cancelled_repo_loading(repo_state);
    effects.extend(std::iter::once(Effect::CancelRepoLoads {
        repo_id,
        load_epoch,
    }));
}

fn append_open_repo_effect_if_not_loaded(
    repo_state: &mut RepoState,
    effects: &mut impl Extend<Effect>,
) {
    if matches!(repo_state.open, Loadable::NotLoaded) {
        repo_state.set_open(Loadable::Loading);
        effects.extend(std::iter::once(Effect::OpenRepo {
            repo_id: repo_state.id,
            path: repo_state.spec.workdir.clone(),
        }));
    }
}

pub(in crate::store::reducer) enum SelectedHistoryReload {
    FileHistory(PathBuf),
    Blame {
        path: PathBuf,
        source: gitcomet_core::domain::BlameSource,
    },
    CommitDetails(gitcomet_core::domain::CommitId),
}

pub(in crate::store::reducer) fn selected_history_reloads_for_activation(
    repo_state: &RepoState,
) -> SmallVec<[SelectedHistoryReload; 3]> {
    let mut reloads = SmallVec::new();

    if matches!(repo_state.history_state.file_history, Loadable::NotLoaded)
        && let Some(path) = repo_state.history_state.file_history_path.clone()
    {
        reloads.push(SelectedHistoryReload::FileHistory(path));
    }

    if matches!(repo_state.history_state.blame, Loadable::NotLoaded)
        && let Some(path) = repo_state.history_state.blame_path.clone()
        && let Some(source) = repo_state.history_state.blame_source.clone()
    {
        reloads.push(SelectedHistoryReload::Blame { path, source });
    }

    if matches!(repo_state.history_state.commit_details, Loadable::NotLoaded)
        && let Some(commit_id) = repo_state.history_state.selected_commit.clone()
    {
        reloads.push(SelectedHistoryReload::CommitDetails(commit_id));
    }

    reloads
}

pub(in crate::store::reducer) fn append_selected_history_reload_effects(
    repo_id: RepoId,
    repo_state: &mut RepoState,
    reloads: SmallVec<[SelectedHistoryReload; 3]>,
    effects: &mut impl EffectAccumulator,
) {
    for reload in reloads {
        match reload {
            SelectedHistoryReload::FileHistory(path) => {
                repo_state.history_state.file_history = Loadable::Loading;
                effects.push_effect(Effect::LoadFileHistory {
                    repo_id,
                    path,
                    limit: REACTIVATED_FILE_HISTORY_LIMIT,
                });
            }
            SelectedHistoryReload::Blame { path, source } => {
                repo_state.history_state.blame = Loadable::Loading;
                effects.push_effect(Effect::LoadBlame {
                    repo_id,
                    path,
                    source,
                });
            }
            SelectedHistoryReload::CommitDetails(commit_id) => {
                repo_state.set_commit_details(Loadable::Loading);
                effects.push_effect(Effect::LoadCommitDetails { repo_id, commit_id });
            }
        }
    }
}

pub(super) fn open_repo(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: PathBuf,
) -> Vec<Effect> {
    open_repo_with_mode(repos, id_alloc, state, path, OpenRepoMode::Standard)
}

pub(super) fn open_repo_from_external_drop(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: PathBuf,
) -> Vec<Effect> {
    open_repo_with_mode(
        repos,
        id_alloc,
        state,
        path,
        OpenRepoMode::ProvisionalExternalDrop,
    )
}

fn open_repo_with_mode(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: PathBuf,
    mode: OpenRepoMode,
) -> Vec<Effect> {
    let now = SystemTime::now();
    let path = normalize_repo_path(path);
    if let Some((repo_id, existing_is_provisional_drop)) = state
        .repos
        .iter()
        .find(|r| r.spec.workdir == path)
        .map(|r| (r.id, r.is_provisional_external_drop_open()))
    {
        // Re-opening an already open repository should still refresh primary state, so stale
        // status/diff data gets reconciled immediately.
        let mut effects = set_active_repo(repos, state, repo_id);
        if mode == OpenRepoMode::Standard || !existing_is_provisional_drop {
            effects.push(persist_recent_repo_effect(Some(repo_id), path));
        }
        return effects;
    }

    let previous_active = state.active_repo;
    let repo_id = RepoId(id_alloc.fetch_add(1, Ordering::Relaxed));
    let spec = RepoSpec { workdir: path };
    let session_preferences = session::load_repo_session_preferences();
    let workdir_key = session::path_storage_key(&spec.workdir);
    let saved_history_mode = session_preferences
        .repo_history_modes
        .get(&workdir_key)
        .copied();
    let history_mode = saved_history_mode
        .or_else(|| {
            session_preferences
                .repo_history_scopes
                .get(&workdir_key)
                .copied()
        })
        .or(session_preferences.default_history_mode)
        .unwrap_or_default();

    state.repos.push({
        let mut repo_state = match mode {
            OpenRepoMode::Standard => RepoState::new_opening(repo_id, spec.clone()),
            OpenRepoMode::ProvisionalExternalDrop => {
                RepoState::new_external_drop_opening(repo_id, spec.clone(), previous_active)
            }
        };
        repo_state.history_state.history_scope = history_mode;
        repo_state.history_state.history_author_filter = session_preferences
            .repo_history_author_filters
            .get(&workdir_key)
            .cloned()
            .flatten();
        if let Some(enabled) = session_preferences
            .repo_fetch_prune_deleted_remote_tracking_branches
            .get(&workdir_key)
            .copied()
        {
            repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
        }
        repo_state.last_active_at = Some(now);
        repo_state
    });
    state.active_repo = Some(repo_id);
    let mut effects = Vec::new();
    append_cancel_repo_loads_effect_for_repo(state, previous_active, &mut effects);
    effects.push(Effect::OpenRepo {
        repo_id,
        path: spec.workdir.clone(),
    });
    if mode == OpenRepoMode::Standard {
        effects.push(persist_recent_repo_effect(
            Some(repo_id),
            spec.workdir.clone(),
        ));
        effects.push(persist_session_effect(
            state,
            Some(repo_id),
            "opening a repository",
        ));
        if saved_history_mode.is_none() {
            effects.push(persist_repo_history_mode_effect(
                Some(repo_id),
                spec.workdir,
                history_mode,
            ));
        }
    }
    effects
}

pub(super) fn restore_session(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    open_repos: Vec<PathBuf>,
    active_repo: Option<PathBuf>,
) -> Vec<Effect> {
    let now = SystemTime::now();
    repos.clear();
    state.repos.clear();
    state.active_repo = None;

    let session_preferences = session::load_repo_session_preferences();
    let default_history_mode = session_preferences.default_history_mode.unwrap_or_default();
    let active_repo = active_repo.map(normalize_repo_path);
    let mut active_repo_id: Option<RepoId> = None;
    let mut history_mode_persist_updates = Vec::new();

    let open_repos = dedup_paths_in_order(open_repos);
    let mut effects = Vec::with_capacity(4);
    let mut seen_workdirs: FxHashSet<PathBuf> = FxHashSet::default();
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
        let workdir_key = session::path_storage_key(&spec.workdir);
        let saved_history_mode = session_preferences
            .repo_history_modes
            .get(&workdir_key)
            .copied();
        let history_mode = saved_history_mode
            .or_else(|| {
                session_preferences
                    .repo_history_scopes
                    .get(&workdir_key)
                    .copied()
            })
            .unwrap_or(default_history_mode);

        let mut repo_state = {
            let mut repo_state = crate::model::RepoState::new_opening(repo_id, spec.clone());
            repo_state.history_state.history_scope = history_mode;
            repo_state.history_state.history_author_filter = session_preferences
                .repo_history_author_filters
                .get(&workdir_key)
                .cloned()
                .flatten();
            if let Some(enabled) = session_preferences
                .repo_fetch_prune_deleted_remote_tracking_branches
                .get(&workdir_key)
                .copied()
            {
                repo_state.fetch_prune_deleted_remote_tracking_branches = enabled;
            }
            repo_state
        };
        repo_state.set_open(Loadable::NotLoaded);
        state.repos.push(repo_state);
        if saved_history_mode.is_none() {
            history_mode_persist_updates.push((spec.workdir.clone(), history_mode));
        }
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
        append_open_repo_effect_if_not_loaded(repo_state, &mut effects);
    }

    if !history_mode_persist_updates.is_empty() {
        effects.push(Effect::PersistRepoHistoryModesBatch {
            repo_id: state.active_repo,
            updates: history_mode_persist_updates,
            action: "updating history mode",
        });
    }

    effects.push(persist_session_effect(
        state,
        state.active_repo,
        "restoring repository session",
    ));
    effects
}

pub(super) fn close_repo(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    clear_banner_error_for_repo(state, repo_id);
    if state
        .branch_exists_prompt
        .as_ref()
        .is_some_and(|prompt| prompt.repo_id == repo_id)
    {
        state.branch_exists_prompt = None;
    }
    let mut effects = Vec::with_capacity(3 + SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY);
    let Some(removed_repo_ix) = state.repos.iter().position(|repo| repo.id == repo_id) else {
        effects.push(persist_session_effect(
            state,
            state.active_repo,
            "closing a repository",
        ));
        return effects;
    };

    append_cancel_repo_loads_effect_for_repo(state, Some(repo_id), &mut effects);
    let was_active = state.active_repo == Some(repo_id);
    // Recorded here rather than at the affordance that asked for the close, so
    // the Recently Closed list is ordered by when repositories were closed no
    // matter which of them (tab `x`, tab menu, picker row menu, close-others)
    // the user reached for.
    let closed_repo = &state.repos[removed_repo_ix];
    let closed_workdir = closed_repo.spec.workdir.clone();
    let persist_closed_recent = !closed_repo.is_provisional_external_drop_open();
    state.repos.remove(removed_repo_ix);
    repos.remove(&repo_id);
    // The worktree scan's cached handles are pruned only by that repo's own scan,
    // and a closed repo never scans again. This is the one place that knows the
    // repo is gone rather than merely idle -- `CancelRepoLoads` also fires on tab
    // switches and reloads, where the handles are still worth keeping.
    crate::store::effects::release_worktree_scan_handles(repo_id);
    if persist_closed_recent {
        effects.push(persist_recent_repo_effect(Some(repo_id), closed_workdir));
    }
    if was_active {
        let next_active_repo = if state.repos.is_empty() {
            None
        } else if removed_repo_ix > 0 {
            state.repos.get(removed_repo_ix - 1).map(|repo| repo.id)
        } else {
            state.repos.first().map(|repo| repo.id)
        };
        if let Some(active_repo_id) = next_active_repo {
            let mut activation_effects = SetActiveRepoEffects::new();
            fill_set_active_repo_inline_impl(
                repos,
                state,
                active_repo_id,
                &mut activation_effects,
                false,
            );
            effects.extend(activation_effects);
        } else {
            state.active_repo = None;
        }
    }
    effects.push(persist_session_effect(
        state,
        state.active_repo,
        "closing a repository",
    ));
    effects
}

pub(super) fn close_repos(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_ids: Vec<RepoId>,
    activate_after: Option<RepoId>,
) -> Vec<Effect> {
    let mut close_ids = FxHashSet::default();
    for repo_id in repo_ids {
        if state.repos.iter().any(|repo| repo.id == repo_id) {
            close_ids.insert(repo_id);
        }
    }
    if close_ids.is_empty() {
        return Vec::new();
    }
    if state
        .branch_exists_prompt
        .as_ref()
        .is_some_and(|prompt| close_ids.contains(&prompt.repo_id))
    {
        state.branch_exists_prompt = None;
    }

    let original_order: Vec<RepoId> = state.repos.iter().map(|repo| repo.id).collect();
    let original_active = state.active_repo;
    let original_active_ix =
        original_active.and_then(|repo_id| original_order.iter().position(|id| *id == repo_id));

    let mut effects =
        Vec::with_capacity(2 * close_ids.len() + 2 + SET_ACTIVE_REPO_INLINE_EFFECT_CAPACITY);
    // Tab order, not `close_ids` iteration order: a `FxHashSet` would leave the
    // Recently Closed entries for one bulk close in an order that varies run to
    // run. Walking left to right puts the rightmost tab at the top of the list.
    for repo_id in original_order.iter().copied() {
        if !close_ids.contains(&repo_id) {
            continue;
        }
        clear_banner_error_for_repo(state, repo_id);
        append_cancel_repo_loads_effect_for_repo(state, Some(repo_id), &mut effects);
        if let Some(repo) = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .filter(|repo| !repo.is_provisional_external_drop_open())
        {
            effects.push(persist_recent_repo_effect(
                Some(repo_id),
                repo.spec.workdir.clone(),
            ));
        }
        repos.remove(&repo_id);
        crate::store::effects::release_worktree_scan_handles(repo_id);
    }

    state.repos.retain(|repo| !close_ids.contains(&repo.id));

    let repo_still_open =
        |repo_id: RepoId, state: &AppState| state.repos.iter().any(|repo| repo.id == repo_id);
    let requested_active = activate_after.filter(|repo_id| repo_still_open(*repo_id, state));
    let active_was_closed = original_active.is_some_and(|repo_id| close_ids.contains(&repo_id));
    let next_active_repo = if state.repos.is_empty() {
        None
    } else if let Some(repo_id) = requested_active {
        Some(repo_id)
    } else if active_was_closed {
        original_active_ix
            .and_then(|ix| {
                original_order[..ix]
                    .iter()
                    .rev()
                    .copied()
                    .find(|repo_id| !close_ids.contains(repo_id))
                    .or_else(|| {
                        original_order[ix + 1..]
                            .iter()
                            .copied()
                            .find(|repo_id| !close_ids.contains(repo_id))
                    })
            })
            .or_else(|| state.repos.first().map(|repo| repo.id))
    } else {
        state
            .active_repo
            .filter(|repo_id| repo_still_open(*repo_id, state))
            .or_else(|| state.repos.first().map(|repo| repo.id))
    };

    if let Some(active_repo_id) = next_active_repo {
        if state.active_repo != Some(active_repo_id) {
            let mut activation_effects = SetActiveRepoEffects::new();
            fill_set_active_repo_inline_impl(
                repos,
                state,
                active_repo_id,
                &mut activation_effects,
                false,
            );
            effects.extend(activation_effects);
        } else {
            state.active_repo = Some(active_repo_id);
        }
    } else {
        state.active_repo = None;
    }

    effects.push(persist_session_effect(
        state,
        state.active_repo,
        "closing repositories",
    ));
    effects
}

pub(super) fn set_active_repo(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    let mut effects = SetActiveRepoEffects::new();
    fill_set_active_repo_inline(repos, state, repo_id, &mut effects);
    effects.into_vec()
}

/// With the sidebar in Files mode, activating a repo (or its open completing)
/// must kick the file-browser listing exactly like switching into Files mode
/// does — otherwise the tree sits on "Loading files..." until the user
/// toggles the sidebar tabs.
fn file_browser_load_for_active_files_mode(
    sidebar_mode: SidebarMode,
    repo_state: &RepoState,
) -> Option<Effect> {
    (sidebar_mode == SidebarMode::Files && repo_state.file_browser.needs_load()).then(|| {
        Effect::LoadFileBrowser {
            repo_id: repo_state.id,
            source: repo_state.file_browser.source.clone(),
        }
    })
}

pub(super) fn fill_set_active_repo_inline(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    effects: &mut SetActiveRepoEffects,
) {
    fill_set_active_repo_inline_impl(repos, state, repo_id, effects, true);
}

fn fill_set_active_repo_inline_impl(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    effects: &mut SetActiveRepoEffects,
    persist_on_change: bool,
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
    let previous_active = state.active_repo;
    let changed = previous_active != Some(repo_id);
    if changed {
        // The inactive tab may have observed a HEAD move while no monitor was
        // attached to it. Refresh its retained path before capturing the diff
        // reload plan below.
        super::refresh_selected_head_gitlink(repos, state, repo_id);
    }
    if changed {
        append_cancel_repo_loads_effect_for_repo(state, previous_active, effects);
    }
    state.active_repo = Some(repo_id);
    let persist_effect = (changed && persist_on_change)
        .then(|| persist_session_effect(state, Some(repo_id), "switching active repository"));
    let git_log_settings = state.git_log_settings;
    let sidebar_mode = state.sidebar_mode;

    let repo_state = &mut state.repos[repo_ix];

    // Capture the retained diff's reload shape before resetting the history
    // selection below. Commit file metadata distinguishes submodules and
    // added/deleted preview-only files, but the selection reset deliberately
    // clears that metadata from the details pane.
    let selected_diff_reload = if changed && matches!(repo_state.open, Loadable::Ready(())) {
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

    if changed {
        // A repository tab is a workspace context, not a suspended history
        // selection. Enter it at that workspace's live tip: the history view
        // interprets no explicit commit as the uncommitted row when it exists,
        // and as HEAD when the worktree is clean. `set_selected_commit` also
        // retires multi/range and linked-worktree selections.
        repo_state.set_selected_commit(None);
        repo_state.set_commit_details(Loadable::NotLoaded);

        // Activation is not a user navigation step, so replace the snapshot at
        // the cursor rather than pushing one. This explicit replacement is
        // required when the tab was parked mid-stack: ordinary non-push
        // reconciliation intentionally leaves such entries untouched.
        let snapshot = repo_state.main_view_snapshot();
        repo_state.navigation.main_history.replace_current(snapshot);
    }

    // Session-restore placeholders and repos still opening do not have a backend handle yet.
    // Defer handle-dependent refreshes until RepoOpenedOk installs the handle and schedules the
    // initial refresh for the active repo.
    if !matches!(repo_state.open, Loadable::Ready(())) {
        repo_state.last_active_at = Some(now);
        append_open_repo_effect_if_not_loaded(repo_state, effects);
        if let Some(effect) = persist_effect {
            effects.push(effect);
        }
        return;
    }

    let use_full_refresh =
        changed && !repo_switch_can_use_primary_refresh(repo_state, git_log_settings, now);
    repo_state.last_active_at = Some(now);

    let selected_history_reloads = if changed {
        selected_history_reloads_for_activation(repo_state)
    } else {
        Default::default()
    };

    // On focus events the UI can re-send SetActiveRepo for the already-active repo. Avoid
    // re-running the full refresh fan-out in that case: prioritize the minimum set that
    // keeps the UI correct and responsive.
    let file_browser_load = file_browser_load_for_active_files_mode(sidebar_mode, repo_state);
    let extra_effect_capacity = background_metadata_effect_capacity()
        + usize::from(selected_diff_reload.is_some())
        + selected_history_reloads.len()
        + usize::from(persist_effect.is_some())
        + usize::from(changed)
        + usize::from(changed && !use_full_refresh)
        + usize::from(file_browser_load.is_some())
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
    if let Some(effect) = file_browser_load {
        effects.push(effect);
    }
    if changed {
        append_auto_background_metadata_effects(repo_state, git_log_settings, effects);
    }

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
                    load_submodule_summary: load_plan.load_submodule_summary,
                    load_file_image: load_plan.load_file_image,
                });
            }
        }
    }
    append_selected_history_reload_effects(repo_id, repo_state, selected_history_reloads, effects);
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
        remote_url_policy: state.remote_url_policy,
        auth: None,
    }]
}

fn parse_clone_progress_percent(line: &str) -> Option<u8> {
    let percent_ix = line.find('%')?;
    let before_percent = &line.as_bytes()[..percent_ix];
    let end = before_percent
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())?
        + 1;
    let start = before_percent[..end]
        .iter()
        .rposition(|byte| !byte.is_ascii_digit())
        .map_or(0, |ix| ix + 1);
    let digits = &before_percent[start..end];
    if digits.is_empty() {
        return None;
    }
    // Byte indices rather than `line[start..end]` because `end` can land inside a
    // multi-byte char -- a trailing `\u{e4}` is neither whitespace nor a digit, so
    // the scans stop on its continuation byte and string slicing would panic
    // there. Every byte that survives both scans is an ASCII digit, so this
    // conversion cannot actually fail.
    let digits = std::str::from_utf8(digits).ok()?;
    digits.parse::<u8>().ok().map(|percent| percent.min(100))
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
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    spec: RepoSpec,
    repo: Arc<dyn GitRepository>,
) -> Vec<Effect> {
    if !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Vec::new();
    }

    repos.insert(repo_id, repo);
    let git_log_settings = state.git_log_settings;

    let spec = RepoSpec {
        workdir: normalize_repo_path(spec.workdir),
    };
    let mut clear_banner = false;
    let mut committed_external_drop = None;
    let should_refresh_worktrees = state.active_repo == Some(repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.set_spec(spec);
        if repo_state.commit_external_drop_open() {
            committed_external_drop = Some((
                repo_state.spec.workdir.clone(),
                repo_state.history_state.history_scope,
            ));
        }
        repo_state.set_open(Loadable::Ready(()));
        repo_state.feedback.missing_on_disk = false;
        if !should_refresh_worktrees {
            clear_cancelled_repo_loading(repo_state);
            repo_state.feedback.last_error = None;
            clear_banner = true;
        } else {
            repo_state.set_head_branch(Loadable::Loading);
            repo_state.set_detached_head_commit(None);
            repo_state.set_upstream_divergence(Loadable::Loading);
            repo_state.set_branches(Loadable::Loading);
            repo_state.set_tags(Loadable::NotLoaded);
            repo_state.set_remote_tags(Loadable::NotLoaded);
            repo_state.set_remotes(Loadable::Loading);
            repo_state.set_remote_branches(Loadable::Loading);
            repo_state.set_status(Loadable::Loading);
            repo_state.set_log(Loadable::Loading);
            repo_state.set_log_loading_more(false);
            repo_state.set_stashes(Loadable::NotLoaded);
            repo_state.set_reflog(Loadable::NotLoaded);
            repo_state.set_rebase_in_progress(Loadable::Loading);
            repo_state.set_sequencer_state(Loadable::Loading);
            repo_state.set_merge_commit_message(Loadable::Loading);
            repo_state.history_state.file_history_path = None;
            repo_state.history_state.file_history = Loadable::NotLoaded;
            repo_state.history_state.blame_path = None;
            repo_state.history_state.blame_source = None;
            repo_state.history_state.blame = Loadable::NotLoaded;
            repo_state.clear_retained_blame();
            repo_state.set_worktrees(Loadable::NotLoaded);
            repo_state.set_submodules(Loadable::NotLoaded);
            repo_state.set_selected_commit(None);
            repo_state.set_commit_details(Loadable::NotLoaded);
            repo_state.set_diff_target(None);
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
            repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
            repo_state.diff_state.inline_submodule_diff = None;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
            repo_state.bump_diff_state_rev();
            repo_state.feedback.last_error = None;
            // Reopening resets the whole repo view; saved back/forward snapshots
            // may reference commits or file revisions from before the reopen, so
            // start the navigation stacks fresh.
            repo_state.navigation.main_history.clear();
            repo_state.navigation.view_history.clear();
            clear_banner = true;
        }
    }

    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }

    let mut effects = Vec::new();
    if let Some((workdir, history_mode)) = committed_external_drop {
        effects.push(persist_recent_repo_effect(Some(repo_id), workdir.clone()));
        effects.push(persist_session_effect(
            state,
            Some(repo_id),
            "opening a dropped repository",
        ));
        effects.push(persist_repo_history_mode_effect(
            Some(repo_id),
            workdir,
            history_mode,
        ));
    }

    if !should_refresh_worktrees {
        return effects;
    }
    let sidebar_mode = state.sidebar_mode;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        effects.extend(refresh_full_effects(repo_state, git_log_settings));
        if should_refresh_worktrees
            && repo_state
                .loads_in_flight
                .request(RepoLoadsInFlight::WORKTREES)
        {
            repo_state.set_worktrees(Loadable::Loading);
            effects.push(Effect::LoadWorktrees { repo_id });
        }
        // The history rows want this from the moment the repo opens, and the
        // switch-time trigger fires before the handle exists, so this is the
        // first point where the scan can actually run.
        if let Some(effect) = super::effects::request_worktree_dirty_effect(repo_state) {
            effects.push(effect);
        }
        if should_refresh_worktrees {
            append_ensure_sidebar_data_effects(repo_state, &mut effects);
            if let Some(effect) = file_browser_load_for_active_files_mode(sidebar_mode, repo_state)
            {
                effects.push(effect);
            }
            append_auto_background_metadata_effects(repo_state, git_log_settings, &mut effects);
        }
        return effects;
    }

    effects
}

fn discard_failed_repo_open(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    workdir: &std::path::Path,
    message: String,
    remove_from_recents: bool,
    activation: FailedOpenActivation,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    clear_banner_error_for_repo(state, repo_id);
    push_notification(state, AppNotificationKind::Warning, message);

    if remove_from_recents {
        let remove_recent_result = session::remove_recent_repo(workdir);
        handle_session_persist_result(
            state,
            Some(repo_id),
            "removing an invalid repository from recent repositories",
            remove_recent_result,
        );
    }

    repos.remove(&repo_id);
    if let Some(ix) = state.repos.iter().position(|r| r.id == repo_id) {
        let was_active = state.active_repo == Some(repo_id);
        state.repos.remove(ix);
        if was_active {
            let adjacent_repo = if ix > 0 {
                state.repos.get(ix - 1).map(|r| r.id)
            } else {
                state.repos.get(ix).map(|r| r.id)
            };
            let next_active_repo = match activation {
                FailedOpenActivation::AdjacentTab => adjacent_repo,
                FailedOpenActivation::PreviousActive(None) => None,
                FailedOpenActivation::PreviousActive(Some(previous_active)) => state
                    .repos
                    .iter()
                    .any(|repo| repo.id == previous_active)
                    .then_some(previous_active)
                    .or(adjacent_repo),
            };
            if let Some(active_repo_id) = next_active_repo {
                // Opening the provisional tab canceled the previous tab's
                // in-flight loads. Run the ordinary activation path so those
                // loads are requested again instead of leaving Ready repos
                // with status/log/branch fields stuck at NotLoaded.
                let mut activation_effects = SetActiveRepoEffects::new();
                fill_set_active_repo_inline_impl(
                    repos,
                    state,
                    active_repo_id,
                    &mut activation_effects,
                    false,
                );
                effects.extend(activation_effects);
            } else {
                state.active_repo = None;
            }
        }
        let persist_result = session::persist_from_state(state);
        handle_session_persist_result(
            state,
            state.active_repo,
            "discarding a failed repository open from the session",
            persist_result,
        );
    }
    effects
}

pub(super) fn repo_opened_err(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    spec: RepoSpec,
    error: Error,
) -> Vec<Effect> {
    let Some((provisional_external_drop, previous_active_repo)) = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| {
            (
                repo.is_provisional_external_drop_open(),
                repo.external_drop_previous_active_repo(),
            )
        })
    else {
        return Vec::new();
    };

    let spec = RepoSpec {
        workdir: normalize_repo_path(spec.workdir),
    };
    let not_a_repository = matches!(error.kind(), ErrorKind::NotARepository);
    if not_a_repository || provisional_external_drop {
        let message = if not_a_repository {
            format!(
                "No valid Git repository was found at {}.",
                spec.workdir.display()
            )
        } else {
            format!(
                "Could not open repository at {}: {error}",
                spec.workdir.display()
            )
        };
        return discard_failed_repo_open(
            repos,
            state,
            repo_id,
            &spec.workdir,
            message,
            not_a_repository,
            if provisional_external_drop {
                FailedOpenActivation::PreviousActive(previous_active_repo)
            } else {
                FailedOpenActivation::AdjacentTab
            },
        );
    }

    let mut clear_banner = false;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.set_spec(spec);
        repo_state.set_open(Loadable::Error(error.to_string()));
        repo_state.feedback.missing_on_disk = is_missing_repo_error(&error);
        if repo_state.feedback.missing_on_disk {
            repo_state.feedback.last_error = None;
            clear_banner = true;
        } else {
            repo_state.feedback.last_error = Some(error.to_string());
            push_diagnostic(repo_state, DiagnosticKind::Error, error.to_string());
        }
    }
    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::parse_clone_progress_percent;

    #[test]
    fn clone_progress_percent_reads_the_digits_before_the_sign() {
        assert_eq!(
            parse_clone_progress_percent("Receiving objects:  42% (42/100)"),
            Some(42)
        );
        assert_eq!(
            parse_clone_progress_percent("remote: Counting 7% done"),
            Some(7)
        );
        assert_eq!(
            parse_clone_progress_percent("Updating files: 100% (4/4), done."),
            Some(100)
        );
    }

    #[test]
    fn clone_progress_percent_skips_whitespace_between_digits_and_sign() {
        assert_eq!(
            parse_clone_progress_percent("Receiving objects: 42 %"),
            Some(42)
        );
        assert_eq!(
            parse_clone_progress_percent("Receiving objects: 42\t%"),
            Some(42)
        );
    }

    #[test]
    fn clone_progress_percent_stops_at_the_first_non_digit() {
        // The digit run is bounded backwards, so an earlier number in the same
        // line must not be folded into it.
        assert_eq!(parse_clone_progress_percent("12 34%"), Some(34));
        assert_eq!(parse_clone_progress_percent("(42/100) 7%"), Some(7));
    }

    #[test]
    fn clone_progress_percent_rejects_lines_without_usable_digits() {
        assert_eq!(parse_clone_progress_percent("no sign here"), None);
        // `%` at byte 0 leaves an empty slice for both backward scans.
        assert_eq!(parse_clone_progress_percent("%"), None);
        assert_eq!(parse_clone_progress_percent("% (0/0)"), None);
        // Whitespace-only prefix: the non-whitespace scan finds nothing at all.
        assert_eq!(parse_clone_progress_percent("   %"), None);
        assert_eq!(parse_clone_progress_percent("done%"), None);
    }

    #[test]
    fn clone_progress_percent_handles_multibyte_text_around_the_digits() {
        // A continuation byte is neither ASCII whitespace nor an ASCII digit, so
        // both scans stop on it -- the indices must stay byte indices.
        assert_eq!(
            parse_clone_progress_percent("l\u{e4}hetet\u{e4}\u{e4}n: 42%"),
            Some(42)
        );
        assert_eq!(parse_clone_progress_percent("valmis \u{e4}%"), None);
        assert_eq!(parse_clone_progress_percent("\u{5360}7%"), Some(7));
    }

    #[test]
    fn clone_progress_percent_clamps_and_rejects_out_of_range_values() {
        // Git can report over 100 while it deltifies; anything a `u8` cannot
        // hold is treated as unparseable rather than clamped.
        assert_eq!(
            parse_clone_progress_percent("Resolving deltas: 150%"),
            Some(100)
        );
        assert_eq!(
            parse_clone_progress_percent("Resolving deltas: 255%"),
            Some(100)
        );
        assert_eq!(parse_clone_progress_percent("Resolving deltas: 300%"), None);
    }
}
