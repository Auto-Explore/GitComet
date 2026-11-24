//! Open tabs do not own backend resources indefinitely. Keep a small MRU set
//! and let document leases and unfinished operations explicitly pin a runtime.

use super::repository_runtime::RepositoryRuntimes;
use crate::model::{
    AppState, AuthRetryOperation, AutoFetchStatus, BareModeConversionState, Loadable, RepoId,
    RepoState,
};
use crate::msg::InternalMsg;
use std::time::{Duration, SystemTime};

pub(super) const WARM_INACTIVE_REPOSITORIES: usize = 2;
pub(super) const DESCRIPTOR_RECOVERY_COOLDOWN: Duration = Duration::from_secs(30);

pub(super) fn is_pinned(state: &AppState, repo: &RepoState) -> bool {
    !repo.diff_sessions.is_empty()
        || !repo.change_lists.is_empty()
        || state.watch_leases.contains_key(&repo.id)
        || state
            .worktree_watch_leases
            .keys()
            .any(|(id, lifetime, _)| *id == repo.id && *lifetime == repo.lifetime())
        || has_pending_continuation(state, repo.id)
        || repo.maintenance.recommended
        || has_unfinished_operation(repo)
}

fn has_pending_continuation(state: &AppState, repo_id: RepoId) -> bool {
    // Prompt acceptance resumes the existing operation directly, without a
    // tab activation that would reopen a parked backend first.
    state
        .auth_prompt
        .as_ref()
        .is_some_and(|prompt| match &prompt.operation {
            AuthRetryOperation::RepoCommand { repo_id: id, .. }
            | AuthRetryOperation::SafePushAfterCommit { repo_id: id, .. }
            | AuthRetryOperation::Commit { repo_id: id, .. } => *id == repo_id,
            AuthRetryOperation::Clone { .. } => false,
        })
        || state
            .branch_exists_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .force_delete_branch_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .force_remove_worktree_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .submodule_trust_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.repo_id == repo_id)
        || state
            .submodule_trust_check_pending
            .as_ref()
            .is_some_and(|pending| pending.repo_id == repo_id)
}

pub(super) fn has_unfinished_operation(repo: &RepoState) -> bool {
    repo.pull_in_flight > 0
        || repo.push_in_flight > 0
        || repo.worktrees_in_flight > 0
        || repo.local_actions_in_flight > 0
        || repo.sequencer_actions_in_flight > 0
        || repo.commit_in_flight > 0
        || repo.push_preflights_in_flight > 0
        || repo.submodule_add_in_flight.is_some()
        || repo.maintenance.running
        || repo.bare_mode_conversion.is_busy()
        || matches!(
            repo.bare_mode_conversion,
            BareModeConversionState::AwaitingConfirmation(_)
        )
        || matches!(repo.auto_fetch.status, AutoFetchStatus::Running)
        || repo.diff_state.edit_mode
        || repo.conflict_state.conflict_session.is_some()
}

pub(super) fn cold_repositories(
    state: &AppState,
    repos: &RepositoryRuntimes,
    now: SystemTime,
) -> Vec<RepoId> {
    let warm = super::warm_inactive_repos(
        state,
        repos.warm_repository_count(now),
        RepoState::last_used_at,
    );
    state
        .repos
        .iter()
        .filter(|repo| {
            state.active_repo != Some(repo.id)
                && !warm.contains(&repo.id)
                && !is_pinned(state, repo)
                && matches!(repo.open, Loadable::Ready(()))
                && repos.has_repository(&repo.id)
        })
        .map(|repo| repo.id)
        .collect()
}

/// Called under the state lock. Actual handle destruction is deferred until
/// the worker releases that lock, as it is when closing a tab.
pub(super) fn park(state: &mut AppState, repos: &mut RepositoryRuntimes, cold: &[RepoId]) {
    for repo in state
        .repos
        .iter_mut()
        .filter(|repo| cold.contains(&repo.id))
    {
        super::reducer::park_repository(repo, repos);
    }
}

pub(super) fn park_inactive(state: &mut AppState, repos: &mut RepositoryRuntimes, now: SystemTime) {
    let cold = cold_repositories(state, repos, now);
    park(state, repos, &cold);
}

/// Only ordinary idempotent repository reads may initiate a reopen. A push,
/// commit or other mutation's error is always delivered to its normal reducer.
/// Request-specific popovers/previews keep their normal errors too: replaying
/// a full activation would not reproduce those exact requests.
pub(super) fn descriptor_read_failure(message: &InternalMsg) -> Option<RepoId> {
    let (repo_id, error) = match message {
        InternalMsg::RepoOpenedErr { repo_id, error, .. }
        | InternalMsg::BranchesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemotesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemoteBranchesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StagedStatusLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::HeadTrackingLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::UpstreamDivergenceLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::LogLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::TagsLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RemoteTagsLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::StashesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::RebaseStateLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::MergeCommitMessageLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::WorktreesLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::WorktreeDirtyLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::SubmodulesLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::LargeFileSupportLoaded {
            repo_id,
            result: Err(error),
        }
        | InternalMsg::DiffLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffFileLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffFileImageLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::DiffPreviewTextFileLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::SubmoduleSummaryLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::TextAttributesLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::FileHistoryLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::BlameLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::CommitDetailsLoaded {
            repo_id,
            result: Err(error),
            ..
        }
        | InternalMsg::FileBrowserLoaded {
            repo_id,
            result: Err(error),
            ..
        } => (repo_id, error),
        _ => return None,
    };
    error.is_file_descriptor_exhaustion().then_some(*repo_id)
}

#[cfg(test)]
mod tests;
