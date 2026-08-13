use crate::model::{AppState, ConflictFileLoadMode};
use crate::msg::Msg;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession, ConflictStageParts};
use gitcomet_core::domain::{DiffArea, DiffPreviewTextSide, DiffTarget, LogCursor, LogScope};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use gitcomet_core::services::{CancellationToken, ConflictFileStages, GitBackend, GitRepository};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{
    RepoMap, send_or_log, spawn_detached_with_repo_or_else, spawn_with_repo,
    spawn_with_repo_or_else,
};

pub(super) struct SelectedDiffLoadOptions {
    pub(super) load_patch_diff: bool,
    pub(super) load_file_text: bool,
    pub(super) preview_text_side: Option<DiffPreviewTextSide>,
    pub(super) load_submodule_summary: bool,
    pub(super) load_file_image: bool,
}

#[derive(Clone)]
pub(super) struct SelectedDiffLoadGuard {
    thread_state: Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
    target: DiffTarget,
    target_rev: u64,
}

impl SelectedDiffLoadGuard {
    pub(super) fn new(
        thread_state: Arc<RwLock<Arc<AppState>>>,
        repo_id: RepoId,
        target: DiffTarget,
        target_rev: u64,
    ) -> Self {
        Self {
            thread_state,
            repo_id,
            target,
            target_rev,
        }
    }

    fn is_current(&self) -> bool {
        let state = self.thread_state.read().unwrap_or_else(|e| e.into_inner());
        state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .is_some_and(|repo| {
                repo.diff_state.diff_target_rev == self.target_rev
                    && repo.diff_state.diff_target.as_ref() == Some(&self.target)
            })
    }
}

fn spawn_with_selected_diff_guard(
    executor: &TaskExecutor,
    repos: &RepoMap,
    repo_id: RepoId,
    msg_tx: StoreWorkerSender,
    guard: SelectedDiffLoadGuard,
    task: impl FnOnce(Arc<dyn GitRepository>, StoreWorkerSender, SelectedDiffLoadGuard) + Send + 'static,
) -> bool {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        if !guard.is_current() {
            return;
        }
        task(repo, msg_tx, guard);
    })
}

#[cfg(test)]
mod selected_diff_guard_tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::RepoSpec;

    fn target(path: &str) -> DiffTarget {
        DiffTarget::WorkingTree {
            path: PathBuf::from(path),
            area: DiffArea::Unstaged,
        }
    }

    fn thread_state_with_target(
        repo_id: RepoId,
        selected: DiffTarget,
        target_rev: u64,
    ) -> Arc<RwLock<Arc<AppState>>> {
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/selected-diff-guard-test"),
            },
        );
        repo.diff_state.diff_target = Some(selected);
        repo.diff_state.diff_target_rev = target_rev;

        let mut state = AppState::default();
        state.repos.push(repo);
        Arc::new(RwLock::new(Arc::new(state)))
    }

    #[test]
    fn selected_diff_guard_accepts_current_target_and_revision() {
        let repo_id = RepoId(1);
        let selected = target("src/lib.rs");
        let guard = SelectedDiffLoadGuard::new(
            thread_state_with_target(repo_id, selected.clone(), 7),
            repo_id,
            selected,
            7,
        );

        assert!(guard.is_current());
    }

    #[test]
    fn selected_diff_guard_rejects_stale_target_or_revision() {
        let repo_id = RepoId(1);
        let selected = target("src/lib.rs");
        let thread_state = thread_state_with_target(repo_id, selected.clone(), 7);

        let stale_revision =
            SelectedDiffLoadGuard::new(Arc::clone(&thread_state), repo_id, selected.clone(), 6);
        assert!(!stale_revision.is_current());

        let stale_target =
            SelectedDiffLoadGuard::new(thread_state, repo_id, target("src/main.rs"), 7);
        assert!(!stale_target.is_current());
    }
}

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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-branches",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
                    repo_id,
                    result: repo.list_branches_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-remotes",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
                    repo_id,
                    result: repo.list_remotes_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-remote-branches",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
                    repo_id,
                    result: repo.list_remote_branches_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-status",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
                    repo_id,
                    result: repo.status_cancellable(&cancellation),
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

pub(super) fn schedule_load_worktree_status(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-worktree-status",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
                    repo_id,
                    result: repo.worktree_status_cancellable(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_staged_status(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-staged-status",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
                    repo_id,
                    result: repo.staged_status_cancellable(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-head-branch",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                    repo_id,
                    result: repo.current_branch_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-upstream-divergence",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
                    repo_id,
                    result: repo.upstream_divergence_cancellable(&cancellation),
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

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_load_log(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    seq: crate::model::LogLoadSeq,
    scope: LogScope,
    author: Option<String>,
    limit: usize,
    cursor: Option<LogCursor>,
    cancellation: CancellationToken,
) {
    let cursor_on_missing = cursor.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-log",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = {
                let cursor_ref = cursor.as_ref();
                // Report the page as it fills in. Finding one page of a rare
                // author means walking the whole history — over ten seconds on
                // a repository with a million commits — and the user should not
                // be looking at the previous filter's rows for all of it.
                let chunk_tx = msg_tx.clone();
                let mut on_chunk = |chunk: gitcomet_core::services::LogChunk| {
                    send_or_log(
                        &chunk_tx,
                        Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
                            repo_id,
                            seq,
                            commits: chunk.commits,
                            scanned: chunk.scanned,
                        }),
                    );
                };
                repo.log_history_mode_page_streaming(
                    scope,
                    author.as_deref(),
                    limit,
                    cursor_ref,
                    &cancellation,
                    &mut on_chunk,
                )
            };
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                    repo_id,
                    seq,
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
                    seq,
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
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
                    result: repo.list_tags_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
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
                    result: repo.list_remote_tags_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    limit: usize,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-stashes",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let mut entries = repo.stash_list_cancellable(&cancellation);
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
    msg_tx: StoreWorkerSender,
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
                                ours: d.old,
                                theirs: d.new,
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
    msg_tx: StoreWorkerSender,
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
    msg_tx: StoreWorkerSender,
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
) {
    use gitcomet_core::domain::BlameSource;
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = match &source {
            BlameSource::Revision(rev) => repo.blame_file(&path, rev.as_deref()),
            BlameSource::WorkingTree(area) => repo.blame_worktree_file(&path, *area),
        };
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
                repo_id,
                path: path.clone(),
                source: source.clone(),
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_worktrees(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-worktrees",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                    repo_id,
                    result: repo.list_worktrees_cancellable(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_ref_metadata(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-ref-metadata",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
                    repo_id,
                    result: repo.list_ref_metadata_cancellable(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_submodules(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                    repo_id,
                    result: repo.list_submodules_cancellable(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_file_browser(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    source: gitcomet_core::domain::FileSource,
    _cancellation: CancellationToken,
) {
    let source_for_err = source.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = match &source {
                gitcomet_core::domain::FileSource::WorkingDirectory => repo.list_tree_files(),
                gitcomet_core::domain::FileSource::Commit(commit_id) => {
                    repo.list_tree_files_at_commit(commit_id)
                }
                gitcomet_core::domain::FileSource::Branch(_name) => {
                    Err(Error::new(gitcomet_core::error::ErrorKind::Backend(
                        "branch file listing is not yet implemented".to_string(),
                    )))
                }
            };
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
                    repo_id,
                    source,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
                    repo_id,
                    source: source_for_err,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_rebase_state(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-rebase-state",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.sequencer_state_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-rebase-and-merge-state",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.sequencer_state_cancellable(&cancellation),
                }),
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-merge-commit-message",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message_cancellable(&cancellation),
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
    msg_tx: StoreWorkerSender,
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

/// Resolve a possibly abbreviated reference and load its details in one call.
///
/// `commit_details` runs the reference through `rev-parse`, so this answers
/// "does it exist, and is it unambiguous?" as a side effect of the load the
/// details pane needs anyway.
pub(super) fn schedule_resolve_commit_for_reveal(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    reference: gitcomet_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
                repo_id,
                reference: reference.clone(),
                result: repo.commit_details(&reference),
            }),
        );
    });
}

pub(super) fn schedule_load_range_files(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    from: gitcomet_core::domain::CommitId,
    to: Option<gitcomet_core::domain::CommitId>,
    request: u64,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_range_files(&from, to.as_ref());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
                repo_id,
                from: from.clone(),
                to: to.clone(),
                request,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_squash_message_preview(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    oldest: gitcomet_core::domain::CommitId,
    head: gitcomet_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SquashMessagePreviewLoaded {
                repo_id,
                oldest: oldest.clone(),
                head: head.clone(),
                result: repo.squash_message_preview(&oldest, &head),
            }),
        );
    });
}

/// Payload for scheduling a squash-via-rebase setup load. Bundled so the
/// scheduler stays within the argument-count budget and the fields travel
/// together into the resulting `SquashRebaseSetupLoaded` message.
pub(super) struct SquashRebaseSetupRequest {
    pub base: gitcomet_core::domain::CommitId,
    pub actual_head: gitcomet_core::domain::CommitId,
    pub selected_ids: Vec<gitcomet_core::domain::CommitId>,
    pub reword_id: gitcomet_core::domain::CommitId,
    pub message: String,
    pub count: usize,
}

pub(super) fn schedule_load_squash_rebase_setup(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    request: SquashRebaseSetupRequest,
) {
    let SquashRebaseSetupRequest {
        base,
        actual_head,
        selected_ids,
        reword_id,
        message,
        count,
    } = request;
    let base_str = base.as_ref().to_string();
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.list_commits_for_interactive_rebase(&base_str);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SquashRebaseSetupLoaded {
                repo_id,
                base: base_str,
                actual_head,
                selected_ids,
                reword_id,
                message,
                count,
                result,
            }),
        );
    });
}

pub(super) fn schedule_open_file_at_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::path::PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        // Resolve the file's name in the target commit (it may differ from the
        // path we hold due to a rename), then open content there. On failure or
        // when no mapping is found, fall back to the path as-is.
        let resolved = repo
            .resolve_file_path_at_commit(&path, &commit_id)
            .ok()
            .flatten()
            .unwrap_or(path);
        send_or_log(
            &msg_tx,
            Msg::OpenFileContent {
                repo_id,
                source: gitcomet_core::domain::FileSource::Commit(commit_id),
                path: resolved,
            },
        );
    });
}

pub(super) fn schedule_open_file_at_commit_parent(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::path::PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        match repo.commit_details(&commit_id) {
            Ok(details) => {
                if let Some(parent) = details.parent_ids.first() {
                    // Resolve the file's name in the parent (it may differ from
                    // the path we hold due to a rename), mirroring
                    // `schedule_open_file_at_commit`. Falls back to the path
                    // as-is on failure or when no mapping is found.
                    let resolved = repo
                        .resolve_file_path_at_commit(&path, parent)
                        .ok()
                        .flatten()
                        .unwrap_or(path);
                    send_or_log(
                        &msg_tx,
                        Msg::OpenFileContent {
                            repo_id,
                            source: gitcomet_core::domain::FileSource::Commit(parent.clone()),
                            path: resolved,
                        },
                    );
                }
                // Root commit: no prior revision to open.
            }
            Err(e) => {
                // Could not resolve the commit's parent (e.g. backend/object
                // error). The affordance was shown, so surface the failure
                // instead of silently doing nothing.
                send_or_log(
                    &msg_tx,
                    Msg::ShowBannerError {
                        repo_id: Some(repo_id),
                        message: format!("Could not open file at parent commit: {e}"),
                    },
                );
            }
        }
    });
}

pub(super) fn schedule_load_recent_commit_messages(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    limit: usize,
    request_rev: u64,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
                repo_id,
                request_rev,
                result: repo.recent_commit_messages(limit),
            }),
        );
    });
}

pub(super) fn schedule_load_diff(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
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
    msg_tx: StoreWorkerSender,
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

pub(super) fn schedule_load_diff_preview_text_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
    side: DiffPreviewTextSide,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_preview_text_file(&target, side);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                repo_id,
                target,
                side,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_submodule_summary(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.submodule_diff_summary(&target);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend
            .open(&submodule_repo_path)
            .and_then(|repo| repo.diff_parsed(&target));
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffLoaded {
                repo_id,
                inline_rev,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff_file(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend
            .open(&submodule_repo_path)
            .and_then(|repo| repo.diff_file_text(&target));
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
                repo_id,
                inline_rev,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff_file_image(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend
            .open(&submodule_repo_path)
            .and_then(|repo| repo.diff_file_image(&target));
        send_or_log(
            &msg_tx,
            Msg::Internal(
                crate::msg::InternalMsg::InlineSubmoduleDiffFileImageLoaded {
                    repo_id,
                    inline_rev,
                    target,
                    result,
                },
            ),
        );
    });
}

pub(super) fn schedule_load_diff_file_image(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
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
    thread_state: Arc<RwLock<Arc<AppState>>>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
    target_rev: u64,
    cancellation: CancellationToken,
    options: SelectedDiffLoadOptions,
) {
    let guard = SelectedDiffLoadGuard::new(thread_state, repo_id, target.clone(), target_rev);
    if options.load_submodule_summary {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.submodule_diff_summary_cancellable(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_file_image {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_file_image_cancellable(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if let Some(side) = options.preview_text_side {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_preview_text_file_cancellable(&target, side, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                        repo_id,
                        target,
                        side,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_file_text {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_file_text_cancellable(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_patch_diff {
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            repos,
            repo_id,
            msg_tx,
            guard,
            move |repo, msg_tx, guard| {
                // UI consumes this parsed diff through paged/lazy row adapters.
                let result = repo.diff_parsed_cancellable(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
}

/// Loads the full `%B` message of every selected cherry-pick source commit.
/// Rewording stays unavailable if any lookup fails: falling back to the
/// subject-only seed would make saving the dialog destructive.
pub(super) fn schedule_load_interactive_cherry_pick_messages(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    ids: Vec<String>,
) {
    let fallback_ids = ids.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-interactive-cherry-pick-messages",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let commit_ids = ids
                .iter()
                .map(|id| gitcomet_core::domain::CommitId(id.clone().into()))
                .collect::<Vec<_>>();
            let result = repo
                .topologically_order_commits(&commit_ids)
                .and_then(|ordered_ids| {
                    repo.commit_messages(&ordered_ids).map(|messages| {
                        ordered_ids
                            .into_iter()
                            .map(|id| id.as_ref().to_string())
                            .zip(messages)
                            .collect()
                    })
                });
            send_or_log(
                &msg_tx,
                Msg::Internal(
                    crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                        repo_id,
                        requested_ids: ids,
                        result,
                    },
                ),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(
                    crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                        repo_id,
                        requested_ids: fallback_ids,
                        result: Err(Error::new(ErrorKind::Backend(
                            "repository unavailable while loading cherry-pick commit messages"
                                .to_string(),
                        ))),
                    },
                ),
            );
        },
    );
}

pub(super) fn schedule_load_interactive_rebase_setup(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    base: String,
) {
    let base_for_call = base.clone();
    let base_for_err = base.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-interactive-rebase-setup",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.list_commits_for_interactive_rebase(&base_for_call);
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
                    repo_id,
                    base,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
                    repo_id,
                    base: base_for_err,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}
