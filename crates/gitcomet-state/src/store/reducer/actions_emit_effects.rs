use super::util::{
    DiffReloadMode, SelectedConflictTarget, apply_selected_diff_load_plan_state,
    apply_selected_diff_load_plan_state_with_reload_mode, clear_banner_error_for_repo,
    diff_reload_effects, format_failure_summary, push_action_log, push_command_log,
    refresh_full_effects, refresh_primary_effects, selected_conflict_target,
    selected_diff_load_plan, start_conflict_target_reload, start_current_conflict_target_reload,
};
use crate::model::{
    AppState, InteractiveCherryPickSetup, InteractiveRebaseSetup, Loadable, RepoId,
    RepoLoadsInFlight, RepoState,
};
use crate::msg::{Effect, RepoCommandKind, RepoPathList};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::conflict_session::{ConflictRegionResolution, ConflictResolverStrategy};
use gitcomet_core::domain::{DiffTarget, FileConflictKind};
use gitcomet_core::error::Error;
use gitcomet_core::services::{
    CommandOutput, GitRepository, InteractiveRebaseEntry, PullMode, RemoteUrlKind, ResetMode,
    SafePushAfterCommitTarget,
};
use rustc_hash::FxHashMap as HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn checkout_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::CheckoutBranch { repo_id, name }]
}

pub(super) fn checkout_remote_branch(
    repo_id: RepoId,
    remote: String,
    branch: String,
    local_branch: String,
) -> Vec<Effect> {
    vec![Effect::CheckoutRemoteBranch {
        repo_id,
        remote,
        branch,
        local_branch,
    }]
}

pub(super) fn checkout_commit(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::CheckoutCommit { repo_id, commit_id }]
}

pub(super) fn cherry_pick_commit(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    commit: bool,
    mainline: Option<usize>,
    summary: String,
) -> Vec<Effect> {
    vec![Effect::CherryPickCommit {
        repo_id,
        commit_id,
        commit,
        mainline,
        summary,
    }]
}

<<<<<<< New base: Support explicit commit ranges when cherry-picking onto a new branch (#17)
pub(super) fn cherry_pick_range_onto_new_branch(
    repo_id: RepoId,
    base: String,
    range: String,
    source: String,
    new_branch: String,
) -> Vec<Effect> {
    vec![Effect::CherryPickRangeOntoNewBranch {
        repo_id,
        base,
        range,
        source,
        new_branch,
    }]
}

||||||| Common ancestor
=======
pub(super) fn cherry_pick_range_onto_new_branch(
    repo_id: RepoId,
    base: String,
    source: String,
    new_branch: String,
) -> Vec<Effect> {
    vec![Effect::CherryPickRangeOntoNewBranch {
        repo_id,
        base,
        source,
        new_branch,
    }]
}

>>>>>>> Current commit: Add cherry-pick branch A onto B as new branch C from the action bar
pub(super) fn revert_commit(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::RevertCommit { repo_id, commit_id }]
}

pub(super) fn create_branch(repo_id: RepoId, name: String, target: String) -> Vec<Effect> {
    vec![Effect::CreateBranch {
        repo_id,
        name,
        target,
    }]
}

pub(super) fn create_branch_and_checkout(
    repo_id: RepoId,
    name: String,
    target: String,
) -> Vec<Effect> {
    vec![Effect::CreateBranchAndCheckout {
        repo_id,
        name,
        target,
    }]
}

pub(super) fn rename_branch(repo_id: RepoId, old_name: String, new_name: String) -> Vec<Effect> {
    vec![Effect::RenameBranch {
        repo_id,
        old_name,
        new_name,
    }]
}

pub(super) fn delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::DeleteBranch { repo_id, name }]
}

pub(super) fn force_delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::ForceDeleteBranch { repo_id, name }]
}

pub(super) fn delete_branches(repo_id: RepoId, names: Vec<String>, force: bool) -> Vec<Effect> {
    vec![Effect::DeleteBranches {
        repo_id,
        names,
        force,
    }]
}

pub(super) fn export_patch(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    dest: PathBuf,
) -> Vec<Effect> {
    vec![Effect::ExportPatch {
        repo_id,
        commit_id,
        dest,
    }]
}

pub(super) fn apply_patch(repo_id: RepoId, patch: PathBuf) -> Vec<Effect> {
    vec![Effect::ApplyPatch { repo_id, patch }]
}

pub(super) fn add_worktree(
    repo_id: RepoId,
    path: PathBuf,
    reference: Option<String>,
) -> Vec<Effect> {
    vec![Effect::AddWorktree {
        repo_id,
        path,
        reference,
    }]
}

pub(super) fn remove_worktree(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::RemoveWorktree { repo_id, path }]
}

pub(super) fn force_remove_worktree(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::ForceRemoveWorktree { repo_id, path }]
}

pub(super) fn add_submodule(
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
    approved_sources: Vec<gitcomet_core::services::SubmoduleTrustTarget>,
) -> Vec<Effect> {
    vec![Effect::AddSubmodule {
        repo_id,
        url,
        path,
        branch,
        name,
        force,
        approved_sources,
        auth: None,
    }]
}

pub(super) fn update_submodules(
    repo_id: RepoId,
    approved_sources: Vec<gitcomet_core::services::SubmoduleTrustTarget>,
) -> Vec<Effect> {
    vec![Effect::UpdateSubmodules {
        repo_id,
        approved_sources,
        auth: None,
    }]
}

pub(super) fn load_submodule(
    repo_id: RepoId,
    path: PathBuf,
    approved_sources: Vec<gitcomet_core::services::SubmoduleTrustTarget>,
) -> Vec<Effect> {
    vec![Effect::LoadSubmodule {
        repo_id,
        path,
        approved_sources,
        auth: None,
    }]
}

pub(super) fn change_submodule_pointer(
    repo_id: RepoId,
    path: PathBuf,
    reference: String,
) -> Vec<Effect> {
    vec![Effect::ChangeSubmodulePointer {
        repo_id,
        path,
        reference,
    }]
}

pub(super) fn remove_submodule(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::RemoveSubmodule { repo_id, path }]
}

pub(super) fn stage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::StagePath { repo_id, path }]
}

pub(super) fn stage_paths(repo_id: RepoId, paths: RepoPathList) -> Vec<Effect> {
    vec![Effect::StagePaths { repo_id, paths }]
}

pub(super) fn unstage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::UnstagePath { repo_id, path }]
}

pub(super) fn unstage_paths(repo_id: RepoId, paths: RepoPathList) -> Vec<Effect> {
    vec![Effect::UnstagePaths { repo_id, paths }]
}

pub(super) fn discard_worktree_changes_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::DiscardWorktreeChangesPath { repo_id, path }]
}

pub(super) fn discard_worktree_changes_paths(repo_id: RepoId, paths: Vec<PathBuf>) -> Vec<Effect> {
    vec![Effect::DiscardWorktreeChangesPaths { repo_id, paths }]
}

pub(super) fn save_worktree_file(
    repo_id: RepoId,
    path: PathBuf,
    contents: String,
    stage: bool,
) -> Vec<Effect> {
    vec![Effect::SaveWorktreeFile {
        repo_id,
        path,
        contents,
        stage,
    }]
}

pub(super) fn append_gitignore_patterns(repo_id: RepoId, patterns: Vec<String>) -> Vec<Effect> {
    vec![Effect::AppendGitignorePatterns { repo_id, patterns }]
}

pub(super) fn commit(repo_id: RepoId, message: String) -> Vec<Effect> {
    vec![Effect::Commit {
        repo_id,
        message,
        auth: None,
    }]
}

pub(super) fn commit_amend(repo_id: RepoId, message: String) -> Vec<Effect> {
    vec![Effect::CommitAmend {
        repo_id,
        message,
        auth: None,
    }]
}

pub(super) fn safe_push_after_commit(
    repo_id: RepoId,
    context: gitcomet_core::services::SafePushAfterCommitContext,
) -> Vec<Effect> {
    vec![Effect::SafePushAfterCommit {
        repo_id,
        context,
        auth: None,
    }]
}

enum InFlightKind {
    Pull,
    Push,
}

fn bump_in_flight(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    kind: InFlightKind,
) {
    if !repos.contains_key(&repo_id) {
        return;
    }
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match kind {
            InFlightKind::Pull => {
                repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_add(1);
            }
            InFlightKind::Push => {
                repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
            }
        }
        repo_state.bump_ops_rev();
    }
}

pub(super) fn fetch_all(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    let prune = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .is_some_and(|repo_state| repo_state.fetch_prune_deleted_remote_tracking_branches);
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::FetchAll {
        repo_id,
        prune,
        auth: None,
    }]
}

pub(super) fn prune_merged_branches(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PruneMergedBranches { repo_id }]
}

pub(super) fn prune_local_tags(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PruneLocalTags { repo_id }]
}

pub(super) fn pull(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    mode: PullMode,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::Pull {
        repo_id,
        mode,
        auth: None,
    }]
}

pub(super) fn pull_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PullBranch {
        repo_id,
        remote,
        branch,
        auth: None,
    }]
}

pub(super) fn merge_ref(repo_id: RepoId, reference: String) -> Vec<Effect> {
    vec![Effect::MergeRef { repo_id, reference }]
}

pub(super) fn squash_ref(repo_id: RepoId, reference: String) -> Vec<Effect> {
    vec![Effect::SquashRef { repo_id, reference }]
}

pub(super) fn push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::Push {
        repo_id,
        auth: None,
    }]
}

pub(super) fn push_after_commit(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    target: SafePushAfterCommitTarget,
    set_upstream: bool,
) -> Vec<Effect> {
    push_after_commit_with_auth(repos, state, repo_id, target, set_upstream, None)
}

fn push_after_commit_with_auth(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    target: SafePushAfterCommitTarget,
    set_upstream: bool,
    auth: Option<StagedGitAuth>,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::PushAfterCommit {
        repo_id,
        target,
        set_upstream,
        auth,
    }]
}

pub(super) fn force_push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::ForcePush {
        repo_id,
        auth: None,
    }]
}

pub(super) fn force_push_with_lease(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    lease: gitcomet_core::services::ForcePushLease,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::ForcePushWithLease {
        repo_id,
        lease,
        auth: None,
    }]
}

pub(super) fn push_set_upstream(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::PushSetUpstream {
        repo_id,
        remote,
        branch,
        auth: None,
    }]
}

pub(super) fn set_upstream_branch(
    repo_id: RepoId,
    branch: String,
    upstream: String,
) -> Vec<Effect> {
    vec![Effect::SetUpstreamBranch {
        repo_id,
        branch,
        upstream,
    }]
}

pub(super) fn unset_upstream_branch(repo_id: RepoId, branch: String) -> Vec<Effect> {
    vec![Effect::UnsetUpstreamBranch { repo_id, branch }]
}

pub(super) fn delete_remote_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::DeleteRemoteBranch {
        repo_id,
        remote,
        branch,
        auth: None,
    }]
}

pub(super) fn delete_remote_branches(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branches: Vec<String>,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::DeleteRemoteBranches {
        repo_id,
        remote,
        branches,
        auth: None,
    }]
}

pub(super) fn reset(repo_id: RepoId, target: String, mode: ResetMode) -> Vec<Effect> {
    vec![Effect::Reset {
        repo_id,
        target,
        mode,
    }]
}

pub(super) fn squash_commits(
    state: &mut AppState,
    repo_id: RepoId,
    oldest: gitcomet_core::domain::CommitId,
    expected_head: gitcomet_core::domain::CommitId,
    message: String,
    count: usize,
) -> Vec<Effect> {
    // Re-validate against the current selection and log: both may have
    // changed between opening the prompt and confirming.
    let plan = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(super::effects::squash_plan_for_repo);
    let still_valid = plan
        .as_ref()
        .is_some_and(|p| p.oldest == oldest && p.head == expected_head);
    if !still_valid || message.trim().is_empty() {
        super::util::push_notification(
            state,
            crate::model::AppNotificationKind::Warning,
            "Squash cancelled: the selected commits are no longer squashable.".to_string(),
        );
        return Vec::new();
    }
    let plan = plan.unwrap();

    // Range ends at HEAD: use the fast commit-tree + update-ref path that
    // does not touch the worktree or index.
    if plan.head == plan.actual_head {
        super::begin_local_action(state, repo_id);
        return vec![Effect::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        }];
    }

    // Intermediate range: load the full commit list from base..HEAD so we
    // can build a rebase todo that squashes only the selected commits.
    vec![Effect::LoadSquashRebaseSetup {
        repo_id,
        base: plan.oldest_parent,
        actual_head: plan.actual_head,
        selected_ids: plan.ordered_ids,
        reword_id: oldest,
        message,
        count,
    }]
}

pub(super) fn rebase(repo_id: RepoId, onto: String) -> Vec<Effect> {
    vec![Effect::Rebase { repo_id, onto }]
}

pub(super) fn rebase_continue(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::RebaseContinue {
        repo_id,
        auth: None,
    }]
}

pub(super) fn rebase_abort(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::RebaseAbort { repo_id }]
}

pub(super) fn merge_abort(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::MergeAbort { repo_id }]
}

pub(super) fn load_interactive_rebase_setup(
    state: &mut AppState,
    repo_id: RepoId,
    base: String,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.interactive_rebase_setup = Some(InteractiveRebaseSetup {
            base: base.clone(),
            entries: Loadable::Loading,
        });
    }
    vec![Effect::LoadInteractiveRebaseSetup { repo_id, base }]
}

pub(super) fn interactive_rebase(
    repo_id: RepoId,
    base: String,
    entries: Vec<InteractiveRebaseEntry>,
) -> Vec<Effect> {
    vec![Effect::InteractiveRebase {
        repo_id,
        base,
        entries,
        interactive: true,
    }]
}

pub(super) fn open_interactive_cherry_pick_setup(
    state: &mut AppState,
    repo_id: RepoId,
    entries: Vec<InteractiveRebaseEntry>,
    source_colors: Vec<(String, u8)>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.interactive_rebase_setup = None;
        // The entries arrive seeded with subjects only (the log page carries
        // no bodies); load the full messages so a reword edit doesn't start
        // from — and then silently commit — a body-less seed.
        let ids = entries
            .iter()
            .map(|entry| entry.commit_id.clone())
            .collect();
        repo_state.interactive_cherry_pick_setup = Some(InteractiveCherryPickSetup {
            entries,
            source_colors,
            full_messages: Loadable::Loading,
        });
        return vec![Effect::LoadInteractiveCherryPickMessages { repo_id, ids }];
    }
    vec![]
}

pub(super) fn interactive_cherry_pick(
    repo_id: RepoId,
    entries: Vec<InteractiveRebaseEntry>,
) -> Vec<Effect> {
    vec![Effect::InteractiveCherryPick { repo_id, entries }]
}

pub(super) fn cancel_interactive_rebase_setup(
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.interactive_rebase_setup = None;
    }
    vec![]
}

pub(super) fn cancel_interactive_cherry_pick_setup(
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.interactive_cherry_pick_setup = None;
    }
    vec![]
}

pub(super) fn create_tag(
    repo_id: RepoId,
    name: String,
    target: String,
    message: Option<String>,
    annotated: bool,
) -> Vec<Effect> {
    vec![Effect::CreateTag {
        repo_id,
        name,
        target,
        message,
        annotated,
    }]
}

pub(super) fn delete_tag(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::DeleteTag { repo_id, name }]
}

pub(super) fn push_tag(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    name: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::PushTag {
        repo_id,
        remote,
        name,
        auth: None,
    }]
}

pub(super) fn delete_remote_tag(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    name: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::DeleteRemoteTag {
        repo_id,
        remote,
        name,
        auth: None,
    }]
}

pub(super) fn add_remote(repo_id: RepoId, name: String, url: String) -> Vec<Effect> {
    vec![Effect::AddRemote { repo_id, name, url }]
}

pub(super) fn remove_remote(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::RemoveRemote { repo_id, name }]
}

pub(super) fn set_remote_url(
    repo_id: RepoId,
    name: String,
    url: String,
    kind: RemoteUrlKind,
) -> Vec<Effect> {
    vec![Effect::SetRemoteUrl {
        repo_id,
        name,
        url,
        kind,
    }]
}

pub(super) fn checkout_conflict_side(
    repo_id: RepoId,
    path: PathBuf,
    side: gitcomet_core::services::ConflictSide,
) -> Vec<Effect> {
    vec![Effect::CheckoutConflictSide {
        repo_id,
        path,
        side,
    }]
}

pub(super) fn accept_conflict_deletion(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::AcceptConflictDeletion { repo_id, path }]
}

pub(super) fn checkout_conflict_base(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::CheckoutConflictBase { repo_id, path }]
}

pub(super) fn launch_mergetool(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::LaunchMergetool { repo_id, path }]
}

pub(super) fn stash(repo_id: RepoId, message: String, include_untracked: bool) -> Vec<Effect> {
    vec![Effect::Stash {
        repo_id,
        message,
        include_untracked,
    }]
}

pub(super) fn apply_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::ApplyStash { repo_id, index }]
}

pub(super) fn pop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::PopStash { repo_id, index }]
}

pub(super) fn drop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::DropStash { repo_id, index }]
}

/// Drop any loaded blame once the content it describes is known to be stale —
/// after a working-tree mutation (stage/unstage/apply patch/commit), after a
/// reload whose result actually differed (`diff_loaded`/`diff_file_loaded`), or
/// after a git-state event that may have moved HEAD. The blame annotation column
/// is derived from the same content the diff shows; leaving blame `Ready` would
/// make `request_blame_for_current_target` treat the target as already attempted
/// and keep painting stale attribution and staged/unstaged labels. `blame_path`
/// and `blame_source` are intentionally preserved so the view reloads the same
/// target's blame against the new content.
pub(super) fn invalidate_loaded_blame(repo_state: &mut RepoState) {
    if !matches!(repo_state.history_state.blame, Loadable::NotLoaded) {
        // Keep the outgoing annotations available to the view so the column
        // stays painted across the reload; the target is unchanged, so they
        // still describe the right file.
        repo_state.retain_blame_while_loading();
        repo_state.history_state.blame = Loadable::NotLoaded;
    }
}

pub(super) fn commit_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    let mut clear_banner = false;
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
    repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
    repo_state.bump_ops_rev();
    match result {
        Ok(()) => {
            repo_state.last_error = None;
            clear_banner = true;
            repo_state.set_recent_commit_messages(Loadable::NotLoaded);
            repo_state.set_diff_target(None);
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
            repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
            repo_state.diff_state.inline_submodule_diff = None;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
            repo_state.bump_diff_state_rev();
            invalidate_loaded_blame(repo_state);
            push_action_log(
                repo_state,
                true,
                "Commit".to_string(),
                "Commit: Completed".to_string(),
                None,
            );
        }
        Err(e) => {
            let summary = format_failure_summary("Commit", &e);
            repo_state.last_error = Some(summary.clone());
            push_action_log(repo_state, false, "Commit".to_string(), summary, Some(&e));
        }
    }
    if clear_banner {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        let effects = refresh_primary_effects(repo_state);
        clear_banner_error_for_repo(state, repo_id);
        return effects;
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    refresh_primary_effects(repo_state)
}

pub(super) fn commit_amend_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    let mut clear_banner = false;
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
    repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
    repo_state.bump_ops_rev();
    match result {
        Ok(()) => {
            repo_state.last_error = None;
            clear_banner = true;
            repo_state.set_recent_commit_messages(Loadable::NotLoaded);
            repo_state.set_diff_target(None);
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
            repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
            repo_state.diff_state.inline_submodule_diff = None;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
            repo_state.bump_diff_state_rev();
            invalidate_loaded_blame(repo_state);
            push_action_log(
                repo_state,
                true,
                "Amend".to_string(),
                "Amend: Completed".to_string(),
                None,
            );
        }
        Err(e) => {
            let summary = format_failure_summary("Amend", &e);
            repo_state.last_error = Some(summary.clone());
            push_action_log(repo_state, false, "Amend".to_string(), summary, Some(&e));
        }
    }
    if clear_banner {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        let effects = refresh_primary_effects(repo_state);
        clear_banner_error_for_repo(state, repo_id);
        return effects;
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    refresh_primary_effects(repo_state)
}

pub(super) fn safe_push_after_commit_finished(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    auth: Option<StagedGitAuth>,
    result: std::result::Result<gitcomet_core::services::SafePushAfterCommitDecision, Error>,
) -> Vec<Effect> {
    match result {
        Ok(gitcomet_core::services::SafePushAfterCommitDecision::Push { target }) => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_force_push_lease = None;
            }
            push_after_commit_with_auth(repos, state, repo_id, target, false, auth)
        }
        Ok(gitcomet_core::services::SafePushAfterCommitDecision::PushSetUpstream { target }) => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_force_push_lease = None;
            }
            push_after_commit_with_auth(repos, state, repo_id, target, true, auth)
        }
        Ok(gitcomet_core::services::SafePushAfterCommitDecision::Blocked { summary, lease }) => {
            let git_log_settings = state.git_log_settings;
            let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
                return Vec::new();
            };
            let full_summary = format!("Push after commit blocked: {summary}");
            repo_state.pending_force_push_lease = lease;
            repo_state.last_error = Some(full_summary.clone());
            push_action_log(
                repo_state,
                false,
                "Push after commit".to_string(),
                full_summary,
                None,
            );
            refresh_full_effects(repo_state, git_log_settings)
        }
        Err(e) => {
            let git_log_settings = state.git_log_settings;
            let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
                return Vec::new();
            };
            repo_state.pending_force_push_lease = None;
            let summary = format_failure_summary("Push after commit", &e);
            repo_state.last_error = Some(summary.clone());
            push_action_log(
                repo_state,
                false,
                "Push after commit".to_string(),
                summary,
                Some(&e),
            );
            refresh_full_effects(repo_state, git_log_settings)
        }
    }
}

fn tracks_local_actions_in_flight(command: &RepoCommandKind) -> bool {
    matches!(
        command,
        RepoCommandKind::MergeRef { .. }
            | RepoCommandKind::SquashRef { .. }
            | RepoCommandKind::Reset { .. }
            | RepoCommandKind::SquashCommits { .. }
            | RepoCommandKind::Rebase { .. }
            | RepoCommandKind::RebaseContinue
            | RepoCommandKind::RebaseAbort
            | RepoCommandKind::InteractiveRebase { .. }
            | RepoCommandKind::InteractiveCherryPick { .. }
            | RepoCommandKind::CherryPick { .. }
            | RepoCommandKind::CherryPickRangeOntoNewBranch { .. }
            | RepoCommandKind::MergeAbort
            | RepoCommandKind::CreateTag { .. }
            | RepoCommandKind::DeleteTag { .. }
            | RepoCommandKind::AddRemote { .. }
            | RepoCommandKind::RemoveRemote { .. }
            | RepoCommandKind::SetRemoteUrl { .. }
            | RepoCommandKind::SetUpstreamBranch { .. }
            | RepoCommandKind::UnsetUpstreamBranch { .. }
            | RepoCommandKind::CheckoutConflict { .. }
            | RepoCommandKind::AcceptConflictDeletion { .. }
            | RepoCommandKind::CheckoutConflictBase { .. }
            | RepoCommandKind::LaunchMergetool { .. }
            | RepoCommandKind::SaveWorktreeFile { .. }
            | RepoCommandKind::AppendGitignorePatterns { .. }
            | RepoCommandKind::ExportPatch { .. }
            | RepoCommandKind::ApplyPatch { .. }
            | RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules { .. }
            | RepoCommandKind::LoadSubmodule { .. }
            | RepoCommandKind::ChangeSubmodulePointer { .. }
            | RepoCommandKind::RemoveSubmodule { .. }
            | RepoCommandKind::StageHunk
            | RepoCommandKind::UnstageHunk
            | RepoCommandKind::ApplyWorktreePatch { .. }
    )
}

fn command_clears_pending_force_push_lease(command: &RepoCommandKind) -> bool {
    matches!(
        command,
        RepoCommandKind::Pull { .. }
            | RepoCommandKind::PullBranch { .. }
            | RepoCommandKind::MergeRef { .. }
            | RepoCommandKind::SquashRef { .. }
            | RepoCommandKind::Push
            | RepoCommandKind::PushAfterCommit { .. }
            | RepoCommandKind::ForcePush
            | RepoCommandKind::ForcePushWithLease { .. }
            | RepoCommandKind::PushSetUpstream { .. }
            | RepoCommandKind::Reset { .. }
            | RepoCommandKind::Rebase { .. }
            | RepoCommandKind::RebaseContinue
            | RepoCommandKind::RebaseAbort
            | RepoCommandKind::InteractiveRebase { .. }
            | RepoCommandKind::InteractiveCherryPick { .. }
            | RepoCommandKind::CherryPick { .. }
            | RepoCommandKind::CherryPickRangeOntoNewBranch { .. }
            | RepoCommandKind::MergeAbort
    )
}

fn changed_submodule_path(command: &RepoCommandKind) -> Option<&std::path::Path> {
    match command {
        RepoCommandKind::LoadSubmodule { path, .. }
        | RepoCommandKind::ChangeSubmodulePointer { path, .. } => Some(path.as_path()),
        _ => None,
    }
}

fn selected_submodule_target_changed_by_command(
    repo_state: &RepoState,
    command: &RepoCommandKind,
) -> Option<DiffTarget> {
    let target = repo_state.diff_state.diff_target.as_ref()?;
    let DiffTarget::WorkingTree { path, .. } = target else {
        return None;
    };
    if let Some(changed_path) = changed_submodule_path(command) {
        if path.as_path() != changed_path {
            return None;
        }
    } else if !matches!(command, RepoCommandKind::UpdateSubmodules { .. }) {
        return None;
    }

    let summary_is_active = !matches!(repo_state.diff_state.submodule_summary, Loadable::NotLoaded);
    (summary_is_active || selected_diff_load_plan(repo_state, target).load_submodule_summary)
        .then(|| target.clone())
}

pub(super) fn repo_command_finished(
    state: &mut AppState,
    repo_id: RepoId,
    command: RepoCommandKind,
    result: std::result::Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let refresh_worktrees = matches!(
        &command,
        RepoCommandKind::AddWorktree { .. }
            | RepoCommandKind::RemoveWorktree { .. }
            | RepoCommandKind::ForceRemoveWorktree { .. }
            | RepoCommandKind::CherryPickRangeOntoNewBranch { .. }
    ) && result.is_ok();
    // The command creates a branch and moves HEAD onto it.
    let refresh_branches = matches!(&command, RepoCommandKind::CherryPickRangeOntoNewBranch { .. })
        && result.is_ok();
    let refresh_submodules = matches!(
        &command,
        RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules { .. }
            | RepoCommandKind::LoadSubmodule { .. }
            | RepoCommandKind::ChangeSubmodulePointer { .. }
            | RepoCommandKind::RemoveSubmodule { .. }
    ) && result.is_ok();
    let command_succeeded = result.is_ok();
    let refresh_tags = command_succeeded
        && matches!(
            &command,
            RepoCommandKind::CreateTag { .. }
                | RepoCommandKind::DeleteTag { .. }
                | RepoCommandKind::PruneLocalTags
        );
    let mut clear_banner = false;

    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let mut extra_effects = Vec::new();
    match &command {
        RepoCommandKind::FetchAll
        | RepoCommandKind::PruneMergedBranches
        | RepoCommandKind::PruneLocalTags
        | RepoCommandKind::Pull { .. }
        | RepoCommandKind::PullBranch { .. } => {
            repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        RepoCommandKind::Push
        | RepoCommandKind::PushAfterCommit { .. }
        | RepoCommandKind::ForcePush
        | RepoCommandKind::ForcePushWithLease { .. }
        | RepoCommandKind::PushSetUpstream { .. }
        | RepoCommandKind::DeleteRemoteBranch { .. }
        | RepoCommandKind::DeleteRemoteBranches { .. }
        | RepoCommandKind::PushTag { .. }
        | RepoCommandKind::DeleteRemoteTag { .. } => {
            repo_state.push_in_flight = repo_state.push_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        RepoCommandKind::AddWorktree { .. }
        | RepoCommandKind::RemoveWorktree { .. }
        | RepoCommandKind::ForceRemoveWorktree { .. } => {
            repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_sub(1);
        }
        _ if tracks_local_actions_in_flight(&command) => {
            repo_state.local_actions_in_flight =
                repo_state.local_actions_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        _ => {}
    }

    if matches!(&command, RepoCommandKind::AddSubmodule { .. }) {
        repo_state.submodule_add_in_flight = None;
    }

    match result {
        Ok(output) => {
            repo_state.last_error = None;
            clear_banner = true;
            if command_clears_pending_force_push_lease(&command) {
                repo_state.pending_force_push_lease = None;
            }
            repo_state.set_recent_commit_messages(Loadable::NotLoaded);
            if matches!(
                &command,
                RepoCommandKind::Reset { .. }
                    | RepoCommandKind::SquashCommits { .. }
                    | RepoCommandKind::Rebase { .. }
                    | RepoCommandKind::RebaseContinue
                    | RepoCommandKind::RebaseAbort
                    | RepoCommandKind::InteractiveRebase { .. }
                    | RepoCommandKind::InteractiveCherryPick { .. }
                    | RepoCommandKind::CherryPick { .. }
                    | RepoCommandKind::CherryPickRangeOntoNewBranch { .. }
                    | RepoCommandKind::MergeAbort
            ) {
                repo_state.set_diff_target(None);
                repo_state.diff_state.diff = Loadable::NotLoaded;
                repo_state.diff_state.diff_file = Loadable::NotLoaded;
                repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
                repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
                repo_state.diff_state.inline_submodule_diff = None;
                repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
                repo_state.bump_diff_state_rev();
            }
            if matches!(
                &command,
                RepoCommandKind::SquashCommits { .. }
                    | RepoCommandKind::InteractiveRebase { .. }
                    | RepoCommandKind::InteractiveCherryPick { .. }
            ) {
                // The squashed/rebased commits may no longer exist; clear the
                // selection and the prompt's preview.
                repo_state.set_selected_commit(None);
                repo_state.set_commit_details(Loadable::NotLoaded);
                repo_state.history_state.squash_preview_pending = None;
                repo_state.set_squash_preview(Loadable::NotLoaded);
            }
            push_command_log(repo_state, true, &command, &output, None);
        }
        Err(e) => {
            push_command_log(
                repo_state,
                false,
                &command,
                &CommandOutput::default(),
                Some(&e),
            );
            repo_state.last_error = repo_state
                .command_log
                .last()
                .map(|entry| entry.summary.clone());
        }
    }
    if command_succeeded && sync_conflict_session_after_resolution_command(repo_state, &command) {
        repo_state.bump_conflict_rev();
    }

    if refresh_worktrees {
        repo_state.set_worktrees(Loadable::Loading);
        extra_effects.push(Effect::LoadWorktrees { repo_id });
    }
    if refresh_branches && repo_state.loads_in_flight.request(RepoLoadsInFlight::BRANCHES) {
        repo_state.set_branches(Loadable::NotLoaded);
        extra_effects.push(Effect::LoadBranches { repo_id });
    }
    if command_succeeded
        && let Some(target) = selected_submodule_target_changed_by_command(repo_state, &command)
    {
        let load_plan = selected_diff_load_plan(repo_state, &target);
        apply_selected_diff_load_plan_state(repo_state, load_plan);
        repo_state.diff_state.inline_submodule_diff = None;
        repo_state.bump_diff_state_rev();
        extra_effects.extend(diff_reload_effects(repo_state, repo_id, target));
    }
    if refresh_submodules {
        repo_state.set_submodules(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::SUBMODULES)
        {
            extra_effects.push(Effect::LoadSubmodules { repo_id });
        }
    }
    if refresh_tags {
        repo_state.set_tags(Loadable::NotLoaded);
        if repo_state.loads_in_flight.request(RepoLoadsInFlight::TAGS) {
            extra_effects.push(Effect::LoadTags { repo_id });
        }
    }
    if matches!(
        &command,
        RepoCommandKind::StageHunk
            | RepoCommandKind::UnstageHunk
            | RepoCommandKind::ApplyWorktreePatch { .. }
    ) && let Some(target) = repo_state.diff_state.diff_target.clone()
    {
        // The annotation column is recomputed from the same content the diff
        // shows, so staging/unstaging/patching must invalidate blame too.
        invalidate_loaded_blame(repo_state);
        if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
            // Blanked, so there is nothing stale left to guard against.
            repo_state.diff_state.diff_reload_in_flight = false;
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_preview_text_file = Loadable::NotLoaded;
            repo_state.diff_state.submodule_summary = Loadable::NotLoaded;
            repo_state.diff_state.inline_submodule_diff = None;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
            repo_state.bump_diff_state_rev();
            match conflict_target {
                SelectedConflictTarget::Current => {
                    extra_effects.extend(start_current_conflict_target_reload(repo_state));
                }
                SelectedConflictTarget::Path(path) => {
                    extra_effects.extend(start_conflict_target_reload(repo_state, path));
                }
            }
        } else {
            let load_plan = selected_diff_load_plan(repo_state, &target);
            // The diff target did not change — only its contents did — so the
            // reload keeps showing what is already there. Blanking it would make
            // the pane flash "Loading" on every staged hunk or line.
            apply_selected_diff_load_plan_state_with_reload_mode(
                repo_state,
                load_plan,
                DiffReloadMode::KeepLoaded,
            );
            repo_state.bump_diff_state_rev();
            extra_effects.extend(diff_reload_effects(repo_state, repo_id, target));
        }
    }
    let mut effects = refresh_full_effects(repo_state, state.git_log_settings);
    effects.extend(extra_effects);
    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }
    effects
}

fn sync_conflict_session_after_resolution_command(
    repo_state: &mut RepoState,
    command: &RepoCommandKind,
) -> bool {
    let Some(path) = resolution_command_path(command) else {
        return false;
    };

    let tracked_path_matches = repo_state
        .conflict_state
        .conflict_file_path
        .as_ref()
        .is_some_and(|tracked| tracked.as_path() == path.as_path());
    if !tracked_path_matches {
        return false;
    }

    if matches!(command, RepoCommandKind::LaunchMergetool { .. }) {
        clear_conflict_context(repo_state);
        return true;
    }

    let Some(session_view) = repo_state.conflict_state.conflict_session.as_ref() else {
        return false;
    };
    if session_view.path.as_path() != path.as_path() {
        return false;
    }

    if session_view.strategy == ConflictResolverStrategy::BinarySidePick
        && session_view.regions.is_empty()
    {
        clear_conflict_context(repo_state);
        return true;
    }

    let resolution = match command {
        RepoCommandKind::CheckoutConflict { side, .. } => match side {
            gitcomet_core::services::ConflictSide::Ours => ConflictRegionResolution::PickOurs,
            gitcomet_core::services::ConflictSide::Theirs => ConflictRegionResolution::PickTheirs,
        },
        RepoCommandKind::CheckoutConflictBase { .. } => ConflictRegionResolution::PickBase,
        RepoCommandKind::AcceptConflictDeletion { .. } => {
            deletion_resolution_for_kind(session_view.conflict_kind)
        }
        _ => return false,
    };

    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return false;
    };

    apply_resolution_to_all_regions(session, &resolution) > 0
}

fn resolution_command_path(command: &RepoCommandKind) -> Option<&std::path::PathBuf> {
    match command {
        RepoCommandKind::CheckoutConflict { path, .. }
        | RepoCommandKind::CheckoutConflictBase { path }
        | RepoCommandKind::AcceptConflictDeletion { path }
        | RepoCommandKind::LaunchMergetool { path } => Some(path),
        _ => None,
    }
}

fn clear_conflict_context(repo_state: &mut RepoState) {
    repo_state.conflict_state.conflict_file_path = None;
    repo_state.conflict_state.conflict_file_load_mode =
        crate::model::ConflictFileLoadMode::CurrentOnly;
    repo_state.conflict_state.conflict_file = Loadable::NotLoaded;
    repo_state.conflict_state.session_pending_restore = None;
    repo_state.conflict_state.conflict_session = None;
    repo_state.conflict_state.conflict_hide_resolved = false;
}

fn deletion_resolution_for_kind(conflict_kind: FileConflictKind) -> ConflictRegionResolution {
    match conflict_kind {
        FileConflictKind::DeletedByUs
        | FileConflictKind::AddedByThem
        | FileConflictKind::BothDeleted => ConflictRegionResolution::PickOurs,
        FileConflictKind::DeletedByThem | FileConflictKind::AddedByUs => {
            ConflictRegionResolution::PickTheirs
        }
        FileConflictKind::BothAdded | FileConflictKind::BothModified => {
            ConflictRegionResolution::PickOurs
        }
    }
}

fn apply_resolution_to_all_regions(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    resolution: &ConflictRegionResolution,
) -> usize {
    let mut changed = 0usize;
    for region in &mut session.regions {
        if matches!(resolution, ConflictRegionResolution::PickBase) && region.base.is_none() {
            continue;
        }
        if &region.resolution != resolution {
            region.resolution = resolution.clone();
            changed += 1;
        }
    }
    changed
}
