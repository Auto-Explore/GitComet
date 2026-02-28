use super::*;
use crate::model::ConflictFile;
use gitgpui_core::conflict_session::{ConflictPayload, ConflictResolverStrategy, ConflictSession};
use gitgpui_core::domain::{FileConflictKind, FileStatus, FileStatusKind, RepoStatus};

/// Helper: set up a repo state with a conflicted status entry.
fn setup_repo_with_conflict(
    state: &mut AppState,
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    path: &str,
    conflict_kind: FileConflictKind,
) -> RepoId {
    reduce(
        repos,
        id_alloc,
        state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    reduce(
        repos,
        id_alloc,
        state,
        Msg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        },
    );

    // Inject a status with the conflict entry.
    let repo_state = state.repos.iter_mut().find(|r| r.id == repo_id).unwrap();
    repo_state.status = Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from(path),
            kind: FileStatusKind::Conflicted,
            conflict: Some(conflict_kind),
        }],
        staged: vec![],
    }));
    // Set the conflict file path (simulates LoadConflictFile dispatch).
    repo_state.set_conflict_file_path(Some(PathBuf::from(path)));

    repo_id
}

#[test]
fn conflict_file_loaded_builds_session_with_regions() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );

    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(
            b"a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n".to_vec(),
        ),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some("a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n".to_string()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();

    // ConflictSession should be populated.
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("conflict_session should be built");
    assert_eq!(session.path, PathBuf::from("file.txt"));
    assert_eq!(session.conflict_kind, FileConflictKind::BothModified);
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);

    // Should have parsed 1 region from the markers.
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.regions[0].ours, "ours\n");
    assert_eq!(session.regions[0].theirs, "theirs\n");
}

#[test]
fn conflict_file_loaded_builds_session_for_delete_conflict() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "deleted.txt",
        FileConflictKind::DeletedByThem,
    );

    let file = ConflictFile {
        path: PathBuf::from("deleted.txt"),
        base_bytes: Some(b"original\n".to_vec()),
        ours_bytes: Some(b"modified\n".to_vec()),
        theirs_bytes: None,
        current_bytes: Some(b"modified\n".to_vec()),
        base: Some("original\n".to_string()),
        ours: Some("modified\n".to_string()),
        theirs: None,
        current: Some("modified\n".to_string()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("deleted.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session should exist");
    assert_eq!(session.conflict_kind, FileConflictKind::DeletedByThem);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.theirs.is_absent());
    // No conflict markers in the current text, so no regions.
    assert_eq!(session.total_regions(), 0);
}

#[test]
fn conflict_file_loaded_builds_binary_session() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "image.png",
        FileConflictKind::BothModified,
    );

    // Binary file: bytes present but text is None (non-UTF8).
    let file = ConflictFile {
        path: PathBuf::from("image.png"),
        base_bytes: Some(vec![0x89, 0x50, 0x4E, 0x47]),
        ours_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48]),
        theirs_bytes: Some(vec![0x89, 0x50, 0x4E, 0x49]),
        current_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48]),
        base: None,
        ours: None,
        theirs: None,
        current: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("image.png"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session should exist");
    assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);
    assert!(session.base.is_binary());
    assert!(session.ours.is_binary());
    assert!(session.theirs.is_binary());
}

#[test]
fn conflict_file_loaded_clears_session_on_error() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Err(Error::new(ErrorKind::Backend("test error".into())))),
            conflict_session: None,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert!(repo_state.conflict_session.is_none());
}

#[test]
fn load_conflict_file_clears_previous_session() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );

    // First load — builds a session.
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: None,
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(b"<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>>\n".to_vec()),
        base: None,
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some("<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>>\n".to_string()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        },
    );
    assert!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_session
            .is_some()
    );

    // Now dispatch LoadConflictFile for a different file — session should be cleared.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("other.txt"),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadConflictFile { .. }))
    );
    assert!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_session
            .is_none()
    );
}

#[test]
fn conflict_file_loaded_prefers_backend_session_when_provided() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );

    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(b"<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n".to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some("<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n".to_string()),
    };
    let provided_session = ConflictSession::new(
        PathBuf::from("file.txt"),
        FileConflictKind::BothDeleted,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: Some(provided_session.clone()),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.path, provided_session.path);
    assert_eq!(session.conflict_kind, provided_session.conflict_kind);
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
}
