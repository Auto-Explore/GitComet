//! Git operation lifecycle: hook activity around an operation's completion,
//! and cancelling a running operation.

use super::{external_and_history, git_hook_activity, reduce};
use crate::model::{AppState, GitOperationOuterOutcome, RepoId};
use crate::msg::{Effect, InternalMsg, Msg};
use gitcomet_core::git_operation::{GitOperationEvent, GitOperationId};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime};

pub(super) fn cancel_git_operation(
    state: &mut AppState,
    repo_id: RepoId,
    operation_id: GitOperationId,
) -> Vec<Effect> {
    let requested = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .is_some_and(|repo| git_hook_activity::request_cancel(repo, operation_id));
    requested
        .then_some(Effect::CancelGitOperation {
            repo_id,
            operation_id,
        })
        .into_iter()
        .collect()
}

pub(super) fn git_operation_started(
    state: &mut AppState,
    repo_id: RepoId,
    operation_id: GitOperationId,
    label: String,
    context: Option<String>,
    time: SystemTime,
    progress_lane: bool,
) -> Vec<Effect> {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        git_hook_activity::started(repo, operation_id, label, context, time, progress_lane);
    }
    Vec::new()
}

pub(super) fn git_operation_event(
    state: &mut AppState,
    repo_id: RepoId,
    operation_id: GitOperationId,
    event: GitOperationEvent,
) -> Vec<Effect> {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        git_hook_activity::apply_event(repo, operation_id, event);
    }
    Vec::new()
}

pub(super) fn git_operation_finished(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    repo_id: RepoId,
    operation_id: GitOperationId,
    outer_outcome: GitOperationOuterOutcome,
    duration: Duration,
    message: Box<InternalMsg>,
) -> Vec<Effect> {
    let (is_reportable, has_hooks, all_hooks_succeeded) = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            repo.feedback
                .hook_activity
                .iter()
                .find(|operation| operation.id == operation_id)
        })
        .map(|operation| {
            (
                operation.is_reportable(),
                operation.has_hooks(),
                operation.has_hooks()
                    && operation
                        .hooks
                        .iter()
                        .all(|hook| hook.status == crate::model::GitHookRunStatus::Succeeded),
            )
        })
        .unwrap_or_default();
    let outer_failure_after_successful_hooks =
        outer_outcome == crate::model::GitOperationOuterOutcome::Failed && all_hooks_succeeded;
    let suppress_nested_diagnostics = has_hooks
        && !outer_failure_after_successful_hooks
        && matches!(
            message.as_ref(),
            crate::msg::InternalMsg::RepoActionFinished { .. }
                | crate::msg::InternalMsg::RepoPathsActionFinished { .. }
                | crate::msg::InternalMsg::RepoActionFinishedInWorktree { .. }
        );
    let previous_diagnostics = suppress_nested_diagnostics
        .then(|| {
            state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .map(|repo| {
                    (
                        repo.feedback.diagnostics.clone(),
                        repo.feedback.diagnostics_seq,
                    )
                })
        })
        .flatten();
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo.feedback.command_log_operation_id = is_reportable.then_some(operation_id);
    }

    let mut effects = reduce(repos, id_alloc, state, Msg::Internal(*message));

    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo.feedback.command_log_operation_id = None;
        if let Some((entries, seq)) = previous_diagnostics {
            repo.feedback.diagnostics = entries;
            repo.feedback.diagnostics_seq = seq;
        }
        git_hook_activity::finished(repo, operation_id, outer_outcome, duration);
    }
    if outer_outcome == crate::model::GitOperationOuterOutcome::Cancelled
        && !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { repo_id: id, .. } if *id == repo_id))
    {
        // Cancellation may leave partial Git changes, so refresh the
        // retained panes. Explicit Reload would discard history and
        // selection after the nested action already refreshed them.
        effects.extend(external_and_history::repo_externally_changed(
            repos,
            state,
            repo_id,
            crate::msg::RepoExternalChange::all(),
        ));
    }
    effects
}
