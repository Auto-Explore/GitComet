//! App settings the reducer holds; a change that affects commit signature
//! verification re-verifies the loaded history.

use super::util;
use crate::model::{AppState, DefaultTagType, GitLogTagFetchMode, RemoteSettings};
use crate::msg::Effect;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::signing_tools::SigningToolsState;

pub(super) fn set_git_runtime_state(state: &mut AppState, runtime: GitRuntimeState) -> Vec<Effect> {
    if state.git_runtime == runtime {
        return Vec::new();
    }
    state.git_runtime = runtime;
    state.signing_tools = Default::default();
    // A different Git resolves `git lfs` / `git annex` differently.
    state.large_file_tools = Default::default();
    if state.git_log_settings.verify_commit_signatures {
        util::reverify_all_commit_signatures_effects(state)
    } else {
        Vec::new()
    }
}

pub(super) fn set_signing_tools_state(
    state: &mut AppState,
    tools: SigningToolsState,
) -> Vec<Effect> {
    if state.signing_tools == tools {
        return Vec::new();
    }
    state.signing_tools = tools;
    if !state.git_log_settings.verify_commit_signatures {
        return Vec::new();
    }
    // A verifier was installed or went missing: badges must follow it.
    util::reverify_all_commit_signatures_effects(state)
}

pub(super) fn set_remote_url_policy(state: &mut AppState, policy: RemoteUrlPolicy) -> Vec<Effect> {
    state.remote_url_policy = policy;
    Vec::new()
}

pub(super) fn set_git_log_settings(
    state: &mut AppState,
    show_history_tags: bool,
    tag_fetch_mode: GitLogTagFetchMode,
    verify_commit_signatures: bool,
) -> Vec<Effect> {
    state.git_log_settings.show_history_tags = show_history_tags;
    state.git_log_settings.tag_fetch_mode = tag_fetch_mode;
    let verification_toggled =
        state.git_log_settings.verify_commit_signatures != verify_commit_signatures;
    state.git_log_settings.verify_commit_signatures = verify_commit_signatures;
    if !verification_toggled {
        return Vec::new();
    }
    // A fresh opt-in waits for discovery before starting any verifier.
    state.signing_tools = Default::default();
    util::reverify_all_commit_signatures_effects(state)
}

pub(super) fn set_remote_settings(state: &mut AppState, settings: RemoteSettings) -> Vec<Effect> {
    state.remote_settings = settings;
    Vec::new()
}

pub(super) fn set_default_tag_type(state: &mut AppState, tag_type: DefaultTagType) -> Vec<Effect> {
    state.default_tag_type = tag_type;
    Vec::new()
}

pub(super) fn set_large_file_tools_state(
    state: &mut AppState,
    tools: gitcomet_core::large_file_tools::LargeFileToolsState,
) -> Vec<Effect> {
    state.large_file_tools = tools;
    Vec::new()
}

pub(super) fn set_large_file_settings(
    state: &mut AppState,
    settings: crate::model::LargeFileSettings,
) -> Vec<Effect> {
    state.large_file_settings = settings;
    for repo in &mut state.repos {
        repo.sync_annex_refs_hidden(settings.hide_annex_refs);
    }
    Vec::new()
}
