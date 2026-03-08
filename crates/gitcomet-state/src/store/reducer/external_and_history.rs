use super::util::{
    diff_reload_effects, handle_session_persist_result, push_diagnostic, refresh_full_effects,
    refresh_primary_effects,
};
use crate::model::{AppState, DiagnosticKind, Loadable, RepoLoadsInFlight};
use crate::msg::{Effect, RepoExternalChange};
use crate::session;
use gitcomet_core::domain::{DiffTarget, LogCursor, LogPage, LogScope};
use gitcomet_core::error::Error;
use std::sync::Arc;

pub(super) fn reload_repo(state: &mut AppState, repo_id: crate::model::RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_head_branch(Loadable::Loading);
    repo_state.set_branches(Loadable::Loading);
    repo_state.set_tags(Loadable::Loading);
    repo_state.set_remote_tags(Loadable::Loading);
    repo_state.set_remotes(Loadable::Loading);
    repo_state.set_remote_branches(Loadable::Loading);
    repo_state.set_status(Loadable::Loading);
    repo_state.set_log(Loadable::Loading);
    repo_state.log_loading_more = false;
    repo_state.set_stashes(Loadable::Loading);
    repo_state.reflog = Loadable::NotLoaded;
    repo_state.set_rebase_in_progress(Loadable::Loading);
    repo_state.set_merge_commit_message(Loadable::Loading);
    repo_state.file_history_path = None;
    repo_state.file_history = Loadable::NotLoaded;
    repo_state.blame_path = None;
    repo_state.blame_rev = None;
    repo_state.blame = Loadable::NotLoaded;
    repo_state.set_worktrees(Loadable::NotLoaded);
    repo_state.set_submodules(Loadable::NotLoaded);
    repo_state.set_selected_commit(None);
    repo_state.set_commit_details(Loadable::NotLoaded);

    refresh_full_effects(repo_state)
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
    let mut effects = match change {
        RepoExternalChange::Worktree => {
            if repo_state
                .loads_in_flight
                .request(RepoLoadsInFlight::STATUS)
            {
                vec![Effect::LoadStatus { repo_id }]
            } else {
                Vec::new()
            }
        }
        RepoExternalChange::GitState | RepoExternalChange::Both => {
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
        }
    };

    if let Some(target) = repo_state.diff_target.clone()
        && matches!(target, DiffTarget::WorkingTree { .. })
    {
        effects.extend(diff_reload_effects(repo_id, target));
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
        if repo_state.history_scope == scope {
            return Vec::new();
        }

        repo_state.set_log_scope(scope);
        repo_state.set_log(Loadable::Loading);
        repo_state.log_loading_more = false;
        repo_state.spec.workdir.clone()
    };
    let persist_result = session::persist_repo_history_scope(&workdir, scope);
    handle_session_persist_result(
        state,
        Some(repo_id),
        "updating history scope",
        persist_result,
    );

    if state.repos[repo_ix]
        .loads_in_flight
        .request_log(scope, 200, None)
    {
        vec![Effect::LoadLog {
            repo_id,
            scope,
            limit: 200,
            cursor: None,
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_more_history(
    state: &mut AppState,
    repo_id: crate::model::RepoId,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    if repo_state.log_loading_more {
        return Vec::new();
    }

    let Loadable::Ready(page) = &repo_state.log else {
        return Vec::new();
    };
    let Some(cursor) = page.next_cursor.clone() else {
        return Vec::new();
    };

    repo_state.set_log_loading_more(true);
    if repo_state
        .loads_in_flight
        .request_log(repo_state.history_scope, 200, Some(cursor.clone()))
    {
        vec![Effect::LoadLog {
            repo_id,
            scope: repo_state.history_scope,
            limit: 200,
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

        if repo_state.history_scope != scope {
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
                    let existing = Arc::make_mut(existing);
                    existing.commits.append(&mut page.commits);
                    existing.next_cursor = page.next_cursor;
                    repo_state.log_rev = repo_state.log_rev.wrapping_add(1);
                } else {
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
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
        repo_state.bump_ops_rev();
        match result {
            Ok(()) => repo_state.last_error = None,
            Err(e) => {
                repo_state.last_error = Some(e.to_string());
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
            }
        }
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let mut effects = refresh_primary_effects(repo_state);
    if let Some(target) = repo_state.diff_target.clone()
        && matches!(target, DiffTarget::WorkingTree { .. })
    {
        effects.extend(diff_reload_effects(repo_id, target));
    }
    effects
}
