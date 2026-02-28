use crate::msg::{Msg, RepoCommandKind};
use gitgpui_core::error::{Error, ErrorKind};
use gitgpui_core::services::{CommandOutput, ConflictSide, PullMode, RemoteUrlKind, ResetMode};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::super::{RepoId, executor::TaskExecutor};
use super::util::{RepoMap, spawn_with_repo};

pub(super) fn schedule_save_worktree_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    contents: String,
    stage: bool,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let full = repo.spec().workdir.join(&path);
        let result = (|| -> Result<CommandOutput, Error> {
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            }
            std::fs::write(&full, contents.as_bytes())
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            if stage {
                let path_ref: &Path = &path;
                repo.stage(&[path_ref])?;
            }
            Ok(CommandOutput {
                command: format!(
                    "Save {}{}",
                    path.display(),
                    if stage { " (staged)" } else { "" }
                ),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: Some(0),
            })
        })();

        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::SaveWorktreeFile { path, stage },
            result,
        });
    });
}

pub(super) fn schedule_export_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
    dest: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ExportPatch {
                commit_id: commit_id.clone(),
                dest: dest.clone(),
            },
            result: repo.export_patch_with_output(&commit_id, &dest),
        });
    });
}

pub(super) fn schedule_apply_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    patch: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ApplyPatch {
                patch: patch.clone(),
            },
            result: repo.apply_patch_with_output(&patch),
        });
    });
}

pub(super) fn schedule_add_worktree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    reference: Option<String>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddWorktree {
                path: path.clone(),
                reference: reference.clone(),
            },
            result: repo.add_worktree_with_output(&path, reference.as_deref()),
        });
    });
}

pub(super) fn schedule_remove_worktree(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveWorktree { path: path.clone() },
            result: repo.remove_worktree_with_output(&path),
        });
    });
}

pub(super) fn schedule_add_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    url: String,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddSubmodule {
                url: url.clone(),
                path: path.clone(),
            },
            result: repo.add_submodule_with_output(&url, &path),
        });
    });
}

pub(super) fn schedule_update_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UpdateSubmodules,
            result: repo.update_submodules_with_output(),
        });
    });
}

pub(super) fn schedule_remove_submodule(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveSubmodule { path: path.clone() },
            result: repo.remove_submodule_with_output(&path),
        });
    });
}

pub(super) fn schedule_stage_hunk(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    patch: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: repo.apply_unified_patch_to_index_with_output(&patch, false),
        });
    });
}

pub(super) fn schedule_unstage_hunk(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    patch: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UnstageHunk,
            result: repo.apply_unified_patch_to_index_with_output(&patch, true),
        });
    });
}

pub(super) fn schedule_apply_worktree_patch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    patch: String,
    reverse: bool,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ApplyWorktreePatch { reverse },
            result: repo.apply_unified_patch_to_worktree_with_output(&patch, reverse),
        });
    });
}

pub(super) fn schedule_fetch_all(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    prune: bool,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchAll,
            result: repo.fetch_all_with_output_prune(prune),
        });
    });
}

pub(super) fn schedule_pull(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    mode: PullMode,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull { mode },
            result: repo.pull_with_output(mode),
        });
    });
}

pub(super) fn schedule_pull_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    remote: String,
    branch: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PullBranch {
                remote: remote.clone(),
                branch: branch.clone(),
            },
            result: repo.pull_branch_with_output(&remote, &branch),
        });
    });
}

pub(super) fn schedule_merge_ref(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    reference: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::MergeRef {
                reference: reference.clone(),
            },
            result: repo.merge_ref_with_output(&reference),
        });
    });
}

pub(super) fn schedule_push(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: repo.push_with_output(),
        });
    });
}

pub(super) fn schedule_force_push(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ForcePush,
            result: repo.push_force_with_output(),
        });
    });
}

pub(super) fn schedule_push_set_upstream(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    remote: String,
    branch: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PushSetUpstream {
                remote: remote.clone(),
                branch: branch.clone(),
            },
            result: repo.push_set_upstream_with_output(&remote, &branch),
        });
    });
}

pub(super) fn schedule_delete_remote_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    remote: String,
    branch: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteRemoteBranch {
                remote: remote.clone(),
                branch: branch.clone(),
            },
            result: repo.delete_remote_branch_with_output(&remote, &branch),
        });
    });
}

pub(super) fn schedule_reset(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: String,
    mode: ResetMode,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Reset {
                mode,
                target: target.clone(),
            },
            result: repo.reset_with_output(&target, mode),
        });
    });
}

pub(super) fn schedule_rebase(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    onto: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Rebase { onto: onto.clone() },
            result: repo.rebase_with_output(&onto),
        });
    });
}

pub(super) fn schedule_rebase_continue(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RebaseContinue,
            result: repo.rebase_continue_with_output(),
        });
    });
}

pub(super) fn schedule_rebase_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RebaseAbort,
            result: repo.rebase_abort_with_output(),
        });
    });
}

pub(super) fn schedule_merge_abort(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::MergeAbort,
            result: repo.merge_abort_with_output(),
        });
    });
}

pub(super) fn schedule_create_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
    target: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CreateTag {
                name: name.clone(),
                target: target.clone(),
            },
            result: repo.create_tag_with_output(&name, &target),
        });
    });
}

pub(super) fn schedule_delete_tag(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteTag { name: name.clone() },
            result: repo.delete_tag_with_output(&name),
        });
    });
}

pub(super) fn schedule_add_remote(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
    url: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddRemote {
                name: name.clone(),
                url: url.clone(),
            },
            result: repo.add_remote_with_output(&name, &url),
        });
    });
}

pub(super) fn schedule_remove_remote(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveRemote { name: name.clone() },
            result: repo.remove_remote_with_output(&name),
        });
    });
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
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::SetRemoteUrl {
                name: name.clone(),
                url: url.clone(),
                kind,
            },
            result: repo.set_remote_url_with_output(&name, &url, kind),
        });
    });
}

pub(super) fn schedule_checkout_conflict_side(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    side: ConflictSide,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.checkout_conflict_side(&path, side);
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflict {
                path: path.clone(),
                side,
            },
            result,
        });
    });
}

pub(super) fn schedule_checkout_conflict_base(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.checkout_conflict_base(&path);
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase { path: path.clone() },
            result,
        });
    });
}

pub(super) fn schedule_launch_mergetool(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.launch_mergetool(&path);
        let cmd_result = match result {
            Ok(mergetool_result) => {
                if mergetool_result.success {
                    Ok(CommandOutput {
                        command: format!("mergetool ({})", mergetool_result.tool_name),
                        stdout: mergetool_result.output.stdout,
                        stderr: mergetool_result.output.stderr,
                        exit_code: mergetool_result.output.exit_code,
                    })
                } else {
                    Err(gitgpui_core::error::Error::new(
                        gitgpui_core::error::ErrorKind::Backend(format!(
                            "Mergetool '{}' did not complete successfully",
                            mergetool_result.tool_name
                        )),
                    ))
                }
            }
            Err(e) => Err(e),
        };
        let _ = msg_tx.send(Msg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LaunchMergetool { path: path.clone() },
            result: cmd_result,
        });
    });
}
