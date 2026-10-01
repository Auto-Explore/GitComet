//! Auth prompts raised by failed commands, and replaying the command with
//! the credentials the user then enters.

use super::{
    actions_emit_effects, begin_commit_action, reduce, refresh_selected_head_gitlink,
    repo_management, util,
};
use crate::model::AuthPromptKind;
use crate::model::{AppState, AuthPromptState, AuthRetryOperation, PendingCommitRetry, RepoId};
use crate::msg::{Effect, Msg, RepoCommandKind};
#[cfg(test)]
use gitcomet_core::auth::stage_git_auth;
use gitcomet_core::auth::{
    GitAuthKind, SSH_PASSPHRASE_PROMPT_MARKER, StagedGitAuth, clear_staged_git_auth,
};
use gitcomet_core::error::{Error, ErrorKind, GitFailure};
use gitcomet_core::services::{
    CommandOutput, CommitOperationOutcome, GitRepository, SafePushAfterCommitContext,
    SafePushAfterCommitDecision,
};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

fn auth_prompt_for_repo_command(
    repo_id: RepoId,
    command: &RepoCommandKind,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = detect_auth_prompt_kind(error)?;
    let operation = AuthRetryOperation::RepoCommand {
        repo_id,
        command: command.clone(),
    };
    retry_msg_for_auth_operation(operation.clone())?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation,
    })
}

fn auth_prompt_for_safe_push_after_commit(
    repo_id: RepoId,
    context: SafePushAfterCommitContext,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = detect_auth_prompt_kind(error)?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::SafePushAfterCommit { repo_id, context },
    })
}

fn auth_prompt_for_commit(
    repo_id: RepoId,
    pending: Option<PendingCommitRetry>,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = detect_auth_prompt_kind(error)?;
    let pending = pending?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: pending.message,
            amend: pending.amend,
            push_after_commit: pending.push_after_commit,
        },
    })
}

fn auth_prompt_for_clone(
    url: &str,
    dest: &std::path::Path,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = detect_auth_prompt_kind(error)?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::Clone {
            url: url.to_string(),
            dest: dest.to_path_buf(),
        },
    })
}

fn retry_msg_for_auth_operation(operation: AuthRetryOperation) -> Option<Msg> {
    match operation {
        AuthRetryOperation::RepoCommand { repo_id, command } => {
            retry_msg_for_repo_command(repo_id, command)
        }
        AuthRetryOperation::SafePushAfterCommit { repo_id, context } => {
            Some(Msg::SafePushAfterCommit { repo_id, context })
        }
        AuthRetryOperation::Commit {
            repo_id,
            message,
            amend,
            push_after_commit,
        } => Some(if amend {
            Msg::CommitAmend {
                repo_id,
                message,
                push_after_commit,
            }
        } else {
            Msg::Commit {
                repo_id,
                message,
                push_after_commit,
            }
        }),
        AuthRetryOperation::Clone { url, dest } => Some(Msg::CloneRepo { url, dest }),
    }
}

fn retry_msg_for_repo_command(repo_id: RepoId, command: RepoCommandKind) -> Option<Msg> {
    Some(match command {
        RepoCommandKind::FetchAll => Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
        RepoCommandKind::FetchRefspecs { remote, refspecs } => {
            Msg::Fetch(crate::msg::FetchMsg::Refspecs {
                repo_id,
                remote,
                refspecs,
            })
        }
        RepoCommandKind::PruneMergedBranches => Msg::PruneMergedBranches { repo_id },
        RepoCommandKind::PruneLocalTags => Msg::PruneLocalTags { repo_id },
        RepoCommandKind::Pull { mode } => Msg::Pull { repo_id, mode },
        RepoCommandKind::PullBranch { remote, branch } => Msg::PullBranch {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::MergeRef { reference } => Msg::MergeRef { repo_id, reference },
        RepoCommandKind::SquashRef { reference } => Msg::SquashRef { repo_id, reference },
        RepoCommandKind::Push => Msg::Push { repo_id },
        RepoCommandKind::PushWithTags { request } => Msg::PushWithTags { repo_id, request },
        RepoCommandKind::PushAfterCommit {
            target,
            set_upstream,
        } => Msg::PushAfterCommit {
            repo_id,
            target,
            set_upstream,
        },
        RepoCommandKind::ForcePush => Msg::ForcePush { repo_id },
        RepoCommandKind::ForcePushWithLease { lease } => Msg::ForcePushWithLease { repo_id, lease },
        RepoCommandKind::PushSetUpstream { remote, branch } => Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::SetUpstreamBranch { branch, upstream } => Msg::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        },
        RepoCommandKind::UnsetUpstreamBranch { branch } => {
            Msg::UnsetUpstreamBranch { repo_id, branch }
        }
        RepoCommandKind::DeleteRemoteBranch { remote, branch } => Msg::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::DeleteRemoteBranches { remote, branches } => Msg::DeleteRemoteBranches {
            repo_id,
            remote,
            branches,
        },
        RepoCommandKind::Reset { mode, target } => Msg::Reset {
            repo_id,
            target,
            mode,
        },
        RepoCommandKind::SquashCommits {
            oldest,
            expected_head,
            message,
            count,
        } => Msg::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        },
        RepoCommandKind::Rebase { onto } => Msg::Rebase { repo_id, onto },
        RepoCommandKind::RebaseContinue => Msg::RebaseContinue { repo_id },
        RepoCommandKind::RebaseAbort => Msg::RebaseAbort { repo_id },
        // Sequencer commands only reach an auth prompt through a signing
        // passphrase failure, and by then git has already left cherry-pick
        // or rebase state on disk: replaying the original plan would be
        // rejected as already in progress (and its effect has no auth slot).
        // Continue the paused sequencer with the staged auth instead.
        RepoCommandKind::InteractiveCherryPick { commit: true, .. } => {
            Msg::RebaseContinue { repo_id }
        }
        // Uncommitted picks never sign or leave a sequencer; replay them, and
        // the steps that already landed merge again as no-ops.
        RepoCommandKind::InteractiveCherryPick {
            entries,
            commit: false,
        } => Msg::InteractiveCherryPick {
            repo_id,
            entries,
            commit: false,
        },
        // Replayed whole, like revert: a pick beside staged work rolls back a
        // failed commit step, and one stopped at its commit step resumes there.
        RepoCommandKind::CherryPick {
            commit_id,
            commit,
            mainline,
            summary,
        } => Msg::CherryPickCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
        },
        // Replayed whole: the auth may be for the `--no-commit` step (a
        // promisor fetch), and a revert stopped at its commit step resumes
        // there with the same hooks skipped, which `revert --continue` would not.
        RepoCommandKind::Revert {
            commit_id,
            commit,
            mainline,
            summary,
        } => Msg::RevertCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
        },
        // Only a command with the failed commit's checkpoint can skip apply.
        RepoCommandKind::ApplyFileChange {
            target,
            commit,
            commit_retry,
        } => Msg::ApplyFileChange {
            repo_id,
            target,
            commit,
            commit_retry,
        },
        RepoCommandKind::MergeAbort => Msg::MergeAbort { repo_id },
        RepoCommandKind::CreateTag {
            name,
            target,
            message,
            annotated,
        } => Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        },
        RepoCommandKind::DeleteTag { name } => Msg::DeleteTag { repo_id, name },
        RepoCommandKind::PushTag { remote, name } => Msg::PushTag {
            repo_id,
            remote,
            name,
        },
        RepoCommandKind::DeleteRemoteTag { remote, name } => Msg::DeleteRemoteTag {
            repo_id,
            remote,
            name,
        },
        RepoCommandKind::AddRemote { name, url } => Msg::AddRemote { repo_id, name, url },
        RepoCommandKind::RemoveRemote { name } => Msg::RemoveRemote { repo_id, name },
        RepoCommandKind::SetRemoteUrl { name, url, kind } => Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        },
        RepoCommandKind::CheckoutConflict { path, side } => Msg::CheckoutConflictSide {
            repo_id,
            path,
            side,
        },
        RepoCommandKind::AcceptConflictDeletion { path } => {
            Msg::AcceptConflictDeletion { repo_id, path }
        }
        RepoCommandKind::CheckoutConflictBase { path } => {
            Msg::CheckoutConflictBase { repo_id, path }
        }
        RepoCommandKind::LaunchMergetool { path } => Msg::LaunchMergetool { repo_id, path },
        RepoCommandKind::ExportPatch { commit_id, dest } => Msg::ExportPatch {
            repo_id,
            commit_id,
            dest,
        },
        RepoCommandKind::ApplyPatch { patch } => Msg::ApplyPatch { repo_id, patch },
        RepoCommandKind::AddWorktree { path, reference } => Msg::AddWorktree {
            repo_id,
            path,
            reference,
        },
        RepoCommandKind::RemoveWorktree { path } => Msg::RemoveWorktree { repo_id, path },
        RepoCommandKind::ForceRemoveWorktree { path } => Msg::ForceRemoveWorktree { repo_id, path },
        RepoCommandKind::AddSubmodule {
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        } => Msg::AddSubmoduleTrusted {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        },
        RepoCommandKind::UpdateSubmodules { approved_sources } => Msg::UpdateSubmodulesTrusted {
            repo_id,
            approved_sources,
        },
        RepoCommandKind::LoadSubmodule {
            path,
            approved_sources,
        } => Msg::LoadSubmoduleTrusted {
            repo_id,
            path,
            approved_sources,
        },
        RepoCommandKind::ChangeSubmodulePointer { path, reference } => {
            Msg::ChangeSubmodulePointer {
                repo_id,
                path,
                reference,
            }
        }
        RepoCommandKind::RemoveSubmodule { path } => Msg::RemoveSubmodule { repo_id, path },
        // A signing failure mid-rebase leaves git's state (and GitComet's
        // persisted reword messages) on disk; continue it with the staged
        // auth like the cherry-pick commands above.
        RepoCommandKind::InteractiveRebase { .. } => Msg::RebaseContinue { repo_id },
        // Writes `.gitignore` on the local filesystem, so it never fails for
        // want of credentials — and this replay path exists only to re-run a
        // command after an auth prompt. Retaining `patterns` would make a replay
        // possible; there is just nothing here that an auth prompt could fix.
        RepoCommandKind::AppendGitignorePatterns { .. }
        | RepoCommandKind::AppendGitattributesRule { .. } => return None,
        // Not replayable because command metadata does not retain original content.
        RepoCommandKind::SaveWorktreeFile { .. }
        | RepoCommandKind::StageHunk
        | RepoCommandKind::UnstageHunk
        | RepoCommandKind::ApplyWorktreePatch { .. } => return None,
    })
}

fn attach_git_auth_to_effects(mut effects: Vec<Effect>, auth: StagedGitAuth) -> Vec<Effect> {
    let Some(first) = effects.first_mut() else {
        return effects;
    };

    match first {
        Effect::CloneRepo { auth: slot, .. }
        | Effect::AddSubmodule { auth: slot, .. }
        | Effect::UpdateSubmodules { auth: slot, .. }
        | Effect::LoadSubmodule { auth: slot, .. }
        | Effect::Commit { auth: slot, .. }
        | Effect::CommitAmend { auth: slot, .. }
        | Effect::SafePushAfterCommit { auth: slot, .. }
        | Effect::FetchAll { auth: slot, .. }
        | Effect::FetchRefspecs { auth: slot, .. }
        | Effect::Pull { auth: slot, .. }
        | Effect::PullBranch { auth: slot, .. }
        | Effect::PushWithTags { auth: slot, .. }
        | Effect::Push { auth: slot, .. }
        | Effect::PushAfterCommit { auth: slot, .. }
        | Effect::ForcePush { auth: slot, .. }
        | Effect::ForcePushWithLease { auth: slot, .. }
        | Effect::PushSetUpstream { auth: slot, .. }
        | Effect::DeleteRemoteBranch { auth: slot, .. }
        | Effect::DeleteRemoteBranches { auth: slot, .. }
        | Effect::PushTag { auth: slot, .. }
        | Effect::DeleteRemoteTag { auth: slot, .. }
        | Effect::RebaseContinue { auth: slot, .. }
        | Effect::CherryPickCommit { auth: slot, .. }
        | Effect::RevertCommit { auth: slot, .. }
        | Effect::ApplyFileChange { auth: slot, .. } => {
            *slot = Some(auth);
        }
        _ => {}
    }

    effects
}

pub(super) fn submit_auth_prompt(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    username: Option<String>,
    secret: String,
) -> Vec<Effect> {
    let Some(prompt) = state.auth_prompt.take() else {
        return Vec::new();
    };

    let username = username
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let auth = match prepare_staged_git_auth(prompt.kind, username.as_deref(), &secret) {
        Ok(auth) => auth,
        Err(err) => {
            state.auth_prompt = Some(prompt);
            return if let Some(repo_state) = state
                .active_repo
                .and_then(|repo_id| state.repos.iter_mut().find(|r| r.id == repo_id))
            {
                util::push_diagnostic(
                    repo_state,
                    crate::model::DiagnosticKind::Error,
                    util::format_error_for_user(&err),
                );
                Vec::new()
            } else {
                Vec::new()
            };
        }
    };

    match retry_msg_for_auth_operation(prompt.operation) {
        Some(msg) => attach_git_auth_to_effects(reduce(repos, id_alloc, state, msg), auth),
        None => Vec::new(),
    }
}

pub(super) fn cancel_auth_prompt(state: &mut AppState) -> Vec<Effect> {
    state.auth_prompt = None;
    clear_staged_git_auth_env();
    Vec::new()
}

pub(super) fn clone_repo_finished(
    state: &mut AppState,
    url: String,
    dest: PathBuf,
    result: Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let auth_prompt = result
        .as_ref()
        .err()
        .and_then(|error| auth_prompt_for_clone(&url, &dest, error));
    let effects = repo_management::clone_repo_finished(state, url, dest, result);
    if let Some(prompt) = auth_prompt {
        clear_staged_git_auth_env();
        state.auth_prompt = Some(prompt);
    }
    effects
}

pub(super) fn commit(
    state: &mut AppState,
    repo_id: RepoId,
    message: String,
    push_after_commit: bool,
) -> Vec<Effect> {
    begin_commit_action(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.pending.commit_retry = Some(PendingCommitRetry {
            message: message.clone(),
            amend: false,
            push_after_commit,
        });
    }
    actions_emit_effects::commit(repo_id, message)
}

pub(super) fn commit_amend(
    state: &mut AppState,
    repo_id: RepoId,
    message: String,
    push_after_commit: bool,
) -> Vec<Effect> {
    begin_commit_action(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.pending.commit_retry = Some(PendingCommitRetry {
            message: message.clone(),
            amend: true,
            push_after_commit,
        });
    }
    actions_emit_effects::commit_amend(repo_id, message)
}

pub(super) fn commit_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<CommitOperationOutcome, Error>,
) -> Vec<Effect> {
    let pending_commit = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.pending.commit_retry.clone());
    let outcome = result.as_ref().ok().cloned();
    let push_after_commit = outcome.is_some()
        && pending_commit
            .as_ref()
            .is_some_and(|pending| pending.push_after_commit);
    let auth_prompt = result
        .as_ref()
        .err()
        .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit.clone(), error));
    let commit_result = result.map(|_| ());
    let mut effects = actions_emit_effects::commit_finished(state, repo_id, commit_result);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.pending.commit_retry = None;
    }
    if let Some(prompt) = auth_prompt {
        clear_staged_git_auth_env();
        state.auth_prompt = Some(prompt);
    }
    if push_after_commit && let (Some(outcome), Some(pending_commit)) = (outcome, pending_commit) {
        effects.extend(actions_emit_effects::safe_push_after_commit(
            repo_id,
            SafePushAfterCommitContext {
                amend: pending_commit.amend,
                local_branch: outcome.local_branch,
                pre_head: outcome.pre_head,
                post_head: outcome.post_head,
            },
        ));
    }
    effects
}

pub(super) fn commit_amend_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<CommitOperationOutcome, Error>,
) -> Vec<Effect> {
    let pending_commit = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.pending.commit_retry.clone());
    let outcome = result.as_ref().ok().cloned();
    let push_after_commit = outcome.is_some()
        && pending_commit
            .as_ref()
            .is_some_and(|pending| pending.push_after_commit);
    let auth_prompt = result
        .as_ref()
        .err()
        .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit.clone(), error));
    let commit_result = result.map(|_| ());
    let mut effects = actions_emit_effects::commit_amend_finished(state, repo_id, commit_result);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.pending.commit_retry = None;
    }
    if let Some(prompt) = auth_prompt {
        clear_staged_git_auth_env();
        state.auth_prompt = Some(prompt);
    }
    if push_after_commit && let (Some(outcome), Some(pending_commit)) = (outcome, pending_commit) {
        effects.extend(actions_emit_effects::safe_push_after_commit(
            repo_id,
            SafePushAfterCommitContext {
                amend: pending_commit.amend,
                local_branch: outcome.local_branch,
                pre_head: outcome.pre_head,
                post_head: outcome.post_head,
            },
        ));
    }
    effects
}

pub(super) fn safe_push_after_commit_finished(
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    context: SafePushAfterCommitContext,
    auth: Option<StagedGitAuth>,
    result: Result<SafePushAfterCommitDecision, Error>,
) -> Vec<Effect> {
    let auth_prompt = result
        .as_ref()
        .err()
        .and_then(|error| auth_prompt_for_safe_push_after_commit(repo_id, context.clone(), error));
    let effects =
        actions_emit_effects::safe_push_after_commit_finished(repos, state, repo_id, auth, result);
    if let Some(prompt) = auth_prompt {
        clear_staged_git_auth_env();
        state.auth_prompt = Some(prompt);
    }
    effects
}

pub(super) fn repo_command_finished(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    command: RepoCommandKind,
    result: Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let auth_prompt = result
        .as_ref()
        .err()
        .and_then(|error| auth_prompt_for_repo_command(repo_id, &command, error));
    let removed_worktree_path = match (&command, &result) {
        (RepoCommandKind::RemoveWorktree { path }, Ok(_)) => Some(path.clone()),
        (RepoCommandKind::ForceRemoveWorktree { path }, Ok(_)) => Some(path.clone()),
        _ => None,
    };
    // Their start cleared the HEAD gitlink cache; reclassify the
    // retained selection before the completion reloads it.
    if matches!(
        &command,
        RepoCommandKind::CherryPick { .. }
            | RepoCommandKind::InteractiveCherryPick { .. }
            | RepoCommandKind::Revert { .. }
    ) {
        refresh_selected_head_gitlink(repos, state, repo_id);
    }

    let effects = actions_emit_effects::repo_command_finished(state, repo_id, command, result);

    if let Some(path) = removed_worktree_path {
        let repo_ids_to_close = state
            .repos
            .iter()
            .filter(|repo| repo.spec.workdir == path)
            .map(|repo| repo.id)
            .collect::<Vec<_>>();
        for repo_id in repo_ids_to_close {
            let _ = repo_management::close_repo(repos, state, repo_id);
        }
    }

    if let Some(prompt) = auth_prompt {
        clear_staged_git_auth_env();
        state.auth_prompt = Some(prompt);
    }

    effects
}

// Recognizing auth failures and staging the credentials the user enters.

pub(super) fn detect_auth_prompt_kind(error: &Error) -> Option<AuthPromptKind> {
    match error.kind() {
        ErrorKind::Git(failure) => detect_auth_prompt_kind_from_git_failure(failure),
        ErrorKind::Backend(message) => detect_auth_prompt_kind_from_message(message),
        _ => None,
    }
}

pub(super) fn detect_auth_prompt_kind_from_message(message: &str) -> Option<AuthPromptKind> {
    let lower = message.to_ascii_lowercase();

    let host_verification = lower.contains("host key verification failed")
        || lower.contains("the authenticity of host")
        || lower.contains("this key is not known by any other names")
        || (lower.contains("are you sure you want to continue connecting")
            && lower.contains("yes/no"));
    if host_verification {
        return Some(AuthPromptKind::HostVerification);
    }

    let passphrase = lower.contains("could not read passphrase")
        // OpenSSH uses "for key '<path>'", while ssh-keygen signing uses
        // "for \"<path>\"".
        || lower.contains("enter passphrase for")
        || lower.contains("read_passphrase")
        || lower.contains("passphrase for key")
        || lower.contains("incorrect passphrase supplied to decrypt private key")
        || lower.contains(&SSH_PASSPHRASE_PROMPT_MARKER.to_ascii_lowercase())
        || (lower.contains("passphrase") && lower.contains("terminal prompts disabled"));
    let ssh_publickey = lower.contains("permission denied (publickey")
        || (lower.contains("could not read from remote repository") && lower.contains("publickey"));
    if passphrase || ssh_publickey {
        return Some(AuthPromptKind::Passphrase);
    }

    let user_password = lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("authentication failed")
        || lower.contains("invalid username or password")
        || lower.contains("http basic: access denied")
        || (lower.contains("terminal prompts disabled")
            && (lower.contains("https://")
                || lower.contains("http://")
                || lower.contains("username")
                || lower.contains("password")));
    if user_password {
        return Some(AuthPromptKind::UsernamePassword);
    }

    None
}

pub(super) fn clear_staged_git_auth_env() {
    clear_staged_git_auth();
}

pub(super) fn prepare_staged_git_auth(
    kind: AuthPromptKind,
    username: Option<&str>,
    secret: &str,
) -> Result<StagedGitAuth, Error> {
    let normalized_secret = match kind {
        AuthPromptKind::HostVerification => {
            let trimmed = secret.trim();
            if trimmed.eq_ignore_ascii_case("yes") {
                "yes".to_string()
            } else {
                trimmed.to_string()
            }
        }
        AuthPromptKind::UsernamePassword | AuthPromptKind::Passphrase => secret.to_string(),
    };

    if normalized_secret.trim().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "credential/passphrase/confirmation cannot be empty".to_string(),
        )));
    }
    if kind.requires_username() && username.unwrap_or_default().trim().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "username cannot be empty".to_string(),
        )));
    }

    Ok(StagedGitAuth {
        kind: match kind {
            AuthPromptKind::UsernamePassword => GitAuthKind::UsernamePassword,
            AuthPromptKind::Passphrase => GitAuthKind::Passphrase,
            AuthPromptKind::HostVerification => GitAuthKind::HostVerification,
        },
        username: username.map(ToOwned::to_owned),
        secret: normalized_secret,
    })
}

#[cfg(test)]
pub(super) fn stage_git_auth_env(
    kind: AuthPromptKind,
    username: Option<&str>,
    secret: &str,
) -> Result<(), Error> {
    stage_git_auth(prepare_staged_git_auth(kind, username, secret)?);
    Ok(())
}

fn detect_auth_prompt_kind_from_git_failure(failure: &GitFailure) -> Option<AuthPromptKind> {
    let stderr = String::from_utf8_lossy(failure.stderr());
    detect_auth_prompt_kind_from_message(&stderr)
        .or_else(|| {
            detect_auth_prompt_kind_from_message(&String::from_utf8_lossy(failure.stdout()))
        })
        .or_else(|| detect_auth_prompt_kind_from_message(&failure.to_string()))
}
