use crate::model::{AppState, ConflictFileLoadMode};
use crate::msg::Msg;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession, ConflictStageParts};
use gitcomet_core::domain::{
    DiffArea, DiffPreviewTextSide, DiffTarget, LogCursor, LogScope, RepoStatus, Worktree,
    WorktreeDirtySummary, count_file_statuses,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CancellationToken, ConflictFileStages, GitBackend, GitRepository};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
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

/// Scans every *other* linked worktree for uncommitted changes.
///
/// The worktree list is re-read inside the task rather than passed in from
/// state: this way the scan can never disagree with the paths it walks, and it
/// has no ordering dependency on `LoadWorktrees` having landed first. One `git
/// worktree list` is negligible next to the per-worktree status scans that
/// follow.
///
/// A worktree that cannot be opened or stat'd is skipped rather than failing the
/// whole scan — removed directories and unmounted volumes are routine.
pub(super) fn schedule_load_worktree_dirty(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    own_workdir: PathBuf,
    files_for: Option<PathBuf>,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-worktree-dirty",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let own_workdir = canonicalize_or_original(own_workdir);
            let result = repo
                .list_worktrees_cancellable(&cancellation)
                .inspect_err(|_| {
                    // The listing is what names the live worktrees, so a failed
                    // one leaves nothing to prune against -- and its causes (a
                    // concurrent `git worktree prune`, a permissions blip, an
                    // unmounted volume) are exactly when stale handles pile up.
                    // Drop this repo's cache outright: it is only a cache, and
                    // the next scan reopens whatever it still needs. A cancelled
                    // listing is not evidence of anything, so it keeps its
                    // handles.
                    if !cancellation.is_cancelled() {
                        retain_worktree_scan_handles(worktree_scan_handles(), repo_id, &[]);
                    }
                })
                .map(|worktrees| {
                    let mut summaries = Vec::new();
                    let mut scanned = Vec::with_capacity(worktrees.len());
                    for worktree in worktrees {
                        if cancellation.is_cancelled() {
                            break;
                        }
                        if is_own_worktree(&worktree.path, &own_workdir) {
                            continue;
                        }
                        scanned.push(worktree.path.clone());
                        let Some(handle) = worktree_scan_handle(
                            worktree_scan_handles(),
                            &*backend,
                            repo_id,
                            &worktree.path,
                        ) else {
                            continue;
                        };
                        let status = match handle.status_cancellable(&cancellation) {
                            Ok(status) => status,
                            // Cancellation surfaces as an `Err` here like any
                            // other failure, and it says nothing about the
                            // handle. `CancelRepoLoads` fires on every tab
                            // switch, reload and completed action, so treating
                            // it as a broken handle would discard healthy ones
                            // routinely and pay full discovery to reopen them.
                            Err(_) if cancellation.is_cancelled() => break,
                            Err(_) => {
                                // A handle that cannot report status is not
                                // worth keeping: the worktree may have been
                                // removed or replaced underneath it.
                                forget_worktree_scan_handle(
                                    worktree_scan_handles(),
                                    repo_id,
                                    &worktree.path,
                                );
                                continue;
                            }
                        };
                        // Only the selected worktree's files are carried back;
                        // see `WorktreeDirtySummary`.
                        let keep_files = files_for.as_deref() == Some(worktree.path.as_path());
                        let summary = worktree_dirty_summary(worktree, status, keep_files);
                        if summary.is_dirty() {
                            summaries.push(summary);
                        }
                    }
                    // A cancelled scan saw only part of the list, so its `scanned`
                    // set would prune handles that are still live.
                    if !cancellation.is_cancelled() {
                        retain_worktree_scan_handles(worktree_scan_handles(), repo_id, &scanned);
                    }
                    summaries
                });
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded { repo_id, result }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Repository handles for the *other* worktrees, kept between scans.
///
/// The scan runs on every git-state flush, and opening a repository is discovery
/// plus config parsing — several milliseconds each, paid per worktree per scan
/// for handles that were identical the last time round. Status itself re-reads
/// the index and the worktree, so a reused handle reports fresh results; a handle
/// that fails is dropped ([`forget_worktree_scan_handle`]) and reopened next time.
///
/// Keyed by repo as well as path: entries are pruned against the worktree list of
/// the scan that owns them ([`retain_worktree_scan_handles`]), dropped outright
/// when the repo's tab closes ([`release_worktree_scan_handles`]), and one repo's
/// scan must not evict another's.
static WORKTREE_SCAN_HANDLES: OnceLock<Mutex<WorktreeScanHandles>> = OnceLock::new();

/// Handles held open across scans. Each one keeps file descriptors and mapped
/// index data alive, so the total is capped rather than left to grow with every
/// worktree the session has ever looked at.
const WORKTREE_SCAN_HANDLE_LIMIT: usize = 16;

#[derive(Default)]
struct WorktreeScanHandles {
    entries: HashMap<(RepoId, PathBuf), WorktreeScanHandle>,
    /// Ticks once per lookup; the entry holding the highest tick is the hottest.
    clock: u64,
}

struct WorktreeScanHandle {
    handle: Arc<dyn GitRepository>,
    last_used: u64,
}

impl WorktreeScanHandles {
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    /// Frees a slot for a new entry belonging to `repo_id`.
    ///
    /// The *most* recently used of that repo's own entries goes, not the coldest.
    /// A scan walks `list_worktrees` in order and every scan walks the same list,
    /// so the access pattern is a cycle -- LRU's textbook worst case. Under LRU a
    /// repo with more worktrees than the limit evicts precisely the entry its next
    /// scan reaches first, every time, and reopens all of them on every pass: the
    /// same behaviour clearing the map wholesale had, which is the case this cache
    /// exists for. Giving up the entry the scan has just finished with instead
    /// leaves a stable prefix of the repo's worktrees cached and confines the
    /// reopens to the tail.
    ///
    /// A repo with nothing cached yet has nothing of its own to give up, so it
    /// takes the globally coldest entry -- there LRU is right, because another
    /// repo's oldest handle really is the least likely to be wanted next.
    fn evict_one_for(&mut self, repo_id: RepoId) -> bool {
        let victim = self
            .entries
            .iter()
            .filter(|((entry_repo, _), _)| *entry_repo == repo_id)
            .max_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
            .or_else(|| {
                self.entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(key, _)| key.clone())
            });
        match victim {
            Some(victim) => {
                self.entries.remove(&victim);
                true
            }
            None => false,
        }
    }
}

fn worktree_scan_handles() -> &'static Mutex<WorktreeScanHandles> {
    WORKTREE_SCAN_HANDLES.get_or_init(|| Mutex::new(WorktreeScanHandles::default()))
}

fn lock_worktree_scan_handles(
    handles: &Mutex<WorktreeScanHandles>,
) -> std::sync::MutexGuard<'_, WorktreeScanHandles> {
    handles
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The map is threaded in rather than reached for through the static so tests can
/// drive an instance of their own. Sharing the process-wide one made the handle
/// tests evict each other's entries whenever cargo ran them in parallel.
fn worktree_scan_handle(
    handles: &Mutex<WorktreeScanHandles>,
    backend: &dyn GitBackend,
    repo_id: RepoId,
    path: &Path,
) -> Option<Arc<dyn GitRepository>> {
    let key = (repo_id, path.to_path_buf());
    {
        let mut handles = lock_worktree_scan_handles(handles);
        let now = handles.tick();
        if let Some(entry) = handles.entries.get_mut(&key) {
            entry.last_used = now;
            return Some(Arc::clone(&entry.handle));
        }
    }

    // Opened outside the lock: this is the several-millisecond discovery-and-
    // config-parse the cache exists to avoid, and holding the map across it would
    // stall every other repo's scan behind it.
    let handle = backend.open(path).ok()?;
    let mut handles = lock_worktree_scan_handles(handles);
    while handles.entries.len() >= WORKTREE_SCAN_HANDLE_LIMIT && handles.evict_one_for(repo_id) {}
    let last_used = handles.tick();
    handles.entries.insert(
        key,
        WorktreeScanHandle {
            handle: Arc::clone(&handle),
            last_used,
        },
    );
    Some(handle)
}

/// Drops this repo's handles for worktrees the current scan did not walk — ones
/// that have been pruned, removed, or unmounted since the last scan. Called with
/// the paths the scan actually saw, so a repo never accumulates handles for
/// worktrees that no longer exist.
fn retain_worktree_scan_handles(
    handles: &Mutex<WorktreeScanHandles>,
    repo_id: RepoId,
    seen: &[PathBuf],
) {
    lock_worktree_scan_handles(handles)
        .entries
        .retain(|(entry_repo, path), _| *entry_repo != repo_id || seen.contains(path));
}

fn forget_worktree_scan_handle(handles: &Mutex<WorktreeScanHandles>, repo_id: RepoId, path: &Path) {
    lock_worktree_scan_handles(handles)
        .entries
        .remove(&(repo_id, path.to_path_buf()));
}

/// Drops every handle a repo holds, for when the repo itself goes away.
///
/// Nothing else can: the per-scan prune only runs from that repo's own scan, so a
/// closed tab's handles -- file descriptors and mapped index data, one set per
/// linked worktree -- would otherwise sit there for the life of the process,
/// released only if unrelated repos happened to push the map to its limit.
pub(in crate::store) fn release_worktree_scan_handles(repo_id: RepoId) {
    lock_worktree_scan_handles(worktree_scan_handles())
        .entries
        .retain(|(entry_repo, _), _| *entry_repo != repo_id);
}

/// Whether `worktree_path` is the worktree this tab already has open — those
/// changes belong in the pinned working-tree row, not in a linked-worktree one.
///
/// `own_workdir` is canonicalized: the spec's workdir goes through
/// `canonicalize_path` when the repo is opened. Git reports the path it recorded
/// verbatim, so the two can spell the same directory differently — `/tmp/x`
/// against `/private/tmp/x` on macOS, or either side of a symlinked repo root on
/// any platform. A raw comparison misses that, and this tab's own changes come
/// back a second time as a duplicate row anchored on HEAD. The cheap equality is
/// tried first so the common case costs no syscall.
fn is_own_worktree(worktree_path: &Path, own_workdir: &Path) -> bool {
    worktree_path == own_workdir
        || canonicalize_or_original(worktree_path.to_path_buf()) == own_workdir
}

/// Counts always; the file lists only when this is the worktree the details pane
/// is showing. The counts are derived before the lists are dropped, so a summary
/// without files still reports exactly what the row renders.
fn worktree_dirty_summary(
    worktree: Worktree,
    status: RepoStatus,
    keep_files: bool,
) -> WorktreeDirtySummary {
    let (added, modified, deleted) = count_file_statuses(&status.unstaged);
    let (staged_added, staged_modified, staged_deleted) = count_file_statuses(&status.staged);
    let (staged, unstaged) = if keep_files {
        (status.staged, status.unstaged)
    } else {
        (Vec::new(), Vec::new())
    };
    WorktreeDirtySummary {
        path: worktree.path,
        head: worktree.head,
        branch: worktree.branch,
        detached: worktree.detached,
        added: added + staged_added,
        modified: modified + staged_modified,
        deleted: deleted + staged_deleted,
        staged,
        unstaged,
    }
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

pub(super) fn schedule_load_hover_commit_message(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    let fallback_id = commit_id.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-hover-commit-message",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            // `commit_messages` is message-only by design, so hovering a row
            // does not pay for the tree diff `commit_details` computes.
            let result = repo
                .commit_messages(std::slice::from_ref(&commit_id))
                .and_then(|mut messages| {
                    if messages.is_empty() {
                        Err(Error::new(ErrorKind::Backend(format!(
                            "no message for commit {}",
                            commit_id.as_ref()
                        ))))
                    } else {
                        Ok(messages.remove(0))
                    }
                });
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
                    repo_id,
                    commit_id,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
                    repo_id,
                    commit_id: fallback_id,
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

#[cfg(test)]
mod worktree_dirty_tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, FileStatus, FileStatusKind};

    fn status(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind,
            conflict: None,
        }
    }

    fn worktree() -> Worktree {
        Worktree {
            path: PathBuf::from("/wt/side"),
            head: Some(CommitId("abc123".into())),
            branch: Some("side".into()),
            detached: false,
        }
    }

    #[test]
    fn a_summary_sums_staged_and_unstaged_into_the_three_buckets() {
        let repo_status = RepoStatus {
            staged: vec![status("gone.txt", FileStatusKind::Deleted)],
            unstaged: vec![
                status("edited.txt", FileStatusKind::Modified),
                status("new.txt", FileStatusKind::Untracked),
            ],
        };

        let summary = worktree_dirty_summary(worktree(), repo_status, true);
        assert_eq!(
            (summary.added, summary.modified, summary.deleted),
            (1, 1, 1)
        );
        assert!(summary.is_dirty());
        // The rows list these files, so the scan has to hand them over rather
        // than reducing them to counts and dropping them.
        assert_eq!(summary.unstaged.len(), 2);
        assert_eq!(summary.staged.len(), 1);
        assert_eq!(summary.staged[0].path, PathBuf::from("gone.txt"));
    }

    /// The row is anchored by HEAD and labelled by branch, so the scan has to
    /// carry both through; the repo's own worktree list is loaded lazily and
    /// cannot be relied on.
    #[test]
    fn a_summary_carries_the_worktrees_identity() {
        let summary = worktree_dirty_summary(worktree(), RepoStatus::default(), true);
        assert_eq!(summary.path, PathBuf::from("/wt/side"));
        assert_eq!(summary.head.as_ref().map(|id| id.as_ref()), Some("abc123"));
        assert_eq!(summary.branch.as_deref(), Some("side"));
        assert!(!summary.detached);
        assert!(!summary.is_dirty(), "a clean tree must not be reported");
    }

    /// The scan's own workdir arrives canonicalized while git reports the path it
    /// recorded, so the two spell the same directory differently whenever the repo
    /// root is reached through a symlink. Missing that match scans this tab's own
    /// worktree and renders its changes twice.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_repo_root_is_still_recognised_as_our_own_worktree() {
        let dir = tempfile::tempdir().expect("temp dir");
        let real = dir.path().join("real");
        std::fs::create_dir(&real).expect("create real worktree dir");
        let linked = dir.path().join("linked");
        std::os::unix::fs::symlink(&real, &linked).expect("symlink");

        let own_workdir = canonicalize_or_original(linked.clone());
        assert_ne!(
            own_workdir, linked,
            "fixture must actually produce two spellings of one directory"
        );

        assert!(
            is_own_worktree(&linked, &own_workdir),
            "the path git reports must still match our canonicalized workdir"
        );
        assert!(
            is_own_worktree(&real, &own_workdir),
            "and so must the resolved one"
        );
        assert!(
            !is_own_worktree(&dir.path().join("other"), &own_workdir),
            "a genuinely different worktree is still scanned"
        );
    }
}

#[cfg(test)]
mod worktree_scan_handle_tests {
    use super::*;

    use crate::store::tests::DummyRepo;

    /// Counts opens so the tests can tell a cache hit from a reopen.
    struct CountingBackend {
        opens: std::sync::atomic::AtomicUsize,
    }

    impl GitBackend for CountingBackend {
        fn open(&self, workdir: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            self.opens
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Arc::new(DummyRepo::new(&workdir.to_string_lossy())))
        }
    }

    impl CountingBackend {
        fn new() -> Self {
            Self {
                opens: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn opens(&self) -> usize {
            self.opens.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    /// A map of this test's own. These used to share the process-wide static, and
    /// the over-the-limit test below evicts sixteen entries as it runs — under
    /// cargo's parallel harness it was evicting the *other* tests' entries, and
    /// they failed on an open count one higher than they asked for.
    fn handles() -> Mutex<WorktreeScanHandles> {
        Mutex::new(WorktreeScanHandles::default())
    }

    /// Opening a repository is discovery plus config parsing, and the scan runs on
    /// every git-state flush. A repo with more worktrees than the cache holds must
    /// still keep its hottest ones: clearing the map wholesale at the limit made
    /// exactly that case reopen nearly everything every scan.
    #[test]
    fn a_repo_over_the_handle_limit_keeps_its_hottest_worktrees() {
        let handles = handles();
        let backend = CountingBackend::new();
        let repo_id = RepoId(4_001);
        let hot = PathBuf::from("/wt/hot-4001");
        let paths: Vec<PathBuf> = (0..WORKTREE_SCAN_HANDLE_LIMIT + 4)
            .map(|ix| PathBuf::from(format!("/wt/cold-4001-{ix}")))
            .collect();

        // The hot worktree is touched before and after every cold one. Eviction
        // takes the entry the scan has just finished with, which is always the
        // cold one -- touching `hot` again immediately afterwards makes it the
        // most recent, but by then the slot has already been taken.
        worktree_scan_handle(&handles, &backend, repo_id, &hot).expect("stub backend opens");
        for path in &paths {
            worktree_scan_handle(&handles, &backend, repo_id, path).expect("stub backend opens");
            worktree_scan_handle(&handles, &backend, repo_id, &hot).expect("stub backend opens");
        }

        let opens_before = backend.opens();
        worktree_scan_handle(&handles, &backend, repo_id, &hot).expect("stub backend opens");
        assert_eq!(
            backend.opens(),
            opens_before,
            "the repeatedly used worktree must still be cached"
        );
    }

    /// The access pattern this cache actually sees: every scan walks
    /// `list_worktrees` in the same order, so a repo with more worktrees than the
    /// limit revisits them cyclically. That is LRU's worst case — it evicts
    /// precisely the entry the next scan reaches first, and every handle is
    /// reopened on every pass, which is the behaviour the cache exists to prevent.
    #[test]
    fn repeated_scans_of_an_oversized_repo_do_not_reopen_everything() {
        let handles = handles();
        let backend = CountingBackend::new();
        let repo_id = RepoId(4_005);
        let worktrees: Vec<PathBuf> = (0..WORKTREE_SCAN_HANDLE_LIMIT + 4)
            .map(|ix| PathBuf::from(format!("/wt/seq-4005-{ix}")))
            .collect();

        let scan = || {
            let before = backend.opens();
            for path in &worktrees {
                worktree_scan_handle(&handles, &backend, repo_id, path)
                    .expect("stub backend opens");
            }
            backend.opens() - before
        };

        assert_eq!(
            scan(),
            worktrees.len(),
            "the first scan has nothing cached and opens every worktree"
        );

        let second = scan();
        let third = scan();
        // Under LRU both of these were `worktrees.len()` -- a complete miss on
        // every worktree, on every scan, forever.
        assert!(
            second < worktrees.len() && third < worktrees.len(),
            "a cyclic rescan must keep hitting the cache, got {second} then {third} \
             reopens of {} worktrees",
            worktrees.len()
        );
        assert!(
            third <= worktrees.len() - WORKTREE_SCAN_HANDLE_LIMIT + 1,
            "all but the tail past the limit should stay cached, got {third} reopens"
        );
    }

    /// Handles are pruned against the worktree list the scan actually walked, so a
    /// removed worktree does not keep a repository open for the process lifetime.
    #[test]
    fn a_scan_drops_handles_for_worktrees_it_no_longer_lists() {
        let handles = handles();
        let backend = CountingBackend::new();
        let repo_id = RepoId(4_002);
        let kept = PathBuf::from("/wt/kept-4002");
        let removed = PathBuf::from("/wt/removed-4002");

        worktree_scan_handle(&handles, &backend, repo_id, &kept).expect("stub backend opens");
        worktree_scan_handle(&handles, &backend, repo_id, &removed).expect("stub backend opens");
        assert_eq!(backend.opens(), 2);

        retain_worktree_scan_handles(&handles, repo_id, std::slice::from_ref(&kept));

        worktree_scan_handle(&handles, &backend, repo_id, &kept).expect("stub backend opens");
        assert_eq!(
            backend.opens(),
            2,
            "the worktree still in the list keeps its handle"
        );
        worktree_scan_handle(&handles, &backend, repo_id, &removed).expect("stub backend opens");
        assert_eq!(
            backend.opens(),
            3,
            "the worktree dropped from the list must have been released"
        );
    }

    /// One repo's scan must not evict another's handles: the map is process-wide.
    #[test]
    fn pruning_one_repos_handles_leaves_another_repos_alone() {
        let handles = handles();
        let backend = CountingBackend::new();
        let mine = RepoId(4_003);
        let theirs = RepoId(4_004);
        let path = PathBuf::from("/wt/shared-4003");

        worktree_scan_handle(&handles, &backend, mine, &path).expect("stub backend opens");
        worktree_scan_handle(&handles, &backend, theirs, &path).expect("stub backend opens");
        assert_eq!(backend.opens(), 2);

        retain_worktree_scan_handles(&handles, mine, &[]);

        worktree_scan_handle(&handles, &backend, theirs, &path).expect("stub backend opens");
        assert_eq!(
            backend.opens(),
            2,
            "the other repo's handle must survive this repo's prune"
        );
    }

    /// Closing a repo is the only thing that can release its handles: the per-scan
    /// prune runs from that repo's own scan, and a closed repo never scans again.
    /// Without this the handles sat there for the life of the process.
    #[test]
    fn closing_a_repo_releases_every_handle_it_held() {
        let backend = CountingBackend::new();
        let closed = RepoId(4_006);
        let other = RepoId(4_007);
        let paths = [
            PathBuf::from("/wt/closing-4006-a"),
            PathBuf::from("/wt/closing-4006-b"),
        ];

        // The release hook reaches the static directly -- it is called from the
        // reducer, which has no map to hand it -- so this one test uses it, and
        // cleans up after itself by releasing both repos.
        let shared = worktree_scan_handles();
        for path in &paths {
            worktree_scan_handle(shared, &backend, closed, path).expect("stub backend opens");
        }
        worktree_scan_handle(shared, &backend, other, &paths[0]).expect("stub backend opens");

        release_worktree_scan_handles(closed);

        let held = |repo_id: RepoId| {
            lock_worktree_scan_handles(shared)
                .entries
                .keys()
                .filter(|(entry_repo, _)| *entry_repo == repo_id)
                .count()
        };
        assert_eq!(held(closed), 0, "the closed repo must hold nothing");
        assert_eq!(held(other), 1, "and must not have taken anyone else's");

        release_worktree_scan_handles(other);
    }
}

#[cfg(test)]
mod worktree_dirty_files_tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, FileStatus, FileStatusKind};

    fn status(paths: &[&str]) -> RepoStatus {
        RepoStatus {
            staged: Vec::new(),
            unstaged: paths
                .iter()
                .map(|path| FileStatus {
                    path: PathBuf::from(path),
                    kind: FileStatusKind::Modified,
                    conflict: None,
                })
                .collect(),
        }
    }

    fn worktree() -> Worktree {
        Worktree {
            path: PathBuf::from("/wt/side"),
            head: Some(CommitId("abc123".into())),
            branch: Some("side".into()),
            detached: false,
        }
    }

    /// The file lists are the expensive part of a summary -- an un-ignored build
    /// directory puts tens of thousands of paths in them -- and only the selected
    /// worktree's are ever rendered. The counts have to survive either way, since
    /// every row shows them.
    #[test]
    fn only_the_selected_worktree_carries_its_files() {
        let kept = worktree_dirty_summary(worktree(), status(&["a.rs", "b.rs"]), true);
        assert_eq!((kept.added, kept.modified, kept.deleted), (0, 2, 0));
        assert_eq!(
            kept.unstaged.len(),
            2,
            "the selected worktree keeps its files"
        );

        let counted = worktree_dirty_summary(worktree(), status(&["a.rs", "b.rs"]), false);
        assert_eq!(
            (counted.added, counted.modified, counted.deleted),
            (0, 2, 0),
            "counts are derived before the lists are dropped"
        );
        assert!(
            counted.unstaged.is_empty() && counted.staged.is_empty(),
            "an unselected worktree carries counts alone"
        );
        assert!(
            counted.is_dirty(),
            "and still reports as dirty, or its row would disappear"
        );
    }
}
