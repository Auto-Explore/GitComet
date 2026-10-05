mod actions_emit_effects;
mod auth;
mod branch_exists_prompt;
#[cfg(test)]
mod comparison_tests;
mod conflict_interactions;
mod diff_selection;
mod diff_session;
mod effects;
mod external_and_history;
mod filesystem;
mod git_hook_activity;
mod git_operations;
mod history_authors;
mod history_find;
#[cfg(test)]
mod history_selection_ack_tests;
mod indexed_history;
#[cfg(test)]
mod line_stats_tests;
mod loads;
pub(super) mod maintenance;
#[cfg(test)]
mod nav_history_tests;
mod repo_management;
mod repo_watch;
mod settings;
mod submodule_trust;
mod util;
mod workspace;

use crate::model::{AppState, Loadable, RepoId};
use crate::msg::{ConflictRegionChoice, Effect, Msg, RepoPath, RepoPathList};
use auth::{annex_adjusted_refusal, annex_takeover};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

#[cfg(test)]
pub(crate) use auth::repo_command_replay_msg_for_test;
#[cfg(feature = "benchmarks")]
pub(crate) use diff_selection::SelectDiffEffects;
pub(crate) use repo_management::{ReorderRepoTabsEffects, SetActiveRepoEffects};

pub(crate) const SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY: usize = 1;
pub(crate) type SinglePathActionEffects =
    SmallVec<[Effect; SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY]>;
pub(crate) type BatchPathActionEffects =
    SmallVec<[Effect; SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY]>;

#[cfg(test)]
pub(super) fn normalize_repo_path(path: std::path::PathBuf) -> std::path::PathBuf {
    util::normalize_repo_path(path)
}

fn normalize_repo_relative_path(
    repo_workdir: &std::path::Path,
    path: std::path::PathBuf,
) -> std::path::PathBuf {
    let path = if path.is_relative() {
        repo_workdir.join(path)
    } else {
        path
    };
    util::canonicalize_path(path)
}

fn cache_selected_deleted_gitlink(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    target: &gitcomet_core::domain::DiffTarget,
) {
    let gitcomet_core::domain::DiffTarget::WorkingTree { path, area, .. } = target else {
        return;
    };
    if !head_gitlink_lookup_is_worth_it(state, repo_id, *area, path) {
        return;
    }

    refresh_head_gitlink_path(repos, state, repo_id, path);
}

/// Whether classifying `path` against HEAD can still change what is rendered.
///
/// `refresh_head_gitlink_path` opens the repository and peels HEAD, so it is
/// real filesystem work on the store worker while the state write lock is held.
/// `diff_target_is_submodule` only consults the cache for `Deleted` entries, so
/// for any other kind the lookup is pure waste — and this runs on every
/// external git-state event, which arrive in bursts during a fetch or rebase.
///
/// An unknown kind still classifies: `reload_repo` blanks the status lane while
/// deliberately retaining the diff target, and the entry has to be in place
/// before the fresh status lands.
fn head_gitlink_lookup_is_worth_it(
    state: &AppState,
    repo_id: RepoId,
    area: gitcomet_core::domain::DiffArea,
    path: &std::path::Path,
) -> bool {
    let Some(repo) = state.repos.iter().find(|repo| repo.id == repo_id) else {
        return false;
    };
    match repo.status_entries_for_area(area) {
        Some(entries) => entries
            .iter()
            .find(|entry| entry.path == path)
            .is_some_and(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Deleted),
        None => true,
    }
}

fn refresh_head_gitlink_path(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    path: &std::path::Path,
) {
    let Some(is_gitlink) = repos
        .get(&repo_id)
        .and_then(|repo| repo.head_path_is_gitlink(path).ok())
    else {
        return;
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return;
    };
    if is_gitlink {
        repo.head_gitlink_paths.insert(path.to_path_buf());
    } else {
        repo.head_gitlink_paths.remove(path);
    }
}

fn refresh_selected_head_gitlink(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) {
    let selected = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.diff_state.diff_target.clone());
    if let Some(target) = selected {
        cache_selected_deleted_gitlink(repos, state, repo_id, &target);
    }
}

#[inline]
fn begin_local_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
}

/// The repo of a command that writes sequencer state or moves HEAD. Counted
/// from the effect that schedules it, and released by the matching
/// [`crate::msg::RepoCommandKind`] in `repo_command_finished`.
fn sequencer_effect_repo(effect: &Effect) -> Option<RepoId> {
    match effect {
        Effect::MergeRef { repo_id, .. }
        | Effect::SquashRef { repo_id, .. }
        | Effect::SquashCommits { repo_id, .. }
        | Effect::Reset { repo_id, .. }
        | Effect::Rebase { repo_id, .. }
        | Effect::RebaseContinue { repo_id, .. }
        | Effect::RebaseAbort { repo_id }
        | Effect::InteractiveRebase { repo_id, .. }
        | Effect::InteractiveCherryPick { repo_id, .. }
        | Effect::CherryPickCommit { repo_id, .. }
        | Effect::RevertCommit { repo_id, .. }
        // Its commit step can wait on a signer after the worktree changed.
        | Effect::ApplyFileChange {
            repo_id,
            commit: true,
            ..
        }
        | Effect::MergeAbort { repo_id } => Some(*repo_id),
        _ => None,
    }
}

fn track_sequencer_effects(state: &mut AppState, effects: &[Effect]) {
    for repo_id in effects.iter().filter_map(sequencer_effect_repo) {
        if let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
            repo_state.sequencer_actions_in_flight =
                repo_state.sequencer_actions_in_flight.saturating_add(1);
            repo_state.bump_ops_rev();
        }
    }
}

/// Continue and Abort act on sequencer state another command may still be
/// writing: a revert shows REVERT_HEAD while its commit step waits on a slow
/// signer, and an Abort then would reset under the commit. Only such commands
/// count — a merge tool or a submodule clone can run for minutes without
/// touching it.
fn sequencer_step_blocked(state: &mut AppState, repo_id: RepoId) -> bool {
    let busy = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_some_and(|repo| repo.sequencer_actions_in_flight > 0);
    if busy {
        util::push_notification(
            state,
            crate::model::AppNotificationKind::Warning,
            "Wait for the running Git operation to finish, then continue or abort.".to_string(),
        );
    }
    busy
}

fn begin_commit_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_add(1);
        repo_state.pending.force_push_lease = None;
        repo_state.bump_ops_rev();
    }
}

fn begin_head_changing_local_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.clear_head_dependent_cached_state();
        repo_state.bump_ops_rev();
    }
}

pub(crate) fn msg_requires_available_git(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::OpenRepo(_)
            | Msg::OpenRepoFromExternalDrop(_)
            | Msg::RestoreSession { .. }
            | Msg::ReloadRepo { .. }
            | Msg::RepoActivated { .. }
            | Msg::RepoExternallyChanged { .. }
            | Msg::SetHistoryScope { .. }
            | Msg::SetHistoryAuthorFilter { .. }
            | Msg::LoadMoreHistory { .. }
            | Msg::SelectCommit { .. }
            | Msg::CompareCommitRange { .. }
            | Msg::CompareWithOptions { .. }
            | Msg::CompareWithMarked { .. }
            | Msg::CompareWithWorkingTree { .. }
            | Msg::SelectDiff { .. }
            | Msg::SetTextOverride { .. }
            | Msg::SelectConflictDiff { .. }
            | Msg::SelectWorktreeUncommitted { .. }
            | Msg::LoadStashes { .. }
            | Msg::LoadConflictFile { .. }
            | Msg::LoadReflog { .. }
            | Msg::LoadRecentCommitMessages { .. }
            | Msg::PreviewTagPush { .. }
            | Msg::LoadHoverCommitMessage { .. }
            | Msg::ResolveCommitLookup { .. }
            | Msg::LoadFileHistory { .. }
            | Msg::LoadBlame { .. }
            | Msg::LoadWorktrees { .. }
            | Msg::LoadWorktreeDirty { .. }
            | Msg::LoadRefMetadata { .. }
            | Msg::LoadSubmodules { .. }
            | Msg::LoadSubmodule { .. }
            | Msg::LoadTags { .. }
            | Msg::LoadRemoteTags { .. }
            | Msg::RefreshBranches { .. }
            | Msg::LoadFileBrowser { .. }
            | Msg::OpenFileContent { .. }
            | Msg::OpenFileEditor { .. }
            | Msg::OpenFileAtCommitParent { .. }
            | Msg::OpenFileAtCommit { .. }
            | Msg::ShowFileChangesAtCommit { .. }
            | Msg::BrowseRepositoryAtCommit { .. }
            | Msg::RevealCommit { .. }
            | Msg::ResetBrowseToLive { .. }
            | Msg::ViewerNavBack { .. }
            | Msg::ViewerNavForward { .. }
            | Msg::GlobalNavBack { .. }
            | Msg::GlobalNavForward { .. }
            | Msg::StageHunk { .. }
            | Msg::UnstageHunk { .. }
            | Msg::ApplyWorktreePatch { .. }
            | Msg::CheckoutBranch { .. }
            | Msg::CheckoutRemoteBranch { .. }
            | Msg::CheckoutCommit { .. }
            | Msg::CherryPickCommit { .. }
            | Msg::RevertCommit { .. }
            | Msg::ApplyFileChange { .. }
            | Msg::CreateBranch { .. }
            | Msg::CreateBranchAndCheckout { .. }
            | Msg::RenameBranch { .. }
            | Msg::DeleteBranch { .. }
            | Msg::ForceDeleteBranch { .. }
            | Msg::DeleteBranches { .. }
            | Msg::CloneRepo { .. }
            | Msg::ExportPatch { .. }
            | Msg::ApplyPatch { .. }
            | Msg::AddWorktree { .. }
            | Msg::RemoveWorktree { .. }
            | Msg::ForceRemoveWorktree { .. }
            | Msg::AddSubmodule { .. }
            | Msg::UpdateSubmodules { .. }
            | Msg::ChangeSubmodulePointer { .. }
            | Msg::RemoveSubmodule { .. }
            | Msg::StagePath { .. }
            | Msg::StagePaths { .. }
            | Msg::UnstagePath { .. }
            | Msg::UnstagePaths { .. }
            | Msg::DiscardWorktreeChangesPath { .. }
            | Msg::DiscardWorktreeChangesPaths { .. }
            | Msg::SaveWorktreeFile { .. }
            | Msg::AppendGitignorePatterns { .. }
            | Msg::RunLargeFileCommand { .. }
            | Msg::AppendGitattributesRule { .. }
            | Msg::Commit { .. }
            | Msg::CommitAmend { .. }
            | Msg::SafePushAfterCommit { .. }
            | Msg::Fetch(crate::msg::FetchMsg::All { .. })
            | Msg::Fetch(crate::msg::FetchMsg::Refspecs { .. })
            | Msg::PruneMergedBranches { .. }
            | Msg::PruneLocalTags { .. }
            | Msg::StartRepoMaintenance { .. }
            | Msg::Pull { .. }
            | Msg::PullBranch { .. }
            | Msg::MergeRef { .. }
            | Msg::SquashRef { .. }
            | Msg::PushWithTags { .. }
            | Msg::Push { .. }
            | Msg::PushAfterCommit { .. }
            | Msg::ForcePush { .. }
            | Msg::ForcePushWithLease { .. }
            | Msg::PushSetUpstream { .. }
            | Msg::SetUpstreamBranch { .. }
            | Msg::UnsetUpstreamBranch { .. }
            | Msg::DeleteRemoteBranch { .. }
            | Msg::DeleteRemoteBranches { .. }
            | Msg::Reset { .. }
            | Msg::PrepareSquash { .. }
            | Msg::SquashCommits { .. }
            | Msg::Rebase { .. }
            | Msg::RebaseContinue { .. }
            | Msg::RebaseAbort { .. }
            | Msg::InteractiveRebase { .. }
            | Msg::InteractiveCherryPick { .. }
            | Msg::MergeAbort { .. }
            | Msg::CreateTag { .. }
            | Msg::DeleteTag { .. }
            | Msg::PushTag { .. }
            | Msg::DeleteRemoteTag { .. }
            | Msg::AddRemote { .. }
            | Msg::RemoveRemote { .. }
            | Msg::SetRemoteUrl { .. }
            | Msg::CheckoutConflictSide { .. }
            | Msg::AcceptConflictDeletion { .. }
            | Msg::CheckoutConflictBase { .. }
            | Msg::LaunchMergetool { .. }
            | Msg::Stash { .. }
            | Msg::ApplyStash { .. }
            | Msg::PopStash { .. }
            | Msg::DropStash { .. }
    )
}

#[cfg(test)]
pub(super) fn push_diagnostic(
    repo_state: &mut crate::model::RepoState,
    kind: crate::model::DiagnosticKind,
    message: String,
) {
    util::push_diagnostic(repo_state, kind, message)
}

#[cfg(test)]
pub(super) fn handle_session_persist_result(
    state: &mut crate::model::AppState,
    repo_id: Option<crate::model::RepoId>,
    action: &'static str,
    result: std::io::Result<()>,
) {
    util::handle_session_persist_result(state, repo_id, action, result)
}

/// Record an error for the UI to show: on its repo when there is one still
/// open, else as an app notification.
fn report_error(state: &mut AppState, repo_id: Option<RepoId>, message: String) {
    if message.trim().is_empty() {
        return;
    }
    match repo_id.and_then(|repo_id| state.repos.iter_mut().find(|r| r.id == repo_id)) {
        Some(repo_state) => {
            util::push_diagnostic(repo_state, crate::model::DiagnosticKind::Error, message)
        }
        None => util::push_notification(state, crate::model::AppNotificationKind::Error, message),
    }
}

pub(crate) fn fill_set_active_repo_inline(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    effects: &mut SetActiveRepoEffects,
) {
    // The store handles tab switches on this inline path instead of calling
    // `reduce`, so bracket the mutation with the same navigation reconciliation
    // and state finalizers the ordinary reducer wrapper applies.
    reconcile_active_nav_history(state, false);
    repo_management::fill_set_active_repo_inline(repos, state, repo_id, effects);
    effects::follow_history_selection(state, effects);
    finalize_reduced_state(state, Some(false));
}

pub(crate) fn fill_reorder_repo_tabs_inline(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
    effects: &mut ReorderRepoTabsEffects,
) {
    repo_management::fill_reorder_repo_tabs_inline(state, repo_id, insert_before, effects)
}

// The only non-benchmark consumers of `fill_select_diff_inline` live inside
// the reducer submodule (via the unconditional `pub(super)` definition in
// `diff_selection.rs`). This public re-export exists solely for the benchmark
// helper in `store/mod.rs` so that the inline reduce path can be measured.
#[cfg(feature = "benchmarks")]
pub(crate) fn fill_select_diff_inline(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    target: gitcomet_core::domain::DiffTarget,
    content_preview: bool,
    effects: &mut SelectDiffEffects,
) {
    let mode = if content_preview {
        diff_selection::ContentViewMode::Preview
    } else {
        diff_selection::ContentViewMode::Diff
    };
    diff_selection::fill_select_diff_inline(repos, state, repo_id, target, mode, effects)
}

#[inline]
pub(crate) fn fill_stage_path_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    effects: &mut SinglePathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::StagePath { repo_id, path });
}

#[inline]
pub(crate) fn fill_stage_paths_inline(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    effects: &mut BatchPathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::StagePaths { repo_id, paths });
}

#[inline]
pub(crate) fn fill_unstage_path_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    effects: &mut SinglePathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::UnstagePath { repo_id, path });
}

#[inline]
pub(crate) fn fill_unstage_paths_inline(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    effects: &mut BatchPathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::UnstagePaths { repo_id, paths });
}

#[inline]
pub(crate) fn set_conflict_region_choice_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) {
    conflict_interactions::set_region_choice_inline(state, repo_id, path, region_index, choice);
}

#[inline]
pub(crate) fn reset_conflict_resolutions_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
) {
    conflict_interactions::reset_resolutions_inline(state, repo_id, path);
}

pub(super) fn reduce(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: Msg,
) -> Vec<Effect> {
    // History shows the repository's own worktree; a linked worktree's file
    // belongs to a hosted session, never to History's selection.
    if let Msg::SelectDiff { target, .. } = &msg
        && target.worktree().is_some()
    {
        return Vec::new();
    }
    let reconcile = !matches!(
        msg,
        Msg::GlobalNavBack { .. } | Msg::GlobalNavForward { .. }
    );
    let push = is_view_navigation(&msg);
    let selection_request = match &msg {
        Msg::SelectCommit {
            repo_id,
            request_id,
            ..
        }
        | Msg::SelectCommitMulti {
            repo_id,
            request_id,
            ..
        }
        | Msg::SelectWorktreeUncommitted {
            repo_id,
            request_id,
            ..
        }
        | Msg::ClearCommitSelection {
            repo_id,
            request_id,
            ..
        }
        | Msg::IndexedHistory(crate::indexed_history::IndexedHistoryMsg::Select {
            repo_id,
            request_id,
            ..
        }) => request_id.map(|id| (*repo_id, id)),
        _ => None,
    };

    if reconcile {
        reconcile_active_nav_history(state, false);
    }

    let mut effects = reduce_inner(repos, id_alloc, state, msg);
    track_sequencer_effects(state, &effects);
    effects::follow_history_selection(state, &mut effects);

    finalize_reduced_state(state, reconcile.then_some(push));
    // Acknowledge processing, including no-ops and rejected stale projections.
    // The published selection is the authoritative outcome of this request.
    if let Some((repo_id, request_id)) = selection_request
        && let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
    {
        repo.history_state.selection_ack = Some(request_id);
    }

    effects
}

/// Apply invariants that must hold whenever a reducer mutation is published.
///
/// Control-message fast paths call this too, so adding a finalizer here keeps
/// them from exposing an intermediate state that the ordinary `reduce` wrapper
/// would have repaired before returning.
fn finalize_reduced_state(state: &mut AppState, nav_push: Option<bool>) {
    // Enforced here rather than at each of the places a worktree selection can
    // end; see the helper.
    effects::retire_orphaned_worktree_diffs(state);
    for repo in &mut state.repos {
        repo.prepare_history_squash_plan();
    }
    // A closed repository's leases go with it; releasing them later is a
    // no-op. Free when nothing is leased.
    if state
        .watch_leases
        .keys()
        .any(|repo_id| !state.repos.iter().any(|repo| repo.id == *repo_id))
    {
        let open: Vec<RepoId> = state.repos.iter().map(|repo| repo.id).collect();
        Arc::make_mut(&mut state.watch_leases).retain(|repo_id, _| open.contains(repo_id));
    }

    if state.worktree_watch_leases.keys().any(|(id, lifetime, _)| {
        !state
            .repos
            .iter()
            .any(|repo| repo.id == *id && repo.lifetime() == *lifetime)
    }) {
        let open: Vec<_> = state
            .repos
            .iter()
            .map(|repo| (repo.id, repo.lifetime()))
            .collect();
        Arc::make_mut(&mut state.worktree_watch_leases)
            .retain(|(id, lifetime, _), _| open.contains(&(*id, *lifetime)));
    }

    if let Some(push) = nav_push {
        reconcile_active_nav_history(state, push);
    }
}

/// Whether `msg` is a user-initiated navigation that should create a new global
/// back/forward step (as opposed to a background change folded into the current
/// step). `GlobalNav*` replays are handled separately and never reach here as a
/// "push".
fn is_view_navigation(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::SelectDiff { .. }
            | Msg::SelectConflictDiff { .. }
            | Msg::SelectCommit { .. }
            // Selecting a linked-worktree row is a destination like any other
            // history selection; it just is not a commit.
            | Msg::SelectWorktreeUncommitted { .. }
            | Msg::CompareCommitRange { .. }
            | Msg::CompareWithOptions { .. }
            | Msg::CompareWithMarked { .. }
            | Msg::CompareWithWorkingTree { .. }
            | Msg::OpenFileContent { .. }
            | Msg::OpenFileEditor { .. }
            // Leaving the editor is a destination of its own, so Back returns to
            // the editor rather than skipping past it to whatever preceded it.
            | Msg::ExitDiffEditMode { .. }
            | Msg::OpenFileAtCommit { .. }
            | Msg::ShowFileChangesAtCommit { .. }
            | Msg::BrowseRepositoryAtCommit { .. }
            // A reveal moves the main view when its reference resolves, not
            // when it is asked for.
            | Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved { .. })
            | Msg::ResetBrowseToLive { .. }
            | Msg::OpenInlineSubmoduleDiff { .. }
            | Msg::SelectInlineSubmoduleDiff { .. }
    )
}

/// Sync the active repo's global navigation history against the current
/// main-view snapshot. See [`crate::model::NavStack::reconcile`].
fn reconcile_active_nav_history(state: &mut AppState, push: bool) {
    let Some(repo_id) = state.active_repo else {
        return;
    };
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    // Hot path: most messages don't move the main view, so the snapshot still
    // matches the current entry and `reconcile` would no-op. Compare by borrow
    // first and bail before cloning a `MainViewSnapshot` (which owns a `PathBuf`)
    // — this runs twice per dispatched message.
    let cursor = repo.navigation.main_history.cursor;
    if let Some(current) = repo.navigation.main_history.entries.get(cursor)
        && repo.main_view_snapshot_matches(current)
    {
        return;
    }
    let cur = repo.main_view_snapshot();
    repo.navigation.main_history.reconcile(cur, push);
}

fn reduce_inner(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: Msg,
) -> Vec<Effect> {
    if msg_requires_available_git(&msg) && !state.git_runtime.is_available() {
        return Vec::new();
    }

    match msg {
        Msg::OpenDocumentRepository { path, activate } => {
            repo_management::open_document_repository(repos, id_alloc, state, path, activate)
        }
        Msg::RememberDocumentInRepository { repo_id, path } => {
            if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo.navigation
                    .view_history
                    .record(crate::model::ViewHistoryEntry {
                        source: gitcomet_core::domain::FileSource::WorkingDirectory,
                        path,
                        old_path: None,
                    });
            }
            vec![]
        }
        Msg::FilesystemRequest(request) => {
            if state.filesystem.pending.contains_key(&request.id) {
                return vec![];
            }
            state.filesystem.pending.insert(request.id, request.clone());
            vec![Effect::Filesystem(request)]
        }
        Msg::FilesystemProgress(progress) => {
            if state.filesystem.pending.contains_key(&progress.id) {
                state.filesystem.progress = Some(progress);
            }
            vec![]
        }
        Msg::AcknowledgeFilesystemResults(ids) => {
            state.filesystem.completed.retain(|r| !ids.contains(&r.id));
            vec![]
        }
        Msg::FilesystemJournalUpdated { undo, redo } => {
            state.filesystem.undo_available = undo;
            state.filesystem.redo_available = redo;
            vec![]
        }
        Msg::FilesystemFinished(result) => {
            state.filesystem.pending.remove(&result.id);
            if state
                .filesystem
                .progress
                .as_ref()
                .is_some_and(|p| p.id == result.id)
            {
                state.filesystem.progress = None;
            }
            state.filesystem.undo_available = result.undo_available;
            state.filesystem.redo_available = result.redo_available;
            let effects = filesystem::paths_changed(state, &result.changes);
            state.filesystem.completed.push_back(result);
            effects
        }
        Msg::FilesystemPathsChanged(changes) => filesystem::paths_changed(state, &changes),
        Msg::SelectExplorerPath {
            repo_id,
            path,
            visible,
            toggle,
            range,
            context_menu,
        } => {
            if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo.file_browser
                    .selection
                    .click(path, &visible, toggle, range, context_menu);
                repo.file_browser.bump_rev();
            }
            vec![]
        }
        Msg::FocusExplorerPath { repo_id, path } => {
            if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo.file_browser.selection.focused = Some(path);
                repo.file_browser.bump_rev();
            }
            Vec::new()
        }
        Msg::SelectAllExplorerPaths { repo_id, visible } => {
            if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo.file_browser.selection.select_all(&visible);
                repo.file_browser.bump_rev();
            }
            vec![]
        }
        Msg::SetExplorerVisibility {
            repo_id,
            hidden,
            ignored,
        } => {
            if let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo.file_browser.show_hidden = hidden;
                repo.file_browser.show_ignored = ignored;
                if !ignored {
                    repo.file_browser.pending_recursive_expansions.clear();
                }
                // Keyboard actions must not reach rows that just disappeared.
                if !hidden {
                    let revealed = &repo.file_browser.revealed_paths;
                    repo.file_browser.selection.retain(|path| {
                        !crate::explorer::is_hidden_path(path)
                            || revealed.iter().any(|shown| shown.starts_with(path))
                    });
                }
                repo.file_browser.stale = true;
                repo.file_browser.bump_rev();
                return vec![Effect::LoadFileBrowser {
                    repo_id,
                    source: repo.file_browser.source.clone(),
                }];
            }
            vec![]
        }
        Msg::OpenRepo(path) => repo_management::open_repo(repos, id_alloc, state, path),
        Msg::OpenRepoFromExternalDrop(path) => {
            repo_management::open_repo_from_external_drop(repos, id_alloc, state, path)
        }
        Msg::AcknowledgeRepoOpenFailures { through_revision } => {
            if state
                .repo_open_failures
                .values()
                .any(|revision| *revision <= through_revision)
            {
                Arc::make_mut(&mut state.repo_open_failures)
                    .retain(|_, revision| *revision > through_revision);
            }
            Vec::new()
        }
        Msg::RestoreSession {
            open_repos,
            active_repo,
        } => repo_management::restore_session(repos, id_alloc, state, open_repos, active_repo),
        Msg::AcquireWatchLease { repo_id, lifetime } => {
            repo_watch::acquire_lease(state, repo_id, lifetime)
        }
        Msg::ReleaseWatchLease { repo_id, lifetime } => {
            repo_watch::release_lease(state, repo_id, lifetime)
        }
        Msg::CloseRepo { repo_id } => repo_management::close_repo(repos, state, repo_id),
        Msg::MoveRepoOut { repo_id } => repo_management::move_repo_out(repos, state, repo_id),
        Msg::CloseRepos {
            repo_ids,
            activate_after,
        } => repo_management::close_repos(repos, state, repo_ids, activate_after),
        Msg::ReportError { repo_id, message } => {
            report_error(state, repo_id, message);
            Vec::new()
        }
        Msg::DismissRepoError { repo_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.feedback.last_error = None;
            }
            Vec::new()
        }
        Msg::CancelGitOperation {
            repo_id,
            operation_id,
        } => git_operations::cancel_git_operation(state, repo_id, operation_id),
        Msg::SubmitAuthPrompt { username, secret } => {
            auth::submit_auth_prompt(repos, id_alloc, state, username, secret)
        }
        Msg::CancelAuthPrompt => auth::cancel_auth_prompt(state),
        Msg::SetGitRuntimeState(runtime) => settings::set_git_runtime_state(state, runtime),
        Msg::SetCommitSignatureTargets {
            repo_id,
            epoch,
            commit_ids,
        } => util::set_commit_signature_targets(state, repo_id, epoch, commit_ids),
        Msg::SetLargeFileToolsState(tools) => settings::set_large_file_tools_state(state, tools),
        Msg::SetSigningToolsState(tools) => settings::set_signing_tools_state(state, tools),
        Msg::SetRemoteUrlPolicy(policy) => settings::set_remote_url_policy(state, policy),
        Msg::SetGitLogSettings {
            show_history_tags,
            tag_fetch_mode,
            verify_commit_signatures,
        } => settings::set_git_log_settings(
            state,
            show_history_tags,
            tag_fetch_mode,
            verify_commit_signatures,
        ),
        Msg::SetRemoteSettings(settings) => settings::set_remote_settings(state, settings),
        Msg::SetLargeFileSettings(settings) => settings::set_large_file_settings(state, settings),
        Msg::SetMaintenanceSettings(settings) => maintenance::set_settings(state, settings),
        Msg::SetFileBrowserSettings(settings) => {
            effects::set_file_browser_settings(state, settings)
        }
        Msg::SetDefaultTagType(tag_type) => settings::set_default_tag_type(state, tag_type),
        Msg::SetActiveRepo { repo_id } => repo_management::set_active_repo(repos, state, repo_id),
        Msg::ReorderRepoTabs {
            repo_id,
            insert_before,
        } => repo_management::reorder_repo_tabs(state, repo_id, insert_before),
        Msg::Internal(crate::msg::InternalMsg::GitOperationStarted {
            repo_id,
            operation_id,
            label,
            context,
            time,
            progress_lane,
        }) => git_operations::git_operation_started(
            state,
            repo_id,
            operation_id,
            label,
            context,
            time,
            progress_lane,
        ),
        Msg::Internal(crate::msg::InternalMsg::GitOperationEvent {
            repo_id,
            operation_id,
            event,
        }) => git_operations::git_operation_event(state, repo_id, operation_id, event),
        Msg::Internal(crate::msg::InternalMsg::GitOperationFinished {
            repo_id,
            operation_id,
            outer_outcome,
            duration,
            message,
        }) => git_operations::git_operation_finished(
            repos,
            id_alloc,
            state,
            repo_id,
            operation_id,
            outer_outcome,
            duration,
            message,
        ),
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id,
            action,
            error,
        }) => {
            util::handle_session_persist_result(
                state,
                repo_id,
                action,
                Err(std::io::Error::other(error)),
            );
            Vec::new()
        }
        Msg::ReloadRepo { repo_id } => external_and_history::reload_repo(repos, state, repo_id),
        Msg::RepoActivated { .. } => Vec::new(),
        Msg::WatchWorktree {
            repo_id,
            lifetime,
            path,
            watch,
        } => repo_watch::watch_worktree(state, repo_id, lifetime, path, watch),
        Msg::WorktreeExternallyChanged {
            repo_id,
            lifetime,
            path,
            change,
        } => repo_watch::worktree_changed(state, repo_id, lifetime, path, change),
        Msg::RepoExternallyChanged { repo_id, change } => {
            external_and_history::repo_externally_changed(repos, state, repo_id, change)
        }
        Msg::RepoWatchDegraded { repo_id: _, reason } => repo_watch::watch_degraded(state, reason),
        Msg::SetHistoryScope { repo_id, scope } => {
            external_and_history::set_history_scope(state, repo_id, scope)
        }
        Msg::SetHistoryAuthorFilter { repo_id, author } => {
            external_and_history::set_history_author_filter(state, repo_id, author)
        }
        Msg::LoadMoreHistory { repo_id } => external_and_history::load_more_history(state, repo_id),
        Msg::SelectCommit {
            repo_id, commit_id, ..
        } => effects::select_commit(state, repo_id, commit_id),
        Msg::SelectCommitMulti {
            repo_id,
            commit_id,
            mode,
            clicked_index,
            visible_order,
            ..
        } => effects::select_commit_multi(
            state,
            repo_id,
            commit_id,
            mode,
            clicked_index,
            visible_order,
        ),
        Msg::ClearCommitSelection { repo_id, .. } => {
            effects::clear_commit_selection(state, repo_id)
        }
        Msg::CompareCommitRange {
            repo_id,
            from,
            to,
            from_label,
            to_label,
        } => effects::compare_range(
            state,
            repo_id,
            from,
            Some(to),
            from_label,
            to_label,
            effects::ComparisonSource::Explicit,
        ),
        Msg::CompareWithWorkingTree {
            repo_id,
            from,
            from_label,
        } => effects::compare_range(
            state,
            repo_id,
            from,
            None,
            from_label,
            "Working tree".to_string(),
            effects::ComparisonSource::Explicit,
        ),
        Msg::CompareWithOptions {
            repo_id,
            from,
            to,
            options,
            from_label,
            to_label,
        } => effects::compare_range_with_options(
            state,
            repo_id,
            from,
            to,
            from_label,
            to_label,
            options,
            effects::ComparisonSource::Explicit,
        ),
        Msg::ClearComparison { repo_id } => effects::clear_comparison(state, repo_id),
        Msg::MarkForComparison {
            repo_id,
            commit_id,
            label,
        } => effects::mark_for_comparison(state, repo_id, commit_id, label),
        Msg::CompareWithMarked {
            repo_id,
            commit_id,
            label,
        } => effects::compare_with_marked(state, repo_id, commit_id, label),
        Msg::ClearComparisonMark { repo_id } => effects::clear_comparison_mark(state, repo_id),
        Msg::SelectDiff { repo_id, target } => {
            diff_selection::select_diff(repos, state, repo_id, target)
        }
        Msg::SetTextOverride {
            repo_id,
            path,
            value,
        } => diff_selection::set_text_override(state, repo_id, path, value),
        Msg::OpenInlineSubmoduleDiff {
            repo_id,
            origin,
            submodule_repo_path,
            parent_submodule_path,
            entries,
            selected_ix,
        } => diff_selection::open_inline_submodule_diff(
            state,
            repo_id,
            origin,
            submodule_repo_path,
            parent_submodule_path,
            entries,
            selected_ix,
        ),
        Msg::SelectInlineSubmoduleDiff {
            repo_id,
            selected_ix,
        } => diff_selection::select_inline_submodule_diff(state, repo_id, selected_ix),
        Msg::CloseInlineSubmoduleDiff { repo_id } => {
            diff_selection::close_inline_submodule_diff(state, repo_id)
        }
        Msg::SelectConflictDiff { repo_id, path } => {
            diff_selection::select_conflict_diff(state, repo_id, path)
        }
        Msg::ClearDiffSelection { repo_id } => diff_selection::clear_diff_selection(state, repo_id),
        Msg::EnsureSidebarData { repo_id, request } => {
            effects::ensure_sidebar_data(state, repo_id, request)
        }
        Msg::LoadStashes { repo_id } => effects::load_stashes(state, repo_id),
        Msg::LoadConflictFile {
            repo_id,
            path,
            mode,
        } => effects::load_conflict_file(state, repo_id, path, mode),
        Msg::LoadReflog { repo_id } => effects::load_reflog(state, repo_id),
        Msg::LoadHoverCommitMessage { repo_id, commit_id } => {
            effects::load_hover_commit_message(state, repo_id, commit_id)
        }
        Msg::LoadRecentCommitMessages { repo_id, limit } => {
            effects::load_recent_commit_messages(state, repo_id, limit)
        }
        Msg::LoadFileHistory {
            repo_id,
            path,
            limit,
        } => effects::load_file_history(state, repo_id, path, limit),
        Msg::LoadBlame {
            repo_id,
            path,
            source,
        } => effects::load_blame(state, repo_id, path, source),
        Msg::LoadWorktrees { repo_id } => effects::load_worktrees(state, repo_id),
        Msg::LoadWorktreeDirty { repo_id } => effects::load_worktree_dirty(state, repo_id),
        Msg::SelectWorktreeUncommitted { repo_id, path, .. } => {
            effects::select_worktree_uncommitted(state, repo_id, path)
        }
        Msg::LoadRefMetadata { repo_id } => effects::load_ref_metadata(state, repo_id),
        Msg::LoadSubmodules { repo_id } => effects::load_submodules(state, repo_id),
        Msg::LoadTags { repo_id } => effects::load_tags(state, repo_id),
        Msg::LoadRemoteTags { repo_id } => effects::load_remote_tags(state, repo_id),
        Msg::RefreshBranches { repo_id } => effects::refresh_branches(state, repo_id),
        Msg::LoadFileBrowser { repo_id, source } => {
            effects::load_file_browser(state, repo_id, source)
        }
        Msg::ToggleFileBrowserDir { repo_id, path } => {
            effects::toggle_file_browser_dir(state, repo_id, path)
        }
        Msg::SetFileBrowserDirExpandedRecursive {
            repo_id,
            path,
            expanded,
        } => effects::set_file_browser_dir_expanded_recursive(state, repo_id, path, expanded),
        Msg::SetFileBrowserSearch { repo_id, query } => {
            effects::set_file_browser_search(state, repo_id, query)
        }
        Msg::RevealFileBrowserPath { repo_id, path } => {
            effects::reveal_file_browser_path(state, repo_id, path)
        }
        Msg::SetFileBrowserSource { repo_id, source } => {
            effects::set_file_browser_source(state, repo_id, source)
        }
        Msg::OpenFileContent {
            repo_id,
            source,
            path,
        } => diff_selection::open_file_content(repos, state, repo_id, source, path),
        Msg::OpenFileEditor { repo_id, path } => {
            diff_selection::open_file_editor(repos, state, repo_id, path)
        }
        Msg::ExitDiffEditMode { repo_id } => {
            diff_selection::exit_diff_edit_mode(repos, state, repo_id)
        }
        Msg::OpenFileAtCommitParent {
            repo_id,
            commit_id,
            path,
        } => vec![Effect::OpenFileAtCommitParent {
            repo_id,
            commit_id,
            path,
        }],
        Msg::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
        } => vec![Effect::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
            content_preview: true,
        }],
        Msg::ShowFileChangesAtCommit {
            repo_id,
            commit_id,
            path,
        } => vec![Effect::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
            content_preview: false,
        }],
        Msg::BrowseRepositoryAtCommit { repo_id, commit_id } => {
            effects::browse_repository_at_commit(state, repo_id, commit_id)
        }
        Msg::RevealCommit { repo_id, reference } => {
            effects::reveal_commit(state, repo_id, reference)
        }
        Msg::FinishCommitReveal { repo_id } => effects::finish_commit_reveal(state, repo_id),
        Msg::ResolveCommitLookup {
            repo_id,
            reference,
            purpose,
        } => effects::resolve_commit_lookup(state, repo_id, reference, purpose),
        Msg::ResetBrowseToLive { repo_id } => effects::reset_browse_to_live(state, repo_id),
        Msg::ViewerNavBack { repo_id } => {
            diff_selection::viewer_nav(repos, state, repo_id, crate::model::ViewNavDir::Back)
        }
        Msg::ViewerNavForward { repo_id } => {
            diff_selection::viewer_nav(repos, state, repo_id, crate::model::ViewNavDir::Forward)
        }
        Msg::GlobalNavBack { repo_id } => {
            diff_selection::global_nav(repos, state, repo_id, crate::model::ViewNavDir::Back)
        }
        Msg::GlobalNavForward { repo_id } => {
            diff_selection::global_nav(repos, state, repo_id, crate::model::ViewNavDir::Forward)
        }
        Msg::SetSidebarMode { mode } => effects::set_sidebar_mode(state, mode),
        Msg::StageHunk { repo_id, patch } => {
            begin_local_action(state, repo_id);
            diff_selection::stage_hunk(repo_id, patch)
        }
        Msg::UnstageHunk { repo_id, patch } => {
            begin_local_action(state, repo_id);
            diff_selection::unstage_hunk(repo_id, patch)
        }
        Msg::ApplyWorktreePatch {
            repo_id,
            patch,
            reverse,
        } => {
            begin_local_action(state, repo_id);
            diff_selection::apply_worktree_patch(repo_id, patch, reverse)
        }
        Msg::CheckoutBranch { repo_id, name } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_branch(repo_id, name)
        }
        Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
            mode,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_remote_branch(
                repo_id,
                remote,
                branch,
                local_branch,
                mode,
            )
        }
        Msg::CheckoutCommit { repo_id, commit_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(Some(commit_id.clone()));
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_commit(repo_id, commit_id)
        }
        Msg::CherryPickCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
        } => {
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::cherry_pick_commit(repo_id, commit_id, commit, mainline, summary)
        }
        Msg::RevertCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
        } => {
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::revert_commit(repo_id, commit_id, commit, mainline, summary)
        }
        Msg::ApplyFileChange {
            repo_id,
            target,
            commit,
            commit_retry,
        } => {
            if commit {
                begin_head_changing_local_action(state, repo_id);
            } else {
                begin_local_action(state, repo_id);
            }
            actions_emit_effects::apply_file_change(repo_id, target, commit, commit_retry)
        }
        Msg::CreateBranch {
            repo_id,
            name,
            target,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_branch(repo_id, name, target)
        }
        Msg::CreateBranchAndCheckout {
            repo_id,
            name,
            target,
            force,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::create_branch_and_checkout(repo_id, name, target, force)
        }
        Msg::ResolveBranchExistsPrompt { prompt, choice } => {
            branch_exists_prompt::resolve(state, prompt, choice)
        }
        Msg::ShowBranchExistsPrompt { prompt } => branch_exists_prompt::show(state, prompt),
        Msg::RenameBranch {
            repo_id,
            old_name,
            new_name,
            force,
        } => {
            if force {
                // Replacing the checked-out branch moves HEAD's commit.
                if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                    repo_state.set_detached_head_commit(None);
                }
                begin_head_changing_local_action(state, repo_id);
            } else {
                begin_local_action(state, repo_id);
            }
            actions_emit_effects::rename_branch(repo_id, old_name, new_name, force)
        }
        Msg::DeleteBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::delete_branch(repo_id, name)
        }
        Msg::ForceDeleteBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::force_delete_branch(repo_id, name)
        }
        Msg::DeleteBranches {
            repo_id,
            names,
            force,
        } => {
            if names.is_empty() {
                return Vec::new();
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::delete_branches(repo_id, names, force)
        }
        Msg::CloneRepo { url, dest } => repo_management::clone_repo(state, url, dest),
        Msg::AbortCloneRepo { dest } => repo_management::abort_clone_repo(state, dest),
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress { dest, line }) => {
            repo_management::clone_repo_progress(state, dest, line)
        }
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished { url, dest, result }) => {
            auth::clone_repo_finished(state, url, dest, result)
        }
        Msg::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::export_patch(repo_id, commit_id, dest)
        }
        Msg::ApplyPatch { repo_id, patch } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::apply_patch(repo_id, patch)
        }
        Msg::AddWorktree {
            repo_id,
            path,
            reference,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
            }
            actions_emit_effects::add_worktree(repo_id, path, reference)
        }
        Msg::RemoveWorktree { repo_id, path } => {
            let normalized_path = if let Some(repo_state) =
                state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
                normalize_repo_relative_path(&repo_state.spec.workdir, path)
            } else {
                path
            };
            actions_emit_effects::remove_worktree(repo_id, normalized_path)
        }
        Msg::ForceRemoveWorktree { repo_id, path } => {
            let normalized_path = if let Some(repo_state) =
                state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
                normalize_repo_relative_path(&repo_state.spec.workdir, path)
            } else {
                path
            };
            actions_emit_effects::force_remove_worktree(repo_id, normalized_path)
        }
        Msg::AddSubmodule {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
        } => submodule_trust::add_submodule(state, repo_id, url, path, branch, name, force),
        Msg::AddSubmoduleTrusted {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        } => submodule_trust::add_submodule_approved(
            state,
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        ),
        Msg::UpdateSubmodules { repo_id } => submodule_trust::update_submodules(state, repo_id),
        Msg::UpdateSubmodulesTrusted {
            repo_id,
            approved_sources,
        } => submodule_trust::update_submodules_approved(state, repo_id, approved_sources),
        Msg::LoadSubmodule { repo_id, path } => {
            submodule_trust::load_submodule(state, repo_id, path)
        }
        Msg::LoadSubmoduleTrusted {
            repo_id,
            path,
            approved_sources,
        } => submodule_trust::load_submodule_approved(state, repo_id, path, approved_sources),
        Msg::ConfirmSubmoduleTrustPrompt => submodule_trust::confirm_prompt(state),
        Msg::CancelSubmoduleTrustPrompt => submodule_trust::cancel_prompt(state),
        Msg::ChangeSubmodulePointer {
            repo_id,
            path,
            reference,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::change_submodule_pointer(repo_id, path, reference)
        }
        Msg::RemoveSubmodule { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::remove_submodule(repo_id, path)
        }
        Msg::StagePath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stage_path(repo_id, path)
        }
        Msg::StagePaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stage_paths(repo_id, paths)
        }
        Msg::UnstagePath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unstage_path(repo_id, path)
        }
        Msg::UnstagePaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unstage_paths(repo_id, paths)
        }
        Msg::DiscardWorktreeChangesPath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::discard_worktree_changes_path(repo_id, path)
        }
        Msg::DiscardWorktreeChangesPaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::discard_worktree_changes_paths(repo_id, paths)
        }
        Msg::SaveWorktreeFile {
            repo_id,
            path,
            contents,
            expected_contents,
            stage,
            completion,
        } => {
            let expected_contents = expected_contents.or_else(|| {
                state.repos.iter().find(|r| r.id == repo_id).and_then(|r| {
                    match &r.conflict_state.conflict_file {
                        crate::model::Loadable::Ready(Some(file))
                            if file.path.as_path() == path =>
                        {
                            file.current_bytes.clone().or_else(|| {
                                file.current
                                    .as_ref()
                                    .map(|text| Arc::<[u8]>::from(text.as_bytes()))
                            })
                        }
                        _ => None,
                    }
                })
            });
            begin_local_action(state, repo_id);
            actions_emit_effects::save_worktree_file(
                repo_id,
                path,
                contents,
                expected_contents,
                stage,
                completion,
            )
        }
        Msg::AppendGitignorePatterns { repo_id, patterns } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::append_gitignore_patterns(repo_id, patterns)
        }
        Msg::RunLargeFileCommand { repo_id, command } => {
            actions_emit_effects::run_large_file_command(repos, state, repo_id, command)
        }
        Msg::LoadAnnexWhereis { repo_id, keys } => {
            let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
                return Vec::new();
            };
            // Retain shared sides when switching revisions, without accumulating
            // a repository-wide cache. Replies for discarded keys are ignored.
            let before = repo.annex_whereis.len();
            repo.annex_whereis.retain(|key, _| keys.contains(key));
            if repo.annex_whereis.len() != before {
                repo.annex_whereis_rev = repo.annex_whereis_rev.wrapping_add(1);
            }
            let mut effects = Vec::new();
            for key in keys {
                // This is an explicit lookup: allow reloading locations changed
                // by other tools, while coalescing duplicate/in-flight keys.
                if matches!(repo.annex_whereis_for(&key), Some(Loadable::Loading)) {
                    continue;
                }
                repo.set_annex_whereis(key.clone(), Loadable::Loading);
                effects.push(Effect::LoadAnnexWhereis { repo_id, key });
            }
            effects
        }
        Msg::Internal(crate::msg::InternalMsg::AnnexWhereisLoaded {
            repo_id,
            key,
            result,
        }) => {
            if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id)
                && repo.annex_whereis.contains_key(&key)
            {
                let loaded = match result {
                    Ok(whereis) => Loadable::Ready(Arc::new(whereis)),
                    Err(error) => Loadable::Error(error.to_string()),
                };
                repo.set_annex_whereis(key, loaded);
            }
            Vec::new()
        }
        Msg::LoadAnnexUnused { repo_id } => state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo_id)
            .and_then(effects::request_annex_unused_effect)
            .into_iter()
            .collect(),
        Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded { repo_id, result }) => {
            effects::annex_unused_loaded(state, repo_id, result)
        }
        Msg::LoadLfsLocks { repo_id } => state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo_id)
            .and_then(effects::request_lfs_locks_effect)
            .into_iter()
            .collect(),
        Msg::Internal(crate::msg::InternalMsg::LfsLocksLoaded { repo_id, result }) => {
            effects::lfs_locks_loaded(state, repo_id, result)
        }
        Msg::AppendGitattributesRule { repo_id, rule } => {
            begin_local_action(state, repo_id);
            vec![Effect::AppendGitattributesRule { repo_id, rule }]
        }
        Msg::Commit {
            repo_id,
            message,
            push_after_commit,
        } => auth::commit(state, repo_id, message, push_after_commit),
        Msg::CommitAmend {
            repo_id,
            message,
            push_after_commit,
        } => auth::commit_amend(state, repo_id, message, push_after_commit),
        Msg::SafePushAfterCommit { repo_id, context } => {
            match annex_takeover(repos, state, repo_id, false) {
                Some(effects) => effects,
                None => actions_emit_effects::safe_push_after_commit(repo_id, context),
            }
        }
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id }) => {
            actions_emit_effects::fetch_all(repos, state, repo_id)
        }
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id,
            remote,
            refspecs,
        }) => actions_emit_effects::fetch_refspecs(repos, state, repo_id, remote, refspecs),
        Msg::PruneMergedBranches { repo_id } => {
            actions_emit_effects::prune_merged_branches(repos, state, repo_id)
        }
        Msg::PruneLocalTags { repo_id } => {
            actions_emit_effects::prune_local_tags(repos, state, repo_id)
        }
        Msg::StartRepoMaintenance { repo_id } => maintenance::start(state, repo_id),
        Msg::SnoozeRepoMaintenance { repo_id } => maintenance::snooze(state, repo_id),
        Msg::Internal(crate::msg::InternalMsg::RepoMaintenanceChecked { repo_id, needed }) => {
            maintenance::checked(state, repo_id, needed)
        }
        Msg::Pull { repo_id, mode } => match annex_takeover(repos, state, repo_id, true) {
            Some(effects) => effects,
            None => actions_emit_effects::pull(repos, state, repo_id, mode),
        },
        Msg::PullBranch {
            repo_id,
            remote,
            branch,
        } => match annex_adjusted_refusal(state, repo_id, "Pull from another branch") {
            Some(effects) => effects,
            None => actions_emit_effects::pull_branch(repos, state, repo_id, remote, branch),
        },
        Msg::MergeRef { repo_id, reference } => {
            if let Some(effects) = annex_adjusted_refusal(state, repo_id, "Merge") {
                return effects;
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::merge_ref(repo_id, reference)
        }
        Msg::SquashRef { repo_id, reference } => {
            if let Some(effects) = annex_adjusted_refusal(state, repo_id, "Squash merge") {
                return effects;
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::squash_ref(repo_id, reference)
        }
        Msg::PushWithTags { repo_id, request } => {
            match annex_adjusted_refusal(state, repo_id, "Push with tags") {
                Some(effects) => effects,
                None => actions_emit_effects::push_with_tags(repos, state, repo_id, request),
            }
        }
        Msg::PreviewTagPush {
            repo_id,
            request,
            cancellation,
        } => loads::preview_tag_push(state, repo_id, request, cancellation),
        Msg::Internal(crate::msg::InternalMsg::TagPushPreviewLoaded {
            repo_id,
            mode,
            generation,
            result,
        }) => loads::tag_push_preview_loaded(state, repo_id, mode, generation, result),
        Msg::Push { repo_id } => match annex_takeover(repos, state, repo_id, false) {
            Some(effects) => effects,
            None => actions_emit_effects::push(repos, state, repo_id),
        },
        Msg::PushAfterCommit {
            repo_id,
            target,
            set_upstream,
        } => match annex_takeover(repos, state, repo_id, false) {
            Some(effects) => effects,
            None => {
                actions_emit_effects::push_after_commit(repos, state, repo_id, target, set_upstream)
            }
        },
        Msg::ForcePush { repo_id } => match annex_adjusted_refusal(state, repo_id, "Force push") {
            Some(effects) => effects,
            None => actions_emit_effects::force_push(repos, state, repo_id),
        },
        Msg::ForcePushWithLease { repo_id, lease } => {
            match annex_adjusted_refusal(state, repo_id, "Force push") {
                Some(effects) => effects,
                None => actions_emit_effects::force_push_with_lease(repos, state, repo_id, lease),
            }
        }
        // An adjusted branch has no upstream; `git annex push` needs none.
        Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        } => match annex_takeover(repos, state, repo_id, false) {
            Some(effects) => effects,
            None => actions_emit_effects::push_set_upstream(repos, state, repo_id, remote, branch),
        },
        Msg::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::set_upstream_branch(repo_id, branch, upstream)
        }
        Msg::UnsetUpstreamBranch { repo_id, branch } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unset_upstream_branch(repo_id, branch)
        }
        Msg::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::delete_remote_branch(repos, state, repo_id, remote, branch),
        Msg::DeleteRemoteBranches {
            repo_id,
            remote,
            branches,
        } => {
            if branches.is_empty() {
                return Vec::new();
            }
            actions_emit_effects::delete_remote_branches(repos, state, repo_id, remote, branches)
        }
        Msg::Reset {
            repo_id,
            target,
            mode,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::reset(repo_id, target, mode)
        }
        Msg::PrepareSquash { repo_id } => effects::prepare_squash(state, repo_id),
        Msg::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        } => actions_emit_effects::squash_commits(
            state,
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        ),
        Msg::Rebase { repo_id, onto } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase(repo_id, onto)
        }
        Msg::RebaseContinue { repo_id } => {
            if sequencer_step_blocked(state, repo_id) {
                return Vec::new();
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase_continue(repo_id)
        }
        Msg::RebaseAbort { repo_id } => {
            if sequencer_step_blocked(state, repo_id) {
                return Vec::new();
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase_abort(repo_id)
        }
        Msg::LoadInteractiveRebaseSetup { repo_id, base } => {
            actions_emit_effects::load_interactive_rebase_setup(state, repo_id, base)
        }
        Msg::OpenInteractiveCherryPickSetup {
            repo_id,
            entries,
            source_colors,
        } => actions_emit_effects::open_interactive_cherry_pick_setup(
            state,
            repo_id,
            entries,
            source_colors,
        ),
        Msg::InteractiveRebase {
            repo_id,
            base,
            entries,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::interactive_rebase(repo_id, base, entries)
        }
        Msg::InteractiveCherryPick {
            repo_id,
            entries,
            commit,
        } => {
            // A multi-pick can land some commits and then fail (a hook or
            // signer on a later step), so HEAD-dependent caches must be
            // invalidated up front like the single-pick path — the error
            // completion path does not clear them.
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::interactive_cherry_pick(repo_id, entries, commit)
        }
        Msg::CancelInteractiveRebaseSetup { repo_id } => {
            actions_emit_effects::cancel_interactive_rebase_setup(state, repo_id)
        }
        Msg::CancelInteractiveCherryPickSetup { repo_id } => {
            actions_emit_effects::cancel_interactive_cherry_pick_setup(state, repo_id)
        }
        Msg::MergeAbort { repo_id } if sequencer_step_blocked(state, repo_id) => Vec::new(),
        Msg::MergeAbort { repo_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::merge_abort(repo_id)
        }
        Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_tag(repo_id, name, target, message, annotated)
        }
        Msg::DeleteTag { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::delete_tag(repo_id, name)
        }
        Msg::PushTag {
            repo_id,
            remote,
            name,
        } => actions_emit_effects::push_tag(repos, state, repo_id, remote, name),
        Msg::DeleteRemoteTag {
            repo_id,
            remote,
            name,
        } => actions_emit_effects::delete_remote_tag(repos, state, repo_id, remote, name),
        Msg::AddRemote { repo_id, name, url } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::add_remote(repo_id, name, url, state.remote_url_policy)
        }
        Msg::RemoveRemote { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::remove_remote(repo_id, name)
        }
        Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::set_remote_url(repo_id, name, url, kind, state.remote_url_policy)
        }
        Msg::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_conflict_side(repo_id, path, side)
        }
        Msg::AcceptConflictDeletion { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::accept_conflict_deletion(repo_id, path)
        }
        Msg::CheckoutConflictBase { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_conflict_base(repo_id, path)
        }
        Msg::LaunchMergetool { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::launch_mergetool(repo_id, path)
        }
        Msg::RecordConflictAutosolveTelemetry {
            repo_id,
            path,
            mode,
            total_conflicts_before,
            total_conflicts_after,
            unresolved_before,
            unresolved_after,
            stats,
        } => conflict_interactions::record_autosolve_telemetry(
            state,
            repo_id,
            path,
            mode,
            total_conflicts_before,
            total_conflicts_after,
            unresolved_before,
            unresolved_after,
            stats,
        ),
        Msg::ConflictSetHideResolved {
            repo_id,
            path,
            hide_resolved,
        } => conflict_interactions::set_hide_resolved(state, repo_id, path, hide_resolved),
        Msg::ConflictApplyBulkChoice {
            repo_id,
            path,
            choice,
            scope,
        } => conflict_interactions::apply_bulk_choice(state, repo_id, path, choice, scope),
        Msg::ConflictSetRegionChoice {
            repo_id,
            path,
            region_index,
            choice,
        } => conflict_interactions::set_region_choice(state, repo_id, path, region_index, choice),
        Msg::ConflictToggleRegionSource {
            repo_id,
            path,
            region_index,
            source,
        } => {
            conflict_interactions::toggle_region_source(state, repo_id, path, region_index, source)
        }
        Msg::ConflictReplaceRegionSelection {
            repo_id,
            path,
            region_index,
            selection,
        } => conflict_interactions::replace_region_selection(
            state,
            repo_id,
            path,
            region_index,
            selection,
        ),
        Msg::ConflictTogglePlanBlockSource {
            repo_id,
            path,
            block_id,
            source,
        } => {
            conflict_interactions::toggle_plan_block_source(state, repo_id, path, block_id, source)
        }
        Msg::ConflictReplacePlanBlockSelection {
            repo_id,
            path,
            block_id,
            selection,
        } => conflict_interactions::replace_plan_block_selection(
            state, repo_id, path, block_id, selection,
        ),
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path,
            updates,
        } => conflict_interactions::sync_region_resolutions(state, repo_id, path, updates),
        Msg::ConflictApplyAutosolve {
            repo_id,
            path,
            mode,
            whitespace_normalize,
        } => {
            conflict_interactions::apply_autosolve(state, repo_id, path, mode, whitespace_normalize)
        }
        Msg::ConflictResetResolutions { repo_id, path } => {
            conflict_interactions::reset_resolutions(state, repo_id, path)
        }
        Msg::ConflictSplitRegion {
            repo_id,
            path,
            region_index,
            boundaries,
            expected_conflict_rev,
        } => {
            let effects = conflict_interactions::split_region(
                state,
                repo_id,
                path,
                region_index,
                boundaries,
                expected_conflict_rev,
            );
            if !effects.is_empty() {
                begin_local_action(state, repo_id);
            }
            effects
        }
        Msg::ConflictAddManualAlignment {
            repo_id,
            path,
            alignment,
            expected_conflict_rev,
        } => conflict_interactions::add_manual_alignment(
            state,
            repo_id,
            path,
            alignment,
            expected_conflict_rev,
        ),
        Msg::ConflictClearManualAlignments {
            repo_id,
            path,
            expected_conflict_rev,
        } => conflict_interactions::clear_manual_alignments(
            state,
            repo_id,
            path,
            expected_conflict_rev,
        ),
        Msg::ConflictJoinRegions {
            repo_id,
            path,
            region_index,
            expected_conflict_rev,
        } => {
            let effects = conflict_interactions::join_regions(
                state,
                repo_id,
                path,
                region_index,
                expected_conflict_rev,
            );
            if !effects.is_empty() {
                begin_local_action(state, repo_id);
            }
            effects
        }
        Msg::Stash {
            repo_id,
            message,
            include_untracked,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stash(repo_id, message, include_untracked)
        }
        Msg::ApplyStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::apply_stash(repo_id, index)
        }
        Msg::PopStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::pop_stash(repo_id, index)
        }
        Msg::DropStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::drop_stash(repo_id, index)
        }
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec,
            repo,
        }) => repo_management::repo_opened_ok(repos, state, repo_id, spec, repo),
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch,
            message,
        }) => loads::repo_load_finished(repos, id_alloc, state, repo_id, load_epoch, message),
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id,
            spec,
            error,
        }) => repo_management::repo_opened_err(repos, state, repo_id, spec, error),
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded { repo_id, result }) => {
            effects::branches_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemotesLoaded { repo_id, result }) => {
            effects::remotes_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded { repo_id, result }) => {
            effects::remote_branches_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded { repo_id, result }) => {
            effects::worktree_status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded { repo_id, result }) => {
            effects::staged_status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::UncommittedLineStatsLoaded {
            repo_id,
            generation,
            result,
            large_files,
        }) => {
            effects::uncommitted_line_stats_loaded(state, repo_id, generation, result, large_files)
        }
        Msg::Internal(crate::msg::InternalMsg::LargeFileSupportLoaded { repo_id, result }) => {
            effects::large_file_support_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded { repo_id, result }) => {
            effects::status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded { repo_id, result }) => {
            effects::head_branch_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded { repo_id, result }) => {
            effects::upstream_divergence_loaded(state, repo_id, result)
        }
        Msg::IndexedHistory(event) => indexed_history::reduce(state, event),
        Msg::DiffSession(event) => diff_session::reduce(state, event),
        Msg::HistoryAuthors(event) => history_authors::reduce(state, event),
        Msg::HistoryFind(event) => history_find::reduce(state, event),
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
            scope,
            cursor,
            result,
        }) => external_and_history::log_loaded(state, repo_id, seq, scope, cursor, result),
        Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
            repo_id,
            seq,
            commits,
            scanned,
        }) => external_and_history::log_chunk_loaded(state, repo_id, seq, commits, scanned),
        Msg::Internal(crate::msg::InternalMsg::TagsLoaded { repo_id, result }) => {
            effects::tags_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemoteTagsLoaded { repo_id, result }) => {
            effects::remote_tags_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StashesLoaded { repo_id, result }) => {
            effects::stashes_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::ReflogLoaded { repo_id, result }) => {
            effects::reflog_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded { repo_id, result }) => {
            external_and_history::rebase_state_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
            repo_id,
            base,
            result,
        }) => external_and_history::interactive_rebase_setup_loaded(state, repo_id, base, result),
        Msg::Internal(crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
            repo_id,
            requested_ids,
            result,
        }) => external_and_history::interactive_cherry_pick_messages_loaded(
            state,
            repo_id,
            requested_ids,
            result,
        ),
        Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggestionConsumed {
            repo_id,
            message,
        }) => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
                && repo_state.suggested_commit_message.as_deref() == Some(message.as_str())
            {
                repo_state.set_suggested_commit_message(None);
            }
            Vec::new()
        }
        Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggested { repo_id, message }) => {
            if let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
                repo_state.set_suggested_commit_message(Some(message));
            }
            Vec::new()
        }
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded { repo_id, result }) => {
            external_and_history::merge_commit_message_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
            repo_id,
            commit_id,
            result,
        }) => effects::hover_commit_message_loaded(state, repo_id, commit_id, result),
        Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
            repo_id,
            path,
            cursor,
            result,
        }) => effects::file_history_loaded(state, repo_id, path, cursor, result),
        Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
            repo_id,
            path,
            source,
            result,
        }) => effects::blame_loaded(state, repo_id, path, source, result),
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path,
            result,
            conflict_session,
        }) => effects::conflict_file_loaded(
            state,
            repo_id,
            path,
            *result,
            conflict_session.map(|session| *session),
        ),
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded { repo_id, result }) => {
            effects::worktrees_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded { repo_id, result }) => {
            effects::worktree_dirty_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded { repo_id, result }) => {
            effects::ref_metadata_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded { repo_id, result }) => {
            effects::submodules_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
            cancellation,
            repo_id,
            source,
            result,
        }) => {
            if cancellation.is_some_and(|token| token.is_cancelled()) {
                Vec::new()
            } else {
                effects::file_browser_loaded(repos, state, repo_id, source, result)
            }
        }
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            result,
        }) => submodule_trust::add_trust_checked(
            state, repo_id, url, path, branch, name, force, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked { repo_id, result }) => {
            submodule_trust::update_trust_checked(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::SubmoduleLoadTrustChecked {
            repo_id,
            path,
            result,
        }) => submodule_trust::load_trust_checked(state, repo_id, path, result),
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id,
            commit_id,
            result,
        }) => effects::commit_details_loaded(state, repo_id, commit_id, result),
        Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
            repo_id,
            epoch,
            batch,
            result,
        }) => effects::commit_signatures_verified(state, repo_id, epoch, batch, result),
        Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
            repo_id,
            reference,
            result,
        }) => effects::commit_reveal_resolved(state, repo_id, reference, result),
        Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
            repo_id,
            reference,
            request,
            purpose,
            result,
        }) => effects::commit_lookup_resolved(state, repo_id, reference, request, purpose, result),
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from,
            to,
            request,
            result,
        }) => effects::range_files_loaded(state, repo_id, from, to, request, result),
        Msg::Internal(crate::msg::InternalMsg::SquashMessagePreviewLoaded {
            repo_id,
            oldest,
            head,
            result,
        }) => effects::squash_message_preview_loaded(state, repo_id, oldest, head, result),
        Msg::Internal(crate::msg::InternalMsg::SquashRebaseSetupLoaded {
            repo_id,
            base,
            actual_head,
            selected_ids,
            reword_id,
            message,
            count,
            result,
        }) => effects::squash_rebase_setup_loaded(
            state,
            repo_id,
            base,
            actual_head,
            selected_ids,
            reword_id,
            message,
            count,
            result,
        ),
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev,
            result,
        }) => effects::recent_commit_messages_loaded(state, repo_id, request_rev, result),
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_file_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::text_attributes_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
            repo_id,
            target,
            side,
            result,
        }) => diff_selection::diff_preview_text_file_loaded(state, repo_id, target, side, result),
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::submodule_summary_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => {
            diff_selection::inline_submodule_diff_loaded(state, repo_id, inline_rev, target, result)
        }
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => diff_selection::inline_submodule_diff_file_loaded(
            state, repo_id, inline_rev, target, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileImageLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => diff_selection::inline_submodule_diff_file_image_loaded(
            state, repo_id, inline_rev, target, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_file_image_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action,
            result,
        }) => external_and_history::repo_action_finished(repos, state, repo_id, action, result),
        Msg::Internal(crate::msg::InternalMsg::RepoPathsActionFinished {
            repo_id,
            action,
            paths,
            result,
        }) => external_and_history::repo_paths_action_finished(
            repos, state, repo_id, action, paths, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::BranchAlreadyExists { action, prompt }) => {
            external_and_history::branch_already_exists(repos, state, action, prompt)
        }
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id,
            action,
            worktree_path,
            result,
        }) => external_and_history::repo_action_finished_in_worktree(
            repos,
            id_alloc,
            state,
            repo_id,
            action,
            worktree_path,
            result,
        ),
        Msg::Internal(crate::msg::InternalMsg::CommitFinished { repo_id, result }) => {
            auth::commit_finished(repos, state, repo_id, result, false)
        }
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished { repo_id, result }) => {
            auth::commit_finished(repos, state, repo_id, result, true)
        }
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context,
            auth,
            result,
        }) => auth::safe_push_after_commit_finished(repos, state, repo_id, context, auth, result),
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result,
        }) => auth::repo_command_finished(repos, state, repo_id, command, result),

        // ── Workspace ───────────────────────────────────────────
        Msg::LoadWorkspace { repo_id } => workspace::load_workspace(state, repo_id),
        Msg::ApplyWorkspaceEdit { repo_id, edit } => {
            workspace::apply_workspace_edit(state, repo_id, edit)
        }
        Msg::SetWorkspaceBranchApplied {
            repo_id,
            name,
            applied,
        } => workspace::set_branch_applied(state, repo_id, name, applied),
        Msg::CommitWorkspaceBranch {
            repo_id,
            name,
            message,
        } => workspace::commit_branch(state, repo_id, name, message),
        Msg::CommitWorkspaceFile { repo_id, path } => {
            workspace::commit_single_file(state, repo_id, path)
        }
        Msg::DismissWorkspaceConflict { repo_id } => workspace::dismiss_conflict(state, repo_id),
        Msg::PushWorkspaceBranch { repo_id, name } => workspace::push_branch(state, repo_id, name),
        Msg::Internal(crate::msg::InternalMsg::WorkspaceLoaded {
            repo_id,
            result,
            assignments,
            workspace_commit,
        }) => workspace::workspace_loaded(state, repo_id, result, assignments, workspace_commit),
        Msg::Internal(crate::msg::InternalMsg::WorkspaceEditFinished {
            repo_id,
            edit,
            result,
        }) => workspace::workspace_edit_finished(state, repo_id, edit, result),
        Msg::Internal(crate::msg::InternalMsg::WorkspaceActiveFinished {
            repo_id,
            active,
            checkout_base,
            result,
        }) => workspace::workspace_active_finished(state, repo_id, active, checkout_base, result),
        Msg::Internal(crate::msg::InternalMsg::WorkspaceAssignFinished {
            repo_id,
            path,
            hunk,
            branch,
            result,
        }) => workspace::workspace_assign_finished(state, repo_id, path, hunk, branch, result),
        Msg::Internal(crate::msg::InternalMsg::WorkspacePushFinished {
            repo_id,
            name,
            result,
        }) => workspace::workspace_push_finished(state, repo_id, name, result),
    }
}
