use super::util::push_diagnostic;
use crate::model::{
    AppState, ConflictFileLoadMode, DiagnosticKind, Loadable, RepoId, RepoLoadsInFlight,
};
use crate::msg::Effect;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitId, FileStatusKind, LogPage, ReflogEntry, Remote, RemoteBranch,
    RemoteTag, RepoStatus, StashEntry, Submodule, Tag, UpstreamDivergence, Worktree,
};
use gitcomet_core::error::Error;
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
    rev: Option<String>,
    result: std::result::Result<Vec<gitcomet_core::services::BlameLine>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.blame_path.as_ref() == Some(&path)
        && repo_state.history_state.blame_rev == rev
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
    let conflict_kind = match &repo_state.status {
        Loadable::Ready(status) => status
            .unstaged
            .iter()
            .find(|e| e.path == file.path && e.kind == FileStatusKind::Conflicted)
            .and_then(|e| e.conflict),
        _ => None,
    }?;

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
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let worktrees = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_worktrees(worktrees);
    }
    Vec::new()
}

pub(super) fn submodules_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<Vec<Submodule>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let submodules = match result {
            Ok(v) => Loadable::Ready(v),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_submodules(submodules);
    }
    Vec::new()
}

pub(super) fn select_commit(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

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

pub(super) fn load_stashes(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
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
    rev: Option<String>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.history_state.blame_path = Some(path.clone());
    repo_state.history_state.blame_rev = rev.clone();
    repo_state.history_state.blame = Loadable::Loading;
    vec![Effect::LoadBlame { repo_id, path, rev }]
}

pub(super) fn load_worktrees(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.set_worktrees(Loadable::Loading);
    vec![Effect::LoadWorktrees { repo_id }]
}

pub(super) fn load_submodules(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.set_submodules(Loadable::Loading);
    vec![Effect::LoadSubmodules { repo_id }]
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
        let should_replay_pending = match result {
            Ok(next) => {
                let status_unchanged = matches!(
                    &repo_state.status,
                    Loadable::Ready(prev) if prev.as_ref() == &next
                );
                if !status_unchanged {
                    repo_state.set_status(Loadable::Ready(Arc::new(next)));
                }
                clear_resolved_conflict_context(repo_state);
                !status_unchanged
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_status(Loadable::Error(e.to_string()));
                true
            }
        };
        // Replaying an unchanged status payload can self-sustain refresh loops when file-system
        // events are produced by the status read itself (for example `.git/index` churn).
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::STATUS) {
            if should_replay_pending {
                effects.push(Effect::LoadStatus { repo_id });
            } else {
                // `finish` marks STATUS back as in-flight when there was pending work. We are
                // intentionally dropping that replay request, so clear the in-flight bit too.
                let _ = repo_state.loads_in_flight.finish(RepoLoadsInFlight::STATUS);
            }
        }
    }
    effects
}

/// Clear conflict-file/session state when the tracked conflict path is no longer
/// present as an unresolved conflict in status.
fn clear_resolved_conflict_context(repo_state: &mut crate::model::RepoState) {
    let Some(conflict_path) = repo_state.conflict_state.conflict_file_path.as_ref() else {
        return;
    };
    let still_conflicted = match &repo_state.status {
        Loadable::Ready(status) => status
            .unstaged
            .iter()
            .any(|entry| entry.path == *conflict_path && entry.kind == FileStatusKind::Conflicted),
        _ => true,
    };
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

pub(super) fn commit_details_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
    result: std::result::Result<CommitDetails, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.history_state.selected_commit.as_ref() == Some(&commit_id)
    {
        let value = match result {
            Ok(v) => Loadable::Ready(Arc::new(v)),
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                Loadable::Error(e.to_string())
            }
        };
        repo_state.set_commit_details(value);
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConflictFile, RepoState};
    use gitcomet_core::domain::{FileConflictKind, FileStatus, RepoSpec};
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
        assert!(blame_loaded(&mut state, repo_id, path.clone(), None, Ok(Vec::new())).is_empty());
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
        assert!(load_blame(&mut state, repo_id, path.clone(), Some("HEAD".to_string())).is_empty());
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
    fn blame_loaded_requires_matching_path_and_rev() {
        let repo_id = RepoId(1);
        let mut state = new_state_with_repo(repo_id);
        let path = PathBuf::from("src/lib.rs");
        let rev = Some("HEAD~1".to_string());

        {
            let repo = repo_mut(&mut state, repo_id);
            repo.history_state.blame_path = Some(path.clone());
            repo.history_state.blame_rev = rev.clone();
        }

        blame_loaded(
            &mut state,
            repo_id,
            path.clone(),
            Some("different".to_string()),
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
            rev.clone(),
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
            rev,
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
            Some("HEAD".to_string()),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadBlame {
                repo_id: rid,
                ref path,
                ref rev
            } if rid == repo_id && path == &blame_path && rev.as_deref() == Some("HEAD")
        ));
        {
            let repo = repo_mut(&mut state, repo_id);
            assert_eq!(repo.history_state.blame_path.as_ref(), Some(&blame_path));
            assert_eq!(repo.history_state.blame_rev.as_deref(), Some("HEAD"));
            assert!(repo.history_state.blame.is_loading());
        }

        let effects = load_worktrees(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadWorktrees { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).worktrees.is_loading());

        let effects = load_submodules(&mut state, repo_id);
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadSubmodules { repo_id: rid } if rid == repo_id
        ));
        assert!(repo_mut(&mut state, repo_id).submodules.is_loading());

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

        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.diagnostics.len(), 9);
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
        mark_pending(&mut state, repo_id, RepoLoadsInFlight::STATUS);
        let effects = status_loaded(&mut state, repo_id, Ok(RepoStatus::default()));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            effects[0],
            Effect::LoadStatus { repo_id: rid } if rid == repo_id
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
}
