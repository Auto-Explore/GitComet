//! What follows a worktree removal: the force prompt a refusal calls for, and
//! deleting the branch the user asked to remove with the worktree.

use crate::model::{ForceRemoveWorktreePrompt, RepoId};
use crate::msg::RepoCommandKind;
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::services::CommandOutput;

/// A plain removal Git refused for a reason `--force` overrides.
pub(super) fn force_prompt(
    generation: u64,
    repo_id: RepoId,
    command: &RepoCommandKind,
    result: &Result<CommandOutput, Error>,
) -> Option<ForceRemoveWorktreePrompt> {
    let (
        RepoCommandKind::RemoveWorktree {
            path,
            delete_branch,
        },
        Err(error),
    ) = (command, result)
    else {
        return None;
    };
    let ErrorKind::Git(failure) = error.kind() else {
        return None;
    };
    let GitFailureId::WorktreeRemovalBlocked(blocker) = failure.id() else {
        return None;
    };
    Some(ForceRemoveWorktreePrompt {
        generation,
        repo_id,
        path: path.clone(),
        delete_branch: delete_branch.clone(),
        blocker,
    })
}

pub(super) fn branch_to_delete(
    command: &RepoCommandKind,
    result: &Result<CommandOutput, Error>,
) -> Option<String> {
    match (command, result) {
        (
            RepoCommandKind::RemoveWorktree {
                delete_branch: Some(branch),
                ..
            }
            | RepoCommandKind::ForceRemoveWorktree {
                delete_branch: Some(branch),
                ..
            },
            Ok(_),
        ) => Some(branch.clone()),
        _ => None,
    }
}
