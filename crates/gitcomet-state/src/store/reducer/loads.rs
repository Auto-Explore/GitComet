//! Load orchestration: repository loads gated on their load epoch, and tag
//! push previews superseded by generation and cancellation token.

use super::reduce;
use crate::model::{AppState, Loadable, RepoId};
use crate::msg::{Effect, InternalMsg, Msg};
use crate::store::repo_load_trace;
use gitcomet_core::services::{CancellationToken, GitRepository};
use gitcomet_core::tag_push::{TagPushMode, TagPushPreview, TagPushRequest};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub(super) fn preview_tag_push(
    state: &mut AppState,
    repo_id: RepoId,
    request: TagPushRequest,
    cancellation: CancellationToken,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return vec![];
    };
    let slot = &mut repo.tag_push_previews[request.mode.index()];
    let generation = slot.as_ref().map_or(1, |previous| {
        previous.cancellation.cancel();
        previous.generation.wrapping_add(1)
    });
    *slot = Some(crate::model::TagPushPreviewState {
        request: request.clone(),
        generation,
        cancellation: cancellation.clone(),
        result: Loadable::Loading,
    });
    vec![Effect::PreviewTagPush {
        repo_id,
        request,
        generation,
        cancellation,
    }]
}

pub(super) fn tag_push_preview_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    mode: TagPushMode,
    generation: u64,
    result: gitcomet_core::services::Result<TagPushPreview>,
) -> Vec<Effect> {
    if let Some(slot) = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.tag_push_previews[mode.index()].as_mut())
        && slot.generation == generation
        && !slot.cancellation.is_cancelled()
    {
        slot.result = match result {
            Ok(preview) => Loadable::Ready(Arc::new(preview)),
            Err(error) => Loadable::Error(error.to_string()),
        };
    }
    vec![]
}

pub(super) fn repo_load_finished(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    repo_id: RepoId,
    load_epoch: u64,
    message: Box<InternalMsg>,
) -> Vec<Effect> {
    let current_load_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.load_epoch);
    if current_load_epoch == Some(load_epoch) {
        repo_load_trace::trace!(
            "apply_repo_load_finished repo_id={:?} load_epoch={} inner={}",
            repo_id,
            load_epoch,
            repo_load_trace::internal_msg_name(&message)
        );
        reduce(repos, id_alloc, state, Msg::Internal(*message))
    } else {
        repo_load_trace::trace!(
            "drop_stale_repo_load_finished repo_id={:?} load_epoch={} current_load_epoch={:?} inner={}",
            repo_id,
            load_epoch,
            current_load_epoch,
            repo_load_trace::internal_msg_name(&message)
        );
        Vec::new()
    }
}
