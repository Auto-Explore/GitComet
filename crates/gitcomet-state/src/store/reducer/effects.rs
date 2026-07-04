use super::util::{
    EffectAccumulator, apply_selected_diff_load_plan_state, diff_reload_effects, push_diagnostic,
    push_notification, selected_diff_load_plan,
};
use crate::model::{
    AppNotificationKind, AppState, ConflictFileLoadMode, DiagnosticKind, Loadable, RepoId,
    RepoLoadsInFlight, RepoState, SidebarDataRequest, SidebarMode,
};
use crate::msg::{CommitSelectMode, Effect};
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitId, FileEntry, FileSource, FileStatusKind, LogPage,
    RecentCommitMessage, ReflogEntry, Remote, RemoteBranch, RemoteTag, RepoStatus, StashEntry,
    Submodule, Tag, UpstreamDivergence, Worktree,
};
use gitcomet_core::error::Error;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn file_history_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: std::result::Result<LogPage, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.file_history_path.as_ref() == Some(&path)
    {
        repo_state.history_state.file_history = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
    }
    Vec::new()
}

pub(super) fn blame_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
    result: std::result::Result<Vec<gitcomet_core::services::BlameLine>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.blame_path.as_ref() == Some(&path)
        && repo_state.history_state.blame_source.as_ref() == Some(&source)
    {
        repo_state.history_state.blame = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
    }
    Vec::new()
}

pub(super) fn conflict_file_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: std::result::Result<Option<crate::model::ConflictFile>, Error>,
    conflict_session: Option<ConflictSession>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.conflict_state.conflict_file_path.as_ref() == Some(&path)
    {
        let existing_session = repo_state.conflict_state.conflict_session.as_ref();
        let session = conflict_session.or_else(|| match &result {
            Ok(Some(file)) => build_conflict_session(repo_state, file),
            _ => None,
        });
        let session = session.map(|mut session| {
            if let Some(existing_session) = existing_session {
                restore_conflict_session_resolutions(existing_session, &mut session);
            }
            session
        });
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_conflict_file(value);
        repo_state.set_conflict_session(session);
    }
    Vec::new()
}

fn restore_conflict_session_resolutions(existing: &ConflictSession, next: &mut ConflictSession) {
    if existing.path != next.path {
        return;
    }

    for (prev, current) in existing.regions.iter().zip(next.regions.iter_mut()) {
        if prev.base == current.base && prev.ours == current.ours && prev.theirs == current.theirs {
            current.resolution = prev.resolution.clone();
        }
    }
}

/// Build a `ConflictSession` from a loaded `ConflictFile` and the current repo status.
///
/// Looks up the `FileConflictKind` from the status entries and constructs
/// a session with parsed conflict regions (for marker-based text conflicts).
fn build_conflict_session(
    repo_state: &crate::model::RepoState,
    file: &crate::model::ConflictFile,
) -> Option<ConflictSession> {
    // Look up the conflict kind from the repo's status entries.
    let conflict_kind = repo_state
        .worktree_status_entries()?
        .iter()
        .find(|e| e.path == file.path && e.kind == FileStatusKind::Conflicted)
        .and_then(|e| e.conflict)?;

    let base = ConflictPayload::from_stage_parts(file.base_bytes.clone(), file.base.clone());
    let ours = ConflictPayload::from_stage_parts(file.ours_bytes.clone(), file.ours.clone());
    let theirs = ConflictPayload::from_stage_parts(file.theirs_bytes.clone(), file.theirs.clone());

    // If we have merged text with markers, parse regions from it.
    if let Some(current) = file.current.as_ref() {
        Some(ConflictSession::from_merged_shared_text(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            current.clone(),
        ))
    } else if let Some(current) = file.current_bytes.as_ref() {
        Some(ConflictSession::new_with_current(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
            ConflictPayload::Binary(current.clone()),
        ))
    } else {
        Some(ConflictSession::new(
            file.path.to_path_buf(),
            conflict_kind,
            base,
            ours,
            theirs,
        ))
    }
}

pub(super) fn worktrees_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Worktree>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let worktrees = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_worktrees(worktrees);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::WORKTREES)
        {
            effects.push(Effect::LoadWorktrees { repo_id });
        }
    }
    effects
}

pub(super) fn submodules_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Submodule>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let submodules = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_submodules(submodules);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::SUBMODULES)
        {
            effects.push(Effect::LoadSubmodules { repo_id });
        }
    }
    effects
}

pub(super) fn select_commit(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    select_commit_multi(
        state,
        repo_id,
        commit_id,
        CommitSelectMode::Single,
        None,
        None,
    )
}

pub(super) fn select_commit_multi(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    mode: CommitSelectMode,
    clicked_index: Option<usize>,
    visible_order: Option<Vec<CommitId>>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let log_rev = repo_state.history_state.log_rev;
    let mut sel = repo_state.history_state.multi_selection.clone();

    let focus = match mode {
        CommitSelectMode::Single => {
            collapse_multi_selection_to(&mut sel, commit_id.clone(), clicked_index, log_rev);
            commit_id
        }
        CommitSelectMode::Toggle => {
            if let Some(ix) = sel.commits.iter().position(|c| *c == commit_id) {
                sel.commits.remove(ix);
                let Some(focus) = sel.commits.last().cloned() else {
                    // Toggled the last commit away: clear the selection
                    // entirely (also dissolves the multi-selection).
                    repo_state.set_selected_commit(None);
                    repo_state.set_commit_details(Loadable::NotLoaded);
                    return Vec::new();
                };
                focus
            } else {
                sel.commits.push(commit_id.clone());
                sel.anchor = Some(commit_id.clone());
                sel.anchor_index = clicked_index;
                sel.anchor_log_rev = Some(log_rev);
                commit_id
            }
        }
        CommitSelectMode::Range => {
            let entries = visible_order.as_deref().unwrap_or(&[]);
            let clicked_ix = commit_selection_entry_index(entries, &commit_id, clicked_index);
            match clicked_ix {
                None => {
                    collapse_multi_selection_to(
                        &mut sel,
                        commit_id.clone(),
                        clicked_index,
                        log_rev,
                    );
                }
                Some(clicked_ix) => {
                    let anchor_ix = sel
                        .anchor
                        .as_ref()
                        .and_then(|anchor| {
                            let trusted_hint = sel
                                .anchor_index
                                .filter(|_| sel.anchor_log_rev == Some(log_rev));
                            commit_selection_entry_index(entries, anchor, trusted_hint)
                        })
                        .unwrap_or(clicked_ix);
                    let (a, b) = if anchor_ix <= clicked_ix {
                        (anchor_ix, clicked_ix)
                    } else {
                        (clicked_ix, anchor_ix)
                    };
                    sel.commits = entries[a..=b].to_vec();
                    if sel.anchor.is_none() {
                        sel.anchor = Some(commit_id.clone());
                    }
                    sel.anchor_index = Some(anchor_ix);
                    sel.anchor_log_rev = Some(log_rev);
                }
            }
            commit_id
        }
        CommitSelectMode::PreserveIfSelected => {
            // Keep an existing multi-selection intact when the clicked commit
            // is already part of it — only the focus moves. Otherwise collapse
            // to the clicked commit like a plain click.
            if !sel.commits.contains(&commit_id) {
                collapse_multi_selection_to(&mut sel, commit_id.clone(), clicked_index, log_rev);
            }
            commit_id
        }
    };

    repo_state.set_commit_multi_selection(sel);
    select_commit_and_load_details(repo_state, repo_id, focus)
}

fn collapse_multi_selection_to(
    sel: &mut crate::model::CommitMultiSelection,
    commit_id: CommitId,
    clicked_index: Option<usize>,
    log_rev: u64,
) {
    sel.commits.clear();
    sel.commits.push(commit_id.clone());
    sel.anchor = Some(commit_id);
    sel.anchor_index = clicked_index;
    sel.anchor_log_rev = Some(log_rev);
}

/// Resolves `target`'s index in `entries`, preferring the index hint when it
/// still points at the target.
fn commit_selection_entry_index(
    entries: &[CommitId],
    target: &CommitId,
    index_hint: Option<usize>,
) -> Option<usize> {
    index_hint
        .filter(|&ix| entries.get(ix) == Some(target))
        .or_else(|| entries.iter().position(|id| id == target))
}

pub(super) fn select_commit_and_load_details(
    repo_state: &mut RepoState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    if repo_state.history_state.selected_commit.as_ref() == Some(&commit_id) {
        return Vec::new();
    }

    repo_state.set_selected_commit(Some(commit_id.clone()));
    let already_loaded = matches!(
        &repo_state.history_state.commit_details,
        Loadable::Ready(details) if details.id == commit_id
    );
    if already_loaded {
        return Vec::new();
    }

    if matches!(
        repo_state.history_state.commit_details,
        Loadable::Error(_) | Loadable::NotLoaded
    ) {
        repo_state.set_commit_details(Loadable::NotLoaded);
    }
    vec![Effect::LoadCommitDetails { repo_id, commit_id }]
}

pub(super) fn clear_commit_selection(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_selected_commit(None);
    repo_state.set_commit_details(Loadable::NotLoaded);
    Vec::new()
}

pub(super) fn append_ensure_sidebar_data_effects(
    repo_state: &mut RepoState,
    effects: &mut impl EffectAccumulator,
) {
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return;
    }

    let repo_id = repo_state.id;
    let request = repo_state.sidebar_data_request;

    if request.worktrees && matches!(repo_state.worktrees, Loadable::NotLoaded) {
        repo_state.set_worktrees(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREES)
        {
            effects.push_effect(Effect::LoadWorktrees { repo_id });
        }
    }

    if request.submodules && matches!(repo_state.submodules, Loadable::NotLoaded) {
        repo_state.set_submodules(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::SUBMODULES)
        {
            effects.push_effect(Effect::LoadSubmodules { repo_id });
        }
    }

    if request.stashes && matches!(repo_state.stashes, Loadable::NotLoaded) {
        repo_state.set_stashes(Loadable::Loading);
        if repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::STASHES)
        {
            effects.push_effect(Effect::LoadStashes { repo_id, limit: 50 });
        }
    }
}

pub(super) fn ensure_sidebar_data(
    state: &mut AppState,
    repo_id: RepoId,
    request: SidebarDataRequest,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    repo_state.set_sidebar_data_request(request);
    let mut effects = Vec::new();
    append_ensure_sidebar_data_effects(repo_state, &mut effects);
    effects
}

pub(super) fn load_stashes(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_stashes(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::STASHES)
    {
        vec![Effect::LoadStashes { repo_id, limit: 50 }]
    } else {
        Vec::new()
    }
}

pub(super) fn refresh_branches(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES)
    {
        vec![Effect::LoadBranches { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_tags(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_tags(Loadable::Loading);
    if repo_state.loads_in_flight.request(RepoLoadsInFlight::TAGS) {
        vec![Effect::LoadTags { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_remote_tags(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_remote_tags(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REMOTE_TAGS)
    {
        vec![Effect::LoadRemoteTags { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_conflict_file(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    mode: ConflictFileLoadMode,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.set_conflict_file_path(Some(path.clone()));
    repo_state.set_conflict_file_load_mode(mode);
    repo_state.set_conflict_file(Loadable::Loading);
    repo_state.set_conflict_session(None);
    repo_state.set_conflict_hide_resolved(false);
    vec![Effect::LoadConflictFile {
        repo_id,
        path,
        mode,
    }]
}

pub(super) fn load_reflog(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.reflog = Loadable::Loading;
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::REFLOG)
    {
        vec![Effect::LoadReflog {
            repo_id,
            limit: 200,
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_recent_commit_messages(
    state: &mut AppState,
    repo_id: RepoId,
    limit: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(()))
        || matches!(repo_state.recent_commit_messages, Loadable::Loading)
    {
        return Vec::new();
    }
    repo_state.set_recent_commit_messages(Loadable::Loading);
    let request_rev = repo_state.recent_commit_messages_rev;
    vec![Effect::LoadRecentCommitMessages {
        repo_id,
        limit,
        request_rev,
    }]
}

pub(super) fn recent_commit_messages_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    request_rev: u64,
    result: std::result::Result<Vec<RecentCommitMessage>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.recent_commit_messages_rev == request_rev
    {
        let value = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_recent_commit_messages(value);
    }
    Vec::new()
}

pub(super) fn load_file_history(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    limit: usize,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.history_state.file_history_path = Some(path.clone());
    repo_state.history_state.file_history = Loadable::Loading;
    vec![Effect::LoadFileHistory {
        repo_id,
        path,
        limit,
    }]
}

pub(super) fn load_blame(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.history_state.blame_path = Some(path.clone());
    repo_state.history_state.blame_source = Some(source.clone());
    repo_state.history_state.blame = Loadable::Loading;
    vec![Effect::LoadBlame {
        repo_id,
        path,
        source,
    }]
}

pub(super) fn load_worktrees(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_worktrees(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREES)
    {
        vec![Effect::LoadWorktrees { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_submodules(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.set_submodules(Loadable::Loading);
    if repo_state
        .loads_in_flight
        .request(RepoLoadsInFlight::SUBMODULES)
    {
        vec![Effect::LoadSubmodules { repo_id }]
    } else {
        Vec::new()
    }
}

pub(super) fn load_file_browser(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !matches!(repo_state.open, Loadable::Ready(())) {
        return Vec::new();
    }
    repo_state.file_browser.source = source.clone();
    repo_state.file_browser.entries = Loadable::Loading;
    repo_state.file_browser.bump_rev();
    vec![Effect::LoadFileBrowser { repo_id, source }]
}

pub(super) fn toggle_file_browser_dir(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let path = Arc::new(path);
        if repo_state.file_browser.expanded_dirs.contains(&path) {
            repo_state.file_browser.expanded_dirs.remove(&path);
        } else {
            repo_state.file_browser.expanded_dirs.insert(path);
        }
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn set_file_browser_search(
    state: &mut AppState,
    repo_id: RepoId,
    query: String,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.file_browser.search_query != query
    {
        repo_state.file_browser.search_query = query;
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn set_file_browser_source(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.file_browser.source != source
    {
        repo_state.file_browser.source = source.clone();
        repo_state.file_browser.entries = Loadable::NotLoaded;
        repo_state.file_browser.expanded_dirs.clear();
        repo_state.file_browser.search_query.clear();
        repo_state.file_browser.bump_rev();
        return vec![Effect::LoadFileBrowser { repo_id, source }];
    }
    Vec::new()
}

pub(super) fn set_sidebar_mode(state: &mut AppState, mode: SidebarMode) -> Vec<Effect> {
    if state.sidebar_mode != mode {
        state.sidebar_mode = mode;

        if mode == SidebarMode::Files
            && let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id)
            && matches!(
                repo.file_browser.entries,
                Loadable::NotLoaded | Loadable::Error(_)
            )
        {
            let source = repo.file_browser.source.clone();
            return vec![Effect::LoadFileBrowser { repo_id, source }];
        }
    }
    Vec::new()
}

pub(super) fn browse_repository_at_commit(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    const BROWSE_HISTORY_CAP: usize = 32;
    // Capture the open file (if any) before re-targeting it to the new point.
    let reopen_path = browse_open_content_path(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && !repo_state.browse_history.contains(&commit_id)
    {
        repo_state.browse_history.push(commit_id.clone());
        if repo_state.browse_history.len() > BROWSE_HISTORY_CAP {
            repo_state.browse_history.remove(0);
        }
    }
    state.sidebar_mode = SidebarMode::Files;
    let mut effects =
        set_file_browser_source(state, repo_id, FileSource::Commit(commit_id.clone()));
    if let Some(path) = reopen_path
        && effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    {
        effects.extend(super::diff_selection::open_file_content(
            state,
            repo_id,
            FileSource::Commit(commit_id),
            path,
        ));
    }
    effects
}

pub(super) fn reset_browse_to_live(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let reopen_path = browse_open_content_path(state, repo_id);
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.browse_history.clear();
    }
    let mut effects = set_file_browser_source(state, repo_id, FileSource::WorkingDirectory);
    if let Some(path) = reopen_path
        && effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    {
        effects.extend(super::diff_selection::open_file_content(
            state,
            repo_id,
            FileSource::WorkingDirectory,
            path,
        ));
    }
    effects
}

/// Path of the file currently shown as full content (if any), so a browse-point
/// change can re-open the same file at the new point.
fn browse_open_content_path(state: &AppState, repo_id: RepoId) -> Option<std::path::PathBuf> {
    let repo = state.repos.iter().find(|r| r.id == repo_id)?;
    if !repo.diff_state.content_preview {
        return None;
    }
    match &repo.diff_state.diff_target {
        Some(gitcomet_core::domain::DiffTarget::Commit { path: Some(p), .. }) => Some(p.clone()),
        Some(gitcomet_core::domain::DiffTarget::WorkingTree { path, .. }) => Some(path.clone()),
        _ => None,
    }
}

pub(super) fn file_browser_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
    result: std::result::Result<Vec<FileEntry>, gitcomet_core::error::Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        if repo_state.file_browser.source != source {
            return Vec::new();
        }
        repo_state.file_browser.entries = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.file_browser.bump_rev();
    }
    Vec::new()
}

pub(super) fn branches_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Branch>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let branches = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_branches(branches);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::BRANCHES)
        {
            effects.push(Effect::LoadBranches { repo_id });
        }
    }
    effects
}

pub(super) fn remotes_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Remote>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let remotes = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_remotes(remotes);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTES)
        {
            effects.push(Effect::LoadRemotes { repo_id });
        }
    }
    effects
}

pub(super) fn remote_branches_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<RemoteBranch>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let branches = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_remote_branches(branches);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTE_BRANCHES)
        {
            effects.push(Effect::LoadRemoteBranches { repo_id });
        }
    }
    effects
}

pub(super) fn status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<RepoStatus, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(
                    &repo_state.status,
                    Loadable::Ready(prev) if prev.as_ref() == &next
                );
                if !status_unchanged {
                    repo_state.set_status(Loadable::Ready(Arc::new(next)));
                }
                clear_resolved_conflict_context(repo_state);
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::WORKTREE_STATUS,
            Effect::LoadWorktreeStatus { repo_id },
            &mut effects,
        );
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::STAGED_STATUS,
            Effect::LoadStagedStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

pub(super) fn worktree_status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<gitcomet_core::domain::FileStatus>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(&repo_state.worktree_status, Loadable::Ready(prev) if prev.as_slice() == next.as_slice());
                if !status_unchanged {
                    repo_state.set_worktree_status(Loadable::Ready(next));
                }
                clear_resolved_conflict_context(repo_state);
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_worktree_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::WORKTREE_STATUS,
            Effect::LoadWorktreeStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

pub(super) fn staged_status_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<gitcomet_core::domain::FileStatus>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match result {
            Ok(next) => {
                let status_unchanged = matches!(&repo_state.staged_status, Loadable::Ready(prev) if prev.as_slice() == next.as_slice());
                if !status_unchanged {
                    repo_state.set_staged_status(Loadable::Ready(next));
                }
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_staged_status(Loadable::Error(e.to_string()));
            }
        }
        finish_status_lane_replay(
            repo_state,
            RepoLoadsInFlight::STAGED_STATUS,
            Effect::LoadStagedStatus { repo_id },
            &mut effects,
        );
    }
    effects
}

fn finish_status_lane_replay(
    repo_state: &mut crate::model::RepoState,
    flag: u32,
    replay_effect: Effect,
    effects: &mut Vec<Effect>,
) {
    // A pending request means a refresh was coalesced while this load was in flight — a genuine
    // external change or a just-completed action. Always replay it, even when the loaded payload
    // matches what is currently displayed: the in-flight load may have read the working tree or
    // index just *before* the change landed, so the coalesced refresh is the only chance to
    // observe it. Suppressing it on an unchanged payload (as a previous revision did) drops real
    // external changes and leaves stale entries in the uncommitted view.
    //
    // This cannot self-sustain a refresh loop: status reads are read-only (the gix backend's
    // `maybe_persist_*` helpers never rewrite `.git/index`, and worktree reads emit only ignored
    // `Access` events), so a completed status load never manufactures the filesystem event that
    // would set `pending` again.
    if repo_state.loads_in_flight.finish(flag) {
        effects.push(replay_effect);
    }
}

/// Clear conflict-file/session state when the tracked conflict path is no longer
/// present as an unresolved conflict in status.
fn clear_resolved_conflict_context(repo_state: &mut crate::model::RepoState) {
    let Some(conflict_path) = repo_state.conflict_state.conflict_file_path.as_ref() else {
        return;
    };
    let still_conflicted = repo_state.worktree_status_entries().is_none_or(|status| {
        status
            .iter()
            .any(|entry| entry.path == *conflict_path && entry.kind == FileStatusKind::Conflicted)
    });
    if still_conflicted {
        return;
    }

    repo_state.set_conflict_file_path(None);
    repo_state.set_conflict_file_load_mode(ConflictFileLoadMode::CurrentOnly);
    repo_state.set_conflict_file(Loadable::NotLoaded);
    repo_state.set_conflict_session(None);
    repo_state.set_conflict_hide_resolved(false);
}

pub(super) fn head_branch_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let head_branch = match result {
            Ok(v) => {
                if v == "HEAD" {
                    if repo_state.detached_head_commit.is_none()
                        && repo_state
                            .history_state
                            .history_scope
                            .guarantees_head_visibility()
                        && let Loadable::Ready(page) = &repo_state.log
                    {
                        repo_state
                            .set_detached_head_commit(page.commits.first().map(|c| c.id.clone()));
                    }
                } else {
                    repo_state.set_detached_head_commit(None);
                }
                Loadable::Ready(v)
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_head_branch(head_branch);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::HEAD_BRANCH)
        {
            effects.push(Effect::LoadHeadBranch { repo_id });
        }
    }
    effects
}

pub(super) fn upstream_divergence_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Option<UpstreamDivergence>, Error>,
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
        repo_state.set_upstream_divergence(value);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::UPSTREAM_DIVERGENCE)
        {
            effects.push(Effect::LoadUpstreamDivergence { repo_id });
        }
    }
    effects
}

pub(super) fn tags_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Tag>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let tags = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) {
                    Loadable::Ready(Vec::new())
                } else if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_tags(tags);
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::TAGS) {
            effects.push(Effect::LoadTags { repo_id });
        }
    }
    effects
}

pub(super) fn remote_tags_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<RemoteTag>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let remote_tags = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                if matches!(e.kind(), gitcomet_core::error::ErrorKind::Unsupported(_)) {
                    Loadable::Ready(Vec::new())
                } else if matches!(e.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                    Loadable::NotLoaded
                } else {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            }
        };
        repo_state.set_remote_tags(remote_tags);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::REMOTE_TAGS)
        {
            effects.push(Effect::LoadRemoteTags { repo_id });
        }
    }
    effects
}

pub(super) fn stashes_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<StashEntry>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let stashes = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_stashes(stashes);
        if repo_state
            .loads_in_flight
            .finish(RepoLoadsInFlight::STASHES)
        {
            effects.push(Effect::LoadStashes { repo_id, limit: 50 });
        }
    }
    effects
}

pub(super) fn reflog_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<ReflogEntry>, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.reflog = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::REFLOG) {
            effects.push(Effect::LoadReflog {
                repo_id,
                limit: 200,
            });
        }
    }
    effects
}

/// Validates the current multi-selection against the loaded log and HEAD.
/// This is the single reducer-side gate for every squash entry point.
pub(super) fn squash_plan_for_repo(
    repo_state: &RepoState,
) -> Option<gitcomet_core::squash::SquashPlan> {
    let Loadable::Ready(page) = &repo_state.log else {
        return None;
    };
    let head = repo_state.head_commit_id()?;
    gitcomet_core::squash::squash_eligibility(
        &page.commits,
        &repo_state.history_state.multi_selection.commits,
        &head,
    )
}

pub(super) fn prepare_squash(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(plan) = squash_plan_for_repo(repo_state) else {
        repo_state.history_state.squash_preview_pending = None;
        repo_state.set_squash_preview(Loadable::NotLoaded);
        return Vec::new();
    };

    repo_state.history_state.squash_preview_pending =
        Some((plan.oldest.clone(), plan.head.clone()));
    repo_state.set_squash_preview(Loadable::Loading);
    vec![Effect::LoadSquashMessagePreview {
        repo_id,
        oldest: plan.oldest,
        head: plan.head,
    }]
}

pub(super) fn squash_message_preview_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    oldest: CommitId,
    head: CommitId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        // Accept the result only if it still matches the range we last asked
        // for. Keying off the recorded request (not the live plan) means a
        // transiently-invalid plan — e.g. HEAD momentarily unresolved during a
        // concurrent reload — does not drop the result and strand the preview
        // on Loading forever.
        let matches_request = repo_state.history_state.squash_preview_pending.as_ref()
            == Some(&(oldest.clone(), head.clone()));
        if matches_request {
            repo_state.history_state.squash_preview_pending = None;
            let value = match result {
                Ok(message) => {
                    let (subject, body) = gitcomet_core::squash::split_subject_body(&message);
                    Loadable::Ready(crate::model::SquashPreview {
                        oldest,
                        head,
                        subject,
                        body,
                    })
                }
                Err(e) => {
                    push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                    Loadable::Error(e.to_string())
                }
            };
            repo_state.set_squash_preview(value);
        }
    }
    Vec::new()
}

pub(super) fn squash_rebase_setup_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    base: String,
    actual_head: CommitId,
    selected_ids: Vec<CommitId>,
    reword_id: CommitId,
    message: String,
    count: usize,
    result: std::result::Result<Vec<InteractiveRebaseEntry>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let entries = match result {
        Ok(entries) => entries,
        Err(e) => {
            push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
            push_notification(
                state,
                AppNotificationKind::Error,
                format!("Failed to load commits for squash rebase: {e}"),
            );
            return Vec::new();
        }
    };

    let selected_strs: HashSet<&str> = selected_ids.iter().map(|id| id.as_ref()).collect();

    // The list loaded asynchronously, so re-validate it against the plan the
    // user confirmed before rewriting history. `git log --reverse base..HEAD`
    // yields commits oldest-first, so the last entry is the live HEAD.
    let head_unchanged = entries
        .last()
        .is_some_and(|e| e.commit_id == actual_head.as_ref());

    let mut matched = 0usize;
    let mut reword_found = false;
    let todo: Vec<InteractiveRebaseEntry> = entries
        .into_iter()
        .map(|mut entry| {
            if entry.commit_id == reword_id.as_ref() {
                entry.action = InteractiveRebaseAction::Reword;
                entry.new_message = Some(message.clone());
                reword_found = true;
                matched += 1;
            } else if selected_strs.contains(entry.commit_id.as_str()) {
                entry.action = InteractiveRebaseAction::Fixup;
                matched += 1;
            }
            entry
        })
        .collect();

    // Every selected commit must appear exactly once in the live range and the
    // oldest must have become the reword anchor; otherwise HEAD moved or the
    // range drifted between confirmation and now, and rewriting would either be
    // a silent no-op or touch the wrong commits. `matched == count` also
    // catches a selection count that disagrees with what was actually planned.
    if !head_unchanged || !reword_found || matched != count {
        push_notification(
            state,
            AppNotificationKind::Warning,
            "Squash cancelled: the selected commits are no longer squashable.".to_string(),
        );
        return Vec::new();
    }

    super::begin_local_action(state, repo_id);
    vec![Effect::InteractiveRebase {
        repo_id,
        base,
        entries: todo,
    }]
}

pub(super) fn commit_details_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    result: std::result::Result<CommitDetails, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.selected_commit.as_ref() == Some(&commit_id)
    {
        let selected_target = repo_state.diff_state.diff_target.clone();
        let previous_plan = selected_target
            .as_ref()
            .map(|target| selected_diff_load_plan(repo_state, target));
        let value = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_commit_details(value);

        if let Some(target @ gitcomet_core::domain::DiffTarget::Commit { .. }) = selected_target {
            let next_plan = selected_diff_load_plan(repo_state, &target);
            if previous_plan != Some(next_plan) {
                apply_selected_diff_load_plan_state(repo_state, next_plan);
                repo_state.bump_diff_state_rev();
                return diff_reload_effects(repo_state, repo_id, target);
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConflictFile, RepoState, SidebarDataRequest, SidebarMode};
    use gitcomet_core::domain::{
        DiffArea, DiffTarget, FileConflictKind, FileEntry, FileEntryKind, FileSource, FileStatus,
        LogScope, RepoSpec,
    };
    use gitcomet_core::error::{Error, ErrorKind};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn backend_error(message: &str) -> Error {
        Error::new(ErrorKind::Backend(message.to_string()))
    }

    fn unsupported_error() -> Error {
        Error::new(ErrorKind::Unsupported("unsupported"))
    }

    fn empty_log_page() -> LogPage {
        LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }
    }

    fn commit_details_for(id: CommitId) -> CommitDetails {
        CommitDetails {
            id,
            message: "message".to_string(),
            committed_at: "now".to_string(),
            parent_ids: Vec::new(),
            files: Vec::new(),
        }
    }

    #[test]
    fn browse_history_pushes_dedups_and_go_live_clears() {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(RepoId(1));

        let a = CommitId("aaaaaaaa".into());
        let b = CommitId("bbbbbbbb".into());

        browse_repository_at_commit(&mut state, RepoId(1), a.clone());
        browse_repository_at_commit(&mut state, RepoId(1), b.clone());
        // Re-browsing an existing point does not duplicate it, just makes it current.
        browse_repository_at_commit(&mut state, RepoId(1), a.clone());

        let repo = &state.repos[0];
        assert_eq!(repo.browse_history, vec![a.clone(), b.clone()]);
        assert_eq!(repo.browsing_commit(), Some(&a));
        assert_eq!(state.sidebar_mode, SidebarMode::Files);

        reset_browse_to_live(&mut state, RepoId(1));
        let repo = &state.repos[0];
        assert!(repo.browse_history.is_empty());
        assert_eq!(repo.browsing_commit(), None);
        assert!(matches!(
            repo.file_browser.source,
            gitcomet_core::domain::FileSource::WorkingDirectory
        ));
    }

    fn conflicted_status(path: &Path, conflict: FileConflictKind) -> RepoStatus {
        RepoStatus {
            staged: Vec::new(),
            unstaged: vec![FileStatus {
                path: path.to_path_buf(),
                kind: FileStatusKind::Conflicted,
                conflict: Some(conflict),
            }],
        }
    }

    fn empty_conflict_file(path: &Path) -> ConflictFile {
        ConflictFile {
            path: path.to_path_buf().into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: None,
        }
    }

    fn new_state_with_repo(repo_id: RepoId) -> AppState {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state
    }

    fn repo_mut(state: &mut AppState, repo_id: RepoId) -> &mut RepoState {
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo_id)
            .expect("repo not found")
    }

    fn mark_repo_open_ready(state: &mut AppState, repo_id: RepoId) {
        repo_mut(state, repo_id).set_open(Loadable::Ready(()));
    }

    fn mark_pending(state: &mut AppState, repo_id: RepoId, flag: u32) {
        let repo = repo_mut(state, repo_id);
        assert!(repo.loads_in_flight.request(flag));
        assert!(!repo.loads_in_flight.request(flag));
    }

    #[test]
    fn unknown_repo_handlers_are_noops() {
        let mut state = AppState::default();
        let repo_id = RepoId(42);
        let path = PathBuf::from("tracked.txt");
        let commit_id = CommitId("abc".into());

        assert!(
            file_history_loaded(&mut state, repo_id, path.clone(), Ok(empty_log_page())).is_empty()
        );
        assert!(
            blame_loaded(
                &mut state,
                repo_id,
                path.clone(),
                gitcomet_core::domain::BlameSource::Revision(None),
                Ok(Vec::new())
            )
            .is_empty()
        );
        assert!(conflict_file_loaded(&mut state, repo_id, path.clone(), Ok(None), None).is_empty());
        assert!(worktrees_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(submodules_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(select_commit(&mut state, repo_id, commit_id.clone()).is_empty());
        assert!(clear_commit_selection(&mut state, repo_id).is_empty());
        assert!(load_stashes(&mut state, repo_id).is_empty());
        assert!(refresh_branches(&mut state, repo_id).is_empty());
        assert!(
            load_conflict_file(
                &mut state,
                repo_id,
                path.clone(),
                ConflictFileLoadMode::CurrentOnly,
            )
            .is_empty()
        );
        assert!(load_reflog(&mut state, repo_id).is_empty());
        assert!(load_file_history(&mut state, repo_id, path.clone(), 25).is_empty());
        assert!(
            load_blame(
                &mut state,
                repo_id,
                path.clone(),
                gitcomet_core::domain::BlameSource::Revision(Some("HEAD".to_string()))
            )
            .is_empty()
        );
        assert!(load_worktrees(&mut state, repo_id).is_empty());
        assert!(load_submodules(&mut state, repo_id).is_empty());
        assert!(branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remotes_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remote_branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(status_loaded(&mut state, repo_id, Ok(RepoStatus::default())).is_empty());
        assert!(head_branch_loaded(&mut state, repo_id, Ok("main".to_string())).is_empty());
        assert!(upstream_divergence_loaded(&mut state, repo_id, Ok(None)).is_empty());
        assert!(tags_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(remote_tags_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(stashes_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(reflog_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
        assert!(
            commit_details_loaded(
                &mut state,
                repo_id,
                commit_id.clone(),
                Ok(commit_details_for(commit_id))
            )
            .is_empty()
        );
        assert!(load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory).is_empty());
        assert!(toggle_file_browser_dir(&mut state, repo_id, PathBuf::from("src")).is_empty());
        assert!(set_file_browser_search(&mut state, repo_id, "query".to_string()).is_empty());
        assert!(
            set_file_browser_source(&mut state, repo_id, FileSource::WorkingDirectory).is_empty()
        );
        assert!(set_sidebar_mode(&mut state, SidebarMode::Files).is_empty());
        assert!(
            file_browser_loaded(
                &mut state,
                repo_id,
                FileSource::WorkingDirectory,
                Ok(Vec::new())
            )
            .is_empty()
        );
    }

    #[test]
    fn file_history_loaded_updates_only_matching_path_and_reports_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let tracked = PathBuf::from("tracked.txt");

        repo_mut(&mut state, repo_id)
            .history_state
            .file_history_path = Some(tracked.clone());
        file_history_loaded(
            &mut state,
            repo_id,
            PathBuf::from("other.txt"),
            Ok(empty_log_page()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.file_history,
            Loadable::NotLoaded
        ));

        file_history_loaded(&mut state, repo_id, tracked.clone(), Ok(empty_log_page()));
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.file_history,
            Loadable::Ready(_)
        ));

        file_history_loaded(
            &mut state,
            repo_id,
            tracked,
            Err(backend_error("file history failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.file_history,
            Loadable::Error(_)
        ));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn blame_loaded_requires_matching_path_and_source() {
        use gitcomet_core::domain::BlameSource;

        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let source = BlameSource::Revision(Some("HEAD~1".to_string()));

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_source = Some(source.clone());
        }

        blame_loaded(
            &mut state,
            repo_id,
            path.clone(),
            BlameSource::Revision(Some("different".to_string())),
            Ok(Vec::new()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.blame,
            Loadable::NotLoaded
        ));

        blame_loaded(
            &mut state,
            repo_id,
            path.clone(),
            source.clone(),
            Ok(Vec::new()),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.blame,
            Loadable::Ready(_)
        ));

        blame_loaded(
            &mut state,
            repo_id,
            path,
            source,
            Err(backend_error("blame failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.history_state.blame, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn conflict_file_loaded_builds_session_from_merged_markers() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("conflict.txt");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
        }

        let file = ConflictFile {
            path: path.clone().into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: Some("base\n".to_string().into()),
            ours: Some("ours\n".to_string().into()),
            theirs: Some("theirs\n".to_string().into()),
            current: Some(
                "pre\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\npost\n"
                    .to_string()
                    .into(),
            ),
        };

        conflict_file_loaded(&mut state, repo_id, path.clone(), Ok(Some(file)), None);
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.conflict_state.conflict_file,
            Loadable::Ready(Some(_))
        ));
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert_eq!(session.path, path);
        assert_eq!(session.conflict_kind, FileConflictKind::BothModified);
        assert!(!session.regions.is_empty());
    }

    #[test]
    fn conflict_file_loaded_uses_synthetic_session_for_non_marker_payloads() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("binary-conflict.bin");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
        }

        let file = ConflictFile {
            path: path.clone().into(),
            base_bytes: Some(vec![0xff, 0x00].into()),
            ours_bytes: Some(b"ours\n".to_vec().into()),
            theirs_bytes: Some(b"theirs\n".to_vec().into()),
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: None,
        };

        conflict_file_loaded(&mut state, repo_id, path, Ok(Some(file)), None);
        let repo = repo_mut(&mut state, repo_id);
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert!(session.base.is_binary());
    }

    #[test]
    fn conflict_file_loaded_prefers_provided_session_and_records_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let tracked_path = PathBuf::from("tracked.txt");
        let other_path = PathBuf::from("other.txt");

        repo_mut(&mut state, repo_id).set_conflict_file_path(Some(tracked_path.clone()));
        let provided = ConflictSession::new(
            tracked_path.clone(),
            FileConflictKind::BothAdded,
            ConflictPayload::Absent,
            ConflictPayload::Text("ours\n".to_string().into()),
            ConflictPayload::Text("theirs\n".to_string().into()),
        );

        conflict_file_loaded(
            &mut state,
            repo_id,
            tracked_path.clone(),
            Err(backend_error("conflict failed")),
            Some(provided.clone()),
        );
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(
                repo.conflict_state.conflict_file,
                Loadable::Error(_)
            ));
            let session = repo
                .conflict_state
                .conflict_session
                .as_ref()
                .expect("session");
            assert_eq!(session.path, provided.path);
            assert_eq!(session.conflict_kind, provided.conflict_kind);
            assert_eq!(session.strategy, provided.strategy);
            assert_eq!(session.ours.as_text(), provided.ours.as_text());
            assert_eq!(session.theirs.as_text(), provided.theirs.as_text());
            assert_eq!(repo.diagnostics.len(), 1);
        }

        conflict_file_loaded(
            &mut state,
            repo_id,
            other_path,
            Ok(Some(empty_conflict_file(&tracked_path))),
            None,
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.conflict_state.conflict_file,
            Loadable::Error(_)
        ));
        let session = repo
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session");
        assert_eq!(session.path, provided.path);
        assert_eq!(session.conflict_kind, provided.conflict_kind);
        assert_eq!(session.strategy, provided.strategy);
    }

    #[test]
    fn load_requests_set_loading_and_emit_effects() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let conflict_path = PathBuf::from("conflict.txt");
        let history_path = PathBuf::from("src/lib.rs");
        let blame_path = PathBuf::from("src/main.rs");
        mark_repo_open_ready(&mut state, repo_id);

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&conflict_path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                conflict_path.clone(),
                FileConflictKind::BothAdded,
                ConflictPayload::Absent,
                ConflictPayload::Text("ours".to_string().into()),
                ConflictPayload::Text("theirs".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }

        let effects = load_conflict_file(
            &mut state,
            repo_id,
            conflict_path.clone(),
            ConflictFileLoadMode::CurrentOnly,
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadConflictFile {
                repo_id: rid,
                ref path,
                mode: ConflictFileLoadMode::CurrentOnly
            } if rid == repo_id && path == &conflict_path
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(
                repo.conflict_state.conflict_file_path.as_ref(),
                Some(&conflict_path)
            );
            assert!(repo.conflict_state.conflict_file.is_loading());
            assert!(repo.conflict_state.conflict_session.is_none());
            assert!(!repo.conflict_state.conflict_hide_resolved);
        }

        let effects = load_file_history(&mut state, repo_id, history_path.clone(), 25);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileHistory {
                repo_id: rid,
                ref path,
                limit
            } if rid == repo_id && path == &history_path && limit == 25
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(
                repo.history_state.file_history_path.as_ref(),
                Some(&history_path)
            );
            assert!(repo.history_state.file_history.is_loading());
        }

        let effects = load_blame(
            &mut state,
            repo_id,
            blame_path.clone(),
            gitcomet_core::domain::BlameSource::Revision(Some("HEAD".to_string())),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBlame {
                repo_id: rid,
                ref path,
                source: gitcomet_core::domain::BlameSource::Revision(Some(ref rev))
            } if rid == repo_id && path == &blame_path && rev == "HEAD"
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.history_state.blame_path.as_ref(), Some(&blame_path));
            assert_eq!(
                repo.history_state.blame_source,
                Some(gitcomet_core::domain::BlameSource::Revision(Some(
                    "HEAD".to_string()
                )))
            );
            assert!(repo.history_state.blame.is_loading());
        }

        let effects = load_worktrees(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktrees { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).worktrees.is_loading());
        assert!(load_worktrees(&mut state, repo_id).is_empty());

        let effects = load_submodules(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadSubmodules { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).submodules.is_loading());

        let effects = load_tags(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadTags { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).tags.is_loading());
        assert!(load_tags(&mut state, repo_id).is_empty());

        let effects = load_stashes(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).stashes.is_loading());

        assert!(load_stashes(&mut state, repo_id).is_empty());

        let effects = refresh_branches(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(refresh_branches(&mut state, repo_id).is_empty());

        let effects = load_reflog(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadReflog {
                repo_id: rid,
                limit: 200
            } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).reflog.is_loading());
        assert!(load_reflog(&mut state, repo_id).is_empty());

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileBrowser {
                repo_id: rid,
                ref source
            } if rid == repo_id && matches!(source, FileSource::WorkingDirectory)
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Loading));
            assert_eq!(repo.file_browser.source, FileSource::WorkingDirectory);
        }
    }

    #[test]
    fn pre_open_worktree_and_submodule_loads_are_noops() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(load_worktrees(&mut state, repo_id).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::NotLoaded
        ));
        assert!(
            !repo_mut(&mut state, repo_id)
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::WORKTREES)
        );

        assert!(load_submodules(&mut state, repo_id).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::NotLoaded
        ));
    }

    #[test]
    fn ensure_sidebar_data_stores_request_before_repo_is_open() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let request = SidebarDataRequest {
            worktrees: true,
            submodules: true,
            stashes: true,
        };

        assert!(ensure_sidebar_data(&mut state, repo_id, request).is_empty());

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.sidebar_data_request, request);
        assert!(matches!(repo.worktrees, Loadable::NotLoaded));
        assert!(matches!(repo.submodules, Loadable::NotLoaded));
        assert!(matches!(repo.stashes, Loadable::NotLoaded));
    }

    #[test]
    fn ensure_sidebar_data_loads_only_missing_requested_sections() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);
        repo_mut(&mut state, repo_id).set_submodules(Loadable::Ready(Vec::new()));

        let request = SidebarDataRequest {
            worktrees: true,
            submodules: false,
            stashes: true,
        };
        let effects = ensure_sidebar_data(&mut state, repo_id, request);

        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: rid } if *rid == repo_id)
        ));
        assert!(!effects.iter().any(
            |effect| matches!(effect, Effect::LoadSubmodules { repo_id: rid } if *rid == repo_id)
        ));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if *rid == repo_id
        )));

        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.worktrees.is_loading());
        assert!(matches!(repo.submodules, Loadable::Ready(_)));
        assert!(repo.stashes.is_loading());

        assert!(ensure_sidebar_data(&mut state, repo_id, request).is_empty());
    }

    #[test]
    fn select_and_clear_commit_selection_cover_all_branches() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let commit_a = CommitId("a".into());
        let commit_b = CommitId("b".into());

        repo_mut(&mut state, repo_id).set_commit_details(Loadable::Error("old".to_string()));
        let effects = select_commit(&mut state, repo_id, commit_a.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadCommitDetails {
                repo_id: rid,
                ref commit_id
            } if rid == repo_id && commit_id == &commit_a
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.history_state.selected_commit.as_ref(), Some(&commit_a));
            assert!(matches!(
                repo.history_state.commit_details,
                Loadable::NotLoaded
            ));
        }

        assert!(select_commit(&mut state, repo_id, commit_a.clone()).is_empty());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_selected_commit(Some(commit_b.clone()));
            repo.set_commit_details(Loadable::Ready(Arc::new(commit_details_for(
                commit_a.clone(),
            ))));
        }
        assert!(select_commit(&mut state, repo_id, commit_a.clone()).is_empty());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_selected_commit(Some(commit_a.clone()));
            repo.set_commit_details(Loadable::Loading);
        }
        let effects = select_commit(&mut state, repo_id, commit_b.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadCommitDetails {
                repo_id: rid,
                ref commit_id
            } if rid == repo_id && commit_id == &commit_b
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::Loading
        ));

        assert!(clear_commit_selection(&mut state, repo_id).is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.selected_commit.is_none());
        assert!(matches!(
            repo.history_state.commit_details,
            Loadable::NotLoaded
        ));
    }

    fn multi_selection(
        state: &mut AppState,
        repo_id: RepoId,
    ) -> crate::model::CommitMultiSelection {
        repo_mut(state, repo_id)
            .history_state
            .multi_selection
            .clone()
    }

    #[test]
    fn toggle_click_adds_and_removes_commits() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone(), b.clone()]);
        assert_eq!(sel.anchor.as_ref(), Some(&b));
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(b.clone())
        );

        // Toggling a selected commit removes it; focus falls back to the last
        // remaining commit.
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(a.clone())
        );

        // Toggling the last commit away clears the whole selection.
        select_commit_multi(
            &mut state,
            repo_id,
            a,
            CommitSelectMode::Toggle,
            Some(0),
            None,
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.selected_commit.is_none());
        assert!(repo.history_state.multi_selection.commits.is_empty());
    }

    #[test]
    fn preserve_if_selected_moves_focus_without_collapsing() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());
        let c = CommitId("c".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            Some(1),
            None,
        );
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(b.clone())
        );

        // Right-click a commit already in the selection: the set is preserved,
        // only the focus moves.
        select_commit_multi(
            &mut state,
            repo_id,
            a.clone(),
            CommitSelectMode::PreserveIfSelected,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone(), b.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(a.clone())
        );

        // Right-click a commit outside the selection: collapse to it.
        select_commit_multi(
            &mut state,
            repo_id,
            c.clone(),
            CommitSelectMode::PreserveIfSelected,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![c.clone()]);
        assert_eq!(
            repo_mut(&mut state, repo_id).history_state.selected_commit,
            Some(c)
        );
    }

    #[test]
    fn squash_preview_accepted_by_pending_request_even_when_plan_invalid() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let oldest = CommitId("old".into());
        let head = CommitId("head".into());
        // A request is in flight but the plan is transiently invalid (no Ready
        // log here). The returning result must still be accepted rather than
        // stranding the preview on Loading forever.
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.squash_preview_pending = Some((oldest.clone(), head.clone()));
            repo.set_squash_preview(Loadable::Loading);
        }
        let effects = squash_message_preview_loaded(
            &mut state,
            repo_id,
            oldest.clone(),
            head.clone(),
            Ok("Subject line\n\nBody text".to_string()),
        );
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        match &repo.history_state.squash_preview {
            Loadable::Ready(preview) => {
                assert_eq!(preview.subject, "Subject line");
                assert_eq!(preview.body, "Body text");
                assert_eq!(preview.oldest, oldest);
                assert_eq!(preview.head, head);
            }
            other => panic!("expected Ready preview, got {other:?}"),
        }
        assert!(repo.history_state.squash_preview_pending.is_none());
    }

    #[test]
    fn squash_preview_dropped_when_request_range_differs() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.squash_preview_pending =
                Some((CommitId("new_old".into()), CommitId("new_head".into())));
            repo.set_squash_preview(Loadable::Loading);
        }
        // A stale result for a range we are no longer waiting on is ignored.
        squash_message_preview_loaded(
            &mut state,
            repo_id,
            CommitId("old".into()),
            CommitId("head".into()),
            Ok("stale".to_string()),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.squash_preview,
            Loadable::Loading
        ));
        assert!(repo.history_state.squash_preview_pending.is_some());
    }

    #[test]
    fn shift_click_selects_range_from_anchor_in_both_directions() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let ids: Vec<CommitId> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| CommitId((*s).into()))
            .collect();

        select_commit(&mut state, repo_id, ids[1].clone());
        select_commit_multi(
            &mut state,
            repo_id,
            ids[3].clone(),
            CommitSelectMode::Range,
            Some(3),
            Some(ids.clone()),
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[1..=3].to_vec());
        assert_eq!(sel.anchor.as_ref(), Some(&ids[1]));

        // Extending upward from the same anchor replaces the range.
        select_commit_multi(
            &mut state,
            repo_id,
            ids[0].clone(),
            CommitSelectMode::Range,
            Some(0),
            Some(ids.clone()),
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[0..=1].to_vec());
    }

    #[test]
    fn shift_click_ignores_stale_anchor_index_hint() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let ids: Vec<CommitId> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| CommitId((*s).into()))
            .collect();

        select_commit(&mut state, repo_id, ids[0].clone());
        {
            // Simulate a log reload shifting rows: the anchor hint index now
            // points elsewhere and the stored log rev no longer matches.
            let repo = repo_mut(&mut state, repo_id);
            let mut sel = repo.history_state.multi_selection.clone();
            sel.anchor_index = Some(3);
            sel.anchor_log_rev = Some(repo.history_state.log_rev.wrapping_add(1));
            repo.set_commit_multi_selection(sel);
        }
        select_commit_multi(
            &mut state,
            repo_id,
            ids[2].clone(),
            CommitSelectMode::Range,
            Some(2),
            Some(ids.clone()),
        );
        // The anchor is re-resolved by id, so the range is a..=c, not c..=d.
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, ids[0..=2].to_vec());
    }

    #[test]
    fn plain_click_collapses_multi_selection() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a.clone());
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Toggle,
            None,
            None,
        );
        assert_eq!(multi_selection(&mut state, repo_id).commits.len(), 2);

        select_commit(&mut state, repo_id, a.clone());
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![a.clone()]);
        assert_eq!(sel.anchor.as_ref(), Some(&a));
    }

    #[test]
    fn range_click_without_entries_falls_back_to_single() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a);
        select_commit_multi(
            &mut state,
            repo_id,
            b.clone(),
            CommitSelectMode::Range,
            None,
            None,
        );
        let sel = multi_selection(&mut state, repo_id);
        assert_eq!(sel.commits, vec![b]);
    }

    #[test]
    fn clearing_selection_dissolves_multi_selection() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let a = CommitId("a".into());
        let b = CommitId("b".into());

        select_commit(&mut state, repo_id, a);
        select_commit_multi(&mut state, repo_id, b, CommitSelectMode::Toggle, None, None);
        assert_eq!(multi_selection(&mut state, repo_id).commits.len(), 2);

        clear_commit_selection(&mut state, repo_id);
        let repo = repo_mut(&mut state, repo_id);
        assert!(repo.history_state.multi_selection.commits.is_empty());
        assert!(repo.history_state.multi_selection.anchor.is_none());
    }

    #[test]
    fn loaded_handlers_reschedule_when_pending() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::BRANCHES);
        let effects = branches_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).branches,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTES);
        let effects = remotes_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemotes { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remotes,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTE_BRANCHES);
        let effects = remote_branches_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemoteBranches { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_branches,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREES);
        let effects = worktrees_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktrees { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::HEAD_BRANCH);
        let effects = head_branch_loaded(&mut state, repo_id, Ok("main".to_string()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadHeadBranch { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).head_branch,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::UPSTREAM_DIVERGENCE);
        let effects = upstream_divergence_loaded(
            &mut state,
            repo_id,
            Ok(Some(UpstreamDivergence {
                ahead: 1,
                behind: 2,
            })),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadUpstreamDivergence { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).upstream_divergence,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::STASHES);
        let effects = stashes_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStashes {
                repo_id: rid,
                limit: 50
            } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).stashes,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REFLOG);
        let effects = reflog_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadReflog {
                repo_id: rid,
                limit: 200
            } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).reflog,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::TAGS);
        let effects = tags_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadTags { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Ready(_)
        ));

        mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTE_TAGS);
        let effects = remote_tags_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadRemoteTags { repo_id: rid } if rid == repo_id
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Ready(_)
        ));
    }

    #[test]
    fn status_lanes_replay_pending_refresh_even_when_payload_unchanged() {
        // A refresh coalesced while a status load was in flight must still be replayed when the
        // load completes with an unchanged payload: the in-flight read may have observed the
        // working tree/index just before an external change landed, so the coalesced refresh is
        // the only chance to pick it up. Dropping it (as a previous revision did) left stale
        // entries in the uncommitted view.
        let repo_id = RepoId(1);

        // Combined status load: an unchanged payload still replays the coalesced refresh and
        // re-arms the lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = status_loaded(&mut state, repo_id, Ok(RepoStatus::default()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));
        assert!(
            repo_mut(&mut state, repo_id)
                .loads_in_flight
                .is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS),
            "the replayed load should re-arm the lane"
        );

        // Worktree-only lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_worktree_status(Loadable::Ready(Vec::new()));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = worktree_status_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));

        // Staged-only lane.
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_staged_status(Loadable::Ready(Vec::new()));
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::STAGED_STATUS);
        let effects = staged_status_loaded(&mut state, repo_id, Ok(Vec::new()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStagedStatus { repo_id: rid } if rid == repo_id
        ));
    }

    #[test]
    fn head_branch_loaded_clears_detached_head_commit_when_attached() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_detached_head_commit(Some(CommitId("c1".into())));

        let _ = head_branch_loaded(&mut state, repo_id, Ok("main".to_string()));

        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "main"));
        assert!(repo.detached_head_commit.is_none());
    }

    #[test]
    fn head_branch_loaded_backfills_detached_head_commit_from_log() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id: CommitId("c1".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "s".into(),
                author: "a".into(),
                time: std::time::SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })));

        let _ = head_branch_loaded(&mut state, repo_id, Ok("HEAD".to_string()));

        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "HEAD"));
        assert_eq!(repo.detached_head_commit, Some(CommitId("c1".into())));
    }

    #[test]
    fn head_branch_loaded_does_not_backfill_detached_head_commit_from_filtered_logs() {
        for (scope, page) in [
            (
                LogScope::NoMerges,
                LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: CommitId("visible-non-merge".into()),
                        parent_ids: smallvec::smallvec![CommitId("hidden-head".into())],
                        summary: "visible".into(),
                        author: "a".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                },
            ),
            (
                LogScope::MergesOnly,
                LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: CommitId("visible-merge".into()),
                        parent_ids: smallvec::smallvec![
                            CommitId("p0".into()),
                            CommitId("p1".into())
                        ],
                        summary: "merge".into(),
                        author: "a".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                },
            ),
        ] {
            let repo_id = RepoId(1);
            let mut state = new_state_with_repo(repo_id);
            repo_mut(&mut state, repo_id).history_state.history_scope = scope;
            repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(page)));

            let _ = head_branch_loaded(&mut state, repo_id, Ok("HEAD".to_string()));

            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.head_branch, Loadable::Ready(ref v) if v == "HEAD"));
            assert!(
                repo.detached_head_commit.is_none(),
                "{scope:?} should not infer detached HEAD from filtered log contents"
            );
        }
    }

    #[test]
    fn loaded_handler_error_paths_record_diagnostics() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(branches_loaded(&mut state, repo_id, Err(backend_error("branches"))).is_empty());
        assert!(remotes_loaded(&mut state, repo_id, Err(backend_error("remotes"))).is_empty());
        assert!(
            remote_branches_loaded(&mut state, repo_id, Err(backend_error("remote branches")))
                .is_empty()
        );
        assert!(head_branch_loaded(&mut state, repo_id, Err(backend_error("head"))).is_empty());
        assert!(
            upstream_divergence_loaded(&mut state, repo_id, Err(backend_error("upstream")))
                .is_empty()
        );
        assert!(stashes_loaded(&mut state, repo_id, Err(backend_error("stashes"))).is_empty());
        assert!(reflog_loaded(&mut state, repo_id, Err(backend_error("reflog"))).is_empty());
        assert!(worktrees_loaded(&mut state, repo_id, Err(backend_error("worktrees"))).is_empty());
        assert!(
            submodules_loaded(&mut state, repo_id, Err(backend_error("submodules"))).is_empty()
        );
        assert!(
            file_browser_loaded(
                &mut state,
                repo_id,
                FileSource::WorkingDirectory,
                Err(backend_error("file_browser")),
            )
            .is_empty()
        );

        assert!(matches!(
            repo_mut(&mut state, repo_id).branches,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remotes,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_branches,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).head_branch,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).upstream_divergence,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).stashes,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).reflog,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).worktrees,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::Error(_)
        ));
        assert!(matches!(
            repo_mut(&mut state, repo_id).file_browser.entries,
            Loadable::Error(_)
        ));

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.diagnostics.len(), 10);
    }

    #[test]
    fn status_loaded_clears_resolved_conflicts_and_preserves_unresolved_ones() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("conflict.txt");

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.set_status(Loadable::Ready(Arc::new(conflicted_status(
                &path,
                FileConflictKind::BothModified,
            ))));
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                path.clone(),
                FileConflictKind::BothModified,
                ConflictPayload::Text("base\n".to_string().into()),
                ConflictPayload::Text("ours\n".to_string().into()),
                ConflictPayload::Text("theirs\n".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::WORKTREE_STATUS);
        let effects = status_loaded(&mut state, repo_id, Ok(RepoStatus::default()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktreeStatus { repo_id: rid } if rid == repo_id
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.status, Loadable::Ready(_)));
            assert!(repo.conflict_state.conflict_file_path.is_none());
            assert!(matches!(
                repo.conflict_state.conflict_file,
                Loadable::NotLoaded
            ));
            assert!(repo.conflict_state.conflict_session.is_none());
            assert!(!repo.conflict_state.conflict_hide_resolved);
        }

        {
            let repo = repo_mut(&mut state, repo_id);
            let unresolved = conflicted_status(&path, FileConflictKind::BothModified);
            repo.set_status(Loadable::Ready(Arc::new(unresolved.clone())));
            repo.set_conflict_file_path(Some(path.clone()));
            repo.set_conflict_file(Loadable::Ready(Some(empty_conflict_file(&path))));
            repo.set_conflict_session(Some(ConflictSession::new(
                path.clone(),
                FileConflictKind::BothModified,
                ConflictPayload::Text("base\n".to_string().into()),
                ConflictPayload::Text("ours\n".to_string().into()),
                ConflictPayload::Text("theirs\n".to_string().into()),
            )));
            repo.set_conflict_hide_resolved(true);
        }
        let unresolved = conflicted_status(&path, FileConflictKind::BothModified);
        assert!(status_loaded(&mut state, repo_id, Ok(unresolved)).is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.conflict_state.conflict_file_path.as_ref(), Some(&path));
            assert!(repo.conflict_state.conflict_session.is_some());
            assert!(repo.conflict_state.conflict_hide_resolved);
        }

        assert!(status_loaded(&mut state, repo_id, Err(backend_error("status"))).is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.status, Loadable::Error(_)));
        assert!(!repo.diagnostics.is_empty());
    }

    #[test]
    fn tags_and_remote_tags_handle_unsupported_as_empty_ready() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        assert!(tags_loaded(&mut state, repo_id, Err(unsupported_error())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Ready(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);

        assert!(remote_tags_loaded(&mut state, repo_id, Err(unsupported_error())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Ready(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);

        assert!(tags_loaded(&mut state, repo_id, Err(backend_error("tags"))).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::Error(_)
        ));

        assert!(
            remote_tags_loaded(&mut state, repo_id, Err(backend_error("remote tags"))).is_empty()
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::Error(_)
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 2);
    }

    #[test]
    fn cancelled_metadata_results_reset_to_not_loaded_without_diagnostics() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let cancelled = || Error::new(ErrorKind::Cancelled);

        assert!(tags_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).tags,
            Loadable::NotLoaded
        ));

        assert!(remote_tags_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).remote_tags,
            Loadable::NotLoaded
        ));

        assert!(submodules_loaded(&mut state, repo_id, Err(cancelled())).is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).submodules,
            Loadable::NotLoaded
        ));
        assert_eq!(repo_mut(&mut state, repo_id).diagnostics.len(), 0);
    }

    #[test]
    fn commit_details_loaded_requires_selected_commit_match() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let selected = CommitId("selected".into());
        let other = CommitId("other".into());

        repo_mut(&mut state, repo_id).set_selected_commit(Some(selected.clone()));
        commit_details_loaded(
            &mut state,
            repo_id,
            other.clone(),
            Ok(commit_details_for(other.clone())),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::NotLoaded
        ));

        commit_details_loaded(
            &mut state,
            repo_id,
            selected.clone(),
            Ok(commit_details_for(selected.clone())),
        );
        assert!(matches!(
            repo_mut(&mut state, repo_id).history_state.commit_details,
            Loadable::Ready(_)
        ));

        commit_details_loaded(&mut state, repo_id, selected, Err(backend_error("details")));
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(
            repo.history_state.commit_details,
            Loadable::Error(_)
        ));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn file_browser_loaded_updates_state_and_records_errors() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::WorkingDirectory;

        let entries = vec![FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
        }];
        let source = FileSource::WorkingDirectory;

        let effects = file_browser_loaded(&mut state, repo_id, source.clone(), Ok(entries));
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Ready(_)));
            if let Loadable::Ready(arc) = &repo.file_browser.entries {
                assert_eq!(arc.len(), 1);
                assert_eq!(arc[0].name, "src");
            }
        }

        file_browser_loaded(
            &mut state,
            repo_id,
            source,
            Err(backend_error("tree failed")),
        );
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }

    #[test]
    fn file_browser_loaded_discards_stale_results() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::Branch("main".to_string());

        let entries = vec![FileEntry {
            name: "stale.txt".to_string(),
            path: Arc::new(PathBuf::from("stale.txt")),
            kind: FileEntryKind::File,
            depth: 0,
        }];
        let wrong_source = FileSource::WorkingDirectory;

        let effects = file_browser_loaded(&mut state, repo_id, wrong_source, Ok(entries));
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
        assert_eq!(
            repo.file_browser.source,
            FileSource::Branch("main".to_string())
        );
    }

    #[test]
    fn toggle_file_browser_dir_expands_and_collapses() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let dir = PathBuf::from("src/sub");

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = toggle_file_browser_dir(&mut state, repo_id, dir.clone());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(
                repo.file_browser
                    .expanded_dirs
                    .contains(&Arc::new(dir.clone()))
            );
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }

        let rev_after_expand = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;
        let effects = toggle_file_browser_dir(&mut state, repo_id, dir.clone());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(!repo.file_browser.expanded_dirs.contains(&Arc::new(dir)));
            assert!(repo.file_browser.file_browser_rev > rev_after_expand);
        }
    }

    #[test]
    fn set_file_browser_search_updates_query_and_rev() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = set_file_browser_search(&mut state, repo_id, "test".to_string());
        assert!(effects.is_empty());
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.file_browser.search_query, "test");
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }

        let rev_after_first = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;
        let effects = set_file_browser_search(&mut state, repo_id, "test".to_string());
        assert!(effects.is_empty());
        assert_eq!(
            repo_mut(&mut state, repo_id).file_browser.file_browser_rev,
            rev_after_first
        );

        let effects = set_file_browser_search(&mut state, repo_id, "".to_string());
        assert!(effects.is_empty());
        assert_eq!(repo_mut(&mut state, repo_id).file_browser.search_query, "");
    }

    #[test]
    fn set_file_browser_source_resets_and_emits_load() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let commit_id = CommitId("abcdefgh".into());
        let source = FileSource::Commit(commit_id);

        let effects = set_file_browser_source(&mut state, repo_id, source.clone());
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadFileBrowser { .. }));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.file_browser.source, source);
            assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
            assert!(repo.file_browser.expanded_dirs.is_empty());
            assert!(repo.file_browser.search_query.is_empty());
        }

        let effects = set_file_browser_source(&mut state, repo_id, source);
        assert!(effects.is_empty());
    }

    #[test]
    fn set_sidebar_mode_triggers_file_browser_load_and_retries_on_error() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert_eq!(state.sidebar_mode, SidebarMode::Files);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );

        repo_mut(&mut state, repo_id).file_browser.entries = Loadable::Ready(Arc::new(Vec::new()));
        set_sidebar_mode(&mut state, SidebarMode::Branches);
        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );

        repo_mut(&mut state, repo_id).file_browser.entries = Loadable::Error("fail".to_string());
        set_sidebar_mode(&mut state, SidebarMode::Branches);
        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
    }

    #[test]
    fn load_file_browser_sets_loading_and_emits_effect() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let initial_rev = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadFileBrowser {
                repo_id: rid,
                ..
            } if rid == repo_id
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert!(matches!(repo.file_browser.entries, Loadable::Loading));
            assert_eq!(repo.file_browser.source, FileSource::WorkingDirectory);
            assert!(repo.file_browser.file_browser_rev > initial_rev);
        }
    }

    #[test]
    fn load_file_browser_noop_when_repo_not_open() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        // open is Loading (set by new_opening), not Ready

        let effects = load_file_browser(&mut state, repo_id, FileSource::WorkingDirectory);
        assert!(effects.is_empty());
        assert!(matches!(
            repo_mut(&mut state, repo_id).file_browser.entries,
            Loadable::NotLoaded
        ));
    }

    #[test]
    fn browse_open_content_path_returns_correct_paths() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);

        // content_preview is false → None
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // Set content_preview = true with Commit target
        let commit_id = CommitId("abc123".into());
        let path = PathBuf::from("src/main.rs");
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(path.clone()),
            });
        }
        assert_eq!(
            browse_open_content_path(&state, repo_id),
            Some(path.clone())
        );

        // WorkingTree target
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
                path: path.clone(),
                area: DiffArea::Unstaged,
            });
        }
        assert_eq!(
            browse_open_content_path(&state, repo_id),
            Some(path.clone())
        );

        // Commit with path: None → None
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id,
                path: None,
            });
        }
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // diff_target is None → None
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.diff_target = None;
        }
        assert!(browse_open_content_path(&state, repo_id).is_none());

        // Unknown repo → None
        assert!(browse_open_content_path(&state, RepoId(999)).is_none());
    }

    #[test]
    fn browse_repository_at_commit_reopens_active_file() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("src/lib.rs");
        let commit_a = CommitId("aaaaaaaa".into());
        let commit_b = CommitId("bbbbbbbb".into());

        // Set up a content-preview file open at commit_a
        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_a.clone(),
                path: Some(file_path.clone()),
            });
        }

        // Browse commit_b — should reopen file at commit_b
        let effects = browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id: rid,
                ..
            } if *rid == repo_id
        )));
    }

    #[test]
    fn reset_browse_to_live_reopens_active_file() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("README.md");
        let commit_id = CommitId("abcd1234".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_path.clone()),
            });
            repo.file_browser.source = FileSource::Commit(commit_id);
        }

        let effects = reset_browse_to_live(&mut state, repo_id);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id: rid,
                ..
            } if *rid == repo_id
        )));
    }

    #[test]
    fn browse_repository_at_commit_no_reopen_when_content_preview_is_false() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let commit_a = CommitId("aaaaaaaa".into());
        let commit_b = CommitId("bbbbbbbb".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.diff_state.content_preview = false;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_a,
                path: Some(PathBuf::from("src/lib.rs")),
            });
        }

        let effects = browse_repository_at_commit(&mut state, repo_id, commit_b);
        // Should not contain LoadSelectedDiff (no file reopen)
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
        );
    }

    #[test]
    fn browse_history_evicts_oldest_when_exceeding_cap() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        const CAP: usize = 32;
        for i in 0..CAP + 3 {
            browse_repository_at_commit(
                &mut state,
                repo_id,
                CommitId(format!("commit{i:08}").into()),
            );
        }

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.browse_history.len(), CAP);
        assert_eq!(
            repo.browse_history[0].0.as_ref(),
            "commit00000003".to_string()
        );
        assert_eq!(
            repo.browse_history[CAP - 1].0.as_ref(),
            format!("commit{:08}", CAP + 2)
        );
    }

    #[test]
    fn browse_history_rebrowse_does_not_move_to_mru() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let a = CommitId("aaaaaaaa".into());
        let b = CommitId("bbbbbbbb".into());
        let c = CommitId("cccccccc".into());

        browse_repository_at_commit(&mut state, repo_id, a.clone());
        browse_repository_at_commit(&mut state, repo_id, b.clone());
        browse_repository_at_commit(&mut state, repo_id, c.clone());
        // Re-browse a — should NOT move to end
        browse_repository_at_commit(&mut state, repo_id, a.clone());

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.browse_history.len(), 3);
        // a stays at position 0, not moved to end
        assert_eq!(repo.browse_history[0], a);
        assert_eq!(repo.browse_history[1], b);
        assert_eq!(repo.browse_history[2], c);
    }

    #[test]
    fn set_sidebar_mode_noop_without_active_repo() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);
        state.active_repo = None;

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        assert!(effects.is_empty());
        assert_eq!(state.sidebar_mode, SidebarMode::Files);
    }

    #[test]
    fn set_sidebar_mode_emits_load_even_when_repo_not_ready() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        // repo.open is Loading (set by new_opening), not Ready

        let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
        // set_sidebar_mode does NOT check repo.open — it emits LoadFileBrowser,
        // but load_file_browser will be a no-op when open isn't Ready.
        // The effect IS emitted (the no-op is downstream in the effect handler).
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
    }

    #[test]
    fn browse_repository_at_commit_same_commit_with_file_open_does_not_reopen() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        state.active_repo = Some(repo_id);
        mark_repo_open_ready(&mut state, repo_id);

        let file_path = PathBuf::from("src/main.rs");
        let commit_id = CommitId("deadbeef".into());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.file_browser.source = FileSource::Commit(commit_id.clone());
            repo.diff_state.content_preview = true;
            repo.diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_path),
            });
        }

        // Browse the SAME commit — source unchanged, no LoadFileBrowser emitted
        let effects = browse_repository_at_commit(&mut state, repo_id, commit_id);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
        );
    }

    #[test]
    fn file_browser_loaded_cancelled_error_records_diagnostic() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        repo_mut(&mut state, repo_id).file_browser.source = FileSource::WorkingDirectory;

        let cancelled = Error::new(ErrorKind::Cancelled);
        let effects = file_browser_loaded(
            &mut state,
            repo_id,
            FileSource::WorkingDirectory,
            Err(cancelled),
        );
        assert!(effects.is_empty());
        let repo = repo_mut(&mut state, repo_id);
        assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
        assert_eq!(repo.diagnostics.len(), 1);
    }
}
