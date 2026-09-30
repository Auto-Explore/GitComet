//! Submodule add, update, and load wait on a trust check of their sources;
//! unapproved sources raise a prompt instead of running the command.

use super::{actions_emit_effects, begin_local_action, report_error, util};
use crate::model::{
    AppState, RepoId, SubmoduleAddProgressState, SubmoduleTrustCheckOperation,
    SubmoduleTrustCheckState, SubmoduleTrustPromptOperation, SubmoduleTrustPromptState,
};
use crate::msg::Effect;
use gitcomet_core::error::Error;
use gitcomet_core::services::{SubmoduleTrustDecision, SubmoduleTrustTarget};
use std::path::PathBuf;

fn start_submodule_add_progress(
    state: &mut AppState,
    repo_id: RepoId,
    url: &str,
    path: &std::path::Path,
) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.submodule_add_in_flight = Some(SubmoduleAddProgressState {
            url: url.to_string(),
            path: path.to_path_buf(),
        });
    }
}

pub(super) fn add_submodule(
    state: &mut AppState,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
) -> Vec<Effect> {
    state.submodule_trust_prompt = None;
    state.submodule_trust_check_pending = Some(SubmoduleTrustCheckState {
        repo_id,
        operation: SubmoduleTrustCheckOperation::Add,
    });
    vec![Effect::CheckSubmoduleAddTrust {
        repo_id,
        url,
        path,
        branch,
        name,
        force,
        remote_url_policy: state.remote_url_policy,
    }]
}

pub(super) fn add_submodule_approved(
    state: &mut AppState,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
    approved_sources: Vec<SubmoduleTrustTarget>,
) -> Vec<Effect> {
    begin_local_action(state, repo_id);
    start_submodule_add_progress(state, repo_id, &url, &path);
    actions_emit_effects::add_submodule(
        repo_id,
        url,
        path,
        branch,
        name,
        force,
        approved_sources,
        state.remote_url_policy,
    )
}

pub(super) fn update_submodules(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    state.submodule_trust_prompt = None;
    state.submodule_trust_check_pending = Some(SubmoduleTrustCheckState {
        repo_id,
        operation: SubmoduleTrustCheckOperation::Update,
    });
    vec![Effect::CheckSubmoduleUpdateTrust {
        repo_id,
        remote_url_policy: state.remote_url_policy,
    }]
}

pub(super) fn update_submodules_approved(
    state: &mut AppState,
    repo_id: RepoId,
    approved_sources: Vec<SubmoduleTrustTarget>,
) -> Vec<Effect> {
    begin_local_action(state, repo_id);
    actions_emit_effects::update_submodules(repo_id, approved_sources, state.remote_url_policy)
}

pub(super) fn load_submodule(state: &mut AppState, repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    state.submodule_trust_prompt = None;
    state.submodule_trust_check_pending = Some(SubmoduleTrustCheckState {
        repo_id,
        operation: SubmoduleTrustCheckOperation::Load,
    });
    vec![Effect::CheckSubmoduleLoadTrust {
        repo_id,
        path,
        remote_url_policy: state.remote_url_policy,
    }]
}

pub(super) fn load_submodule_approved(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    approved_sources: Vec<SubmoduleTrustTarget>,
) -> Vec<Effect> {
    begin_local_action(state, repo_id);
    actions_emit_effects::load_submodule(repo_id, path, approved_sources, state.remote_url_policy)
}

pub(super) fn confirm_prompt(state: &mut AppState) -> Vec<Effect> {
    let Some(prompt) = state.submodule_trust_prompt.take() else {
        return Vec::new();
    };
    match prompt.operation {
        SubmoduleTrustPromptOperation::Add {
            url,
            path,
            branch,
            name,
            force,
        } => {
            begin_local_action(state, prompt.repo_id);
            start_submodule_add_progress(state, prompt.repo_id, &url, &path);
            actions_emit_effects::add_submodule(
                prompt.repo_id,
                url,
                path,
                branch,
                name,
                force,
                prompt.sources,
                state.remote_url_policy,
            )
        }
        SubmoduleTrustPromptOperation::Update => {
            begin_local_action(state, prompt.repo_id);
            actions_emit_effects::update_submodules(
                prompt.repo_id,
                prompt.sources,
                state.remote_url_policy,
            )
        }
        SubmoduleTrustPromptOperation::Load { path } => {
            begin_local_action(state, prompt.repo_id);
            actions_emit_effects::load_submodule(
                prompt.repo_id,
                path,
                prompt.sources,
                state.remote_url_policy,
            )
        }
    }
}

pub(super) fn cancel_prompt(state: &mut AppState) -> Vec<Effect> {
    state.submodule_trust_prompt = None;
    Vec::new()
}

pub(super) fn add_trust_checked(
    state: &mut AppState,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
    result: Result<SubmoduleTrustDecision, Error>,
) -> Vec<Effect> {
    state.submodule_trust_check_pending = None;
    match result {
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
            begin_local_action(state, repo_id);
            start_submodule_add_progress(state, repo_id, &url, &path);
            actions_emit_effects::add_submodule(
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                Vec::new(),
                state.remote_url_policy,
            )
        }
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
            state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                repo_id,
                operation: SubmoduleTrustPromptOperation::Add {
                    url,
                    path,
                    branch,
                    name,
                    force,
                },
                sources,
            });
            Vec::new()
        }
        Err(error) => {
            report_error(
                state,
                Some(repo_id),
                util::format_failure_summary("Submodule trust check", &error),
            );
            Vec::new()
        }
    }
}

pub(super) fn update_trust_checked(
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<SubmoduleTrustDecision, Error>,
) -> Vec<Effect> {
    state.submodule_trust_check_pending = None;
    match result {
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
            begin_local_action(state, repo_id);
            actions_emit_effects::update_submodules(repo_id, Vec::new(), state.remote_url_policy)
        }
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
            state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                repo_id,
                operation: SubmoduleTrustPromptOperation::Update,
                sources,
            });
            Vec::new()
        }
        Err(error) => {
            report_error(
                state,
                Some(repo_id),
                util::format_failure_summary("Submodule trust check", &error),
            );
            Vec::new()
        }
    }
}

pub(super) fn load_trust_checked(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: Result<SubmoduleTrustDecision, Error>,
) -> Vec<Effect> {
    state.submodule_trust_check_pending = None;
    match result {
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
            begin_local_action(state, repo_id);
            actions_emit_effects::load_submodule(repo_id, path, Vec::new(), state.remote_url_policy)
        }
        Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
            state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                repo_id,
                operation: SubmoduleTrustPromptOperation::Load { path },
                sources,
            });
            Vec::new()
        }
        Err(error) => {
            report_error(
                state,
                Some(repo_id),
                util::format_failure_summary("Submodule trust check", &error),
            );
            Vec::new()
        }
    }
}
