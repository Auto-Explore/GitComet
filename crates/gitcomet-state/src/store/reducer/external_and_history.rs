use super::actions_emit_effects::invalidate_loaded_blame;
use super::effects::{append_ensure_sidebar_data_effects, select_commit_and_load_details};
use super::repo_management::{
    append_cancel_repo_loads_effect_for_repo, append_selected_history_reload_effects,
    selected_history_reloads_for_activation,
};
use super::util::{
    SelectedConflictTarget, append_auto_background_metadata_effects,
    append_requested_status_refresh_effects, clear_banner_error_for_repo, diff_reload_effects,
    push_diagnostic, refresh_full_effects, refresh_primary_effects, selected_conflict_target,
    start_conflict_target_reload, start_current_conflict_target_reload,
};
use crate::model::{
    AppState, DiagnosticKind, InteractiveRebaseSetup, Loadable, RepoLoadsInFlight, SidebarMode,
};
use crate::msg::{Effect, RepoActionKind, RepoExternalChange};
use gitcomet_core::domain::{DiffArea, DiffTarget, LogCursor, LogPage, LogScope};
use gitcomet_core::error::Error;
use gitcomet_core::services::{InteractiveRebaseEntry, SequencerState};
use std::sync::Arc;

const LARGE_HISTORY_APPEND_LEN_THRESHOLD: usize = 4_096;
const SMALL_APPEND_GROWTH_RATIO: usize = 8;
const INITIAL_PAGINATED_LOG_APPEND_SLACK_CAP: usize = 512;

fn should_reserve_log_append_exact(existing_len: usize, additional: usize) -> bool {
    existing_len >= LARGE_HISTORY_APPEND_LEN_THRESHOLD
        && additional.saturating_mul(SMALL_APPEND_GROWTH_RATIO) <= existing_len
}

fn reserve_log_append_capacity<T>(existing: &mut Vec<T>, additional: usize) {
    if additional == 0 {
        return;
    }

    let spare = existing.capacity().saturating_sub(existing.len());
    if spare >= additional {
        return;
    }

    let missing = additional - spare;
    if should_reserve_log_append_exact(existing.len(), additional) {
        existing.reserve_exact(missing);
    } else {
        existing.reserve(missing);
    }
}

fn reserve_initial_paginated_log_append_slack<T>(commits: &mut Vec<T>) {
    if commits.is_empty() {
        return;
    }

    let desired_spare = commits.len().min(INITIAL_PAGINATED_LOG_APPEND_SLACK_CAP);
    let spare = commits.capacity().saturating_sub(commits.len());
    if spare >= desired_spare {
        return;
    }

    commits.reserve_exact(desired_spare - spare);
}

pub(super) fn reload_repo(state: &mut AppState, repo_id: crate::model::RepoId) -> Vec<Effect> {
    let git_log_settings = state.git_log_settings;
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_head_branch(Loadable::Loading);
    repo_state.set_detached_head_commit(None);
    repo_state.set_branches(Loadable::Loading);
    repo_state.set_tags(Loadable::NotLoaded);
    repo_state.set_remote_tags(Loadable::NotLoaded);
    repo_state.set_remotes(Loadable::Loading);
    repo_state.set_remote_branches(Loadable::Loading);
    repo_state.set_status(Loadable::Loading);
    repo_state.set_log(Loadable::Loading);
    repo_state.set_log_loading_more(false);
    repo_state.set_stashes(Loadable::NotLoaded);
    repo_state.reflog = Loadable::NotLoaded;
    repo_state.set_rebase_in_progress(Loadable::Loading);
    repo_state.set_sequencer_state(Loadable::Loading);
    repo_state.set_merge_commit_message(Loadable::Loading);
    repo_state.history_state.file_history_path = None;
    repo_state.history_state.file_history = Loadable::NotLoaded;
    repo_state.history_state.blame_path = None;
    repo_state.history_state.blame_source = None;
    repo_state.history_state.blame = Loadable::NotLoaded;
    repo_state.clear_retained_blame();
    repo_state.set_worktrees(Loadable::NotLoaded);
    repo_state.set_submodules(Loadable::NotLoaded);
    repo_state.clear_head_dependent_cached_state();
    repo_state.set_selected_commit(None);
    repo_state.set_commit_details(Loadable::NotLoaded);
    // A reload can follow a reset or a dropped branch, which may have taken the
    // marked commit with it. `set_selected_commit(None)` already dissolved the
    // active comparison; the mark is the same kind of stale reference, and
    // leaving it would keep offering a "Compare with …" that can only fail.
    repo_state.comparison_mark = None;
    // A full reload may rewrite history (rebase/amend/branch switch underneath),
    // so back/forward snapshots can reference commits or file revisions that no
    // longer resolve. Start the navigation stacks fresh.
    repo_state.nav_history.clear();
    repo_state.view_history.clear();

    let mut effects = refresh_full_effects(repo_state, git_log_settings);
    append_auto_background_metadata_effects(repo_state, git_log_settings, &mut effects);
    // Linked-worktree rows survive a reload, so their dirty counts have to be
    // refreshed along with everything else. The monitor only flushes for this
    // repo's own `.git`, so a commit or stash made inside a linked worktree
    // reaches us no other way, and Reload is exactly how a user asks for it.
    if let Some(effect) = super::effects::request_worktree_dirty_effect(repo_state) {
        effects.push(effect);
    }
    effects
}

/// Keep the live Files tree in step with the disk. Commit browsing is immutable
/// and left alone. A refresh costs a full walk, so it runs eagerly only while the
/// user is looking at the tree, and is deferred as `stale` otherwise.
fn file_browser_refresh_for_external_change(
    repo_state: &mut crate::model::RepoState,
    change: RepoExternalChange,
    sidebar_shows_this_files_tree: bool,
) -> Option<Effect> {
    if !(change.worktree || change.index || change.git_state) {
        return None;
    }
    if repo_state.file_browser.source != gitcomet_core::domain::FileSource::WorkingDirectory {
        return None;
    }
    if !sidebar_shows_this_files_tree {
        repo_state.file_browser.stale = true;
        return None;
    }
    // Deliberately no `entries = Loading` and no `expanded_dirs.clear()`: the rows
    // stay put, expansion included, until the new listing replaces them.
    super::effects::request_file_browser_load(repo_state)
}

pub(super) fn repo_externally_changed(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    change: RepoExternalChange,
) -> Vec<Effect> {
    let sidebar_shows_this_files_tree =
        state.sidebar_mode == SidebarMode::Files && state.active_repo == Some(repo_id);
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let file_browser_effect =
        file_browser_refresh_for_external_change(repo_state, change, sidebar_shows_this_files_tree);

    // Coalesce refreshes while a refresh is already in flight.
    let mut effects = if change.git_state {
        // A git-state watcher event can be produced by the safety fetch that
        // prepared a pending force-push lease. Preserve that offer; the force
        // push command validates the branch and HEAD again before pushing.
        repo_state.set_recent_commit_messages(Loadable::NotLoaded);
        let mut effects = refresh_primary_effects(repo_state);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::BRANCHES)
        {
            effects.push(Effect::LoadBranches { repo_id });
        }
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::REMOTE_BRANCHES)
        {
            effects.push(Effect::LoadRemoteBranches { repo_id });
        }
        if let Some(effect) = super::effects::request_worktree_dirty_effect(repo_state) {
            effects.push(effect);
        }
        effects
    } else {
        let mut effects = Vec::new();
        if change.index {
            // The index is one side of BOTH the staged (HEAD↔index) and unstaged (index↔worktree)
            // diffs, so an index change must refresh both lanes — even when no worktree file
            // changed. An external `git add` / `git reset` / `git restore --staged` moves a file
            // between the staged and unstaged sections; refreshing only the staged lane would
            // leave the file lingering (stale) in the unstaged section (or vice-versa).
            append_requested_status_refresh_effects(repo_state, &mut effects);
        } else if change.worktree
            && repo_state
                .loads_in_flight
                .request(RepoLoadsInFlight::WORKTREE_STATUS)
        {
            effects.push(Effect::LoadWorktreeStatus { repo_id });
        }
        effects
    };

    effects.extend(file_browser_effect);

    // Tag reloads are driven by the `tags` flag alone, independent of
    // `git_state`, so any change that sets `tags` refreshes them regardless of
    // which other lanes the event touched.
    if change.tags {
        repo_state.set_tags(Loadable::NotLoaded);
        if repo_state.loads_in_flight.request(RepoLoadsInFlight::TAGS) {
            effects.push(Effect::LoadTags { repo_id });
        }
    }

    let should_reload_diff = repo_state
        .diff_state
        .diff_target
        .as_ref()
        .is_some_and(|target| match target {
            DiffTarget::WorkingTree { area, .. } => {
                change.git_state || change.index || (*area == DiffArea::Unstaged && change.worktree)
            }
            DiffTarget::Commit { .. } => false,
            // A commit↔commit range is immutable; a commit↔working-tree range
            // (to == None) tracks the worktree, so reload it on any change that
            // moves the index or worktree.
            DiffTarget::CommitRange { to_commit_id, .. } => {
                to_commit_id.is_none() && (change.git_state || change.index || change.worktree)
            }
        });

    if should_reload_diff
        && let Some(target) = repo_state.diff_state.diff_target.clone()
        && matches!(
            target,
            DiffTarget::WorkingTree { .. }
                | DiffTarget::CommitRange {
                    to_commit_id: None,
                    ..
                }
        )
    {
        // A moved HEAD (external commit / checkout / rebase) can leave the patch
        // byte-identical while every line's attribution changes ("Not Committed
        // Yet" → a real commit), and nothing downstream can detect that, so drop
        // blame up front for git-state events. A pure worktree/index event leaves
        // blame painted; `diff_loaded`/`diff_file_loaded` then invalidate it only
        // if the reloaded content actually differs, so a refresh that finds no
        // change does not re-run `git blame`. `blame_path`/`blame_source` are
        // preserved either way, so the view reloads the same target.
        if change.git_state {
            invalidate_loaded_blame(repo_state);
        }
        if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
            match conflict_target {
                SelectedConflictTarget::Current => {
                    effects.extend(start_current_conflict_target_reload(repo_state));
                }
                SelectedConflictTarget::Path(path) => {
                    effects.extend(start_conflict_target_reload(repo_state, path));
                }
            }
        } else {
            effects.extend(diff_reload_effects(repo_state, repo_id, target));
        }
    }

    // Refresh the changed-file list of an active commit↔working-tree comparison
    // (to == None) so files appear/disappear as the worktree changes. A
    // commit↔commit comparison is immutable and needs no refresh. `LoadRangeFiles`
    // results are dropped if the selection no longer matches (see
    // `range_files_loaded`), so a late reply after the user re-selects is safe.
    if (change.git_state || change.index || change.worktree)
        && let Some(from) = repo_state
            .history_state
            .range_selection
            .as_ref()
            .filter(|range| range.to.is_none())
            .map(|range| range.from.clone())
        // A refresh means two full-tree `git diff` calls, so a debounced save
        // storm must not stack them up. One in flight absorbs the rest and is
        // re-run once when it lands, the same coalescing the status and tag
        // reloads above get from `RepoLoadsInFlight`.
        && let Some(request) = repo_state.request_range_files_refresh()
    {
        // Keep the current list visible until the refresh lands (no flicker
        // to a loading state on every debounced save).
        effects.push(Effect::LoadRangeFiles {
            repo_id,
            from,
            to: None,
            request,
        });
    }

    effects
}

/// Reloads the first page after the user changed *what* the history shows —
/// its scope or its author filter. The caller has already applied the change to
/// `repo_state`; this holds the previous page on screen while the new walk runs,
/// drops any pagination in progress, and persists the change. `persist` is given
/// the repository's workdir.
fn restart_history_load(
    state: &mut AppState,
    repo_ix: usize,
    persist: impl FnOnce(std::path::PathBuf) -> Effect,
) -> Vec<Effect> {
    let repo_state = &mut state.repos[repo_ix];
    repo_state.retain_log_while_loading();
    repo_state.set_log(Loadable::Loading);
    repo_state.set_log_loading_more(false);
    // Any count on screen belongs to the walk being replaced.
    repo_state.set_log_scan_progress(None);

    let mut effects = vec![persist(repo_state.spec.workdir.clone())];
    let request = super::util::first_page_log_request(repo_state);
    effects.extend(super::util::request_log_effect(repo_state, request));
    effects
}

pub(super) fn set_history_scope(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    scope: LogScope,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if state.repos[repo_ix].history_state.history_scope == scope {
        return Vec::new();
    }
    state.repos[repo_ix].set_log_scope(scope);

    restart_history_load(state, repo_ix, |workdir| Effect::PersistRepoHistoryMode {
        repo_id: Some(repo_id),
        workdir,
        mode: scope,
        action: "updating history mode",
    })
}

pub(super) fn set_history_author_filter(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    author: Option<String>,
) -> Vec<Effect> {
    let Some(repo_ix) = state.repos.iter().position(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if state.repos[repo_ix].history_state.history_author_filter == author {
        return Vec::new();
    }
    state.repos[repo_ix].set_history_author_filter(author.clone());

    restart_history_load(state, repo_ix, |workdir| {
        Effect::PersistRepoHistoryAuthorFilter {
            repo_id: Some(repo_id),
            workdir,
            author,
            action: "updating history author filter",
        }
    })
}

pub(super) fn load_more_history(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    if repo_state.history_state.log_loading_more {
        return Vec::new();
    }

    let Loadable::Ready(page) = &repo_state.log else {
        return Vec::new();
    };
    let Some(cursor) = page.next_cursor.clone() else {
        return Vec::new();
    };

    repo_state.set_log_loading_more(true);
    let request = crate::model::PendingLogLoad {
        cursor: Some(cursor),
        ..super::util::first_page_log_request(repo_state)
    };
    super::util::request_log_effect(repo_state, request)
        .into_iter()
        .collect()
}

pub(super) fn rebase_state_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    result: std::result::Result<SequencerState, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let (sequencer_value, rebase_value) = match result {
            Ok(v) => (
                Loadable::Ready(v),
                Loadable::Ready(v != SequencerState::None),
            ),
            Err(e) => {
                let error = e.to_string();
                push_diagnostic(repo_state, DiagnosticKind::Error, error.clone());
                (Loadable::Error(error.clone()), Loadable::Error(error))
            }
        };
        repo_state.set_sequencer_state(sequencer_value);
        repo_state.set_rebase_in_progress(rebase_value);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REBASE_STATE)
        {
            effects.push(Effect::LoadRebaseState { repo_id });
        }
    }
    effects
}

pub(super) fn interactive_rebase_setup_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    base: String,
    result: std::result::Result<Vec<InteractiveRebaseEntry>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        // Discard stale results: only write if setup is still active for this
        // exact base. Guards against a cancelled setup being revived, or a
        // result for commit X clobbering a newer load already in flight for Y.
        if repo_state
            .interactive_rebase_setup
            .as_ref()
            .is_some_and(|s| s.base == base)
        {
            let entries = match result {
                Ok(v) => Loadable::Ready(v),
                Err(e) => {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            };
            repo_state.interactive_rebase_setup = Some(InteractiveRebaseSetup { base, entries });
        }
    }
    vec![]
}

/// Makes an interactive cherry-pick setup editable only after every selected
/// commit's full `%B` message has loaded. The setup opens with subject-only
/// seeds, so exposing it earlier would let a reword silently drop the body.
/// A response for a selection that has since been replaced is ignored.
pub(super) fn interactive_cherry_pick_messages_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    requested_ids: Vec<String>,
    result: std::result::Result<Vec<(String, String)>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && let Some(setup) = repo_state.interactive_cherry_pick_setup.as_mut()
    {
        let current_ids: Vec<&str> = setup
            .entries
            .iter()
            .map(|entry| entry.commit_id.as_str())
            .collect();
        if current_ids != requested_ids.iter().map(String::as_str).collect::<Vec<_>>() {
            return vec![];
        }

        match result {
            Ok(messages) => {
                let returned_ids = messages
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                if messages.len() != setup.entries.len()
                    || returned_ids.len() != messages.len()
                    || !setup
                        .entries
                        .iter()
                        .all(|entry| returned_ids.contains(entry.commit_id.as_str()))
                {
                    setup.full_messages = Loadable::Error(
                        "Repository returned an invalid ordered cherry-pick selection".to_string(),
                    );
                    return vec![];
                }
                let mut entries_by_id = setup
                    .entries
                    .drain(..)
                    .map(|entry| (entry.commit_id.clone(), entry))
                    .collect::<std::collections::HashMap<_, _>>();
                let mut ordered_entries = Vec::with_capacity(messages.len());
                for (id, message) in messages {
                    let Some(mut entry) = entries_by_id.remove(&id) else {
                        setup.full_messages = Loadable::Error(format!(
                            "Commit ordering returned an unexpected commit {id}"
                        ));
                        return vec![];
                    };
                    entry.message = message;
                    ordered_entries.push(entry);
                }
                if let Some((missing, _)) = entries_by_id.into_iter().next() {
                    setup.full_messages = Loadable::Error(format!(
                        "Failed to load and order selected commit {missing}"
                    ));
                    return vec![];
                }
                setup.entries = ordered_entries;
                setup.full_messages = Loadable::Ready(());
            }
            Err(error) => setup.full_messages = Loadable::Error(error.to_string()),
        }
    }
    vec![]
}

pub(super) fn merge_commit_message_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    result: std::result::Result<Option<String>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_merge_commit_message(value);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE)
        {
            effects.push(Effect::LoadMergeCommitMessage { repo_id });
        }
    }
    effects
}

/// Applies a partially built page while its walk is still running.
///
/// Chunks are prefixes of one another, so this replaces rather than appends and
/// applying one twice is harmless. Only a first page streams: a "load more"
/// merges into the existing page, and a prefix cannot say where that merge
/// starts.
pub(super) fn log_chunk_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    seq: crate::model::LogLoadSeq,
    commits: Vec<gitcomet_core::domain::Commit>,
    scanned: u64,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !repo_state.loads_in_flight.is_active_log_reply(seq)
        || repo_state.loads_in_flight.active_log_is_load_more()
    {
        return Vec::new();
    }

    repo_state.set_log_scan_progress(Some(scanned));
    if commits.is_empty() {
        // Nothing found yet: keep whatever the view is painting and just let the
        // progress readout advance.
        return Vec::new();
    }
    // Published as the page held *while loading*, not as the finished log: the
    // walk is still running, so there is no answer yet to "is there more?", and
    // a `Ready` page with no cursor would claim there is not.
    repo_state.set_partial_log_while_loading(Arc::new(LogPage {
        commits,
        next_cursor: None,
    }));
    Vec::new()
}

pub(super) fn log_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    seq: crate::model::LogLoadSeq,
    scope: LogScope,
    cursor: Option<LogCursor>,
    result: std::result::Result<LogPage, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let is_load_more = cursor.is_some();

        // Drop replies from a walk that a newer request superseded. That walk
        // was cancelled and its replacement is still running, so the in-flight
        // bookkeeping belongs to the newer request and must not be touched here.
        if !repo_state.loads_in_flight.is_active_log_reply(seq) {
            return effects;
        }

        repo_state.set_log_scan_progress(None);
        match result {
            // A cancelled walk is not a failure: the request that replaced it
            // owns the state now, so leave everything as the newer load found it.
            Err(e) if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) => {
                if is_load_more {
                    repo_state.set_log_loading_more(false);
                }
            }
            Ok(mut page) => {
                if is_load_more && let Loadable::Ready(existing) = &mut repo_state.log {
                    // Drop the history_state copy first so the Arc's refcount
                    // goes to 1 and make_mut can mutate in-place instead of
                    // deep-cloning the entire commit list.
                    repo_state.history_state.log = Loadable::NotLoaded;
                    let existing = Arc::make_mut(existing);
                    reserve_log_append_capacity(&mut existing.commits, page.commits.len());
                    existing.commits.append(&mut page.commits);
                    existing.next_cursor = page.next_cursor;
                    // Re-share the updated Arc with history_state.
                    repo_state.history_state.log = repo_state.log.clone();
                    repo_state.bump_log_revs();
                } else {
                    if page.next_cursor.is_some() {
                        reserve_initial_paginated_log_append_slack(&mut page.commits);
                    }
                    repo_state.set_log(Loadable::Ready(Arc::new(page)));
                }
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                if !is_load_more {
                    repo_state.set_log(Loadable::Error(e.to_string()));
                }
            }
        }

        if scope.guarantees_head_visibility()
            && matches!(repo_state.head_branch, Loadable::Ready(ref head) if head == "HEAD")
            && let Loadable::Ready(page) = &repo_state.log
        {
            repo_state.set_detached_head_commit(page.commits.first().map(|c| c.id.clone()));
        }

        // Reconcile the commit multi-selection against the reloaded page: drop
        // ids that no longer exist, and drop the anchor index hint since row
        // indices may have shifted.
        //
        // Only a *replaced* page can say an id no longer exists. A "load more"
        // appends, so an id missing from the grown page was equally missing
        // before, and dropping it there would fight every reveal that is still
        // paging toward its target — one clear per batch, which the details pane
        // shows as a flicker between the commit and the working tree.
        if !is_load_more
            && !repo_state.history_state.multi_selection.commits.is_empty()
            && let Loadable::Ready(page) = &repo_state.log
        {
            let reveal_target = repo_state.history_state.reveal_target.clone();
            let survives = |id: &gitcomet_core::domain::CommitId| {
                reveal_target.as_ref() == Some(id) || page.commits.iter().any(|c| c.id == *id)
            };

            let mut next = repo_state.history_state.multi_selection.clone();
            next.commits.retain(&survives);
            if let Some(anchor) = &next.anchor
                && !survives(anchor)
            {
                next.anchor = None;
            }
            next.anchor_index = None;
            next.anchor_log_rev = None;

            // The focused commit (which drives the details pane) may itself
            // have vanished — an external amend/rebase can replace exactly the
            // focused commit. Re-point focus at a surviving selected commit so
            // the details pane never trails a commit that no longer exists.
            //
            // A reveal's target is exempt: it is deliberately selected ahead of
            // the page that will contain it, and a scope switch mid-reveal
            // restarts the log from its first page.
            let focus_gone = repo_state
                .history_state
                .selected_commit
                .as_ref()
                .is_some_and(|id| !survives(id));
            let refocus = focus_gone.then(|| next.commits.last().cloned()).flatten();

            repo_state.set_commit_multi_selection(next);

            if focus_gone {
                match refocus {
                    Some(commit_id) => {
                        effects.extend(select_commit_and_load_details(
                            repo_state, repo_id, commit_id,
                        ));
                    }
                    None => {
                        repo_state.set_selected_commit(None);
                        repo_state.set_commit_details(Loadable::NotLoaded);
                    }
                }
            }
        }

        if is_load_more {
            repo_state.set_log_loading_more(false);
        }

        if let Some((seq, next)) = repo_state.loads_in_flight.finish_log() {
            repo_state.set_log_loading_more(next.cursor.is_some());
            effects.push(Effect::LoadLog {
                repo_id,
                seq,
                scope: next.scope,
                author: next.author,
                limit: next.limit,
                cursor: next.cursor,
            });
        }
    }
    effects
}

pub(super) fn repo_action_finished(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    action: RepoActionKind,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    let mut clear_banner = false;
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
        repo_state.bump_ops_rev();
        match result {
            Ok(()) => {
                repo_state.last_error = None;
                if repo_action_clears_head_dependent_state(action) {
                    repo_state.clear_head_dependent_cached_state();
                }
                clear_banner = true;
            }
            Err(e) => {
                repo_state.last_error = Some(e.to_string());
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
            }
        }
    }
    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }
    let is_active = state.active_repo == Some(repo_id);

    // A completed action mutated the repo, so every load issued before it is now stale. Bump the
    // load epoch (so those stale results are dropped by the epoch gate), clear all in-flight flags,
    // reset every `Loading` loadable back to `NotLoaded`, and cancel the orphaned worker tasks.
    // This mirrors the invalidation repo activation performs; unlike a partial flag clear it never
    // leaves a non-status load stranded in flight (its flag would otherwise never be cleared).
    let mut effects: Vec<Effect> = Vec::new();
    append_cancel_repo_loads_effect_for_repo(state, Some(repo_id), &mut effects);

    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return effects;
    };

    // Re-issue the primary panes (head branch, ahead/behind, rebase/merge, status, log). The flags
    // were just cleared, so request_* dispatches fresh loads under the new epoch.
    effects.extend(refresh_primary_effects(repo_state));

    // Selected views (branch lists, history, diff) only matter for the repo the user is viewing. A
    // non-active repo's in-flight views were reset to `NotLoaded` and reload when it is next
    // activated; the primary panes above are still refreshed so they are current on return.
    if is_active {
        // Re-issue branch lists when one was in flight before (refresh_primary_effects does not
        // cover them); request() returns true now that the flag was cleared.
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::BRANCHES)
        {
            effects.push(Effect::LoadBranches { repo_id });
        }
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::REMOTE_BRANCHES)
        {
            effects.push(Effect::LoadRemoteBranches { repo_id });
        }

        // Re-issue sidebar data loads (worktrees, submodules, stashes) that were
        // cancelled above; request() returns true now that the flags were cleared.
        append_ensure_sidebar_data_effects(repo_state, &mut effects);

        let history_reloads = selected_history_reloads_for_activation(repo_state);
        append_selected_history_reload_effects(repo_id, repo_state, history_reloads, &mut effects);

        if let Some(target) = repo_state.diff_state.diff_target.clone() {
            if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
                match conflict_target {
                    SelectedConflictTarget::Current => {
                        effects.extend(start_current_conflict_target_reload(repo_state));
                    }
                    SelectedConflictTarget::Path(path) => {
                        effects.extend(start_conflict_target_reload(repo_state, path));
                    }
                }
            } else {
                effects.extend(diff_reload_effects(repo_state, repo_id, target));
            }
        }
    }

    effects
}

fn repo_action_clears_head_dependent_state(action: RepoActionKind) -> bool {
    matches!(
        action,
        RepoActionKind::CheckoutBranch
            | RepoActionKind::CheckoutRemoteBranch
            | RepoActionKind::CheckoutCommit
            | RepoActionKind::CherryPickCommit
            | RepoActionKind::RevertCommit
            | RepoActionKind::CreateBranchAndCheckout
    )
}

#[cfg(test)]
mod tests {
    use super::{
        reserve_initial_paginated_log_append_slack, reserve_log_append_capacity,
        should_reserve_log_append_exact,
    };

    #[test]
    fn large_history_small_page_uses_exact_append_growth() {
        assert!(should_reserve_log_append_exact(5_000, 500));
        assert!(should_reserve_log_append_exact(8_192, 200));
    }

    #[test]
    fn smaller_histories_keep_amortized_append_growth() {
        assert!(!should_reserve_log_append_exact(1_000, 200));
        assert!(!should_reserve_log_append_exact(4_095, 256));
    }

    #[test]
    fn larger_pages_keep_amortized_append_growth() {
        assert!(!should_reserve_log_append_exact(5_000, 700));
        assert!(!should_reserve_log_append_exact(16_000, 2_001));
    }

    #[test]
    fn reserve_log_append_capacity_skips_zero_additional_items() {
        let mut values = vec![1, 2, 3];
        values.reserve(8);
        let capacity = values.capacity();

        reserve_log_append_capacity(&mut values, 0);

        assert_eq!(values.capacity(), capacity);
    }

    #[test]
    fn reserve_log_append_capacity_skips_growth_when_spare_capacity_is_enough() {
        let mut values = Vec::with_capacity(8);
        values.extend([1, 2, 3, 4]);
        let capacity = values.capacity();

        reserve_log_append_capacity(&mut values, 4);

        assert_eq!(values.capacity(), capacity);
    }

    #[test]
    fn initial_paginated_log_keeps_bounded_append_slack() {
        let mut values = Vec::with_capacity(600);
        values.extend(0..600);

        reserve_initial_paginated_log_append_slack(&mut values);

        assert!(values.capacity() >= values.len() + 512);
    }
}
