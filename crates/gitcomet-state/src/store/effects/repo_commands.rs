use crate::msg::{InternalMsg, Msg, RepoCommandKind};
use gitcomet_core::auth::{ScopedStagedGitAuth, StagedGitAuth};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, ConflictSide, ForcePushLease, GitRepository, InteractiveRebaseEntry, PullMode,
    RemoteUrlKind, ResetMode, SafePushAfterCommitContext, SafePushAfterCommitTarget,
    SubmoduleTrustTarget,
};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{
    GitOperationTask, RepoMap, message_subject, send_or_log, short_commit_id, single_line_context,
    spawn_with_repo,
};

const GITIGNORE_FILE_NAME: &str = gitcomet_core::gitignore::FILE_NAME;

fn pull_mode_suffix(mode: PullMode) -> Option<&'static str> {
    match mode {
        PullMode::Default => None,
        PullMode::Merge => Some("merge"),
        PullMode::FastForwardIfPossible => Some("fast-forward if possible"),
        PullMode::FastForwardOnly => Some("fast-forward only"),
        PullMode::Rebase => Some("rebase"),
    }
}

fn repo_command_context(command: &RepoCommandKind) -> Option<String> {
    let context = match command {
        RepoCommandKind::FetchAll => "All remotes".to_string(),
        RepoCommandKind::PruneMergedBranches => "Merged local branches".to_string(),
        RepoCommandKind::PruneLocalTags => "Local tags missing on remotes".to_string(),
        RepoCommandKind::Pull { mode } => pull_mode_suffix(*mode).map_or_else(
            || "Configured upstream → current branch".to_string(),
            |mode| format!("Configured upstream → current branch · {mode}"),
        ),
        RepoCommandKind::PullBranch { remote, branch } => {
            format!("{remote}/{branch} → current branch")
        }
        RepoCommandKind::MergeRef { reference } => format!("{reference} → current branch"),
        RepoCommandKind::SquashRef { reference } => reference.clone(),
        RepoCommandKind::Push | RepoCommandKind::ForcePush => {
            "Current branch → configured upstream".to_string()
        }
        RepoCommandKind::PushAfterCommit { target, .. } => {
            format!(
                "{} → {}/{}",
                target.local_branch, target.remote, target.branch
            )
        }
        RepoCommandKind::ForcePushWithLease { lease } => {
            format!("{} → {}/{}", lease.local_branch, lease.remote, lease.branch)
        }
        RepoCommandKind::PushSetUpstream { remote, branch } => {
            format!("Current branch → {remote}/{branch}")
        }
        RepoCommandKind::SetUpstreamBranch { branch, upstream } => {
            format!("{branch} → {upstream}")
        }
        RepoCommandKind::UnsetUpstreamBranch { branch } => branch.clone(),
        RepoCommandKind::DeleteRemoteBranch { remote, branch } => {
            format!("{remote}/{branch}")
        }
        RepoCommandKind::DeleteRemoteBranches { remote, branches } => format!(
            "{remote}: {}",
            crate::name_summary::elide_names(branches, ", ")
        ),
        RepoCommandKind::Reset { mode, target } => {
            let mode = match mode {
                ResetMode::Soft => "soft",
                ResetMode::Mixed => "mixed",
                ResetMode::Hard => "hard",
            };
            format!("{target} · {mode}")
        }
        RepoCommandKind::SquashCommits { message, count, .. } => {
            let subject = message_subject(message).unwrap_or_else(|| "No commit message".into());
            format!("{count} commits · {subject}")
        }
        RepoCommandKind::Rebase { onto } => format!("Current branch onto {onto}"),
        RepoCommandKind::RebaseContinue => "Current rebase".to_string(),
        RepoCommandKind::RebaseAbort => "Current rebase".to_string(),
        RepoCommandKind::InteractiveRebase { base, .. } => format!("Current branch onto {base}"),
        RepoCommandKind::InteractiveCherryPick { entries } => match entries.as_slice() {
            [] => "Selected commits".to_string(),
            [entry] => {
                message_subject(&entry.summary).unwrap_or_else(|| short_commit_id(&entry.commit_id))
            }
            [first, ..] => format!(
                "{} commits · {}",
                entries.len(),
                message_subject(&first.summary)
                    .unwrap_or_else(|| short_commit_id(&first.commit_id))
            ),
        },
        RepoCommandKind::CherryPick {
            commit_id, summary, ..
        } => message_subject(summary).unwrap_or_else(|| short_commit_id(commit_id.as_ref())),
        RepoCommandKind::MergeAbort => "Current merge".to_string(),
        RepoCommandKind::CreateTag { name, target, .. } => format!("{name} at {target}"),
        RepoCommandKind::DeleteTag { name } => name.clone(),
        RepoCommandKind::PushTag { remote, name } => format!("{name} → {remote}"),
        RepoCommandKind::DeleteRemoteTag { remote, name } => format!("{remote}/{name}"),
        RepoCommandKind::AddRemote { name, .. }
        | RepoCommandKind::RemoveRemote { name }
        | RepoCommandKind::SetRemoteUrl { name, .. } => name.clone(),
        RepoCommandKind::CheckoutConflict { path, side } => {
            let side = match side {
                ConflictSide::Ours => "ours",
                ConflictSide::Theirs => "theirs",
            };
            format!("{} · {side}", path.display())
        }
        RepoCommandKind::AcceptConflictDeletion { path }
        | RepoCommandKind::CheckoutConflictBase { path }
        | RepoCommandKind::LaunchMergetool { path } => path.display().to_string(),
        RepoCommandKind::SaveWorktreeFile { path, stage } => format!(
            "{}{}",
            path.display(),
            if *stage { " · stage after saving" } else { "" }
        ),
        RepoCommandKind::AppendGitignorePatterns { patterns } => match patterns.as_slice() {
            [] => GITIGNORE_FILE_NAME.to_string(),
            [pattern] => pattern.clone(),
            many => format!(
                "{} patterns: {}",
                many.len(),
                crate::name_summary::elide_names(many, ", ")
            ),
        },
        RepoCommandKind::ExportPatch { commit_id, dest } => {
            format!(
                "{} → {}",
                short_commit_id(commit_id.as_ref()),
                dest.display()
            )
        }
        RepoCommandKind::ApplyPatch { patch } => patch.display().to_string(),
        RepoCommandKind::AddWorktree { path, reference } => reference.as_ref().map_or_else(
            || path.display().to_string(),
            |reference| format!("{reference} → {}", path.display()),
        ),
        RepoCommandKind::RemoveWorktree { path }
        | RepoCommandKind::ForceRemoveWorktree { path } => path.display().to_string(),
        RepoCommandKind::AddSubmodule {
            path, branch, name, ..
        } => {
            let mut detail = name.clone().unwrap_or_else(|| path.display().to_string());
            if let Some(branch) = branch {
                detail.push_str(" · ");
                detail.push_str(branch);
            }
            detail
        }
        RepoCommandKind::UpdateSubmodules { .. } => "All submodules".to_string(),
        RepoCommandKind::LoadSubmodule { path, .. } | RepoCommandKind::RemoveSubmodule { path } => {
            path.display().to_string()
        }
        RepoCommandKind::ChangeSubmodulePointer { path, reference } => {
            format!("{} → {reference}", path.display())
        }
        RepoCommandKind::StageHunk
        | RepoCommandKind::UnstageHunk
        | RepoCommandKind::ApplyWorktreePatch { .. } => "Selected hunk".to_string(),
    };
    single_line_context(context)
}

fn schedule_repo_command_with_context<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    command: RepoCommandKind,
    context_override: Option<String>,
    run: F,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<CommandOutput, Error> + Send + 'static,
{
    let label = command.hook_activity_label();
    let context = context_override.or_else(|| repo_command_context(&command));
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation = GitOperationTask::start(repo_id, label, context, &msg_tx);
        let result = {
            let _scope = operation.attach();
            run(repo)
        };
        let outcome = GitOperationTask::outcome(&result);
        operation.finish(
            outcome,
            InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result,
            },
        );
    });
}

fn schedule_repo_command<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    command: RepoCommandKind,
    run: F,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<CommandOutput, Error> + Send + 'static,
{
    schedule_repo_command_with_context(executor, repos, msg_tx, repo_id, command, None, run);
}

fn normalize_worktree_relative_path(path: &Path) -> Result<PathBuf, Error> {
    const OUTSIDE_WORKDIR_ERROR: &str = "refusing to write outside repository workdir";
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(Error::new(ErrorKind::Backend(
                        OUTSIDE_WORKDIR_ERROR.to_string(),
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::new(ErrorKind::Backend(
                    OUTSIDE_WORKDIR_ERROR.to_string(),
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(Error::new(ErrorKind::Backend(
            "worktree file path must not be empty".to_string(),
        )));
    }
    Ok(normalized)
}

/// Where a worktree save for `path` lands, or why it must not.
///
/// Containment is checked twice: lexically, so `..` cannot leave the workdir,
/// and on the filesystem, so a symlink cannot redirect the write outside it.
fn resolve_worktree_save_target(workdir: &Path, path: &Path) -> Result<(PathBuf, PathBuf), Error> {
    let relative_path = normalize_worktree_relative_path(path)?;
    let full = gitcomet_core::path_utils::symlink_free_write_target(workdir, &relative_path)
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::InvalidInput => Error::new(ErrorKind::Backend(err.to_string())),
            kind => Error::new(ErrorKind::Io(kind)),
        })?;
    Ok((relative_path, full))
}

fn run_with_git_auth<R>(
    auth: Option<StagedGitAuth>,
    run: impl FnOnce() -> Result<R, Error>,
) -> Result<R, Error> {
    if let Some(auth) = auth {
        // The guard clears the staged auth on success, error, and panic.
        let _scoped = ScopedStagedGitAuth::stage(auth);
        run()
    } else {
        run()
    }
}

pub(super) struct AddSubmoduleRequest {
    pub(super) url: String,
    pub(super) path: PathBuf,
    pub(super) branch: Option<String>,
    pub(super) name: Option<String>,
    pub(super) force: bool,
    pub(super) approved_sources: Vec<SubmoduleTrustTarget>,
    pub(super) auth: Option<StagedGitAuth>,
}

pub(super) fn schedule_save_worktree_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    contents: String,
    stage: bool,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::SaveWorktreeFile {
            path: command_path,
            stage,
        },
        move |repo| {
            let (relative_path, full) = resolve_worktree_save_target(&repo.spec().workdir, &path)?;
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            }
            std::fs::write(&full, contents.as_bytes())
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            if stage {
                let path_ref: &Path = &relative_path;
                repo.stage(&[path_ref])?;
            }
            Ok(CommandOutput {
                command: format!(
                    "Save {}{}",
                    relative_path.display(),
                    if stage { " (staged)" } else { "" }
                ),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
            })
        },
    );
}

/// Append patterns to the repository-root `.gitignore`.
///
/// The read and the write happen in the same worker closure on purpose. Doing
/// the read in the UI would mean holding the file's contents across a
/// user-visible dialog and then writing the whole thing back, clobbering any
/// competing edit from the file editor, an external editor, or another window.
pub(super) fn schedule_append_gitignore_patterns(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    patterns: Vec<String>,
) {
    let command_patterns = patterns.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AppendGitignorePatterns {
            patterns: command_patterns,
        },
        move |repo| {
            // Routed through the same guard as every other worktree write, even
            // though the name is a constant, so the invariant holds by
            // construction rather than by reading this function.
            let relative_path = normalize_worktree_relative_path(Path::new(GITIGNORE_FILE_NAME))?;
            let full = repo.spec().workdir.join(&relative_path);

            // Read bytes and convert explicitly: `read_to_string` would report a
            // Latin-1 `.gitignore` as a bare `InvalidData` I/O error, which tells
            // the user nothing about what to do next. Never rewrite it lossily.
            let existing = match std::fs::read(&full) {
                Ok(bytes) => String::from_utf8(bytes).map_err(|_| {
                    Error::new(ErrorKind::Backend(format!(
                        "{GITIGNORE_FILE_NAME} is not valid UTF-8; edit it manually"
                    )))
                })?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(Error::new(ErrorKind::Io(e.kind()))),
            };

            let Some(updated) = gitcomet_core::gitignore::append_patterns(&existing, &patterns)
            else {
                return Ok(CommandOutput {
                    command: format!("Update {GITIGNORE_FILE_NAME}"),
                    stdout: gitcomet_core::gitignore::NOTHING_TO_ADD.to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                });
            };

            std::fs::write(&full, updated.as_bytes())
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;

            Ok(CommandOutput {
                command: format!("Update {GITIGNORE_FILE_NAME}"),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
            })
        },
    );
}

pub(super) fn schedule_export_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    dest: PathBuf,
) {
    let command_commit_id = commit_id.clone();
    let command_dest = dest.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ExportPatch {
            commit_id: command_commit_id,
            dest: command_dest,
        },
        move |repo| repo.export_patch_with_output(&commit_id, &dest),
    );
}

pub(super) fn schedule_apply_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    patch: PathBuf,
) {
    let command_patch = patch.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ApplyPatch {
            patch: command_patch,
        },
        move |repo| repo.apply_patch_with_output(&patch),
    );
}

pub(super) fn schedule_add_worktree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    reference: Option<String>,
) {
    let command_path = path.clone();
    let command_reference = reference.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AddWorktree {
            path: command_path,
            reference: command_reference,
        },
        move |repo| repo.add_worktree_with_output(&path, reference.as_deref()),
    );
}

pub(super) fn schedule_remove_worktree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RemoveWorktree { path: command_path },
        move |repo| repo.remove_worktree_with_output(&path),
    );
}

pub(super) fn schedule_force_remove_worktree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ForceRemoveWorktree { path: command_path },
        move |repo| repo.force_remove_worktree_with_output(&path),
    );
}

pub(super) fn schedule_add_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    request: AddSubmoduleRequest,
) {
    let AddSubmoduleRequest {
        url,
        path,
        branch,
        name,
        force,
        approved_sources,
        auth,
    } = request;
    let command_url = url.clone();
    let command_path = path.clone();
    let command_branch = branch.clone();
    let command_name = name.clone();
    let command_sources = approved_sources.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AddSubmodule {
            url: command_url,
            path: command_path,
            branch: command_branch,
            name: command_name,
            force,
            approved_sources: command_sources,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.add_submodule_with_output(
                    &url,
                    &path,
                    branch.as_deref(),
                    name.as_deref(),
                    force,
                    &approved_sources,
                )
            })
        },
    );
}

pub(super) fn schedule_update_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    approved_sources: Vec<SubmoduleTrustTarget>,
    auth: Option<StagedGitAuth>,
) {
    let command_sources = approved_sources.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::UpdateSubmodules {
            approved_sources: command_sources,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.update_submodules_with_output(&approved_sources)
            })
        },
    );
}

pub(super) fn schedule_check_submodule_add_trust(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.check_submodule_add_trust(&url, &path);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                result,
            }),
        );
    });
}

pub(super) fn schedule_check_submodule_update_trust(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.check_submodule_update_trust();
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked { repo_id, result }),
        );
    });
}

pub(super) fn schedule_check_submodule_load_trust(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.check_submodule_load_trust(&path);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleLoadTrustChecked {
                repo_id,
                path,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    approved_sources: Vec<SubmoduleTrustTarget>,
    auth: Option<StagedGitAuth>,
) {
    let command_path = path.clone();
    let command_sources = approved_sources.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::LoadSubmodule {
            path: command_path,
            approved_sources: command_sources,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.load_submodule_with_output(&path, &approved_sources)
            })
        },
    );
}

pub(super) fn schedule_change_submodule_pointer(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    reference: String,
) {
    let command_path = path.clone();
    let command_reference = reference.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ChangeSubmodulePointer {
            path: command_path,
            reference: command_reference,
        },
        move |repo| repo.change_submodule_pointer_with_output(&path, &reference),
    );
}

pub(super) fn schedule_remove_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RemoveSubmodule { path: command_path },
        move |repo| repo.remove_submodule_with_output(&path),
    );
}

pub(super) fn schedule_stage_hunk(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    patch: String,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::StageHunk,
        move |repo| repo.apply_unified_patch_to_index_with_output(&patch, false),
    );
}

pub(super) fn schedule_unstage_hunk(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    patch: String,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::UnstageHunk,
        move |repo| repo.apply_unified_patch_to_index_with_output(&patch, true),
    );
}

pub(super) fn schedule_apply_worktree_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    patch: String,
    reverse: bool,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ApplyWorktreePatch { reverse },
        move |repo| repo.apply_unified_patch_to_worktree_with_output(&patch, reverse),
    );
}

pub(super) fn schedule_fetch_all(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    prune: bool,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::FetchAll,
        move |repo| run_with_git_auth(auth, || repo.fetch_all_with_output_prune(prune)),
    );
}

pub(super) fn schedule_prune_merged_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PruneMergedBranches,
        |repo| repo.prune_merged_branches_with_output(),
    );
}

pub(super) fn schedule_prune_local_tags(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PruneLocalTags,
        |repo| repo.prune_local_tags_with_output(),
    );
}

pub(super) fn schedule_pull(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    mode: PullMode,
    prune: bool,
    tracking: Option<(String, String)>,
    auth: Option<StagedGitAuth>,
) {
    let command = RepoCommandKind::Pull { mode };
    let context = tracking.map(|(local, upstream)| {
        pull_mode_suffix(mode).map_or_else(
            || format!("{upstream} → {local}"),
            |mode| format!("{upstream} → {local} · {mode}"),
        )
    });
    schedule_repo_command_with_context(
        executor,
        repos,
        msg_tx,
        repo_id,
        command,
        context,
        move |repo| run_with_git_auth(auth, || repo.pull_with_output_prune(mode, prune)),
    );
}

pub(super) fn schedule_pull_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branch: String,
    prune: bool,
    local_branch: Option<String>,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_branch = branch.clone();
    let context = local_branch.map(|local| format!("{remote}/{branch} → {local}"));
    schedule_repo_command_with_context(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PullBranch {
            remote: command_remote,
            branch: command_branch,
        },
        context,
        move |repo| {
            run_with_git_auth(auth, || {
                repo.pull_branch_with_output_prune(&remote, &branch, prune)
            })
        },
    );
}

pub(super) fn schedule_merge_ref(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    reference: String,
) {
    let command_reference = reference.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::MergeRef {
            reference: command_reference,
        },
        move |repo| repo.merge_ref_with_output(&reference),
    );
}

pub(super) fn schedule_squash_ref(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    reference: String,
) {
    let command_reference = reference.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::SquashRef {
            reference: command_reference,
        },
        move |repo| repo.squash_ref_with_output(&reference),
    );
}

pub(super) fn schedule_push(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    tracking: Option<(String, String)>,
    auth: Option<StagedGitAuth>,
) {
    let context = tracking.map(|(local, upstream)| format!("{local} → {upstream}"));
    schedule_repo_command_with_context(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::Push,
        context,
        move |repo| run_with_git_auth(auth, || repo.push_with_output()),
    );
}

pub(super) fn schedule_push_after_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: SafePushAfterCommitTarget,
    set_upstream: bool,
    auth: Option<StagedGitAuth>,
) {
    let command_target = target.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PushAfterCommit {
            target: command_target,
            set_upstream,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                if set_upstream {
                    repo.push_after_commit_set_upstream_with_output(&target)
                } else {
                    repo.push_after_commit_with_output(&target)
                }
            })
        },
    );
}

pub(super) fn schedule_safe_push_after_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    context: SafePushAfterCommitContext,
    auth: Option<StagedGitAuth>,
) {
    let finish_context = context.clone();
    let follow_up_auth = auth.clone();
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = run_with_git_auth(auth, || repo.safe_push_after_commit(&context));
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
                repo_id,
                context: finish_context,
                auth: follow_up_auth,
                result,
            }),
        );
    });
}

pub(super) fn schedule_force_push(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    tracking: Option<(String, String)>,
    auth: Option<StagedGitAuth>,
) {
    let context = tracking.map(|(local, upstream)| format!("{local} → {upstream}"));
    schedule_repo_command_with_context(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ForcePush,
        context,
        move |repo| run_with_git_auth(auth, || repo.push_force_with_output()),
    );
}

pub(super) fn schedule_force_push_with_lease(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    lease: ForcePushLease,
    auth: Option<StagedGitAuth>,
) {
    let command_lease = lease.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ForcePushWithLease {
            lease: command_lease,
        },
        move |repo| run_with_git_auth(auth, || repo.push_force_with_lease_with_output(&lease)),
    );
}

pub(super) fn schedule_push_set_upstream(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branch: String,
    local_branch: Option<String>,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_branch = branch.clone();
    let context = local_branch.map(|local| format!("{local} → {remote}/{branch}"));
    schedule_repo_command_with_context(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PushSetUpstream {
            remote: command_remote,
            branch: command_branch,
        },
        context,
        move |repo| {
            run_with_git_auth(auth, || {
                repo.push_set_upstream_with_output(&remote, &branch)
            })
        },
    );
}

pub(super) fn schedule_set_upstream_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    branch: String,
    upstream: String,
) {
    let command_branch = branch.clone();
    let command_upstream = upstream.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::SetUpstreamBranch {
            branch: command_branch,
            upstream: command_upstream,
        },
        move |repo| repo.set_upstream_branch_with_output(&branch, &upstream),
    );
}

pub(super) fn schedule_unset_upstream_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    branch: String,
) {
    let command_branch = branch.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::UnsetUpstreamBranch {
            branch: command_branch,
        },
        move |repo| repo.unset_upstream_branch_with_output(&branch),
    );
}

pub(super) fn schedule_delete_remote_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branch: String,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_branch = branch.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::DeleteRemoteBranch {
            remote: command_remote,
            branch: command_branch,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.delete_remote_branch_with_output(&remote, &branch)
            })
        },
    );
}

pub(super) fn schedule_delete_remote_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branches: Vec<String>,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_branches = branches.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::DeleteRemoteBranches {
            remote: command_remote,
            branches: command_branches,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.delete_remote_branches_with_output(&remote, &branches)
            })
        },
    );
}

pub(super) fn schedule_reset(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: String,
    mode: ResetMode,
) {
    let command_target = target.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::Reset {
            mode,
            target: command_target,
        },
        move |repo| repo.reset_with_output(&target, mode),
    );
}

pub(super) fn schedule_squash_commits(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    oldest: gitcomet_core::domain::CommitId,
    expected_head: gitcomet_core::domain::CommitId,
    message: String,
    count: usize,
) {
    let command = RepoCommandKind::SquashCommits {
        oldest: oldest.clone(),
        expected_head: expected_head.clone(),
        message: message.clone(),
        count,
    };
    schedule_repo_command(executor, repos, msg_tx, repo_id, command, move |repo| {
        repo.squash_commits_with_output(&oldest, &expected_head, &message)
    });
}

pub(super) fn schedule_rebase(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    onto: String,
) {
    let command_onto = onto.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::Rebase { onto: command_onto },
        move |repo| repo.rebase_with_output(&onto),
    );
}

pub(super) fn schedule_rebase_continue(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RebaseContinue,
        move |repo| run_with_git_auth(auth, || repo.rebase_continue_with_output()),
    );
}

pub(super) fn schedule_rebase_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RebaseAbort,
        |repo| repo.rebase_abort_with_output(),
    );
}

pub(super) fn schedule_interactive_rebase(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    base: String,
    entries: Vec<InteractiveRebaseEntry>,
    interactive: bool,
) {
    let base_for_cmd = base.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::InteractiveRebase {
            base: base_for_cmd,
            interactive,
        },
        move |repo| repo.interactive_rebase_with_output(&base, &entries),
    );
}

pub(super) fn schedule_interactive_cherry_pick(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    entries: Vec<InteractiveRebaseEntry>,
) {
    let command_entries = entries.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::InteractiveCherryPick {
            entries: command_entries,
        },
        move |repo| repo.interactive_cherry_pick_with_output(&entries),
    );
}

pub(super) fn schedule_cherry_pick_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    commit: bool,
    mainline: Option<usize>,
    summary: String,
) {
    let command_commit_id = commit_id.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::CherryPick {
            commit_id: command_commit_id,
            commit,
            mainline,
            summary,
        },
        move |repo| repo.cherry_pick_with_output(&commit_id, commit, mainline),
    );
}

pub(super) fn schedule_merge_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::MergeAbort,
        |repo| repo.merge_abort_with_output(),
    );
}

pub(super) fn schedule_create_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    target: String,
    message: Option<String>,
    annotated: bool,
) {
    let command_name = name.clone();
    let command_target = target.clone();
    let command_message = message.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::CreateTag {
            name: command_name,
            target: command_target,
            message: command_message,
            annotated,
        },
        move |repo| repo.create_tag_with_output(&name, &target, message.as_deref(), annotated),
    );
}

pub(super) fn schedule_delete_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let command_name = name.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::DeleteTag { name: command_name },
        move |repo| repo.delete_tag_with_output(&name),
    );
}

pub(super) fn schedule_push_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    name: String,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_name = name.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PushTag {
            remote: command_remote,
            name: command_name,
        },
        move |repo| run_with_git_auth(auth, || repo.push_tag_with_output(&remote, &name)),
    );
}

pub(super) fn schedule_delete_remote_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    name: String,
    auth: Option<StagedGitAuth>,
) {
    let command_remote = remote.clone();
    let command_name = name.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::DeleteRemoteTag {
            remote: command_remote,
            name: command_name,
        },
        move |repo| run_with_git_auth(auth, || repo.delete_remote_tag_with_output(&remote, &name)),
    );
}

pub(super) fn schedule_add_remote(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    url: String,
) {
    let command_name = name.clone();
    let command_url = url.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AddRemote {
            name: command_name,
            url: command_url,
        },
        move |repo| repo.add_remote_with_output(&name, &url),
    );
}

pub(super) fn schedule_remove_remote(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let command_name = name.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RemoveRemote { name: command_name },
        move |repo| repo.remove_remote_with_output(&name),
    );
}

pub(super) fn schedule_set_remote_url(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    url: String,
    kind: RemoteUrlKind,
) {
    let command_name = name.clone();
    let command_url = url.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::SetRemoteUrl {
            name: command_name,
            url: command_url,
            kind,
        },
        move |repo| repo.set_remote_url_with_output(&name, &url, kind),
    );
}

pub(super) fn schedule_checkout_conflict_side(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    side: ConflictSide,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::CheckoutConflict {
            path: command_path,
            side,
        },
        move |repo| repo.checkout_conflict_side(&path, side),
    );
}

pub(super) fn schedule_checkout_conflict_base(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::CheckoutConflictBase { path: command_path },
        move |repo| repo.checkout_conflict_base(&path),
    );
}

pub(super) fn schedule_accept_conflict_deletion(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AcceptConflictDeletion { path: command_path },
        move |repo| repo.accept_conflict_deletion(&path),
    );
}

pub(super) fn schedule_launch_mergetool(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::LaunchMergetool { path: command_path },
        move |repo| {
            let result = repo.launch_mergetool(&path);
            match result {
                Ok(mergetool_result) => {
                    if mergetool_result.success {
                        Ok(CommandOutput {
                            command: format!("mergetool ({})", mergetool_result.tool_name),
                            stdout: mergetool_result.output.stdout,
                            stderr: mergetool_result.output.stderr,
                            exit_code: mergetool_result.output.exit_code,
                        })
                    } else {
                        Err(gitcomet_core::error::Error::new(
                            gitcomet_core::error::ErrorKind::Backend(format!(
                                "Mergetool '{}' did not complete successfully",
                                mergetool_result.tool_name
                            )),
                        ))
                    }
                }
                Err(e) => Err(e),
            }
        },
    );
}

#[cfg(test)]
mod worktree_save_target_tests {
    use super::resolve_worktree_save_target;
    use gitcomet_core::error::ErrorKind;
    use std::path::Path;

    #[test]
    fn nested_relative_path_resolves_under_workdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (relative, full) =
            resolve_worktree_save_target(dir.path(), Path::new("./src/../src/lib.rs"))
                .expect("nested path");
        assert_eq!(relative, Path::new("src/lib.rs"));
        assert_eq!(full, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn parent_dir_escape_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = resolve_worktree_save_target(dir.path(), Path::new("../outside.txt"))
            .expect_err("lexical escape");
        assert!(matches!(err.kind(), ErrorKind::Backend(_)));
    }

    #[cfg(unix)]
    #[test]
    fn save_through_symlink_out_of_workdir_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside dir");
        let victim = outside.path().join("authorized_keys");
        std::fs::write(&victim, "original").expect("write victim");
        std::os::unix::fs::symlink(&victim, dir.path().join("notes.md")).expect("symlink");

        let err = resolve_worktree_save_target(dir.path(), Path::new("notes.md"))
            .expect_err("symlinked file");
        let ErrorKind::Backend(message) = err.kind() else {
            panic!("expected a backend refusal, got {err:?}");
        };
        assert!(message.contains("symlink"), "{message}");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim"),
            "original"
        );
    }
}

#[cfg(test)]
mod hook_activity_context_tests {
    use super::*;
    use gitcomet_core::domain::CommitId;

    #[test]
    fn network_and_history_commands_have_specific_context() {
        assert_eq!(
            repo_command_context(&RepoCommandKind::Pull {
                mode: PullMode::Rebase,
            })
            .as_deref(),
            Some("Configured upstream → current branch · rebase")
        );
        assert_eq!(
            repo_command_context(&RepoCommandKind::PushAfterCommit {
                target: SafePushAfterCommitTarget {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                    local_branch: "feature/hooks".to_string(),
                    local_head: CommitId("0123456789abcdef".into()),
                },
                set_upstream: false,
            })
            .as_deref(),
            Some("feature/hooks → origin/main")
        );
        assert_eq!(
            repo_command_context(&RepoCommandKind::SquashCommits {
                oldest: CommitId("1111111111111111".into()),
                expected_head: CommitId("2222222222222222".into()),
                message: "Squash subject\n\nBody".to_string(),
                count: 3,
            })
            .as_deref(),
            Some("3 commits · Squash subject")
        );
    }

    #[test]
    fn remote_context_does_not_retain_a_credential_bearing_url() {
        let context = repo_command_context(&RepoCommandKind::AddRemote {
            name: "origin".to_string(),
            url: "https://token@example.invalid/private.git".to_string(),
        });
        assert_eq!(context.as_deref(), Some("origin"));
        assert!(!context.unwrap().contains("token"));
    }
}
