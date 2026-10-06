//! Per-repository load tokens: the cancellation scope of a repository's loads,
//! the contexts load effects are scheduled under, and the effects that manage them.

use super::{repo_load, selected_diff_target};
use crate::model::{AppState, Loadable, LogLoadSeq, RepoId};
use crate::store::executor::{LatestTaskSlot, SelectedDiffSlots, TaskExecutor};
use crate::store::repo_load_trace;
use crate::store::worker_channel::StoreWorkerSender;
use gitcomet_core::domain::{DiffPreviewTextSide, DiffTarget, LogCursor, LogScope};
use gitcomet_core::services::{CancellationToken, GitRepository};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone)]
pub(in crate::store) struct RepoTaskToken {
    pub(in crate::store) load_epoch: u64,
    pub(in crate::store) cancellation: CancellationToken,
    /// Cancellation for the *current* log walk alone. An author-filtered walk
    /// on a large repository runs for tens of seconds and the repo-load pool
    /// has one or two threads, so a superseded walk has to be stopped for its
    /// replacement to start at all — but stopping it must not disturb the
    /// repository's other loads, which share [`Self::cancellation`].
    log_cancellation: Arc<Mutex<CancellationToken>>,
    file_browser_cancellation: Arc<Mutex<CancellationToken>>,
    // The revision distinguishes refreshes of the same path. All loads for one
    // selection share a child token, so superseding it leaves log/status work alive.
    pub(super) selected_diff: Arc<Mutex<Option<(DiffTarget, u64, CancellationToken)>>>,
    // Only the store worker changes selections. Avoid locking the shared task
    // slot on unrelated messages or when this repo has no selected-diff work.
    selected_diff_key: Option<(DiffTarget, u64)>,
    selected_diff_slots: SelectedDiffSlots,
    commit_details_slot: LatestTaskSlot,
}

impl RepoTaskToken {
    pub(super) fn new(load_epoch: u64) -> Self {
        Self {
            load_epoch,
            cancellation: CancellationToken::new(),
            log_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            file_browser_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            selected_diff: Arc::new(Mutex::new(None)),
            selected_diff_key: None,
            selected_diff_slots: SelectedDiffSlots::default(),
            commit_details_slot: LatestTaskSlot::default(),
        }
    }

    /// Cancels the log walk in flight, if any, and hands out the token for the
    /// walk that replaces it.
    pub(super) fn take_over_log(&self) -> CancellationToken {
        let mut slot = self
            .log_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        slot.cancel();
        let next = CancellationToken::new();
        *slot = next.clone();
        next
    }

    fn take_over_file_browser(&self) -> CancellationToken {
        let mut current = self
            .file_browser_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        current.cancel();
        *current = CancellationToken::new();
        current.clone()
    }

    pub(super) fn selected_diff_cancellation(
        &mut self,
        target: &DiffTarget,
        revision: u64,
    ) -> CancellationToken {
        self.selected_diff_key = Some((target.clone(), revision));
        let mut slot = self.selected_diff.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((previous, previous_rev, token)) = slot.as_ref() {
            if previous == target && *previous_rev == revision && !token.is_cancelled() {
                return token.clone();
            }
            token.cancel();
        }
        let token = CancellationToken::new().with_parent(self.cancellation.clone());
        *slot = Some((target.clone(), revision, token.clone()));
        token
    }

    pub(in crate::store) fn has_selected_diff_work(&self) -> bool {
        self.selected_diff_key.is_some()
    }

    pub(in crate::store) fn cancel_stale_selected_diff(
        &mut self,
        selected: Option<(&DiffTarget, u64)>,
    ) {
        if self
            .selected_diff_key
            .as_ref()
            .is_none_or(|(target, revision)| selected == Some((target, *revision)))
        {
            return;
        }
        self.selected_diff_key = None;
        let mut slot = self.selected_diff.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|(target, revision, _)| selected != Some((target, *revision)))
            && let Some((_, _, token)) = slot.take()
        {
            token.cancel();
        }
    }

    /// Cancels every task running under this token, log walks included.
    pub(in crate::store) fn cancel(&self) {
        self.cancellation.cancel();
        self.file_browser_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
        self.log_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
    }
}

fn current_repo_load_epoch(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
) -> Option<u64> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.load_epoch)
}

fn ensure_repo_task_token(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    repo_id: RepoId,
) -> Option<RepoTaskToken> {
    let load_epoch = current_repo_load_epoch(thread_state, repo_id).unwrap_or(0);
    if let Some(existing) = repo_task_tokens.get(&repo_id)
        && existing.load_epoch == load_epoch
        && !existing.cancellation.is_cancelled()
    {
        repo_load_trace::trace!(
            "repo_load_token reuse repo_id={:?} load_epoch={}",
            repo_id,
            load_epoch
        );
        return Some(existing.clone());
    }

    let token = RepoTaskToken::new(load_epoch);
    if let Some(previous) = repo_task_tokens.insert(repo_id, token.clone()) {
        repo_load_trace::trace!(
            "repo_load_token replace_and_cancel_previous repo_id={:?} previous_epoch={} new_epoch={}",
            repo_id,
            previous.load_epoch,
            load_epoch
        );
        previous.cancel();
    } else {
        repo_load_trace::trace!(
            "repo_load_token create repo_id={:?} load_epoch={}",
            repo_id,
            load_epoch
        );
    }
    Some(token)
}

pub(super) fn repo_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_token(thread_state, repo_task_tokens, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation.clone());
    Some((msg_tx, token.cancellation))
}

pub(super) fn commit_details_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, LatestTaskSlot)> {
    let token = ensure_repo_task_token(thread_state, repo_task_tokens, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation);
    Some((msg_tx, token.commit_details_slot))
}

/// Like [`repo_load_context`], but for the log walk: the returned token covers
/// this walk alone, and taking it cancels whichever walk it replaces. Messages
/// still ride the repository-wide guard, so a cancelled walk's reply arrives
/// and is dropped by the reducer rather than vanishing silently.
fn log_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_token(thread_state, repo_task_tokens, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation.clone());
    Some((msg_tx, token.take_over_log()))
}

/// Like [`log_load_context`], but a new listing supersedes only the previous
/// listing, and its replies are guarded by that listing's own token.
pub(super) fn file_browser_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_token(thread_state, repo_task_tokens, repo_id)?;
    let cancellation = token.take_over_file_browser();
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, cancellation.clone());
    Some((msg_tx, cancellation))
}

pub(super) fn cancel_repo_loads(
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    repo_id: RepoId,
    load_epoch: u64,
) {
    let matched_token = repo_task_tokens
        .get(&repo_id)
        .is_some_and(|token| token.load_epoch == load_epoch);
    repo_load_trace::trace!(
        "cancel_repo_loads_effect repo_id={:?} load_epoch={} matched_token={}",
        repo_id,
        load_epoch,
        matched_token
    );
    if repo_task_tokens
        .get(&repo_id)
        .is_some_and(|token| token.load_epoch == load_epoch)
        && let Some(token) = repo_task_tokens.remove(&repo_id)
    {
        token.cancel();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_log(
    repo_load_executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    seq: LogLoadSeq,
    scope: LogScope,
    author: Option<String>,
    limit: usize,
    cursor: Option<LogCursor>,
) {
    let request = {
        use gitcomet_core::services::HistoryReadRequest;
        let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
        let repo = state.repos.iter().find(|repo| repo.id == repo_id);
        // Do not let an obsolete effect cancel the replacement walk.
        if repo
            .and_then(|repo| repo.loads_in_flight.active_log_seq())
            .is_some_and(|active| active != seq)
        {
            return;
        }
        let snapshot = repo.and_then(|repo| repo.history_state.log_snapshot.clone());
        match (cursor.as_ref(), repo.map(|repo| &repo.log)) {
            (None, Some(Loadable::Ready(previous))) => HistoryReadRequest::Refresh {
                previous: Arc::clone(previous),
                snapshot,
            },
            _ => HistoryReadRequest::Page {
                limit,
                cursor: cursor.clone(),
                snapshot,
            },
        }
    };
    if let Some((msg_tx, cancellation)) =
        log_load_context(thread_state, repo_task_tokens, msg_tx, repo_id)
    {
        repo_load::schedule_load_log(
            repo_load_executor,
            repos,
            msg_tx,
            repo_id,
            seq,
            scope,
            author,
            cursor,
            request,
            cancellation,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_selected_diff(
    executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>,
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    load_patch_diff: bool,
    load_file_text: bool,
    preview_text_side: Option<DiffPreviewTextSide>,
    load_submodule_summary: bool,
    load_file_image: bool,
) {
    if let Some((target, target_rev)) = selected_diff_target(thread_state, repo_id)
        && let Some((msg_tx, _)) =
            repo_load_context(thread_state, repo_task_tokens, msg_tx, repo_id)
    {
        let cancellation = repo_task_tokens
            .get_mut(&repo_id)
            .expect("repo task token")
            .selected_diff_cancellation(&target, target_rev);
        repo_load::schedule_load_selected_diff(
            executor,
            &repo_task_tokens[&repo_id].selected_diff_slots,
            repos,
            Arc::clone(thread_state),
            msg_tx,
            repo_id,
            (target, target_rev),
            cancellation,
            repo_load::SelectedDiffLoadOptions {
                load_text_attributes: true,
                load_patch_diff,
                load_file_text,
                preview_text_side,
                load_submodule_summary,
                load_file_image,
            },
        );
    }
}
