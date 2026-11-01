use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{send_or_log, spawn_with_repo};
use crate::model::ConflictFileLoadMode;
use crate::msg::Msg;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession, ConflictStageParts};
use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::mergetool_trace::{
    self, MergetoolTraceEvent, MergetoolTraceSideStats, MergetoolTraceStage,
};
use gitcomet_core::services::ConflictFileStages;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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

pub(super) struct ConflictFileRequest {
    pub(super) request: crate::model::ConflictLoadRequestId,
    pub(super) path: PathBuf,
    pub(super) mode: ConflictFileLoadMode,
    pub(super) encoding: Option<gitcomet_core::text_format::TextEncoding>,
}

pub(super) fn schedule_load_conflict_file(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    load: ConflictFileRequest,
) {
    let ConflictFileRequest {
        request,
        path,
        mode,
        encoding,
    } = load;
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let trace_path = path.clone();
        let load_full = matches!(mode, ConflictFileLoadMode::Full);

        let conflict_session_started = Instant::now();
        let conflict_session = load_full
            .then(|| repo.conflict_session(&path, encoding).ok().flatten())
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
                    .diff_file_text(
                        &DiffTarget::working_tree(path.clone(), DiffArea::Unstaged),
                        None,
                        &gitcomet_core::services::CancellationToken::new(),
                    )
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
                request,
                repo_id,
                path,
                result: Box::new(result),
                conflict_session: conflict_session.map(Box::new),
            }),
        );
    });
}
