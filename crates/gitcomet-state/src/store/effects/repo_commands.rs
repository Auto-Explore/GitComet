use crate::msg::{Msg, RepoCommandKind};
use gitcomet_core::auth::{
    StagedGitAuth, clear_staged_git_auth, stage_git_auth_for_current_thread,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CommandOutput, ConflictSide, GitRepository, PullMode, RemoteUrlKind, ResetMode,
};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, mpsc};

use super::super::{RepoId, executor::TaskExecutor};
use super::util::{RepoMap, send_or_log, spawn_with_repo};

fn schedule_repo_command<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    command: RepoCommandKind,
    run: F,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<CommandOutput, Error> + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = run(repo);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result,
            }),
        );
    });
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

fn run_with_git_auth<R>(
    auth: Option<StagedGitAuth>,
    run: impl FnOnce() -> Result<R, Error>,
) -> Result<R, Error> {
    if let Some(auth) = auth {
        stage_git_auth_for_current_thread(auth);
        let result = run();
        clear_staged_git_auth();
        result
    } else {
        run()
    }
}

pub(super) fn schedule_save_worktree_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
            let relative_path = normalize_worktree_relative_path(&path)?;
            let full = repo.spec().workdir.join(&relative_path);
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

pub(super) fn schedule_export_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    auth: Option<StagedGitAuth>,
) {
    let command_url = url.clone();
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AddSubmodule {
            url: command_url,
            path: command_path,
        },
        move |repo| run_with_git_auth(auth, || repo.add_submodule_with_output(&url, &path)),
    );
}

pub(super) fn schedule_update_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::UpdateSubmodules,
        move |repo| run_with_git_auth(auth, || repo.update_submodules_with_output()),
    );
}

pub(super) fn schedule_remove_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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

pub(super) fn schedule_add_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    repository: String,
    reference: String,
    path: PathBuf,
    squash: bool,
    auth: Option<StagedGitAuth>,
) {
    let command_repository = repository.clone();
    let command_reference = reference.clone();
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::AddSubtree {
            repository: command_repository,
            reference: command_reference,
            path: command_path,
            squash,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.add_subtree_with_output(&repository, &reference, &path, squash)
            })
        },
    );
}

pub(super) fn schedule_pull_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    repository: String,
    reference: String,
    path: PathBuf,
    squash: bool,
    auth: Option<StagedGitAuth>,
) {
    let command_repository = repository.clone();
    let command_reference = reference.clone();
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PullSubtree {
            repository: command_repository,
            reference: command_reference,
            path: command_path,
            squash,
        },
        move |repo| {
            run_with_git_auth(auth, || {
                repo.pull_subtree_with_output(&repository, &reference, &path, squash)
            })
        },
    );
}

pub(super) fn schedule_push_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    repository: String,
    refspec: String,
    path: PathBuf,
    auth: Option<StagedGitAuth>,
) {
    let command_repository = repository.clone();
    let command_refspec = refspec.clone();
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::PushSubtree {
            repository: command_repository,
            refspec: command_refspec,
            path: command_path,
        },
        move |repo| {
            run_with_git_auth(auth, || repo.push_subtree_with_output(&repository, &refspec, &path))
        },
    );
}

pub(super) fn schedule_split_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    branch: Option<String>,
) {
    let command_path = path.clone();
    let command_branch = branch.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::SplitSubtree {
            path: command_path,
            branch: command_branch,
        },
        move |repo| repo.split_subtree_with_output(&path, branch.as_deref()),
    );
}

pub(super) fn schedule_remove_subtree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    let command_path = path.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RemoveSubtree { path: command_path },
        move |repo| repo.remove_subtree_with_output(&path),
    );
}

pub(super) fn schedule_stage_hunk(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    mode: PullMode,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::Pull { mode },
        move |repo| run_with_git_auth(auth, || repo.pull_with_output(mode)),
    );
}

pub(super) fn schedule_pull_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        RepoCommandKind::PullBranch {
            remote: command_remote,
            branch: command_branch,
        },
        move |repo| run_with_git_auth(auth, || repo.pull_branch_with_output(&remote, &branch)),
    );
}

pub(super) fn schedule_merge_ref(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::Push,
        move |repo| run_with_git_auth(auth, || repo.push_with_output()),
    );
}

pub(super) fn schedule_force_push(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::ForcePush,
        move |repo| run_with_git_auth(auth, || repo.push_force_with_output()),
    );
}

pub(super) fn schedule_push_set_upstream(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        RepoCommandKind::PushSetUpstream {
            remote: command_remote,
            branch: command_branch,
        },
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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

pub(super) fn schedule_reset(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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

pub(super) fn schedule_rebase(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::RebaseContinue,
        |repo| repo.rebase_continue_with_output(),
    );
}

pub(super) fn schedule_rebase_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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

pub(super) fn schedule_merge_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
    target: String,
) {
    let command_name = name.clone();
    let command_target = target.clone();
    schedule_repo_command(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoCommandKind::CreateTag {
            name: command_name,
            target: command_target,
        },
        move |repo| repo.create_tag_with_output(&name, &target),
    );
}

pub(super) fn schedule_delete_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
    msg_tx: mpsc::Sender<Msg>,
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
