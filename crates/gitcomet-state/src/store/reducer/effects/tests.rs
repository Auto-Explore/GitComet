use super::*;
use crate::model::{
    ConflictFile, PendingFileBrowserReopen, RepoState, SidebarDataRequest, SidebarMode,
};
use gitcomet_core::domain::{
    DiffArea, DiffTarget, FileConflictKind, FileEntry, FileEntryKind, FileSource, FileStatus,
    LogScope, RemoteBranch, RepoSpec,
};
use gitcomet_core::error::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// No backends: these tests exercise browse-point bookkeeping, not the HEAD
/// gitlink classification that `open_file_content` performs behind it.
fn no_repos() -> FxHashMap<RepoId, Arc<dyn GitRepository>> {
    FxHashMap::default()
}

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
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "now".to_string(),
        committed_at_unix: 0,
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
    assert_eq!(repo.navigation.browse_history, vec![a.clone(), b.clone()]);
    assert_eq!(repo.browsing_commit(), Some(&a));
    assert_eq!(state.sidebar_mode, SidebarMode::Files);

    reset_browse_to_live(&mut state, RepoId(1));
    let repo = &state.repos[0];
    assert!(repo.navigation.browse_history.is_empty());
    assert_eq!(repo.browsing_commit(), None);
    assert!(matches!(
        repo.file_browser.source,
        gitcomet_core::domain::FileSource::WorkingDirectory
    ));
}

fn conflicted_status(path: &Path, conflict: FileConflictKind) -> RepoStatus {
    RepoStatus {
        staged: std::sync::Arc::new(Vec::new()),
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: path.to_path_buf(),
            kind: FileStatusKind::Conflicted,
            conflict: Some(conflict),
        }]),
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
        file_history_loaded(
            &mut state,
            repo_id,
            path.clone(),
            None,
            Ok(Arc::new(empty_log_page()))
        )
        .is_empty()
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
    assert!(set_file_browser_source(&mut state, repo_id, FileSource::WorkingDirectory).is_empty());
    assert!(set_sidebar_mode(&mut state, SidebarMode::Files).is_empty());
    assert!(
        file_browser_loaded(
            &no_repos(),
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
        None,
        Ok(Arc::new(empty_log_page())),
    );
    assert!(matches!(
        repo_mut(&mut state, repo_id).history_state.file_history,
        Loadable::NotLoaded
    ));

    file_history_loaded(
        &mut state,
        repo_id,
        tracked.clone(),
        None,
        Ok(Arc::new(empty_log_page())),
    );
    assert!(matches!(
        repo_mut(&mut state, repo_id).history_state.file_history,
        Loadable::Ready(_)
    ));

    file_history_loaded(
        &mut state,
        repo_id,
        tracked,
        None,
        Err(backend_error("file history failed")),
    );
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(
        repo.history_state.file_history,
        Loadable::Error(_)
    ));
    assert_eq!(repo.feedback.diagnostics.len(), 1);
}

fn file_history_cursor(last_seen: &str) -> LogCursor {
    LogCursor {
        last_seen: CommitId(last_seen.into()),
        resume_from: None,
        resume_token: None,
    }
}

fn file_history_page(ids: &[&str], next_cursor: Option<LogCursor>) -> LogPage {
    LogPage {
        commits: ids.iter().map(|id| test_commit(id, None)).collect(),
        next_cursor,
    }
}

fn file_history_ids(state: &mut AppState, repo_id: RepoId) -> Vec<String> {
    let Loadable::Ready(page) = &repo_mut(state, repo_id).history_state.file_history else {
        panic!("file history should be ready");
    };
    page.commits.iter().map(|c| c.id.to_string()).collect()
}

/// The first page is bounded so the picker opens at once. When it reports
/// more, the reducer asks for the rest right away — unbounded, since the
/// backend serves it from one cached follow walk — rather than leaving the
/// list truncated at the page size.
#[test]
fn file_history_first_page_with_more_requests_the_rest() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let tracked = PathBuf::from("tracked.txt");
    let effects = load_file_history(&mut state, repo_id, tracked.clone(), 2);
    assert_eq!(effects.len(), 1);

    let cursor = file_history_cursor("b");
    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked.clone(),
        None,
        Ok(Arc::new(file_history_page(
            &["a", "b"],
            Some(cursor.clone()),
        ))),
    );
    assert!(matches!(
        &effects[..],
        [Effect::LoadFileHistory {
            repo_id: rid,
            path,
            limit: usize::MAX,
            cursor: Some(c),
        }] if *rid == repo_id && path == &tracked && *c == cursor
    ));
    assert_eq!(file_history_ids(&mut state, repo_id), ["a", "b"]);

    // A complete first page has nothing to chain.
    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked,
        None,
        Ok(Arc::new(file_history_page(&["a", "b"], None))),
    );
    assert!(effects.is_empty());
}

#[test]
fn file_history_remainder_extends_the_page_that_requested_it() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let tracked = PathBuf::from("tracked.txt");
    let cursor = file_history_cursor("b");
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.file_history_path = Some(tracked.clone());
        repo.history_state.file_history = Loadable::Ready(Arc::new(file_history_page(
            &["a", "b"],
            Some(cursor.clone()),
        )));
    }

    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked,
        Some(cursor),
        Ok(Arc::new(file_history_page(&["c", "d"], None))),
    );
    assert!(effects.is_empty());
    assert_eq!(file_history_ids(&mut state, repo_id), ["a", "b", "c", "d"]);
    let Loadable::Ready(page) = &repo_mut(&mut state, repo_id).history_state.file_history else {
        panic!("file history should be ready");
    };
    assert!(page.next_cursor.is_none());
}

/// Every open of the popover reloads the first page. A remainder answering an
/// earlier open — or one that arrives while the first page is still loading —
/// belongs to a page that is gone, and appending it would duplicate rows.
#[test]
fn file_history_remainder_for_another_page_is_dropped() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let tracked = PathBuf::from("tracked.txt");
    let stale = file_history_cursor("old");
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.file_history_path = Some(tracked.clone());
        repo.history_state.file_history = Loadable::Loading;
    }

    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked.clone(),
        Some(stale.clone()),
        Ok(Arc::new(file_history_page(&["x"], None))),
    );
    assert!(effects.is_empty());
    assert!(
        repo_mut(&mut state, repo_id)
            .history_state
            .file_history
            .is_loading()
    );

    let current = file_history_cursor("b");
    repo_mut(&mut state, repo_id).history_state.file_history = Loadable::Ready(Arc::new(
        file_history_page(&["a", "b"], Some(current.clone())),
    ));
    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked,
        Some(stale),
        Ok(Arc::new(file_history_page(&["x"], None))),
    );
    assert!(effects.is_empty());
    assert_eq!(file_history_ids(&mut state, repo_id), ["a", "b"]);
    let Loadable::Ready(page) = &repo_mut(&mut state, repo_id).history_state.file_history else {
        panic!("file history should be ready");
    };
    assert_eq!(page.next_cursor.as_ref(), Some(&current));
}

/// A failed remainder must not throw away the rows already on screen; it
/// reports the error and stops promising older commits.
#[test]
fn file_history_remainder_error_keeps_the_first_page() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let tracked = PathBuf::from("tracked.txt");
    let cursor = file_history_cursor("b");
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.file_history_path = Some(tracked.clone());
        repo.history_state.file_history = Loadable::Ready(Arc::new(file_history_page(
            &["a", "b"],
            Some(cursor.clone()),
        )));
    }

    let effects = file_history_loaded(
        &mut state,
        repo_id,
        tracked,
        Some(cursor),
        Err(backend_error("follow failed")),
    );
    assert!(effects.is_empty());
    assert_eq!(file_history_ids(&mut state, repo_id), ["a", "b"]);
    let repo = repo_mut(&mut state, repo_id);
    let Loadable::Ready(page) = &repo.history_state.file_history else {
        panic!("file history should be ready");
    };
    assert!(page.next_cursor.is_none());
    assert_eq!(repo.feedback.diagnostics.len(), 1);
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
    assert_eq!(repo.feedback.diagnostics.len(), 1);
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
        assert_eq!(repo.feedback.diagnostics.len(), 1);
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
            limit,
            cursor: None,
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

    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: rid } if *rid == repo_id)
        )
    );
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

fn multi_selection(state: &mut AppState, repo_id: RepoId) -> crate::model::CommitMultiSelection {
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

fn test_commit(id: &str, parent: Option<&str>) -> gitcomet_core::domain::Commit {
    gitcomet_core::domain::Commit {
        id: CommitId(id.into()),
        parent_ids: parent
            .map(|p| smallvec::smallvec![CommitId(p.into())])
            .unwrap_or_default(),
        summary: "s".into(),
        author: "a".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

#[test]
fn multi_selection_compares_merged_diff_from_oldest_parent_to_newest() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    // Log is newest-first; each of c2..c4 has a parent, c1 is the root.
    repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![
            test_commit("c4", Some("c3")),
            test_commit("c3", Some("c2")),
            test_commit("c2", Some("c1")),
            test_commit("c1", None),
        ],
        next_cursor: None,
    })));

    // Select c4 (newest) and c2 (oldest of the pair). The merged diff spans
    // c2's parent (c1) → c4, so every selected commit's own changes show.
    select_commit(&mut state, repo_id, CommitId("c4".into()));
    let effects = select_commit_multi(
        &mut state,
        repo_id,
        CommitId("c2".into()),
        CommitSelectMode::Toggle,
        Some(2),
        None,
    );

    let range = repo_mut(&mut state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("range comparison active for a multi-selection");
    assert_eq!(range.from, CommitId("c1".into()));
    assert_eq!(range.to, Some(CommitId("c4".into())));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadRangeFiles { from, to, .. }
            if *from == CommitId("c1".into()) && *to == Some(CommitId("c4".into()))
    )));
}

#[test]
fn multi_selection_reaching_root_uses_the_empty_tree_as_base() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    repo_mut(&mut state, repo_id).set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![test_commit("c2", Some("c1")), test_commit("c1", None)],
        next_cursor: None,
    })));

    select_commit(&mut state, repo_id, CommitId("c2".into()));
    select_commit_multi(
        &mut state,
        repo_id,
        CommitId("c1".into()),
        CommitSelectMode::Toggle,
        Some(1),
        None,
    );

    let range = repo_mut(&mut state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("range comparison active");
    // The oldest selected commit is the root and has no parent to diff from.
    // Basing on the root itself would drop everything it introduces from the
    // merged diff, so the empty tree is the base instead.
    assert_eq!(range.from, CommitId(EMPTY_TREE_ID.into()));
    assert_eq!(range.from_label, "start of history");
    assert_eq!(range.to, Some(CommitId("c2".into())));
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
        Loadable::NotLoaded
    ));
    assert!(
        remote_branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty(),
        "the promoted latest load should finish without another refresh"
    );
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
                    parent_ids: smallvec::smallvec![CommitId("p0".into()), CommitId("p1".into())],
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
fn superseded_remote_branch_load_keeps_the_last_complete_snapshot() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let previous = RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("1111111".into()),
    };
    repo_mut(&mut state, repo_id).set_remote_branches(Loadable::Ready(vec![previous.clone()]));
    let previous_rev = repo_mut(&mut state, repo_id).remote_branches_rev;
    mark_pending(&mut state, repo_id, RepoLoadsInFlight::REMOTE_BRANCHES);

    let effects = remote_branches_loaded(&mut state, repo_id, Ok(Vec::new()));

    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadRemoteBranches { repo_id: id }] if *id == repo_id
    ));
    assert!(matches!(
        &repo_mut(&mut state, repo_id).remote_branches,
        Loadable::Ready(branches) if branches.as_slice() == [previous]
    ));
    assert_eq!(
        repo_mut(&mut state, repo_id).remote_branches_rev,
        previous_rev
    );

    let replacement = RemoteBranch {
        remote: "origin".to_string(),
        name: "release".to_string(),
        target: CommitId("2222222".into()),
    };
    assert!(remote_branches_loaded(&mut state, repo_id, Ok(vec![replacement.clone()])).is_empty());
    assert!(matches!(
        &repo_mut(&mut state, repo_id).remote_branches,
        Loadable::Ready(branches) if branches.as_slice() == [replacement]
    ));
    assert_eq!(
        repo_mut(&mut state, repo_id).remote_branches_rev,
        previous_rev.wrapping_add(1)
    );
}

#[test]
fn remote_branch_load_during_pull_keeps_the_last_complete_snapshot() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let previous = RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("1111111".into()),
    };
    let repo = repo_mut(&mut state, repo_id);
    repo.set_remote_branches(Loadable::Ready(vec![previous.clone()]));
    repo.pull_in_flight = 1;
    assert!(
        repo.loads_in_flight
            .request(RepoLoadsInFlight::REMOTE_BRANCHES)
    );

    assert!(remote_branches_loaded(&mut state, repo_id, Ok(Vec::new())).is_empty());
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(
        &repo.remote_branches,
        Loadable::Ready(branches) if branches.as_slice() == [previous]
    ));
    assert!(
        !repo
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::REMOTE_BRANCHES)
    );
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
        upstream_divergence_loaded(&mut state, repo_id, Err(backend_error("upstream"))).is_empty()
    );
    assert!(stashes_loaded(&mut state, repo_id, Err(backend_error("stashes"))).is_empty());
    assert!(reflog_loaded(&mut state, repo_id, Err(backend_error("reflog"))).is_empty());
    assert!(worktrees_loaded(&mut state, repo_id, Err(backend_error("worktrees"))).is_empty());
    assert!(submodules_loaded(&mut state, repo_id, Err(backend_error("submodules"))).is_empty());
    assert!(
        file_browser_loaded(
            &no_repos(),
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
    assert_eq!(repo.feedback.diagnostics.len(), 10);
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
    assert!(!repo.feedback.diagnostics.is_empty());
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
    assert_eq!(repo_mut(&mut state, repo_id).feedback.diagnostics.len(), 0);

    assert!(remote_tags_loaded(&mut state, repo_id, Err(unsupported_error())).is_empty());
    assert!(matches!(
        repo_mut(&mut state, repo_id).remote_tags,
        Loadable::Ready(_)
    ));
    assert_eq!(repo_mut(&mut state, repo_id).feedback.diagnostics.len(), 0);

    assert!(tags_loaded(&mut state, repo_id, Err(backend_error("tags"))).is_empty());
    assert!(matches!(
        repo_mut(&mut state, repo_id).tags,
        Loadable::Error(_)
    ));

    assert!(remote_tags_loaded(&mut state, repo_id, Err(backend_error("remote tags"))).is_empty());
    assert!(matches!(
        repo_mut(&mut state, repo_id).remote_tags,
        Loadable::Error(_)
    ));
    assert_eq!(repo_mut(&mut state, repo_id).feedback.diagnostics.len(), 2);
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
    assert_eq!(repo_mut(&mut state, repo_id).feedback.diagnostics.len(), 0);
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
    assert_eq!(repo.feedback.diagnostics.len(), 1);
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

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        source.clone(),
        Ok(entries),
    );
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
        &no_repos(),
        &mut state,
        repo_id,
        source,
        Err(backend_error("tree failed")),
    );
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
    assert_eq!(repo.feedback.diagnostics.len(), 1);
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

    let effects = file_browser_loaded(&no_repos(), &mut state, repo_id, wrong_source, Ok(entries));
    assert!(effects.is_empty());
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
    assert_eq!(
        repo.file_browser.source,
        FileSource::Branch("main".to_string())
    );
}

#[test]
fn reveal_file_browser_path_expands_every_ancestor_and_clears_the_search() {
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].file_browser.search_query = "main".to_string();
    let rev_before = state.repos[0].file_browser.file_browser_rev;

    reveal_file_browser_path(
        &mut state,
        repo_id,
        PathBuf::from("crates/gitcomet-ui-gpui/src/main.rs"),
    );

    let expanded = &state.repos[0].file_browser.expanded_dirs;
    for dir in [
        "crates",
        "crates/gitcomet-ui-gpui",
        "crates/gitcomet-ui-gpui/src",
    ] {
        assert!(
            expanded.contains(&Arc::new(PathBuf::from(dir))),
            "{dir} must be expanded so the file's row is visible"
        );
    }
    assert!(
        !expanded.contains(&Arc::new(PathBuf::from(
            "crates/gitcomet-ui-gpui/src/main.rs"
        ))),
        "the file itself is not a directory to expand"
    );
    assert!(
        state.repos[0].file_browser.search_query.is_empty(),
        "a filtered tree builds its rows from matches, so the search has to go"
    );
    assert_ne!(state.repos[0].file_browser.file_browser_rev, rev_before);
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

fn tree_file(path: &str) -> FileEntry {
    let path = PathBuf::from(path);
    FileEntry {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        depth: path.components().count().saturating_sub(1),
        path: Arc::new(path),
        kind: FileEntryKind::File,
    }
}

#[test]
fn set_file_browser_source_keeps_tree_shape_and_emits_load() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.file_browser.entries = Loadable::Ready(Arc::new(vec![tree_file("src/lib.rs")]));
        repo.file_browser
            .expanded_dirs
            .insert(Arc::new(PathBuf::from("src")));
        repo.file_browser.search_query = "lib".to_string();
    }
    let rev_before = repo_mut(&mut state, repo_id).file_browser.file_browser_rev;

    let commit_id = CommitId("abcdefgh".into());
    let source = FileSource::Commit(commit_id);

    let effects = set_file_browser_source(&mut state, repo_id, source.clone());
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::LoadFileBrowser { .. }));
    {
        let repo = repo_mut(&mut state, repo_id);
        assert_eq!(repo.file_browser.source, source);
        // The old rows stay up until the listing replaces them, flagged so
        // `needs_load()` still asks for the walk.
        assert!(matches!(repo.file_browser.entries, Loadable::Ready(_)));
        assert!(repo.file_browser.stale);
        assert!(repo.file_browser.needs_load());
        assert!(
            repo.file_browser
                .expanded_dirs
                .contains(&Arc::new(PathBuf::from("src")))
        );
        assert_eq!(repo.file_browser.search_query, "lib");
        assert_ne!(repo.file_browser.file_browser_rev, rev_before);
    }

    let effects = set_file_browser_source(&mut state, repo_id, source);
    assert!(effects.is_empty());
}

#[test]
fn set_file_browser_source_blanks_a_tree_that_never_loaded() {
    // Nothing worth keeping: NotLoaded stays NotLoaded and is not flagged stale.
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let effects = set_file_browser_source(
        &mut state,
        repo_id,
        FileSource::Commit(CommitId("abcdefgh".into())),
    );
    assert_eq!(effects.len(), 1);
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(repo.file_browser.entries, Loadable::NotLoaded));
    assert!(!repo.file_browser.stale);
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

    // Each phase has to deliver its reply the way the executor does, or the
    // in-flight lane coalesces the next request away.
    file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
        Ok(Vec::new()),
    );
    assert!(matches!(
        repo_mut(&mut state, repo_id).file_browser.entries,
        Loadable::Ready(_)
    ));

    set_sidebar_mode(&mut state, SidebarMode::Branches);
    let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    );

    file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Backend("fail".to_string()),
        )),
    );
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
fn browse_open_content_path_captures_previews_only() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);

    // content_preview is false → None
    assert!(browse_open_content_path(repo_mut(&mut state, repo_id)).is_none());

    let commit_id = CommitId("abc123".into());
    let path = PathBuf::from("src/main.rs");
    let repo = repo_mut(&mut state, repo_id);
    repo.diff_state.content_preview = true;
    repo.set_diff_target(Some(DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    }));
    assert_eq!(
        browse_open_content_path(repo),
        Some(PendingFileBrowserReopen {
            path: path.clone(),
            diff_target_rev: repo.diff_state.diff_target_rev,
        })
    );

    // WorkingTree target
    repo.set_diff_target(Some(DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    }));
    assert_eq!(
        browse_open_content_path(repo).map(|reopen| reopen.path),
        Some(path)
    );

    // The editor is never re-targeted.
    repo.diff_state.edit_mode = true;
    assert!(browse_open_content_path(repo).is_none());
    repo.diff_state.edit_mode = false;

    // Commit with path: None → None
    repo.set_diff_target(Some(DiffTarget::Commit {
        commit_id,
        path: None,
    }));
    assert!(browse_open_content_path(repo).is_none());

    // diff_target is None → None
    repo.set_diff_target(None);
    assert!(browse_open_content_path(repo).is_none());
}

/// A file preview open at `commit`, the way a click in the Files tab leaves it.
fn open_preview_at(state: &mut AppState, repo_id: RepoId, commit: &CommitId, path: &str) {
    let repo = repo_mut(state, repo_id);
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = false;
    repo.set_diff_target(Some(DiffTarget::Commit {
        commit_id: commit.clone(),
        path: Some(PathBuf::from(path)),
    }));
}

fn load_selected_diffs(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
        .count()
}

fn pending_reopen_path(state: &mut AppState, repo_id: RepoId) -> Option<PathBuf> {
    repo_mut(state, repo_id)
        .file_browser
        .pending_reopen
        .as_ref()
        .map(|reopen| reopen.path.clone())
}

#[test]
fn browse_repository_at_commit_reopens_active_file_once_the_listing_lands() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");

    // Browsing commit_b only remembers the file: the listing decides whether it
    // exists there, and the walk may not even have started yet (busy lane).
    let effects = browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    assert_eq!(
        pending_reopen_path(&mut state, repo_id),
        Some(PathBuf::from("src/lib.rs"))
    );

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b.clone()),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert_eq!(load_selected_diffs(&effects), 1);
    let repo = repo_mut(&mut state, repo_id);
    assert_eq!(
        repo.diff_state.diff_target,
        Some(DiffTarget::Commit {
            commit_id: commit_b,
            path: Some(PathBuf::from("src/lib.rs")),
        })
    );
    assert!(repo.diff_state.content_preview);
    assert!(repo.file_browser.pending_reopen.is_none());
}

#[test]
fn reset_browse_to_live_reopens_active_file_once_the_listing_lands() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_id = CommitId("abcd1234".into());
    repo_mut(&mut state, repo_id).file_browser.source = FileSource::Commit(commit_id.clone());
    open_preview_at(&mut state, repo_id, &commit_id, "README.md");

    let effects = reset_browse_to_live(&mut state, repo_id);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadFileBrowser { .. }))
    );
    assert_eq!(load_selected_diffs(&effects), 0);

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
        Ok(vec![tree_file("README.md")]),
    );
    assert_eq!(load_selected_diffs(&effects), 1);
    assert_eq!(
        repo_mut(&mut state, repo_id).diff_state.diff_target,
        Some(DiffTarget::WorkingTree {
            path: PathBuf::from("README.md"),
            area: DiffArea::Unstaged,
        })
    );
}

#[test]
fn browse_repository_at_commit_no_reopen_when_content_preview_is_false() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");
    repo_mut(&mut state, repo_id).diff_state.content_preview = false;

    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
    assert!(pending_reopen_path(&mut state, repo_id).is_none());

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    assert!(matches!(
        repo_mut(&mut state, repo_id).diff_state.diff_target,
        Some(DiffTarget::Commit { ref commit_id, .. }) if *commit_id == commit_a
    ));
}

#[test]
fn a_file_missing_from_the_new_listing_closes_the_preview() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("README.md")]),
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    let repo = repo_mut(&mut state, repo_id);
    assert_eq!(repo.diff_state.diff_target, None);
    assert!(!repo.diff_state.content_preview);
    assert!(repo.file_browser.pending_reopen.is_none());
}

#[test]
fn a_reopen_is_dropped_when_the_user_navigated_meanwhile() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
    // The user opened another file before the listing landed.
    open_preview_at(&mut state, repo_id, &commit_a, "src/other.rs");

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("src/lib.rs"), tree_file("src/other.rs")]),
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    assert_eq!(
        repo_mut(&mut state, repo_id).open_file_path(),
        Some(Path::new("src/other.rs"))
    );
}

#[test]
fn a_reopen_is_skipped_when_the_file_is_already_shown_at_that_point() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    // Browsing live while the preview shows the file at commit_b, then
    // browsing commit_b: the preview is already where it should be.
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_b, "src/lib.rs");
    let rev_before = repo_mut(&mut state, repo_id).diff_state.diff_target_rev;
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert!(effects.is_empty());
    assert_eq!(
        repo_mut(&mut state, repo_id).diff_state.diff_target_rev,
        rev_before
    );
}

#[test]
fn the_editor_is_never_retargeted_by_a_browse() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    {
        let repo = repo_mut(&mut state, repo_id);
        repo.diff_state.content_preview = true;
        repo.diff_state.edit_mode = true;
        repo.set_diff_target(Some(DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        }));
    }
    let commit_b = CommitId("bbbbbbbb".into());
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
    assert!(pending_reopen_path(&mut state, repo_id).is_none());

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert!(effects.is_empty());
    assert!(repo_mut(&mut state, repo_id).diff_state.edit_mode);
}

#[test]
fn a_failed_listing_drops_the_pending_reopen() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Err(backend_error("boom")),
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    let repo = repo_mut(&mut state, repo_id);
    assert!(repo.file_browser.pending_reopen.is_none());
    assert!(matches!(
        repo.diff_state.diff_target,
        Some(DiffTarget::Commit { ref commit_id, .. }) if *commit_id == commit_a
    ));
}

#[test]
fn a_pending_reopen_survives_a_reply_for_an_abandoned_source() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    mark_repo_open_ready(&mut state, repo_id);

    let commit_a = CommitId("aaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbb".into());
    let commit_c = CommitId("cccccccc".into());
    open_preview_at(&mut state, repo_id, &commit_a, "src/lib.rs");
    browse_repository_at_commit(&mut state, repo_id, commit_b.clone());
    browse_repository_at_commit(&mut state, repo_id, commit_c.clone());

    // The walk for commit_b ends after the user already moved to commit_c.
    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_b),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert_eq!(load_selected_diffs(&effects), 0);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadFileBrowser { source: FileSource::Commit(c), .. } if *c == commit_c
    )));
    assert_eq!(
        pending_reopen_path(&mut state, repo_id),
        Some(PathBuf::from("src/lib.rs"))
    );

    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::Commit(commit_c.clone()),
        Ok(vec![tree_file("src/lib.rs")]),
    );
    assert_eq!(load_selected_diffs(&effects), 1);
    assert!(matches!(
        repo_mut(&mut state, repo_id).diff_state.diff_target,
        Some(DiffTarget::Commit { ref commit_id, .. }) if *commit_id == commit_c
    ));
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
    assert_eq!(repo.navigation.browse_history.len(), CAP);
    assert_eq!(
        repo.navigation.browse_history[0].0.as_ref(),
        "commit00000003".to_string()
    );
    assert_eq!(
        repo.navigation.browse_history[CAP - 1].0.as_ref(),
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
    assert_eq!(repo.navigation.browse_history.len(), 3);
    // a stays at position 0, not moved to end
    assert_eq!(repo.navigation.browse_history[0], a);
    assert_eq!(repo.navigation.browse_history[1], b);
    assert_eq!(repo.navigation.browse_history[2], c);
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
fn set_sidebar_mode_waits_for_the_repo_to_be_ready_before_claiming_the_lane() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    state.active_repo = Some(repo_id);
    // repo.open is Loading (set by new_opening), not Ready

    let effects = set_sidebar_mode(&mut state, SidebarMode::Files);
    assert!(effects.is_empty());
    assert!(
        !repo_mut(&mut state, repo_id)
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::FILE_BROWSER)
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

fn blame_line(line: &str) -> gitcomet_core::services::BlameLine {
    gitcomet_core::services::BlameLine {
        commit_id: Arc::from("1111111111111111111111111111111111111111"),
        author: Arc::from("Ada"),
        author_time_unix: Some(1_700_000_000),
        summary: Arc::from("initial"),
        body: None,
        line: line.to_string(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    }
}

#[test]
fn load_blame_dedupes_same_target_while_loading() {
    // `MainPaneView::render` dispatches from an asynchronously pushed state
    // snapshot, so a render burst (e.g. during a window resize) can ask for
    // the same blame many times before the `Loading` snapshot arrives. Each
    // duplicate would fork another `git blame` subprocess.
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let path = PathBuf::from("src/lib.rs");
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);

    let effects = load_blame(&mut state, repo_id, path.clone(), source.clone());
    assert_eq!(effects.len(), 1);
    assert!(load_blame(&mut state, repo_id, path.clone(), source.clone()).is_empty());
    assert!(load_blame(&mut state, repo_id, path, source).is_empty());
}

#[test]
fn load_blame_reloads_when_target_changes_while_loading() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);

    load_blame(
        &mut state,
        repo_id,
        PathBuf::from("src/lib.rs"),
        source.clone(),
    );
    let other = PathBuf::from("src/main.rs");
    let effects = load_blame(&mut state, repo_id, other.clone(), source);
    assert_eq!(effects.len(), 1);
    assert_eq!(
        repo_mut(&mut state, repo_id)
            .history_state
            .blame_path
            .as_ref(),
        Some(&other)
    );
}

#[test]
fn load_blame_retains_ready_annotations_for_the_same_target() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let path = PathBuf::from("src/lib.rs");
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
    let lines = Arc::new(vec![blame_line("let x = 1;")]);
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.blame_path = Some(path.clone());
        repo.history_state.blame_source = Some(source.clone());
        repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
    }

    load_blame(&mut state, repo_id, path, source);

    let repo = repo_mut(&mut state, repo_id);
    assert!(repo.history_state.blame.is_loading());
    assert!(
        repo.history_state
            .retained_blame_while_loading
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &lines)),
        "the annotation column must keep painting while the same target reloads"
    );
}

#[test]
fn load_blame_drops_retained_annotations_when_retargeting() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
        repo.history_state.blame_source = Some(source.clone());
        repo.history_state.blame = Loadable::Ready(Arc::new(vec![blame_line("let x = 1;")]));
    }

    load_blame(&mut state, repo_id, PathBuf::from("src/main.rs"), source);

    assert!(
        repo_mut(&mut state, repo_id)
            .history_state
            .retained_blame_while_loading
            .is_none(),
        "annotations for a different file must never be painted"
    );
}

#[test]
fn blame_loaded_reuses_the_retained_allocation_when_unchanged() {
    // An identical reload must not produce a new `Arc`: the view keys its
    // notify fingerprint and its memoized blame time range on Arc identity.
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let path = PathBuf::from("src/lib.rs");
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
    let lines = Arc::new(vec![blame_line("let x = 1;")]);
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.blame_path = Some(path.clone());
        repo.history_state.blame_source = Some(source.clone());
        repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
    }
    load_blame(&mut state, repo_id, path.clone(), source.clone());

    blame_loaded(
        &mut state,
        repo_id,
        path,
        source,
        Ok(vec![blame_line("let x = 1;")]),
    );

    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(&repo.history_state.blame, Loadable::Ready(got) if Arc::ptr_eq(got, &lines)));
    assert!(repo.history_state.retained_blame_while_loading.is_none());
}

#[test]
fn blame_loaded_replaces_the_retained_allocation_when_changed() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    let path = PathBuf::from("src/lib.rs");
    let source = gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged);
    let lines = Arc::new(vec![blame_line("let x = 1;")]);
    {
        let repo = repo_mut(&mut state, repo_id);
        repo.history_state.blame_path = Some(path.clone());
        repo.history_state.blame_source = Some(source.clone());
        repo.history_state.blame = Loadable::Ready(Arc::clone(&lines));
    }
    load_blame(&mut state, repo_id, path.clone(), source.clone());

    blame_loaded(
        &mut state,
        repo_id,
        path,
        source,
        Ok(vec![blame_line("let x = 2;")]),
    );

    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(&repo.history_state.blame, Loadable::Ready(got) if !Arc::ptr_eq(got, &lines)));
    assert!(repo.history_state.retained_blame_while_loading.is_none());
}

#[test]
fn file_browser_loaded_cancelled_error_records_diagnostic() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    repo_mut(&mut state, repo_id).file_browser.source = FileSource::WorkingDirectory;

    let cancelled = Error::new(ErrorKind::Cancelled);
    let effects = file_browser_loaded(
        &no_repos(),
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
        Err(cancelled),
    );
    assert!(effects.is_empty());
    let repo = repo_mut(&mut state, repo_id);
    assert!(matches!(repo.file_browser.entries, Loadable::Error(_)));
    assert_eq!(repo.feedback.diagnostics.len(), 1);
}
