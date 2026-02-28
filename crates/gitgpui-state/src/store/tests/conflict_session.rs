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

fn sample_marker_conflict_file(path: &str) -> ConflictFile {
    ConflictFile {
        path: PathBuf::from(path),
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
    }
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
    // Non-marker two-way conflicts synthesize a single decision region.
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.regions[0].base.as_deref(), Some("original\n"));
    assert_eq!(session.regions[0].ours, "modified\n");
    assert_eq!(session.regions[0].theirs, "");
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
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);
    assert!(!session.is_fully_resolved());
    assert!(session.regions.is_empty());
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
fn status_loaded_clears_conflict_context_when_path_is_resolved() {
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
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("file.txt"),
            hide_resolved: true,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus {
                unstaged: vec![],
                staged: vec![],
            }),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_file_path, None);
    assert!(matches!(repo_state.conflict_file, Loadable::NotLoaded));
    assert!(repo_state.conflict_session.is_none());
    assert!(!repo_state.conflict_hide_resolved);
    assert!(repo_state.conflict_rev > before_rev);
}

#[test]
fn status_loaded_keeps_conflict_context_for_same_conflicted_path() {
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
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus {
                unstaged: vec![FileStatus {
                    path: PathBuf::from("file.txt"),
                    kind: FileStatusKind::Conflicted,
                    conflict: Some(FileConflictKind::BothModified),
                }],
                staged: vec![],
            }),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(
        repo_state.conflict_file_path,
        Some(PathBuf::from("file.txt"))
    );
    assert!(matches!(repo_state.conflict_file, Loadable::Ready(Some(_))));
    assert!(repo_state.conflict_session.is_some());
    assert_eq!(repo_state.conflict_rev, before_rev);
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

#[test]
fn conflict_set_hide_resolved_updates_repo_state() {
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("file.txt"),
            hide_resolved: true,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert!(repo_state.conflict_hide_resolved);
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_apply_bulk_choice_updates_unresolved_session_regions_only() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
middle\n\
<<<<<<< ours\n\
ours two\n\
=======\n\
theirs two\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some(current.to_string()),
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

    {
        let repo_state = state.repos.iter_mut().find(|r| r.id == repo_id).unwrap();
        let session = repo_state
            .conflict_session
            .as_mut()
            .expect("session exists");
        session.regions[0].resolution =
            gitgpui_core::conflict_session::ConflictRegionResolution::PickTheirs;
    }
    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictApplyBulkChoice {
            repo_id,
            path: PathBuf::from("file.txt"),
            choice: crate::msg::ConflictBulkChoice::Ours,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(
        session.regions[1].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::PickOurs
    );
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_set_region_choice_updates_target_session_region() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
middle\n\
<<<<<<< ours\n\
ours two\n\
=======\n\
theirs two\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt"),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(
        session.regions[1].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_set_region_choice_base_noops_when_region_has_no_base() {
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

    // Two-way marker block (no ||||||| ancestor section), so region.base is None.
    let current = "\
<<<<<<< ours\n\
ours only\n\
=======\n\
theirs only\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours only\n".to_vec()),
        theirs_bytes: Some(b"theirs only\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours only\n".to_string()),
        theirs: Some("theirs only\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt"),
            region_index: 0,
            choice: crate::msg::ConflictRegionChoice::Base,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_rev, before_rev);
}

#[test]
fn conflict_reset_resolutions_clears_all_region_choices() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
middle\n\
<<<<<<< ours\n\
ours two\n\
=======\n\
theirs two\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some(current.to_string()),
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
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt"),
            region_index: 0,
            choice: crate::msg::ConflictRegionChoice::Ours,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt"),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictResetResolutions {
            repo_id,
            path: PathBuf::from("file.txt"),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(
        session.regions[1].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_reset_resolutions_noops_when_already_unresolved() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictResetResolutions {
            repo_id,
            path: PathBuf::from("file.txt"),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_rev, before_rev);
}

#[test]
fn conflict_apply_autosolve_safe_updates_conflict_session() {
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

    let current = "\
<<<<<<< ours\n\
same content\n\
=======\n\
same content\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"same content\n".to_vec()),
        theirs_bytes: Some(b"same content\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("same content\n".to_string()),
        theirs: Some("same content\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictApplyAutosolve {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::msg::ConflictAutosolveMode::Safe,
            whitespace_normalize: false,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 0);
    assert!(matches!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::AutoResolved { .. }
    ));
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_sync_region_resolutions_updates_manual_edit_and_pick() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
middle\n\
<<<<<<< ours\n\
ours two\n\
=======\n\
theirs two\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours\n".to_vec()),
        theirs_bytes: Some(b"theirs\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours\n".to_string()),
        theirs: Some("theirs\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path: PathBuf::from("file.txt"),
            updates: vec![
                crate::msg::ConflictRegionResolutionUpdate {
                    region_index: 0,
                    resolution:
                        gitgpui_core::conflict_session::ConflictRegionResolution::ManualEdit(
                            "custom merged one\n".into(),
                        ),
                },
                crate::msg::ConflictRegionResolutionUpdate {
                    region_index: 1,
                    resolution:
                        gitgpui_core::conflict_session::ConflictRegionResolution::PickTheirs,
                },
            ],
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::ManualEdit(
            "custom merged one\n".into()
        )
    );
    assert_eq!(
        session.regions[1].resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(repo_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_sync_region_resolutions_noops_when_resolution_is_unchanged() {
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

    let current = "\
<<<<<<< ours\n\
ours one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt"),
        base_bytes: Some(b"base\n".to_vec()),
        ours_bytes: Some(b"ours one\n".to_vec()),
        theirs_bytes: Some(b"theirs one\n".to_vec()),
        current_bytes: Some(current.as_bytes().to_vec()),
        base: Some("base\n".to_string()),
        ours: Some("ours one\n".to_string()),
        theirs: Some("theirs one\n".to_string()),
        current: Some(current.to_string()),
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

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path: PathBuf::from("file.txt"),
            updates: vec![crate::msg::ConflictRegionResolutionUpdate {
                region_index: 0,
                resolution: gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved,
            }],
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_rev, before_rev);
    assert_eq!(
        repo_state
            .conflict_session
            .as_ref()
            .expect("session exists")
            .regions[0]
            .resolution,
        gitgpui_core::conflict_session::ConflictRegionResolution::Unresolved
    );
}
