use super::*;
use crate::model::{RewordRequest, RewordRequestId, RewordStatus};
use crate::msg::RepoCommandKind;

pub(super) fn start(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    request_id: RewordRequestId,
    commit_id: gitcomet_core::domain::CommitId,
    message: String,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    // A retry keeps its identity. Repeated dispatches and stale completions
    // cannot replace a newer request or finish an operation twice.
    if let Some(request) = repo.reword.requests.get(&request_id)
        && (request.status != RewordStatus::AwaitingAuthentication
            || request.commit_id != commit_id
            || request.message != message)
    {
        return Vec::new();
    }
    let refusal = if !state.git_runtime.is_available() {
        Some("Git is unavailable; restore Git before editing the commit message.")
    } else if repo.history_rewrite_busy()
        || repo
            .reword
            .requests
            .iter()
            .any(|(id, request)| *id != request_id && request.status.is_pending())
    {
        Some("Wait for the running Git operation to finish, then edit the commit message.")
    } else if message.trim().is_empty() {
        Some("Enter the new commit message first.")
    } else {
        None
    };
    if repo.reword.requests.contains_key(&request_id) {
        repo.reword.set_status(
            request_id,
            if refusal.is_some() {
                RewordStatus::Rejected
            } else {
                RewordStatus::Running
            },
        );
    } else {
        repo.reword.insert(
            request_id,
            RewordRequest::new(
                commit_id.clone(),
                message.clone(),
                if refusal.is_some() {
                    RewordStatus::Rejected
                } else {
                    RewordStatus::Running
                },
            ),
        );
    }
    if let Some(refusal) = refusal {
        util::push_notification(
            inputs,
            state,
            crate::model::AppNotificationKind::Warning,
            refusal.to_string(),
        );
        return Vec::new();
    }
    auth::discard_reword_auth_prompt(state, repo_id, request_id);
    begin_local_action(state, repo_id);
    vec![Effect::RepoCommand(RepoCommandEffect::RewordCommit {
        repo_id,
        request_id,
        commit_id,
        message,
        resume: false,
        auth: None,
    })]
}

pub(super) fn resume(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    request_id: RewordRequestId,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let Some(request) = repo.reword.requests.get(&request_id) else {
        return Vec::new();
    };
    if request.status == RewordStatus::Running || repo.reword.continuation != Some(request_id) {
        return Vec::new();
    }
    let (commit_id, message) = (request.commit_id.clone(), request.message.clone());
    let refusal = if !state.git_runtime.is_available() {
        Some("Git is unavailable; restore Git before continuing the commit-message update.")
    } else if repo.local_actions_in_flight > 0 || repo.sequencer_actions_in_flight > 0 {
        Some(
            "Wait for the running Git operation to finish, then continue the commit-message update.",
        )
    } else {
        None
    };
    if let Some(refusal) = refusal {
        repo.reword.set_status(request_id, RewordStatus::Rejected);
        util::push_notification(
            inputs,
            state,
            crate::model::AppNotificationKind::Warning,
            refusal.to_string(),
        );
        return Vec::new();
    }
    repo.reword.set_status(request_id, RewordStatus::Running);
    auth::discard_reword_auth_prompt(state, repo_id, request_id);
    begin_local_action(state, repo_id);
    vec![Effect::RepoCommand(RepoCommandEffect::RewordCommit {
        repo_id,
        request_id,
        commit_id,
        message,
        resume: true,
        auth: None,
    })]
}

/// False for an unrelated, duplicate, or already completed reword result.
pub(super) fn finished(
    state: &mut AppState,
    repo_id: RepoId,
    command: &RepoCommandKind,
    succeeded: bool,
    awaiting_auth: bool,
) -> bool {
    let RepoCommandKind::RewordCommit {
        request_id,
        commit_id,
        message,
        rebase_paused,
        ..
    } = command
    else {
        return true;
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return false;
    };
    if repo.reword.requests.get(request_id).is_none_or(|request| {
        request.status != RewordStatus::Running
            || request.commit_id != *commit_id
            || request.message != *message
    }) {
        return false;
    }
    if *rebase_paused && !succeeded {
        if repo.reword.continuation != Some(*request_id) {
            repo.reword.clear_continuation();
        }
        repo.reword.continuation = Some(*request_id);
    } else if succeeded && repo.reword.continuation != Some(*request_id) {
        // Another tool may have aborted the previous paused rebase. A new
        // successful rewrite proves its retained continuation is now stale.
        repo.reword.clear_continuation();
    } else if repo.reword.continuation == Some(*request_id) {
        repo.reword.continuation = None;
    }
    repo.reword.set_status(
        *request_id,
        if succeeded {
            RewordStatus::Completed
        } else if awaiting_auth {
            RewordStatus::AwaitingAuthentication
        } else {
            RewordStatus::Failed
        },
    );
    true
}

pub(super) fn cancel_auth(state: &mut AppState, operation: &crate::model::AuthRetryOperation) {
    if let crate::model::AuthRetryOperation::RepoCommand {
        repo_id,
        command: RepoCommandKind::RewordCommit { request_id, .. },
    } = operation
        && let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == *repo_id)
        && repo
            .reword
            .requests
            .get(request_id)
            .is_some_and(|request| request.status == RewordStatus::AwaitingAuthentication)
    {
        repo.reword.set_status(*request_id, RewordStatus::Cancelled);
    }
}
