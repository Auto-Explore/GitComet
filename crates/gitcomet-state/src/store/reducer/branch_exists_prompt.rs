//! The prompt shown when a branch to create or check out already exists.

use super::{actions_emit_effects, begin_head_changing_local_action};
use crate::model::{AppState, BranchExistsPromptOperation, BranchExistsPromptState};
use crate::msg::{BranchExistsChoice, Effect};
use gitcomet_core::services::CheckoutRemoteBranchMode;

pub(super) fn resolve(
    state: &mut AppState,
    prompt: BranchExistsPromptState,
    choice: BranchExistsChoice,
) -> Vec<Effect> {
    if state.branch_exists_prompt.as_ref() != Some(&prompt) {
        return Vec::new();
    }
    state.branch_exists_prompt = None;

    match choice {
        BranchExistsChoice::Cancel => Vec::new(),
        BranchExistsChoice::CheckoutExisting => {
            if let Some(repo_state) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == prompt.repo_id)
            {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, prompt.repo_id);
            actions_emit_effects::checkout_branch(prompt.repo_id, prompt.name)
        }
        BranchExistsChoice::OverwriteAndCheckout => {
            if let Some(repo_state) = state
                .repos
                .iter_mut()
                .find(|repo| repo.id == prompt.repo_id)
            {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, prompt.repo_id);
            match prompt.operation {
                BranchExistsPromptOperation::CreateBranch => {
                    actions_emit_effects::create_branch_and_checkout(
                        prompt.repo_id,
                        prompt.name,
                        prompt.target,
                        true,
                    )
                }
                BranchExistsPromptOperation::CheckoutRemoteBranch { remote, branch } => {
                    actions_emit_effects::checkout_remote_branch(
                        prompt.repo_id,
                        remote,
                        branch,
                        prompt.name,
                        CheckoutRemoteBranchMode::Overwrite,
                    )
                }
                BranchExistsPromptOperation::RenameBranch { old_name } => {
                    actions_emit_effects::rename_branch(prompt.repo_id, old_name, prompt.name, true)
                }
            }
        }
    }
}

pub(super) fn show(state: &mut AppState, prompt: BranchExistsPromptState) -> Vec<Effect> {
    if state.repos.iter().any(|repo| repo.id == prompt.repo_id) {
        state.branch_exists_prompt = Some(prompt);
    }
    Vec::new()
}
