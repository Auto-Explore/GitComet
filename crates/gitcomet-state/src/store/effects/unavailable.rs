//! Replies for effects scheduled while Git is unavailable: each fails with
//! the runtime's reason instead of running.

use super::{
    clone, selected_conflict_file_path, selected_diff_target, selected_inline_submodule_diff, util,
};
use crate::model::{AppState, RepoId};
use crate::msg::{Effect, Msg, RepoActionKind, RepoCommandKind};
use crate::store::worker_channel::StoreWorkerSender;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::GitRuntimeState;
use std::sync::{Arc, RwLock};

fn git_unavailable_error(runtime: &GitRuntimeState) -> Error {
    Error::new(ErrorKind::Backend(
        runtime
            .unavailable_detail()
            .unwrap_or("Git executable is unavailable.")
            .to_string(),
    ))
}

fn send_repo_action_unavailable(
    repo_id: RepoId,
    action: RepoActionKind,
    runtime: &GitRuntimeState,
    send: &impl Fn(Msg),
) {
    send(Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
        repo_id,
        action,
        result: Err(git_unavailable_error(runtime)),
    }))
}

pub(super) fn send_unavailable_git_effect_result(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    msg_tx: &StoreWorkerSender,
    effect: Effect,
    runtime: &GitRuntimeState,
) {
    let send = |msg| util::send_or_log(msg_tx, msg);

    match effect {
        Effect::PersistSession { .. }
        | Effect::PersistRecentRepo { .. }
        | Effect::PersistRepoHistoryMode { .. }
        | Effect::PersistRepoHistoryModesBatch { .. }
        | Effect::PersistRepoHistoryAuthorFilter { .. }
        | Effect::CancelRepoLoads { .. }
        | Effect::CancelGitOperation { .. } => {}
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
        Effect::LoadUncommittedLineStats {
            repo_id,
            generation,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::UncommittedLineStatsLoaded {
                repo_id,
                generation,
                result: Err(git_unavailable_error(runtime)),
                large_files: None,
            },
        )),
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
        Effect::IndexedHistory(work) => send(Msg::IndexedHistory(
            work.failed(git_unavailable_error(runtime)),
        )),
        Effect::DiffSession(work) => {
            for reply in work.failed(git_unavailable_error(runtime)) {
                send(Msg::DiffSession(reply));
            }
        }
        Effect::HistoryAuthors(work) => send(Msg::HistoryAuthors(
            work.failed(git_unavailable_error(runtime)),
        )),
        Effect::HistoryFind(work) => send(Msg::HistoryFind(
            work.failed(git_unavailable_error(runtime)),
        )),
        Effect::LoadLog {
            repo_id,
            seq,
            scope,
            cursor,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
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
        Effect::LoadRecentCommitMessages {
            repo_id,
            request_rev,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RecentCommitMessagesLoaded {
                repo_id,
                request_rev,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
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
        Effect::AppendGitignorePatterns { repo_id, patterns } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AppendGitignorePatterns { patterns },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RunLargeFileCommand {
            repo_id, command, ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LargeFile { command },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadLfsLocks { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::LfsLocksLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadAnnexWhereis { repo_id, key } => {
            send(Msg::Internal(crate::msg::InternalMsg::AnnexWhereisLoaded {
                repo_id,
                key,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadAnnexUnused { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::AppendGitattributesRule { repo_id, rule } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::AppendGitattributesRule { rule },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadFileHistory {
            repo_id,
            path,
            cursor,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
            repo_id,
            path,
            cursor,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::LoadBlame {
            repo_id,
            path,
            source,
        } => send(Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
            repo_id,
            path,
            source,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::LoadWorktrees { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadWorktreeDirty { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadRefMetadata { repo_id } => {
            send(Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
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
        Effect::LoadLargeFileSupport { repo_id } => send(Msg::Internal(
            crate::msg::InternalMsg::LargeFileSupportLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadFileBrowser { repo_id, source } => {
            send(Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
                repo_id,
                source,
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
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::SubmoduleAddTrustChecked {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::CheckSubmoduleUpdateTrust { repo_id, .. } => send(Msg::Internal(
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
        Effect::VerifyCommitSignatures {
            repo_id,
            epoch,
            batch,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id,
                epoch,
                batch,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadHoverCommitMessage { repo_id, commit_id } => send(Msg::Internal(
            crate::msg::InternalMsg::HoverCommitMessageLoaded {
                repo_id,
                commit_id,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ResolveCommitForReveal { repo_id, reference } => send(Msg::Internal(
            crate::msg::InternalMsg::CommitRevealResolved {
                repo_id,
                reference,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ResolveCommitLookup {
            repo_id,
            reference,
            purpose,
            request,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::CommitLookupResolved {
                repo_id,
                reference,
                request,
                purpose,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadRangeFiles {
            repo_id,
            from,
            to,
            request,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from,
            to,
            request,
            result: Err(git_unavailable_error(runtime)),
        })),
        Effect::LoadSquashMessagePreview {
            repo_id,
            oldest,
            head,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::SquashMessagePreviewLoaded {
                repo_id,
                oldest,
                head,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadSquashRebaseSetup {
            repo_id,
            base,
            actual_head,
            selected_ids,
            reword_id,
            message,
            count,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::SquashRebaseSetupLoaded {
                repo_id,
                base: base.as_ref().to_string(),
                actual_head,
                selected_ids,
                reword_id,
                message,
                count,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::OpenFileAtCommitParent { .. } | Effect::OpenFileAtCommit { .. } => {
            // No git backend available; nothing to resolve.
        }
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
        Effect::LoadSubmoduleSummary { repo_id, target } => send(Msg::Internal(
            crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                repo_id,
                target,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadInlineSubmoduleSelectedDiff {
            repo_id,
            inline_rev,
        } => {
            let Some((_, target, current_rev)) =
                selected_inline_submodule_diff(thread_state, repo_id)
            else {
                return;
            };
            if current_rev != inline_rev {
                return;
            }
            send(Msg::Internal(
                crate::msg::InternalMsg::InlineSubmoduleDiffLoaded {
                    repo_id,
                    inline_rev,
                    target,
                    result: Err(git_unavailable_error(runtime)),
                },
            ))
        }
        Effect::LoadInlineSubmoduleSelectedDiffFile {
            repo_id,
            inline_rev,
        } => {
            let Some((_, target, current_rev)) =
                selected_inline_submodule_diff(thread_state, repo_id)
            else {
                return;
            };
            if current_rev != inline_rev {
                return;
            }
            send(Msg::Internal(
                crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
                    repo_id,
                    inline_rev,
                    target,
                    result: Err(git_unavailable_error(runtime)),
                },
            ))
        }
        Effect::LoadInlineSubmoduleSelectedDiffFileImage {
            repo_id,
            inline_rev,
        } => {
            let Some((_, target, current_rev)) =
                selected_inline_submodule_diff(thread_state, repo_id)
            else {
                return;
            };
            if current_rev != inline_rev {
                return;
            }
            send(Msg::Internal(
                crate::msg::InternalMsg::InlineSubmoduleDiffFileImageLoaded {
                    repo_id,
                    inline_rev,
                    target,
                    result: Err(git_unavailable_error(runtime)),
                },
            ))
        }
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
            load_submodule_summary,
            load_file_image,
        } => {
            let Some((target, _target_rev)) = selected_diff_target(thread_state, repo_id) else {
                return;
            };
            if load_submodule_summary {
                send(Msg::Internal(
                    crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                        repo_id,
                        target: target.clone(),
                        result: Err(git_unavailable_error(runtime)),
                    },
                ));
            }
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
        Effect::CheckoutBranch { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::CheckoutBranch, runtime, &send)
        }
        Effect::CheckoutRemoteBranch { repo_id, .. } => send_repo_action_unavailable(
            repo_id,
            RepoActionKind::CheckoutRemoteBranch,
            runtime,
            &send,
        ),
        Effect::CheckoutCommit { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::CheckoutCommit, runtime, &send)
        }
        Effect::CherryPickCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::CherryPick {
                    commit_id,
                    commit,
                    mainline,
                    summary,
                },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::RevertCommit {
            repo_id,
            commit_id,
            commit,
            mainline,
            summary,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Revert {
                    commit_id,
                    commit,
                    mainline,
                    summary,
                },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::CreateBranch { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::CreateBranch, runtime, &send)
        }
        Effect::CreateBranchAndCheckout { repo_id, .. } => send_repo_action_unavailable(
            repo_id,
            RepoActionKind::CreateBranchAndCheckout,
            runtime,
            &send,
        ),
        Effect::RenameBranch { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::RenameBranch, runtime, &send)
        }
        Effect::DeleteBranch { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::DeleteBranch, runtime, &send)
        }
        Effect::ForceDeleteBranch { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::ForceDeleteBranch, runtime, &send)
        }
        Effect::DeleteBranches { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::DeleteBranches, runtime, &send)
        }
        Effect::StagePath { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::StagePath, runtime, &send)
        }
        Effect::StagePaths { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::StagePaths, runtime, &send)
        }
        Effect::UnstagePath { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::UnstagePath, runtime, &send)
        }
        Effect::UnstagePaths { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::UnstagePaths, runtime, &send)
        }
        Effect::DiscardWorktreeChangesPath { repo_id, .. } => send_repo_action_unavailable(
            repo_id,
            RepoActionKind::DiscardWorktreeChangesPath,
            runtime,
            &send,
        ),
        Effect::DiscardWorktreeChangesPaths { repo_id, .. } => send_repo_action_unavailable(
            repo_id,
            RepoActionKind::DiscardWorktreeChangesPaths,
            runtime,
            &send,
        ),
        Effect::Stash { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::Stash, runtime, &send)
        }
        Effect::ApplyStash { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::ApplyStash, runtime, &send)
        }
        Effect::PopStash { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::PopStash, runtime, &send)
        }
        Effect::DropStash { repo_id, .. } => {
            send_repo_action_unavailable(repo_id, RepoActionKind::DropStash, runtime, &send)
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
        Effect::CheckSubmoduleLoadTrust { repo_id, path, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::SubmoduleLoadTrustChecked {
                repo_id,
                path,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadSubmodule {
            repo_id,
            path,
            approved_sources,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LoadSubmodule {
                    path,
                    approved_sources,
                },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::ChangeSubmodulePointer {
            repo_id,
            path,
            reference,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ChangeSubmodulePointer { path, reference },
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
        Effect::SafePushAfterCommit {
            repo_id, context, ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::SafePushAfterCommitFinished {
                repo_id,
                context,
                auth: None,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::FetchAll { repo_id, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::FetchAll,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::FetchRefspecs {
            repo_id,
            remote,
            refspecs,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::FetchRefspecs { remote, refspecs },
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
        Effect::PushWithTags {
            repo_id, request, ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PushWithTags { request },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::PreviewTagPush {
            repo_id,
            request,
            generation,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::TagPushPreviewLoaded {
                repo_id,
                mode: request.mode,
                generation,
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
        Effect::PushAfterCommit {
            repo_id,
            target,
            set_upstream,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::PushAfterCommit {
                    target,
                    set_upstream,
                },
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
        Effect::ForcePushWithLease { repo_id, lease, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::ForcePushWithLease { lease },
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
        Effect::DeleteRemoteBranches {
            repo_id,
            remote,
            branches,
            ..
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::DeleteRemoteBranches { remote, branches },
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
        Effect::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::SquashCommits {
                    oldest,
                    expected_head,
                    message,
                    count,
                },
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
        Effect::RebaseContinue { repo_id, .. } => send(Msg::Internal(
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
        Effect::LoadInteractiveRebaseSetup { repo_id, base } => send(Msg::Internal(
            crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
                repo_id,
                base,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::LoadInteractiveCherryPickMessages { repo_id, ids } => send(Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id,
                requested_ids: ids,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::InteractiveRebase {
            repo_id,
            base,
            entries: _,
            interactive,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::InteractiveRebase { base, interactive },
                result: Err(git_unavailable_error(runtime)),
            },
        )),
        Effect::InteractiveCherryPick { repo_id, entries } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::InteractiveCherryPick { entries },
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
            message,
            annotated,
        } => send(Msg::Internal(
            crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::CreateTag {
                    name,
                    target,
                    message,
                    annotated,
                },
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
        Effect::AddRemote {
            repo_id, name, url, ..
        } => send(Msg::Internal(
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
            ..
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
