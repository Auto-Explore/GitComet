mod clone;
mod commit_details;
mod conflict_load;
mod control;
mod diff_session;
mod file_history;
mod history_authors;
mod history_find;
mod indexed_history;
mod load_tokens;
mod open_repo;
mod persistence;
mod references;
mod repo_actions;
mod repo_commands;
mod repo_load;
mod status;
mod unavailable;
mod util;
pub(super) mod worktree_list;

/// Releases one runtime's ownership without disturbing other stores or tabs.
pub(super) use repo_load::release_worktree_scan_handles;

use crate::model::AppState;
use crate::msg::{Effect, Msg};
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, GitBackend};
use std::sync::{Arc, RwLock};

use super::RepoId;
use super::executor::TaskExecutor;
use super::repository_runtime::{RepositoryRuntimes, WorktreeCacheLease};
use super::worker_channel::StoreWorkerSender;

/// Ceiling on how often a running filesystem operation may publish progress.
/// Fast enough to look continuous, slow enough that the per-message AppState
/// copy stays off the critical path.
const FILESYSTEM_PROGRESS_TICK: std::time::Duration = std::time::Duration::from_millis(50);

use load_tokens::repo_load_context;

#[derive(Clone, Copy)]
pub(super) struct EffectExecutors<'a> {
    pub(super) executor: &'a TaskExecutor,
    pub(super) repo_load_executor: &'a TaskExecutor,
    /// Status scans of other checkouts must not occupy foreground load workers.
    pub(super) worktree_scan_executor: &'a std::sync::LazyLock<TaskExecutor>,
    pub(super) session_persist_executor: &'a TaskExecutor,
    pub(super) metadata_executor: &'a TaskExecutor,
    pub(super) signature_executor: &'a TaskExecutor,
    /// Whole-history find scans, one worker per store: one window's scan
    /// must not hold up another's.
    pub(super) history_find_executor: &'a std::sync::LazyLock<TaskExecutor>,
    /// Disk writes and explorer mutations, in submission order.
    pub(super) filesystem_executor: &'a std::sync::LazyLock<TaskExecutor>,
}

fn selected_diff_target(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
) -> Option<(DiffTarget, u64)> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            repo.diff_state
                .diff_target
                .clone()
                .map(|target| (target, repo.diff_state.diff_target_rev))
        })
}

fn repo_worktree_cache(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    runtimes: &mut RepositoryRuntimes,
    repo_id: RepoId,
) -> Option<WorktreeCacheLease> {
    let lifetime = thread_state
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| repo.lifetime_id())?;
    Some(runtimes.worktree_cache(repo_id, lifetime))
}

fn selected_conflict_file_path(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
    request: crate::model::ConflictLoadRequestId,
) -> Option<std::path::PathBuf> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id && repo.conflict_state.load_request() == request)
        .and_then(|repo| repo.conflict_state.conflict_file_path.clone())
}

fn selected_inline_submodule_diff(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
) -> Option<(std::path::PathBuf, DiffTarget, u64)> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .map(|inline| {
            (
                inline.submodule_repo_path.clone(),
                inline.target.clone(),
                inline.rev,
            )
        })
}

pub(super) fn schedule_effect(
    executors: EffectExecutors<'_>,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    backend: &Arc<dyn GitBackend>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    effect: Effect,
) {
    #[cfg(any(test, feature = "test-support"))]
    let _test_scope = super::test_support::TestTaskScope::enter(Arc::clone(&msg_tx.test_tasks));
    let EffectExecutors {
        executor,
        repo_load_executor,
        worktree_scan_executor,
        session_persist_executor,
        metadata_executor,
        signature_executor,
        history_find_executor,
        filesystem_executor,
    } = executors;

    if effect.requires_available_git() {
        let runtime = {
            let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
            state.git_runtime.clone()
        };
        if !runtime.is_available() {
            unavailable::send_unavailable_git_effect_result(
                thread_state,
                &msg_tx,
                effect,
                &runtime,
            );
            return;
        }
    }

    match effect {
        Effect::Persistence(work) => {
            persistence::schedule(session_persist_executor, thread_state, msg_tx, work)
        }
        Effect::Control(work) => control::schedule(repos, msg_tx, work),
        Effect::Status(work) => {
            status::schedule(repo_load_executor, thread_state, repos, msg_tx, work)
        }
        Effect::References(work) => references::schedule(
            repo_load_executor,
            metadata_executor,
            thread_state,
            repos,
            msg_tx,
            work,
        ),
        Effect::RepoCommand(work) => repo_commands::schedule(
            executor,
            filesystem_executor,
            thread_state,
            backend,
            repos,
            msg_tx,
            work,
        ),
        Effect::ValidateAuthPrompt {
            generation,
            challenge,
            auth,
            cancellation,
        } => {
            executor.spawn(move || {
                let result = gitcomet_core::auth::askpass::validate_passphrase(
                    &challenge,
                    &auth,
                    &cancellation,
                );
                util::send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::AuthPromptValidated {
                        generation,
                        result,
                        auth,
                    }),
                );
            });
        }
        Effect::Filesystem(request) => {
            filesystem_executor.spawn(move || {
                // The engine ticks once per item and every dispatch deep-copies
                // AppState, so an unthrottled thousand-file drop spends longer
                // cloning state than moving files -- and starves the progress
                // bar the ticks exist to drive. Only the newest tick is ever
                // read, so dropping the ones in between loses nothing; the last
                // one is held back and flushed so the bar ends where it should.
                let mut last_sent: Option<std::time::Instant> = None;
                let mut withheld: Option<gitcomet_core::filesystem::Progress> = None;
                let result = msg_tx
                    .filesystem
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .execute(request, |progress| {
                        let now = std::time::Instant::now();
                        if last_sent
                            .is_none_or(|sent| now.duration_since(sent) >= FILESYSTEM_PROGRESS_TICK)
                        {
                            last_sent = Some(now);
                            withheld = None;
                            util::send_or_log(&msg_tx, Msg::FilesystemProgress(progress));
                        } else {
                            withheld = Some(progress);
                        }
                    });
                if let Some(progress) = withheld {
                    util::send_or_log(&msg_tx, Msg::FilesystemProgress(progress));
                }
                util::send_or_log(&msg_tx, Msg::FilesystemFinished(result));
            });
        }
        Effect::OpenRepo { repo_id, path } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                open_repo::schedule_open_repo(
                    repo_load_executor,
                    Arc::clone(backend),
                    msg_tx,
                    repo_id,
                    path,
                    cancellation,
                );
            }
        }
        Effect::HistoryAuthors(work) => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, work.repo_id)
            {
                history_authors::schedule(repos, msg_tx, work, cancellation);
            }
        }
        Effect::HistoryFind(work) => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, work.repo_id)
            {
                history_find::schedule(history_find_executor, repos, msg_tx, work, cancellation);
            }
        }
        Effect::IndexedHistory(work) => {
            let repo_id = work.repo_id();
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                indexed_history::schedule(repo_load_executor, repos, msg_tx, work, cancellation);
            }
        }
        Effect::DiffSession(work) => {
            if let Some(cache_lease) = repo_worktree_cache(thread_state, repos, work.repo_id)
                && let Some((msg_tx, cancellation)) =
                    repo_load_context(thread_state, repos, msg_tx, work.repo_id)
            {
                diff_session::schedule(
                    repo_load_executor,
                    repos,
                    Arc::clone(backend),
                    msg_tx,
                    cache_lease,
                    work,
                    cancellation,
                );
            }
        }
        Effect::LoadLog {
            repo_id,
            seq,
            scope,
            author,
            limit,
            cursor,
        } => load_tokens::load_log(
            repo_load_executor,
            thread_state,
            repos,
            msg_tx,
            repo_id,
            seq,
            scope,
            author,
            limit,
            cursor,
        ),
        Effect::LoadStashes { repo_id, limit } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_stashes(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    limit,
                    cancellation,
                );
            }
        }
        Effect::LoadConflictFile {
            request,
            repo_id,
            path,
            mode,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                let encoding = repo_load::file_encoding_override(thread_state, repo_id, &path);
                conflict_load::schedule_load_conflict_file(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    conflict_load::ConflictFileRequest {
                        request,
                        path,
                        mode,
                        encoding,
                    },
                );
            }
        }
        Effect::LoadReflog { repo_id, limit } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_reflog(executor, repos, msg_tx, repo_id, limit);
            }
        }
        Effect::LoadLfsLocks { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_lfs_locks(executor, repos, msg_tx, repo_id, cancellation);
            }
        }
        Effect::LoadAnnexWhereis { repo_id, key } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_annex_whereis(
                    metadata_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    key,
                    cancellation,
                );
            }
        }
        Effect::LoadAnnexUnused { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_annex_unused(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadFileHistory {
            request,
            repo_id,
            path,
            limit,
            cursor,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                file_history::schedule_load_file_history(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    file_history::FileHistoryPageRequest {
                        request,
                        path,
                        limit,
                        cursor,
                    },
                );
            }
        }
        Effect::LoadBlame {
            repo_id,
            path,
            source,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_blame(executor, repos, msg_tx, repo_id, path, source);
            }
        }
        Effect::LoadWorktrees { repo_id } => {
            if let Some((msg_tx, cancellation, publisher)) =
                load_tokens::worktree_list_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_worktrees(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                    publisher,
                );
            }
        }
        Effect::LoadWorktreeDirty {
            repo_id,
            workdir,
            scope,
            files_for,
        } => {
            if let Some(cache_lease) = repo_worktree_cache(thread_state, repos, repo_id)
                && let Some((msg_tx, cancellation, publisher)) =
                    load_tokens::worktree_list_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_worktree_dirty(
                    worktree_scan_executor,
                    backend.clone(),
                    repos,
                    msg_tx,
                    repo_id,
                    cache_lease,
                    workdir,
                    scope,
                    files_for,
                    cancellation,
                    publisher,
                );
            }
        }
        Effect::LoadSubmodules { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_submodules(
                    metadata_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadLargeFileSupport { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_large_file_support(
                    metadata_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadFileBrowser { repo_id, source } => {
            if let Some((msg_tx, cancellation)) =
                load_tokens::file_browser_load_context(thread_state, repos, msg_tx, repo_id)
            {
                let options = {
                    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                    state
                        .repos
                        .iter()
                        .find(|r| r.id == repo_id)
                        .map(|r| repo_load::explorer_listing::Options::from(&r.file_browser))
                        .unwrap_or_default()
                };
                repo_load::schedule_load_file_browser(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    source,
                    cancellation,
                    options,
                );
            }
        }
        Effect::LoadRebaseAndMergeState { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_rebase_and_merge_state(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadRebaseState { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_rebase_state(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadMergeCommitMessage { repo_id } => {
            if let Some((msg_tx, cancellation)) =
                repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                repo_load::schedule_load_merge_commit_message(
                    repo_load_executor,
                    repos,
                    msg_tx,
                    repo_id,
                    cancellation,
                );
            }
        }
        Effect::LoadRecentCommitMessages {
            repo_id,
            limit,
            request_rev,
        } => {
            repo_load::schedule_load_recent_commit_messages(
                executor,
                repos,
                msg_tx,
                repo_id,
                limit,
                request_rev,
            );
        }
        Effect::LoadCommitDetails { repo_id, commit_id } => {
            if let Some((msg_tx, slot)) =
                load_tokens::commit_details_load_context(thread_state, repos, msg_tx, repo_id)
            {
                commit_details::schedule(
                    executor,
                    &slot,
                    repos,
                    Arc::clone(thread_state),
                    msg_tx,
                    repo_id,
                    commit_id,
                );
            }
        }
        Effect::VerifyCommitSignatures {
            repo_id,
            epoch,
            batch,
            cancellation,
            commit_ids,
            formats,
        } => {
            // Signature requests have their own lifetime: staging and tab switches
            // cancel repo loads, but must not silently lose pending verification.
            repo_load::schedule_verify_commit_signatures(
                signature_executor,
                repos,
                msg_tx,
                repo_id,
                epoch,
                batch,
                cancellation,
                commit_ids,
                formats,
            );
        }
        Effect::LoadHoverCommitMessage { repo_id, commit_id } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_hover_commit_message(
                    executor, repos, msg_tx, repo_id, commit_id,
                );
            }
        }
        Effect::ResolveCommitForReveal { repo_id, reference } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_resolve_commit_for_reveal(
                    executor, repos, msg_tx, repo_id, reference,
                );
            }
        }
        Effect::ResolveCommitLookup {
            repo_id,
            reference,
            purpose,
            request,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_resolve_commit_lookup(
                    executor, repos, msg_tx, repo_id, reference, purpose, request,
                );
            }
        }
        Effect::LoadRangeFiles {
            repo_id,
            from,
            to,
            options,
            request,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_range_files(
                    executor, repos, msg_tx, repo_id, from, to, options, request,
                );
            }
        }
        Effect::LoadSquashMessagePreview {
            repo_id,
            oldest,
            head,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_squash_message_preview(
                    executor, repos, msg_tx, repo_id, oldest, head,
                );
            }
        }
        Effect::LoadSquashRebaseSetup {
            repo_id,
            base,
            actual_head,
            selected_ids,
            reword_id,
            message,
            count,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_squash_rebase_setup(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    repo_load::SquashRebaseSetupRequest {
                        base,
                        actual_head,
                        selected_ids,
                        reword_id,
                        message,
                        count,
                    },
                );
            }
        }
        Effect::OpenFileAtCommitParent {
            repo_id,
            commit_id,
            path,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_open_file_at_commit_parent(
                    executor, repos, msg_tx, repo_id, commit_id, path,
                );
            }
        }
        Effect::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
            content_preview,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_open_file_at_commit(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    commit_id,
                    path,
                    content_preview,
                );
            }
        }
        Effect::LoadDiff { repo_id, target } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                let encoding = target.file_path().and_then(|path| {
                    repo_load::file_encoding_override(thread_state, repo_id, path)
                });
                repo_load::schedule_load_diff(executor, repos, msg_tx, repo_id, target, encoding);
            }
        }
        Effect::LoadDiffFile { repo_id, target } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                let encoding = target.file_path().and_then(|path| {
                    repo_load::file_encoding_override(thread_state, repo_id, path)
                });
                repo_load::schedule_load_diff_file(
                    executor, repos, msg_tx, repo_id, target, encoding,
                );
            }
        }
        Effect::LoadDiffPreviewTextFile {
            repo_id,
            target,
            side,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_diff_preview_text_file(
                    executor, repos, msg_tx, repo_id, target, side,
                );
            }
        }
        Effect::LoadSubmoduleSummary { repo_id, target } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_submodule_summary(
                    executor, repos, msg_tx, repo_id, target,
                );
            }
        }
        Effect::LoadInlineSubmoduleSelectedDiff {
            repo_id,
            inline_rev,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_inline_submodule_selected_diff(
                    executor,
                    backend.clone(),
                    msg_tx,
                    repo_id,
                    inline_rev,
                    selected_inline_submodule_diff(thread_state, repo_id),
                );
            }
        }
        Effect::LoadInlineSubmoduleSelectedDiffFile {
            repo_id,
            inline_rev,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_inline_submodule_selected_diff_file(
                    executor,
                    backend.clone(),
                    msg_tx,
                    repo_id,
                    inline_rev,
                    selected_inline_submodule_diff(thread_state, repo_id),
                );
            }
        }
        Effect::LoadInlineSubmoduleSelectedDiffFileImage {
            repo_id,
            inline_rev,
        } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_inline_submodule_selected_diff_file_image(
                    executor,
                    backend.clone(),
                    msg_tx,
                    repo_id,
                    inline_rev,
                    selected_inline_submodule_diff(thread_state, repo_id),
                );
            }
        }
        Effect::LoadDiffFileImage { repo_id, target } => {
            if let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id) {
                repo_load::schedule_load_diff_file_image(executor, repos, msg_tx, repo_id, target);
            }
        }
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff,
            load_file_text,
            preview_text_side,
            load_submodule_summary,
            load_file_image,
        } => load_tokens::load_selected_diff(
            executor,
            thread_state,
            repos,
            msg_tx,
            repo_id,
            load_patch_diff,
            load_file_text,
            preview_text_side,
            load_submodule_summary,
            load_file_image,
        ),
        Effect::LoadSelectedConflictFile {
            request,
            repo_id,
            mode,
        } => {
            if let Some(path) = selected_conflict_file_path(thread_state, repo_id, request)
                && let Some((msg_tx, _)) = repo_load_context(thread_state, repos, msg_tx, repo_id)
            {
                let encoding = repo_load::file_encoding_override(thread_state, repo_id, &path);
                conflict_load::schedule_load_conflict_file(
                    executor,
                    repos,
                    msg_tx,
                    repo_id,
                    conflict_load::ConflictFileRequest {
                        request,
                        path,
                        mode,
                        encoding,
                    },
                );
            }
        }
        Effect::CheckoutBranch { repo_id, name } => {
            repo_actions::schedule_checkout_branch(
                executor,
                repos,
                backend.clone(),
                msg_tx,
                repo_id,
                name,
            );
        }
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
            mode,
        } => repo_actions::schedule_checkout_remote_branch(
            executor,
            repos,
            backend.clone(),
            msg_tx,
            repo_id,
            remote,
            branch,
            local_branch,
            mode,
        ),
        Effect::CheckoutCommit { repo_id, commit_id } => {
            repo_actions::schedule_checkout_commit(executor, repos, msg_tx, repo_id, commit_id);
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
            force,
        } => {
            repo_actions::schedule_create_branch_and_checkout(
                executor,
                repos,
                backend.clone(),
                msg_tx,
                repo_id,
                name,
                target,
                force,
            );
        }
        Effect::RenameBranch {
            repo_id,
            old_name,
            new_name,
            force,
        } => {
            repo_actions::schedule_rename_branch(
                executor,
                repos,
                backend.clone(),
                msg_tx,
                repo_id,
                old_name,
                new_name,
                force,
            );
        }
        Effect::DeleteBranch { repo_id, name } => {
            repo_actions::schedule_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::ForceDeleteBranch { repo_id, name } => {
            repo_actions::schedule_force_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::DeleteBranches {
            repo_id,
            names,
            force,
        } => {
            repo_actions::schedule_delete_branches(executor, repos, msg_tx, repo_id, names, force);
        }
        Effect::CloneRepo {
            url,
            dest,
            remote_url_policy,
            auth,
        } => clone::schedule_clone_repo(executor, msg_tx, url, dest, remote_url_policy, auth),
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
            identity,
            auth,
        } => {
            repo_actions::schedule_commit(
                executor, repos, msg_tx, repo_id, message, identity, auth,
            );
        }
        Effect::CommitAmend {
            repo_id,
            message,
            identity,
            auth,
        } => {
            repo_actions::schedule_commit_amend(
                executor, repos, msg_tx, repo_id, message, identity, auth,
            );
        }
        Effect::PreviewTagPush {
            repo_id,
            request,
            generation,
            cancellation,
        } => {
            util::spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, tx| {
                if cancellation.is_cancelled() {
                    return;
                }
                let result = repo.preview_tag_push(&request, &cancellation);
                if !cancellation.is_cancelled() {
                    util::send_or_log(
                        &tx,
                        Msg::Internal(crate::msg::InternalMsg::TagPushPreviewLoaded {
                            repo_id,
                            mode: request.mode,
                            generation,
                            result,
                        }),
                    );
                }
            });
        }
        Effect::LoadInteractiveRebaseSetup { repo_id, base } => {
            repo_load::schedule_load_interactive_rebase_setup(
                executor, repos, msg_tx, repo_id, base,
            );
        }
        Effect::LoadInteractiveCherryPickMessages { repo_id, ids } => {
            repo_load::schedule_load_interactive_cherry_pick_messages(
                executor, repos, msg_tx, repo_id, ids,
            );
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
mod tests;
