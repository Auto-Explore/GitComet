use super::util::push_diagnostic;
use crate::model::{AppState, DiagnosticKind, Loadable, RepoId, RepoLoadsInFlight};
use crate::msg::Effect;
use gitgpui_core::conflict_session::{ConflictPayload, ConflictSession};
use gitgpui_core::domain::{
    Branch, CommitDetails, CommitId, FileStatusKind, LogPage, ReflogEntry, Remote, RemoteBranch,
    RepoStatus, StashEntry, Submodule, Tag, UpstreamDivergence, Worktree,
};
use gitgpui_core::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn file_history_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    result: std::result::Result<LogPage, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.file_history_path.as_ref() == Some(&path)
    {
        repo_state.file_history = match result {
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
    result: std::result::Result<Vec<gitgpui_core::services::BlameLine>, Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.blame_path.as_ref() == Some(&path)
        && repo_state.blame_rev == rev
    {
        repo_state.blame = match result {
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
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && repo_state.conflict_file_path.as_ref() == Some(&path)
    {
        let session = match &result {
            Ok(Some(file)) => build_conflict_session(repo_state, file),
            _ => None,
        };
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

    let payload_from = |bytes: &Option<Vec<u8>>, text: &Option<String>| -> ConflictPayload {
        if let Some(t) = text {
            ConflictPayload::Text(t.clone())
        } else if let Some(b) = bytes {
            ConflictPayload::from_bytes(b.clone())
        } else {
            ConflictPayload::Absent
        }
    };

    let base = payload_from(&file.base_bytes, &file.base);
    let ours = payload_from(&file.ours_bytes, &file.ours);
    let theirs = payload_from(&file.theirs_bytes, &file.theirs);

    // If we have merged text with markers, parse regions from it.
    if let Some(current) = file.current.as_deref() {
        Some(ConflictSession::from_merged_text(
            file.path.clone(),
            conflict_kind,
            base,
            ours,
            theirs,
            current,
        ))
    } else {
        Some(ConflictSession::new(
            file.path.clone(),
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

    if repo_state.selected_commit.as_ref() == Some(&commit_id) {
        return Vec::new();
    }

    repo_state.set_selected_commit(Some(commit_id.clone()));
    let already_loaded = matches!(
        &repo_state.commit_details,
        Loadable::Ready(details) if details.id == commit_id
    );
    if already_loaded {
        return Vec::new();
    }

    if matches!(
        repo_state.commit_details,
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
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.set_conflict_file_path(Some(path.clone()));
    repo_state.set_conflict_file(Loadable::Loading);
    repo_state.set_conflict_session(None);
    vec![Effect::LoadConflictFile { repo_id, path }]
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
    repo_state.file_history_path = Some(path.clone());
    repo_state.file_history = Loadable::Loading;
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
    repo_state.blame_path = Some(path.clone());
    repo_state.blame_rev = rev.clone();
    repo_state.blame = Loadable::Loading;
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
        match result {
            Ok(next) => {
                let status_unchanged = matches!(
                    &repo_state.status,
                    Loadable::Ready(prev) if prev.as_ref() == &next
                );
                if !status_unchanged {
                    repo_state.set_status(Loadable::Ready(Arc::new(next)));
                }
            }
            Err(e) => {
                push_diagnostic(repo_state, DiagnosticKind::Error, e.to_string());
                repo_state.set_status(Loadable::Error(e.to_string()));
            }
        }
        if repo_state.loads_in_flight.finish(RepoLoadsInFlight::STATUS) {
            effects.push(Effect::LoadStatus { repo_id });
        }
    }
    effects
}

pub(super) fn head_branch_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<String, Error>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        let head_branch = match result {
            Ok(v) => Loadable::Ready(v),
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
                if matches!(e.kind(), gitgpui_core::error::ErrorKind::Unsupported(_)) {
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
        && repo_state.selected_commit.as_ref() == Some(&commit_id)
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
