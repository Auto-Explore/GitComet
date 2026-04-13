mod clone;
mod open_repo;
mod repo_actions;
mod repo_commands;
mod repo_load;
mod util;

use crate::model::AppState;
use crate::msg::{Effect, Msg, RepoCommandKind};
use crate::session;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap as HashMap;
use std::sync::{Arc, RwLock, mpsc};

use super::RepoId;
use super::executor::TaskExecutor;

fn selected_diff_target(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
) -> Option<DiffTarget> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.diff_state.diff_target.clone())
}

fn selected_conflict_file_path(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
) -> Option<std::path::PathBuf> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.conflict_state.conflict_file_path.clone())
}

fn effect_requires_available_git(effect: &Effect) -> bool {
    !matches!(
        effect,
        Effect::PersistSession { .. } | Effect::AbortCloneRepo { .. }
    )
}

fn git_unavailable_error(runtime: &GitRuntimeState) -> Error {
    Error::new(ErrorKind::Backend(
        runtime
            .unavailable_detail()
            .unwrap_or("Git executable is unavailable.")
            .to_string(),
    ))
}

fn send_unavailable_git_effect_result(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    msg_tx: &mpsc::Sender<Msg>,
    effect: Effect,
    runtime: &GitRuntimeState,
) {
    let send = |msg| util::send_or_log(msg_tx, msg);

    match effect {
        Effect::PersistSession { .. } => {}
        Effect::OpenRepo { repo_id, path } => {
            send(Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
                repo_id,
                spec: gitcomet_core::domain::RepoSpec { workdir: path },
                error: git_unavailable_error(runtime),
            }))
        }
        Effect::LoadBranches { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadRemotes { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadRemoteBranches { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RemoteBranchesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadWorktreeStatus { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::WorktreeStatusLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadStagedStatus { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadStatus { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadHeadBranch { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadUpstreamDivergence { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::UpstreamDivergenceLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadLog {
            repo_id,
            scope,
            cursor,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope,
            cursor,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::LoadTags { repo_id } => send(Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::LoadRemoteTags { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::RemoteTagsLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadStashes { repo_id, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadConflictFile { repo_id, path, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id,
                path,
                result: Box::new(Err(git_unavailable_error(runtime))),
                conflict_session: None,
            }))
        }
        Effect::LoadReflog { repo_id, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::ReflogLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::SaveWorktreeFile {
            repo_id,
            path,
            stage,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::SaveWorktreeFile { path, stage },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadFileHistory { repo_id, path, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
                repo_id,
                path,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadBlame { repo_id, path, rev } => {
            send(Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
                repo_id,
                path,
                rev,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadWorktrees { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadSubmodules { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::CheckSubmoduleAddTrust {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
        } => send(Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::CheckSubmoduleUpdateTrust { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::SubmoduleUpdateTrustChecked {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadRebaseAndMergeState { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }));
            send(Msg::Internal(
                crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: Err(git_unavailable_error(runtime)),
                },
            ));
        }
        Effect::LoadRebaseState { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadMergeCommitMessage { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::MergeCommitMessageLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadCommitDetails { repo_id, commit_id } => send(Msg::Internal(
            crate::msg::InternalMsg::CommitDetailsLoaded {
                repo_id,
                commit_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadDiff { repo_id, target } => {
            send(Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                repo_id,
                target,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadDiffFile { repo_id, target } => {
            send(Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                repo_id,
                target,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadDiffPreviewTextFile {
            repo_id,
            target,
            side,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                repo_id,
                target,
                side,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadDiffFileImage { repo_id, target } => send(Msg::Internal(
            crate::msg::InternalMsg::DiffFileImageLoaded {
                repo_id,
                target,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff,
            load_file_text,
            preview_text_side,
            load_file_image,
        } => {
            let Some(target) = selected_diff_target(thread_state, repo_id) else {
                return;
            };
            if load_file_image {
                send(Msg::Internal(
                    crate::msg::InternalMsg::DiffFileImageLoaded {
                        repo_id,
                        target: target.clone(),
                        result: Err(git_unavailable_error(runtime)),
                    },
                ));
            }
            if let Some(side) = preview_text_side {
                send(Msg::Internal(
                    crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                        repo_id,
                        target: target.clone(),
                        side,
                        result: Err(git_unavailable_error(runtime)),
                    },
                ));
            }
            if load_file_text {
                send(Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                    repo_id,
                    target: target.clone(),
                    result: Err(git_unavailable_error(runtime)),
                }));
            }
            if load_patch_diff {
                send(Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                    repo_id,
                    target,
                    result: Err(git_unavailable_error(runtime)),
                }));
            }
        }
        Effect::LoadSelectedConflictFile { repo_id, .. } => {
            let Some(path) = selected_conflict_file_path(thread_state, repo_id) else {
                return;
            };
            send(Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id,
                path,
                result: Box::new(Err(git_unavailable_error(runtime))),
                conflict_session: None,
            }));
        }
        Effect::CheckoutBranch { repo_id, .. }
        | Effect::CheckoutRemoteBranch { repo_id, .. }
        | Effect::CheckoutCommit { repo_id, .. }
        | Effect::CherryPickCommit { repo_id, .. }
        | Effect::RevertCommit { repo_id, .. }
        | Effect::CreateBranch { repo_id, .. }
        | Effect::CreateBranchAndCheckout { repo_id, .. }
        | Effect::DeleteBranch { repo_id, .. }
        | Effect::ForceDeleteBranch { repo_id, .. }
        | Effect::StagePath { repo_id, .. }
        | Effect::StagePaths { repo_id, .. }
        | Effect::UnstagePath { repo_id, .. }
        | Effect::UnstagePaths { repo_id, .. }
        | Effect::DiscardWorktreeChangesPath { repo_id, .. }
        | Effect::DiscardWorktreeChangesPaths { repo_id, .. }
        | Effect::Stash { repo_id, .. }
        | Effect::ApplyStash { repo_id, .. }
        | Effect::PopStash { repo_id, .. }
        | Effect::DropStash { repo_id, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::CloneRepo { url, dest, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                url,
                dest,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::AbortCloneRepo { dest } => clone::schedule_abort_clone_repo(msg_tx.clone(), dest),
        Effect::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ExportPatch { commit_id, dest },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ApplyPatch { repo_id, patch } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ApplyPatch { patch },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::AddWorktree {
            repo_id,
            path,
            reference,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AddWorktree { path, reference },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RemoveWorktree { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::RemoveWorktree { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ForceRemoveWorktree { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ForceRemoveWorktree { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::AddSubmodule {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AddSubmodule {
                    url,
                    path,
                    branch,
                    name,
                    force,
                    approved_sources,
                },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::UpdateSubmodules {
            repo_id,
            approved_sources,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::UpdateSubmodules { approved_sources },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RemoveSubmodule { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::RemoveSubmodule { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::StageHunk { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::StageHunk,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::UnstageHunk { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::UnstageHunk,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ApplyWorktreePatch {
            repo_id, reverse, ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ApplyWorktreePatch { reverse },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::Commit { repo_id, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::CommitFinished {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::CommitAmend { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::CommitAmendFinished {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::FetchAll {
            repo_id, prune: _, ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::FetchAll,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PruneMergedBranches { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PruneMergedBranches,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PruneLocalTags { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PruneLocalTags,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::Pull { repo_id, mode, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Pull { mode },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PullBranch {
            repo_id,
            remote,
            branch,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PullBranch { remote, branch },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::MergeRef { repo_id, reference } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::MergeRef { reference },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::SquashRef { repo_id, reference } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::SquashRef { reference },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::Push { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Push,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ForcePush { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ForcePush,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PushSetUpstream {
            repo_id,
            remote,
            branch,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PushSetUpstream { remote, branch },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::SetUpstreamBranch { branch, upstream },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::UnsetUpstreamBranch { repo_id, branch } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::UnsetUpstreamBranch { branch },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::DeleteRemoteBranch { remote, branch },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::Reset {
            repo_id,
            target,
            mode,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Reset { mode, target },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::Rebase { repo_id, onto } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Rebase { onto },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RebaseContinue { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::RebaseContinue,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RebaseAbort { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::RebaseAbort,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::MergeAbort { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::MergeAbort,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::CreateTag {
            repo_id,
            name,
            target,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::CreateTag { name, target },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::DeleteTag { repo_id, name } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::DeleteTag { name },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PushTag {
            repo_id,
            remote,
            name,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PushTag { remote, name },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::DeleteRemoteTag {
            repo_id,
            remote,
            name,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::DeleteRemoteTag { remote, name },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::AddRemote { repo_id, name, url } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AddRemote { name, url },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RemoveRemote { repo_id, name } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::RemoveRemote { name },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::SetRemoteUrl { name, url, kind },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::CheckoutConflict { path, side },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::AcceptConflictDeletion { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AcceptConflictDeletion { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::CheckoutConflictBase { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::CheckoutConflictBase { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LaunchMergetool { repo_id, path } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LaunchMergetool { path },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
    }
}

pub(super) fn schedule_effect(
    executor: &TaskExecutor,
    session_persist_executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    backend: &Arc<dyn GitBackend>,
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    msg_tx: mpsc::Sender<Msg>,
    effect: Effect,
) {
    if effect_requires_available_git(&effect) {
        let runtime = {
            let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
            state.git_runtime.clone()
        };
        if !runtime.is_available() {
            send_unavailable_git_effect_result(thread_state, &msg_tx, effect, &runtime);
            return;
        }
    }

    match effect {
        Effect::PersistSession { repo_id, action } => {
            let state_snapshot = {
                let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                Arc::clone(&state)
            };
            session_persist_executor.spawn(move || {
                if let Err(error) = session::persist_from_state(&state_snapshot) {
                    util::send_or_log(
                        &msg_tx,
                        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
                            repo_id,
                            action,
                            error: error.to_string(),
                        }),
                    );
                }
            });
        }
        Effect::OpenRepo { repo_id, path } => {
            open_repo::schedule_open_repo(executor, Arc::clone(backend), msg_tx, repo_id, path);
        }
        Effect::LoadBranches { repo_id } => {
            repo_load::schedule_load_branches(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRemotes { repo_id } => {
            repo_load::schedule_load_remotes(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRemoteBranches { repo_id } => {
            repo_load::schedule_load_remote_branches(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadWorktreeStatus { repo_id } => {
            repo_load::schedule_load_worktree_status(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadStagedStatus { repo_id } => {
            repo_load::schedule_load_staged_status(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadStatus { repo_id } => {
            repo_load::schedule_load_status(executor, repos, msg_tx, repo_id)
        }
        Effect::LoadHeadBranch { repo_id } => {
            repo_load::schedule_load_head_branch(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadUpstreamDivergence { repo_id } => {
            repo_load::schedule_load_upstream_divergence(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadLog {
            repo_id,
            scope,
            limit,
            cursor,
        } => repo_load::schedule_load_log(executor, repos, msg_tx, repo_id, scope, limit, cursor),
        Effect::LoadTags { repo_id } => {
            repo_load::schedule_load_tags(executor, repos, msg_tx, repo_id)
        }
        Effect::LoadRemoteTags { repo_id } => {
            repo_load::schedule_load_remote_tags(executor, repos, msg_tx, repo_id)
        }
        Effect::LoadStashes { repo_id, limit } => {
            repo_load::schedule_load_stashes(executor, repos, msg_tx, repo_id, limit);
        }
        Effect::LoadConflictFile {
            repo_id,
            path,
            mode,
        } => {
            repo_load::schedule_load_conflict_file(executor, repos, msg_tx, repo_id, path, mode);
        }
        Effect::LoadReflog { repo_id, limit } => {
            repo_load::schedule_load_reflog(executor, repos, msg_tx, repo_id, limit);
        }
        Effect::SaveWorktreeFile {
            repo_id,
            path,
            contents,
            stage,
        } => repo_commands::schedule_save_worktree_file(
            executor, repos, msg_tx, repo_id, path, contents, stage,
        ),
        Effect::LoadFileHistory {
            repo_id,
            path,
            limit,
        } => repo_load::schedule_load_file_history(executor, repos, msg_tx, repo_id, path, limit),
        Effect::LoadBlame { repo_id, path, rev } => {
            repo_load::schedule_load_blame(executor, repos, msg_tx, repo_id, path, rev);
        }
        Effect::LoadWorktrees { repo_id } => {
            repo_load::schedule_load_worktrees(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadSubmodules { repo_id } => {
            repo_load::schedule_load_submodules(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRebaseAndMergeState { repo_id } => {
            repo_load::schedule_load_rebase_and_merge_state(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRebaseState { repo_id } => {
            repo_load::schedule_load_rebase_state(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadMergeCommitMessage { repo_id } => {
            repo_load::schedule_load_merge_commit_message(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadCommitDetails { repo_id, commit_id } => {
            repo_load::schedule_load_commit_details(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::LoadDiff { repo_id, target } => {
            repo_load::schedule_load_diff(executor, repos, msg_tx, repo_id, target);
        }
        Effect::LoadDiffFile { repo_id, target } => {
            repo_load::schedule_load_diff_file(executor, repos, msg_tx, repo_id, target);
        }
        Effect::LoadDiffPreviewTextFile {
            repo_id,
            target,
            side,
        } => {
            repo_load::schedule_load_diff_preview_text_file(
                executor, repos, msg_tx, repo_id, target, side,
            );
        }
        Effect::LoadDiffFileImage { repo_id, target } => {
            repo_load::schedule_load_diff_file_image(executor, repos, msg_tx, repo_id, target);
        }
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff,
            load_file_text,
            preview_text_side,
            load_file_image,
        } => {
            if let Some(target) = selected_diff_target(thread_state, repo_id) {
                repo_load::schedule_load_selected_diff(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    target,
                    load_patch_diff,
                    load_file_text,
                    preview_text_side,
                    load_file_image,
                );
            }
        }
        Effect::LoadSelectedConflictFile { repo_id, mode } => {
            if let Some(path) = selected_conflict_file_path(thread_state, repo_id) {
                repo_load::schedule_load_conflict_file(
                    executor, repos, msg_tx, repo_id, path, mode,
                );
            }
        }
        Effect::CheckoutBranch { repo_id, name } => {
            repo_actions::schedule_checkout_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        } => repo_actions::schedule_checkout_remote_branch(
            executor,
            repos,
            msg_tx,
            repo_id,
            remote,
            branch,
            local_branch,
        ),
        Effect::CheckoutCommit { repo_id, commit_id } => {
            repo_actions::schedule_checkout_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::CherryPickCommit { repo_id, commit_id } => {
            repo_actions::schedule_cherry_pick_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::RevertCommit { repo_id, commit_id } => {
            repo_actions::schedule_revert_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::CreateBranch {
            repo_id,
            name,
            target,
        } => {
            repo_actions::schedule_create_branch(executor, repos, msg_tx, repo_id, name, target);
        }
        Effect::CreateBranchAndCheckout {
            repo_id,
            name,
            target,
        } => {
            repo_actions::schedule_create_branch_and_checkout(
                executor, repos, msg_tx, repo_id, name, target,
            );
        }
        Effect::DeleteBranch { repo_id, name } => {
            repo_actions::schedule_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::ForceDeleteBranch { repo_id, name } => {
            repo_actions::schedule_force_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::CloneRepo { url, dest, auth } => {
            clone::schedule_clone_repo(executor, msg_tx, url, dest, auth)
        }
        Effect::AbortCloneRepo { dest } => clone::schedule_abort_clone_repo(msg_tx, dest),
        Effect::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => {
            repo_commands::schedule_export_patch(executor, repos, msg_tx, repo_id, commit_id, dest)
        }
        Effect::ApplyPatch { repo_id, patch } => {
            repo_commands::schedule_apply_patch(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::AddWorktree {
            repo_id,
            path,
            reference,
        } => {
            repo_commands::schedule_add_worktree(executor, repos, msg_tx, repo_id, path, reference)
        }
        Effect::RemoveWorktree { repo_id, path } => {
            repo_commands::schedule_remove_worktree(executor, repos, msg_tx, repo_id, path);
        }
        Effect::ForceRemoveWorktree { repo_id, path } => {
            repo_commands::schedule_force_remove_worktree(executor, repos, msg_tx, repo_id, path);
        }
        Effect::CheckSubmoduleAddTrust {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
        } => {
            repo_commands::schedule_check_submodule_add_trust(
                executor, repos, msg_tx, repo_id, url, path, branch, name, force,
            );
        }
        Effect::CheckSubmoduleUpdateTrust { repo_id } => {
            repo_commands::schedule_check_submodule_update_trust(executor, repos, msg_tx, repo_id);
        }
        Effect::AddSubmodule {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
            auth,
        } => {
            repo_commands::schedule_add_submodule(
                executor,
                repos,
                msg_tx,
                repo_id,
                repo_commands::AddSubmoduleRequest {
                    url,
                    path,
                    branch,
                    name,
                    force,
                    approved_sources,
                    auth,
                },
            );
        }
        Effect::UpdateSubmodules {
            repo_id,
            approved_sources,
            auth,
        } => {
            repo_commands::schedule_update_submodules(
                executor,
                repos,
                msg_tx,
                repo_id,
                approved_sources,
                auth,
            );
        }
        Effect::RemoveSubmodule { repo_id, path } => {
            repo_commands::schedule_remove_submodule(executor, repos, msg_tx, repo_id, path);
        }
        Effect::StageHunk { repo_id, patch } => {
            repo_commands::schedule_stage_hunk(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::UnstageHunk { repo_id, patch } => {
            repo_commands::schedule_unstage_hunk(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::ApplyWorktreePatch {
            repo_id,
            patch,
            reverse,
        } => repo_commands::schedule_apply_worktree_patch(
            executor, repos, msg_tx, repo_id, patch, reverse,
        ),
        Effect::StagePath { repo_id, path } => {
            repo_actions::schedule_stage_path(executor, repos, msg_tx, repo_id, path);
        }
        Effect::StagePaths { repo_id, paths } => {
            repo_actions::schedule_stage_paths(executor, repos, msg_tx, repo_id, paths);
        }
        Effect::UnstagePath { repo_id, path } => {
            repo_actions::schedule_unstage_path(executor, repos, msg_tx, repo_id, path);
        }
        Effect::UnstagePaths { repo_id, paths } => {
            repo_actions::schedule_unstage_paths(executor, repos, msg_tx, repo_id, paths);
        }
        Effect::DiscardWorktreeChangesPath { repo_id, path } => {
            repo_actions::schedule_discard_worktree_changes_path(
                executor, repos, msg_tx, repo_id, path,
            );
        }
        Effect::DiscardWorktreeChangesPaths { repo_id, paths } => {
            repo_actions::schedule_discard_worktree_changes_paths(
                executor, repos, msg_tx, repo_id, paths,
            )
        }
        Effect::Commit {
            repo_id,
            message,
            auth,
        } => {
            repo_actions::schedule_commit(executor, repos, msg_tx, repo_id, message, auth);
        }
        Effect::CommitAmend {
            repo_id,
            message,
            auth,
        } => {
            repo_actions::schedule_commit_amend(executor, repos, msg_tx, repo_id, message, auth);
        }
        Effect::FetchAll {
            repo_id,
            prune,
            auth,
        } => repo_commands::schedule_fetch_all(executor, repos, msg_tx, repo_id, prune, auth),
        Effect::PruneMergedBranches { repo_id } => {
            repo_commands::schedule_prune_merged_branches(executor, repos, msg_tx, repo_id)
        }
        Effect::PruneLocalTags { repo_id } => {
            repo_commands::schedule_prune_local_tags(executor, repos, msg_tx, repo_id)
        }
        Effect::Pull {
            repo_id,
            mode,
            auth,
        } => repo_commands::schedule_pull(executor, repos, msg_tx, repo_id, mode, auth),
        Effect::PullBranch {
            repo_id,
            remote,
            branch,
            auth,
        } => repo_commands::schedule_pull_branch(
            executor, repos, msg_tx, repo_id, remote, branch, auth,
        ),
        Effect::MergeRef { repo_id, reference } => {
            repo_commands::schedule_merge_ref(executor, repos, msg_tx, repo_id, reference);
        }
        Effect::SquashRef { repo_id, reference } => {
            repo_commands::schedule_squash_ref(executor, repos, msg_tx, repo_id, reference);
        }
        Effect::Push { repo_id, auth } => {
            repo_commands::schedule_push(executor, repos, msg_tx, repo_id, auth)
        }
        Effect::ForcePush { repo_id, auth } => {
            repo_commands::schedule_force_push(executor, repos, msg_tx, repo_id, auth)
        }
        Effect::PushSetUpstream {
            repo_id,
            remote,
            branch,
            auth,
        } => repo_commands::schedule_push_set_upstream(
            executor, repos, msg_tx, repo_id, remote, branch, auth,
        ),
        Effect::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        } => repo_commands::schedule_set_upstream_branch(
            executor, repos, msg_tx, repo_id, branch, upstream,
        ),
        Effect::UnsetUpstreamBranch { repo_id, branch } => {
            repo_commands::schedule_unset_upstream_branch(executor, repos, msg_tx, repo_id, branch)
        }
        Effect::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
            auth,
        } => repo_commands::schedule_delete_remote_branch(
            executor, repos, msg_tx, repo_id, remote, branch, auth,
        ),
        Effect::Reset {
            repo_id,
            target,
            mode,
        } => repo_commands::schedule_reset(executor, repos, msg_tx, repo_id, target, mode),
        Effect::Rebase { repo_id, onto } => {
            repo_commands::schedule_rebase(executor, repos, msg_tx, repo_id, onto)
        }
        Effect::RebaseContinue { repo_id } => {
            repo_commands::schedule_rebase_continue(executor, repos, msg_tx, repo_id);
        }
        Effect::RebaseAbort { repo_id } => {
            repo_commands::schedule_rebase_abort(executor, repos, msg_tx, repo_id)
        }
        Effect::MergeAbort { repo_id } => {
            repo_commands::schedule_merge_abort(executor, repos, msg_tx, repo_id)
        }
        Effect::CreateTag {
            repo_id,
            name,
            target,
        } => repo_commands::schedule_create_tag(executor, repos, msg_tx, repo_id, name, target),
        Effect::DeleteTag { repo_id, name } => {
            repo_commands::schedule_delete_tag(executor, repos, msg_tx, repo_id, name);
        }
        Effect::PushTag {
            repo_id,
            remote,
            name,
            auth,
        } => repo_commands::schedule_push_tag(executor, repos, msg_tx, repo_id, remote, name, auth),
        Effect::DeleteRemoteTag {
            repo_id,
            remote,
            name,
            auth,
        } => repo_commands::schedule_delete_remote_tag(
            executor, repos, msg_tx, repo_id, remote, name, auth,
        ),
        Effect::AddRemote { repo_id, name, url } => {
            repo_commands::schedule_add_remote(executor, repos, msg_tx, repo_id, name, url);
        }
        Effect::RemoveRemote { repo_id, name } => {
            repo_commands::schedule_remove_remote(executor, repos, msg_tx, repo_id, name);
        }
        Effect::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => repo_commands::schedule_set_remote_url(
            executor, repos, msg_tx, repo_id, name, url, kind,
        ),
        Effect::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => repo_commands::schedule_checkout_conflict_side(
            executor, repos, msg_tx, repo_id, path, side,
        ),
        Effect::AcceptConflictDeletion { repo_id, path } => {
            repo_commands::schedule_accept_conflict_deletion(executor, repos, msg_tx, repo_id, path)
        }
        Effect::CheckoutConflictBase { repo_id, path } => {
            repo_commands::schedule_checkout_conflict_base(executor, repos, msg_tx, repo_id, path)
        }
        Effect::LaunchMergetool { repo_id, path } => {
            repo_commands::schedule_launch_mergetool(executor, repos, msg_tx, repo_id, path);
        }
        Effect::Stash {
            repo_id,
            message,
            include_untracked,
        } => repo_actions::schedule_stash(
            executor,
            repos,
            msg_tx,
            repo_id,
            message,
            include_untracked,
        ),
        Effect::ApplyStash { repo_id, index } => {
            repo_actions::schedule_apply_stash(executor, repos, msg_tx, repo_id, index);
        }
        Effect::PopStash { repo_id, index } => {
            repo_actions::schedule_pop_stash(executor, repos, msg_tx, repo_id, index);
        }
        Effect::DropStash { repo_id, index } => {
            repo_actions::schedule_drop_stash(executor, repos, msg_tx, repo_id, index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_clone_repo_does_not_require_available_git() {
        assert!(!effect_requires_available_git(&Effect::AbortCloneRepo {
            dest: std::path::PathBuf::from("/tmp/example"),
        }));
    }
}
