//! Per-repository load tokens: the cancellation scope of a repository's loads,
//! the contexts load effects are scheduled under, and the effects that manage them.

use super::worktree_list::WorktreeListPublisher;
use super::{repo_load, selected_diff_target};
use crate::model::{AppState, Loadable, LogLoadSeq, RepoId};
use crate::store::executor::{LatestTaskSlot, TaskExecutor};
use crate::store::repository_runtime::{RepoTaskScope, RepositoryRuntimes};
use crate::store::worker_channel::StoreWorkerSender;
use gitcomet_core::domain::{DiffPreviewTextSide, LogCursor, LogScope};
use gitcomet_core::services::CancellationToken;
use std::sync::{Arc, RwLock};

fn ensure_repo_task_scope(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    runtimes: &mut RepositoryRuntimes,
    repo_id: RepoId,
) -> Option<RepoTaskScope> {
    let (lifetime, load_epoch) = {
        let state = thread_state
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
        (repo.lifetime_id(), repo.load_epoch)
    };
    Some(runtimes.start_load_scope(repo_id, lifetime, load_epoch))
}

pub(super) fn repo_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_scope(thread_state, repos, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation.clone());
    Some((msg_tx, token.cancellation))
}

pub(super) fn commit_details_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, LatestTaskSlot)> {
    let token = ensure_repo_task_scope(thread_state, repos, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation);
    Some((msg_tx, token.commit_details_slot))
}

pub(super) fn worktree_list_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken, WorktreeListPublisher)> {
    let token = ensure_repo_task_scope(thread_state, repos, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation.clone());
    Some((msg_tx, token.cancellation, token.worktree_list_publisher))
}

/// Like [`repo_load_context`], but for the log walk: the returned token covers
/// this walk alone, and taking it cancels whichever walk it replaces. Messages
/// still ride the repository-wide guard, so a cancelled walk's reply arrives
/// and is dropped by the reducer rather than vanishing silently.
fn log_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_scope(thread_state, repos, repo_id)?;
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, token.cancellation.clone());
    Some((msg_tx, token.take_over_log()))
}

/// Like [`log_load_context`], but a new listing supersedes only the previous
/// listing, and its replies are guarded by that listing's own token.
pub(super) fn file_browser_load_context(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) -> Option<(StoreWorkerSender, CancellationToken)> {
    let token = ensure_repo_task_scope(thread_state, repos, repo_id)?;
    let cancellation = token.take_over_file_browser();
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, token.load_epoch, cancellation.clone());
    Some((msg_tx, cancellation))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_log(
    repo_load_executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
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
    if let Some((msg_tx, cancellation)) = log_load_context(thread_state, repos, msg_tx, repo_id) {
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
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    load_patch_diff: bool,
    load_file_text: bool,
    preview_text_side: Option<DiffPreviewTextSide>,
    load_submodule_summary: bool,
    load_file_image: bool,
) {
    if let Some((target, target_rev)) = selected_diff_target(thread_state, repo_id)
        && let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id)
    {
        let scope = repos.load_scope_mut(&repo_id).expect("repo task scope");
        let cancellation = scope.selected_diff_cancellation(&target, target_rev);
        let slots = scope.selected_diff_slots.clone();
        repo_load::schedule_load_selected_diff(
            executor,
            &slots,
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
