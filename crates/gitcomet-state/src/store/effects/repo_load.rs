use crate::model::ConflictFileLoadMode;
use crate::msg::Msg;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession, ConflictStageParts};
use gitcomet_core::domain::{DiffArea, DiffTarget, LogCursor, LogScope};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use gitcomet_core::services::ConflictFileStages;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use super::super::{RepoId, executor::TaskExecutor};
use super::util::{RepoMap, send_or_log, spawn_with_repo, spawn_with_repo_or_else};

fn missing_repo_error(repo_id: RepoId) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "Repository handle not found for repo_id {}",
        repo_id.0
    )))
}

fn trace_side_stats(bytes: Option<&[u8]>, text: Option<&str>) -> MergetoolTraceSideStats {
    MergetoolTraceSideStats::from_bytes_and_text(bytes, text)
}

fn trace_payload_stats(payload: Option<&ConflictPayload>) -> MergetoolTraceSideStats {
    MergetoolTraceSideStats::from_bytes_and_text(
        payload.and_then(ConflictPayload::as_bytes),
        payload.and_then(ConflictPayload::as_text),
    )
}

fn conflict_file_stages_from_session(
    path: PathBuf,
    session: &ConflictSession,
) -> ConflictFileStages {
    let (base_bytes, base) = session.base.clone().into_stage_parts();
    let (ours_bytes, ours) = session.ours.clone().into_stage_parts();
    let (theirs_bytes, theirs) = session.theirs.clone().into_stage_parts();

    ConflictFileStages {
        path,
        base_bytes,
        ours_bytes,
        theirs_bytes,
        base,
        ours,
        theirs,
    }
}

fn empty_conflict_file_stages(path: PathBuf) -> ConflictFileStages {
    ConflictFileStages {
        path,
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        base: None,
        ours: None,
        theirs: None,
    }
}

fn conflict_file_current_from_session(session: &ConflictSession) -> Option<ConflictStageParts> {
    session
        .current
        .as_ref()
        .map(|p| p.clone().into_stage_parts())
}

pub(super) fn schedule_load_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
                    repo_id,
                    result: repo.list_branches(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_remotes(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
                    repo_id,
                    result: repo.list_remotes(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_remote_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
                    repo_id,
                    result: repo.list_remote_branches(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_status(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
                    repo_id,
                    result: repo.status(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_head_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                    repo_id,
                    result: repo.current_branch(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_upstream_divergence(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
                    repo_id,
                    result: repo.upstream_divergence(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_log(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    scope: LogScope,
    limit: usize,
    cursor: Option<LogCursor>,
) {
    let cursor_on_missing = cursor.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = {
                let cursor_ref = cursor.as_ref();
                match scope {
                    LogScope::CurrentBranch => repo.log_head_page(limit, cursor_ref),
                    LogScope::AllBranches => repo.log_all_branches_page(limit, cursor_ref),
                }
            };
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                    repo_id,
                    scope,
                    cursor,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                    repo_id,
                    scope,
                    cursor: cursor_on_missing,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_tags(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id,
                    result: repo.list_tags(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_remote_tags(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemoteTagsLoaded {
                    repo_id,
                    result: repo.list_remote_tags(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemoteTagsLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_stashes(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    limit: usize,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let mut entries = repo.stash_list();
            if let Ok(v) = &mut entries {
                v.truncate(limit);
            }
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                    repo_id,
                    result: entries,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_conflict_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    mode: ConflictFileLoadMode,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let trace_path = path.clone();
        let load_full = matches!(mode, ConflictFileLoadMode::Full);

        let conflict_session_started = Instant::now();
        let conflict_session = load_full
            .then(|| repo.conflict_session(&path).ok().flatten())
            .flatten();
        let session_ref = conflict_session.as_ref();
        mergetool_trace::record_with(|| {
            MergetoolTraceEvent::new(
                MergetoolTraceStage::LoadConflictSession,
                Some(trace_path.clone()),
                conflict_session_started.elapsed(),
            )
            .with_base(trace_payload_stats(
                session_ref.map(|session| &session.base),
            ))
            .with_ours(trace_payload_stats(
                session_ref.map(|session| &session.ours),
            ))
            .with_theirs(trace_payload_stats(
                session_ref.map(|session| &session.theirs),
            ))
            .with_conflict_block_count(session_ref.map(|session| session.regions.len()))
        });

        let stages_started = Instant::now();
        let stages = if !load_full {
            Ok(Some(empty_conflict_file_stages(path.clone())))
        } else if let Some(session) = session_ref {
            Ok(Some(conflict_file_stages_from_session(
                path.clone(),
                session,
            )))
        } else {
            match repo.conflict_file_stages(&path) {
                Ok(v) => Ok(v),
                Err(e) if matches!(e.kind(), ErrorKind::Unsupported(_)) => repo
                    .diff_file_text(&DiffTarget::WorkingTree {
                        path: path.clone(),
                        area: DiffArea::Unstaged,
                    })
                    .map(|opt| {
                        opt.map(|d| {
                            let ours_bytes = d
                                .old
                                .as_ref()
                                .map(|text| Arc::<[u8]>::from(text.as_bytes()));
                            let theirs_bytes = d
                                .new
                                .as_ref()
                                .map(|text| Arc::<[u8]>::from(text.as_bytes()));
                            ConflictFileStages {
                                path: d.path,
                                base_bytes: None,
                                ours_bytes,
                                theirs_bytes,
                                base: None,
                                ours: d.old.map(Arc::<str>::from),
                                theirs: d.new.map(Arc::<str>::from),
                            }
                        })
                    }),
                Err(e) => Err(e),
            }
        };
        let stage_ref = stages.as_ref().ok().and_then(|opt| opt.as_ref());
        mergetool_trace::record_with(|| {
            MergetoolTraceEvent::new(
                MergetoolTraceStage::LoadConflictFileStages,
                Some(trace_path.clone()),
                stages_started.elapsed(),
            )
            .with_base(trace_side_stats(
                stage_ref.and_then(|stage| stage.base_bytes.as_deref()),
                stage_ref.and_then(|stage| stage.base.as_deref()),
            ))
            .with_ours(trace_side_stats(
                stage_ref.and_then(|stage| stage.ours_bytes.as_deref()),
                stage_ref.and_then(|stage| stage.ours.as_deref()),
            ))
            .with_theirs(trace_side_stats(
                stage_ref.and_then(|stage| stage.theirs_bytes.as_deref()),
                stage_ref.and_then(|stage| stage.theirs.as_deref()),
            ))
        });

        let current_started = Instant::now();
        let (current_trace_stage, current_bytes, current) = if let Some((current_bytes, current)) =
            session_ref.and_then(conflict_file_current_from_session)
        {
            (
                MergetoolTraceStage::LoadCurrentReuse,
                current_bytes,
                current,
            )
        } else {
            let current_bytes = std::fs::read(repo.spec().workdir.join(&path))
                .ok()
                .map(Arc::<[u8]>::from);
            (MergetoolTraceStage::LoadCurrentRead, current_bytes, None)
        };
        let current_text = current.as_deref().or_else(|| {
            current_bytes
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
        });
        mergetool_trace::record_with(|| {
            MergetoolTraceEvent::new(
                current_trace_stage,
                Some(trace_path),
                current_started.elapsed(),
            )
            .with_current(trace_side_stats(current_bytes.as_deref(), current_text))
        });
        let result = if let Some(session) = session_ref {
            stages.map(|opt| {
                opt.map(|_| {
                    crate::model::ConflictFile::from_shared_conflict_session(path.clone(), session)
                })
            })
        } else {
            stages.map(|opt| {
                opt.map(|d| {
                    let gitcomet_core::services::ConflictFileStages {
                        path,
                        base_bytes,
                        ours_bytes,
                        theirs_bytes,
                        base,
                        ours,
                        theirs,
                    } = d;
                    crate::model::ConflictFile::from_loaded_stage_parts(
                        path,
                        (base_bytes, base),
                        (ours_bytes, ours),
                        (theirs_bytes, theirs),
                        (current_bytes, current),
                    )
                })
            })
        };

        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id,
                path,
                result: Box::new(result),
                conflict_session,
            }),
        );
    });
}

pub(super) fn schedule_load_reflog(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    limit: usize,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::ReflogLoaded {
                    repo_id,
                    result: repo.reflog_head(limit),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::ReflogLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_file_history(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    limit: usize,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
                repo_id,
                path: path.clone(),
                result: repo.log_file_page(&path, limit, None),
            }),
        );
    });
}

pub(super) fn schedule_load_blame(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
    rev: Option<String>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.blame_file(&path, rev.as_deref());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
                repo_id,
                path: path.clone(),
                rev: rev.clone(),
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_worktrees(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                repo_id,
                result: repo.list_worktrees(),
            }),
        );
    });
}

pub(super) fn schedule_load_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                repo_id,
                result: repo.list_submodules(),
            }),
        );
    });
}

pub(super) fn schedule_load_subtrees(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubtreesLoaded {
                repo_id,
                result: repo.list_subtrees(),
            }),
        );
    });
}

pub(super) fn schedule_load_rebase_state(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.rebase_in_progress(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_rebase_and_merge_state(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.rebase_in_progress(),
                }),
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_merge_commit_message(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_commit_details(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
                repo_id,
                commit_id: commit_id.clone(),
                result: repo.commit_details(&commit_id),
            }),
        );
    });
}

pub(super) fn schedule_load_diff(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        // UI consumes this parsed diff through paged/lazy row adapters.
        let result = repo.diff_parsed(&target);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_diff_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_file_text(&target);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_diff_file_image(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_file_image(&target);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_selected_diff(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    target: DiffTarget,
    load_file_text: bool,
    load_file_image: bool,
) {
    if load_file_image {
        schedule_load_diff_file_image(executor, repos, msg_tx.clone(), repo_id, target.clone());
    }
    if load_file_text {
        schedule_load_diff_file(executor, repos, msg_tx.clone(), repo_id, target.clone());
    }
    schedule_load_diff(executor, repos, msg_tx, repo_id, target);
}
