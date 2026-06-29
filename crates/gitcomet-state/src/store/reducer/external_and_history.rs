use super::actions_emit_effects::invalidate_loaded_blame;
use super::effects::append_ensure_sidebar_data_effects;
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
use crate::model::{AppState, DiagnosticKind, Loadable, RepoLoadsInFlight};
use crate::msg::{Effect, RepoActionKind, RepoExternalChange};
use gitcomet_core::domain::{DiffArea, DiffTarget, LogCursor, LogPage, LogScope};
use gitcomet_core::error::Error;
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
    repo_state.set_merge_commit_message(Loadable::Loading);
    repo_state.history_state.file_history_path = None;
    repo_state.history_state.file_history = Loadable::NotLoaded;
    repo_state.history_state.blame_path = None;
    repo_state.history_state.blame_source = None;
    repo_state.history_state.blame = Loadable::NotLoaded;
    repo_state.set_worktrees(Loadable::NotLoaded);
    repo_state.set_submodules(Loadable::NotLoaded);
    repo_state.clear_head_dependent_cached_state();
    repo_state.set_selected_commit(None);
    repo_state.set_commit_details(Loadable::NotLoaded);
    // A full reload may rewrite history (rebase/amend/branch switch underneath),
    // so back/forward snapshots can reference commits or file revisions that no
    // longer resolve. Start the navigation stacks fresh.
    repo_state.nav_history.clear();
    repo_state.view_history.clear();

    let mut effects = refresh_full_effects(repo_state, git_log_settings);
    append_auto_background_metadata_effects(repo_state, git_log_settings, &mut effects);
    effects
}

pub(super) fn repo_externally_changed(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    change: RepoExternalChange,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

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
            DiffTarget::CommitRange { .. } => false,
        });

    if should_reload_diff
        && let Some(target) = repo_state.diff_state.diff_target.clone()
        && matches!(target, DiffTarget::WorkingTree { .. })
    {
        // The working-tree content changed underneath us and the diff is being
        // reloaded; the annotation column is derived from that same content, so
        // drop loaded blame too. `blame_path`/`blame_source` are preserved, so
        // `request_blame_for_current_target` reloads the same target's blame
        // against the new content instead of painting stale attribution.
        invalidate_loaded_blame(repo_state);
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

    let workdir = {
        let repo_state = &mut state.repos[repo_ix];
        if repo_state.history_state.history_scope == scope {
            return Vec::new();
        }

        repo_state.set_log_scope(scope);
        repo_state.retain_log_while_loading();
        repo_state.set_log(Loadable::Loading);
        repo_state.set_log_loading_more(false);
        repo_state.spec.workdir.clone()
    };
    let mut effects = vec![Effect::PersistRepoHistoryMode {
        repo_id: Some(repo_id),
        workdir,
        mode: scope,
        action: "updating history mode",
    }];
    if state.repos[repo_ix].loads_in_flight.request_log(
        scope,
        super::util::DEFAULT_LOG_PAGE_SIZE,
        None,
    ) {
        effects.push(Effect::LoadLog {
            repo_id,
            scope,
            limit: super::util::DEFAULT_LOG_PAGE_SIZE,
            cursor: None,
        });
    }
    effects
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
    if repo_state.loads_in_flight.request_log(
        repo_state.history_state.history_scope,
        super::util::DEFAULT_LOG_PAGE_SIZE,
        Some(cursor.clone()),
    ) {
        vec![Effect::LoadLog {
            repo_id,
            scope: repo_state.history_state.history_scope,
            limit: super::util::DEFAULT_LOG_PAGE_SIZE,
            cursor: Some(cursor),
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn rebase_state_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    result: std::result::Result<bool, Error>,
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
        repo_state.set_rebase_in_progress(value);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REBASE_STATE)
        {
            effects.push(Effect::LoadRebaseState { repo_id });
        }
    }
    effects
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

pub(super) fn log_loaded(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
    scope: LogScope,
    cursor: Option<LogCursor>,
    result: std::result::Result<LogPage, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let is_load_more = cursor.is_some();

        if repo_state.history_state.history_scope != scope {
            if is_load_more {
                repo_state.set_log_loading_more(false);
            }
            if let Some(next) = repo_state.loads_in_flight.finish_log() {
                repo_state.set_log_loading_more(next.cursor.is_some());
                effects.push(Effect::LoadLog {
                    repo_id,
                    scope: next.scope,
                    limit: next.limit,
                    cursor: next.cursor,
                });
            }
            return effects;
        }

        match result {
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

        if is_load_more {
            repo_state.set_log_loading_more(false);
        }

        if let Some(next) = repo_state.loads_in_flight.finish_log() {
            repo_state.set_log_loading_more(next.cursor.is_some());
            effects.push(Effect::LoadLog {
                repo_id,
                scope: next.scope,
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
