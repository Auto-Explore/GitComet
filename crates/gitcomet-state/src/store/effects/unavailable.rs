//! Replies for effects scheduled while Git is unavailable: each fails with
//! the runtime's reason instead of running.

use super::{
    selected_conflict_file_path, selected_diff_target, selected_inline_submodule_diff, util,
};
use crate::model::{AppState, RepoId};
use crate::msg::{Effect, Msg, RepoActionKind};
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
        Effect::Status(work) => send(Msg::Internal(work.failed(git_unavailable_error(runtime)))),
        Effect::References(work) => {
            send(Msg::Internal(work.failed(git_unavailable_error(runtime))))
        }
        Effect::RepoCommand(work) => {
            if let Some(message) = work.failed(git_unavailable_error(runtime)) {
                send(Msg::Internal(message));
            }
        }
        Effect::Filesystem(_)
        | Effect::Persistence(_)
        | Effect::Control(_)
        | Effect::ValidateAuthPrompt { .. } => {}
        Effect::OpenRepo { repo_id, path } => {
            send(Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
                repo_id,
                spec: gitcomet_core::domain::RepoSpec { workdir: path },
                error: git_unavailable_error(runtime),
            }))
        }
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
        Effect::LoadStashes { repo_id, .. } => {
            send(Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                repo_id,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadConflictFile {
            request,
            repo_id,
            path,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            request,
            repo_id,
            path,
            result: Box::new(Err(git_unavailable_error(runtime))),
            conflict_session: None,
        })),
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
        Effect::LoadFileHistory {
            request,
            repo_id,
            path,
            cursor,
            ..
        } => send(Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
            request,
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
                read: crate::msg::WorktreeListRead::explicit(),
                result: Err(git_unavailable_error(runtime)),
            }))
        }
        Effect::LoadWorktreeDirty { repo_id, scope, .. } => send(Msg::Internal(
            crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id,
                scope,
                result: Err(git_unavailable_error(runtime)),
            },
        )),
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
                cancellation: None,
                repo_id,
                source,
                result: Err(git_unavailable_error(runtime)),
            }))
        }
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
        Effect::LoadSelectedConflictFile {
            request, repo_id, ..
        } => {
            let Some(path) = selected_conflict_file_path(thread_state, repo_id, request) else {
                return;
            };
            send(Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                request,
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
    }
}
