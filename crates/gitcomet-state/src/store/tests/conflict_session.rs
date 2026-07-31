use super::*;
use crate::model::ConflictFile;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictResolverStrategy, ConflictSession};
use gitcomet_core::domain::{FileConflictKind, FileStatus, FileStatusKind, RepoStatus};
use gitcomet_core::services::ConflictSide;

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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    // Inject a status with the conflict entry.
    let repo_state = state.repos.iter_mut().find(|r| r.id == repo_id).unwrap();
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from(path),
            kind: FileStatusKind::Conflicted,
            conflict: Some(conflict_kind),
        }],
        staged: vec![],
    })));
    // Set the conflict file path (simulates LoadConflictFile dispatch).
    repo_state.set_conflict_file_path(Some(PathBuf::from(path)));

    repo_id
}

fn sample_marker_conflict_file(path: &str) -> ConflictFile {
    ConflictFile {
        path: PathBuf::from(path).into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours\n".to_vec().into()),
        theirs_bytes: Some(b"theirs\n".to_vec().into()),
        current_bytes: Some(
            b"a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n"
                .to_vec()
                .into(),
        ),
        base: Some("base\n".to_string().into()),
        ours: Some("ours\n".to_string().into()),
        theirs: Some("theirs\n".to_string().into()),
        current: Some(
            "a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n"
                .to_string()
                .into(),
        ),
    }
}

fn two_region_marker_conflict_file(path: &str, current: &str) -> ConflictFile {
    let base = "base one\nmiddle\nbase two\n";
    let ours = "ours one\nmiddle\nours two\n";
    let theirs = "theirs one\nmiddle\ntheirs two\n";
    ConflictFile {
        path: PathBuf::from(path).into(),
        base_bytes: Some(base.as_bytes().to_vec().into()),
        ours_bytes: Some(ours.as_bytes().to_vec().into()),
        theirs_bytes: Some(theirs.as_bytes().to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some(base.to_string().into()),
        ours: Some(ours.to_string().into()),
        theirs: Some(theirs.to_string().into()),
        current: Some(current.to_string().into()),
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

    let current_text: Arc<str> =
        "a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n".into();
    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours\n".to_vec().into()),
        theirs_bytes: Some(b"theirs\n".to_vec().into()),
        current_bytes: Some(
            b"a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n"
                .to_vec()
                .into(),
        ),
        base: Some("base\n".to_string().into()),
        ours: Some("ours\n".to_string().into()),
        theirs: Some("theirs\n".to_string().into()),
        current: Some(current_text.clone()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();

    // ConflictSession should be populated.
    let session = repo_state
        .conflict_state
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
    assert!(!session.regions[0].ours.shares_backing_with(&current_text));
    assert!(!session.regions[0].theirs.shares_backing_with(&current_text));
    assert!(session.merge_plan.is_some());
}

#[test]
fn current_only_session_preserves_first_paint_pick_on_full_upgrade() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

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
    let current = "a\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nb\n";
    let current_only = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: None,
        ours: None,
        theirs: None,
        current: Some(current.into()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(current_only))),
            conflict_session: None,
        }),
    );
    let provisional = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("CurrentOnly text should expose marker-backed regions");
    assert_eq!(provisional.regions.len(), 1);
    assert!(provisional.merge_plan.is_none());
    assert!(provisional.base.is_absent());
    assert!(provisional.ours.is_absent());
    assert!(provisional.theirs.is_absent());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictToggleRegionSource {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            source: gitcomet_core::merge::MergeSource::A,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    let backend_session = ConflictSession::from_merged_shared_text(
        PathBuf::from("file.txt"),
        FileConflictKind::BothModified,
        ConflictPayload::Text("base\n".into()),
        ConflictPayload::Text("ours\n".into()),
        ConflictPayload::Text("theirs\n".into()),
        current.into(),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: Some(backend_session),
        }),
    );

    let upgraded = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("Full load should replace the provisional session");
    assert!(upgraded.merge_plan.is_none());
    assert_eq!(
        upgraded.regions[0].resolution,
        ConflictRegionResolution::Sources(gitcomet_core::merge::MergeSource::B.into()),
        "a pick made against CurrentOnly markers must survive the Full upgrade",
    );
}

#[test]
fn current_only_pick_maps_across_partitioned_stage_plan_boundaries() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;
    use gitcomet_core::merge::MergeSource;

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
    let current = concat!(
        "start\n",
        "<<<<<<< HEAD\n",
        "ours one\n",
        "shared separator\n",
        "ours two\n",
        "=======\n",
        "theirs one\n",
        "shared separator\n",
        "theirs two\n",
        ">>>>>>> topic\n",
        "end\n",
    );
    let current_only = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: None,
        ours: None,
        theirs: None,
        current: Some(current.into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(current_only))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictToggleRegionSource {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            source: MergeSource::A,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );

    let base = "start\nbase one\nshared separator\nbase two\nend\n";
    let ours = "start\nours one\nshared separator\nours two\nend\n";
    let theirs = "start\ntheirs one\nshared separator\ntheirs two\nend\n";
    let full = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(base.as_bytes().to_vec().into()),
        ours_bytes: Some(ours.as_bytes().to_vec().into()),
        theirs_bytes: Some(theirs.as_bytes().to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some(base.into()),
        ours: Some(ours.into()),
        theirs: Some(theirs.into()),
        current: Some(current.into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(full))),
            conflict_session: None,
        }),
    );

    let upgraded = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("Full load should replace the provisional session");
    assert!(upgraded.merge_plan.is_some());
    assert_eq!(upgraded.regions.len(), 2);
    assert_eq!(upgraded.regions[0].ours, "ours one\n");
    assert_eq!(upgraded.regions[1].ours, "ours two\n");
    assert!(upgraded.regions.iter().all(|region| {
        region.resolution == ConflictRegionResolution::Sources(MergeSource::B.into())
    }));
}

#[test]
fn current_only_upgrade_runs_on_open_autosolve_on_full_regions() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

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
    let current = concat!(
        "<<<<<<< ours\n",
        "value=1\n",
        "||||||| base\n",
        "value = 1\n",
        "=======\n",
        "value  =  1\n",
        ">>>>>>> theirs\n",
    );
    let current_only = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: None,
        ours: None,
        theirs: None,
        current: Some(current.into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(current_only))),
            conflict_session: None,
        }),
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::Unresolved,
        "provisional CurrentOnly regions must wait for the stage-backed autosolve",
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    let full = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"value = 1\n".to_vec().into()),
        ours_bytes: Some(b"value=1\n".to_vec().into()),
        theirs_bytes: Some(b"value  =  1\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("value = 1\n".into()),
        ours: Some("value=1\n".into()),
        theirs: Some("value  =  1\n".into()),
        current: Some(current.into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(full))),
            conflict_session: None,
        }),
    );

    let upgraded = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert!(matches!(
        upgraded.regions[0].resolution,
        ConflictRegionResolution::AutoResolved { .. }
    ));
    assert_eq!(upgraded.merge_plan.as_ref().unwrap().unresolved_count(), 0);
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
        path: PathBuf::from("deleted.txt").into(),
        base_bytes: Some(b"original\n".to_vec().into()),
        ours_bytes: Some(b"modified\n".to_vec().into()),
        theirs_bytes: None,
        current_bytes: Some(b"modified\n".to_vec().into()),
        base: Some("original\n".to_string().into()),
        ours: Some("modified\n".to_string().into()),
        theirs: None,
        current: Some("modified\n".to_string().into()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("deleted.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
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
        path: PathBuf::from("image.png").into(),
        base_bytes: Some(vec![0x89, 0x50, 0x4E, 0x47].into()),
        ours_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        theirs_bytes: Some(vec![0x89, 0x50, 0x4E, 0x49].into()),
        current_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        base: None,
        ours: None,
        theirs: None,
        current: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("image.png"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Err(Error::new(ErrorKind::Backend("test error".into())))),
            conflict_session: None,
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert!(repo_state.conflict_state.conflict_session.is_none());
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: Some(b"ours\n".to_vec().into()),
        theirs_bytes: Some(b"theirs\n".to_vec().into()),
        current_bytes: Some(
            b"<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>>\n"
                .to_vec()
                .into(),
        ),
        base: None,
        ours: Some("ours\n".to_string().into()),
        theirs: Some("theirs\n".to_string().into()),
        current: Some(
            "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>>\n"
                .to_string()
                .into(),
        ),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    assert!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_state
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
            mode: crate::model::ConflictFileLoadMode::CurrentOnly,
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
            .conflict_state
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            hide_resolved: true,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus {
                unstaged: vec![],
                staged: vec![],
            }),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_file_path, None);
    assert!(matches!(
        repo_state.conflict_state.conflict_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.conflict_state.conflict_session.is_none());
    assert!(!repo_state.conflict_state.conflict_hide_resolved);
    assert!(repo_state.conflict_state.conflict_rev > before_rev);
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus {
                unstaged: vec![FileStatus {
                    path: PathBuf::from("file.txt"),
                    kind: FileStatusKind::Conflicted,
                    conflict: Some(FileConflictKind::BothModified),
                }],
                staged: vec![],
            }),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(
        repo_state.conflict_state.conflict_file_path,
        Some(PathBuf::from("file.txt"))
    );
    assert!(matches!(
        repo_state.conflict_state.conflict_file,
        Loadable::Ready(Some(_))
    ));
    assert!(repo_state.conflict_state.conflict_session.is_some());
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours\n".to_vec().into()),
        theirs_bytes: Some(b"theirs\n".to_vec().into()),
        current_bytes: Some(
            b"<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n"
                .to_vec()
                .into(),
        ),
        base: Some("base\n".to_string().into()),
        ours: Some("ours\n".to_string().into()),
        theirs: Some("theirs\n".to_string().into()),
        current: Some(
            "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n"
                .to_string()
                .into(),
        ),
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: Some(provided_session.clone()),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
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
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            hide_resolved: true,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert!(repo_state.conflict_state.conflict_hide_resolved);
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
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
    let file = two_region_marker_conflict_file("file.txt", current);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    {
        let repo_state = state.repos.iter_mut().find(|r| r.id == repo_id).unwrap();
        let session = repo_state
            .conflict_state
            .conflict_session
            .as_mut()
            .expect("session exists");
        session.regions[0].resolution =
            gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs;
    }
    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictApplyBulkChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            choice: crate::msg::ConflictBulkChoice::Ours,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(
        session.regions[1].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
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
    let file = two_region_marker_conflict_file("file.txt", current);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(
        session.regions[1].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_ordered_source_messages_toggle_append_and_replace_manual_content() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;
    use gitcomet_core::merge::{MergeSource, OrderedSelection};

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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );

    for source in [MergeSource::B, MergeSource::C] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::ConflictToggleRegionSource {
                repo_id,
                path: PathBuf::from("file.txt").into(),
                region_index: 0,
                source,
            },
        );
    }
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::Sources(OrderedSelection::from_sources([
            MergeSource::B,
            MergeSource::C,
        ]))
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            updates: vec![crate::msg::ConflictRegionResolutionUpdate {
                region_index: 0,
                resolution: ConflictRegionResolution::ManualEdit("manual\n".into()),
            }],
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictToggleRegionSource {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            source: MergeSource::C,
        },
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::Sources(MergeSource::C.into()),
        "a source pick replaces manual block content",
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictReplaceRegionSelection {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            selection: OrderedSelection::from_sources([MergeSource::C, MergeSource::B]),
        },
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::Sources(OrderedSelection::from_sources([
            MergeSource::C,
            MergeSource::B,
        ]))
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictReplaceRegionSelection {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            selection: OrderedSelection::new(),
        },
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::Unresolved
    );
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

    // A genuinely absent stage-1 input uses true two-input mode, so Base is
    // unavailable even if the worktree happens to contain two-way markers.
    let current = "\
<<<<<<< ours\n\
ours only\n\
=======\n\
theirs only\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: Some(b"ours only\n".to_vec().into()),
        theirs_bytes: Some(b"theirs only\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: None,
        ours: Some("ours only\n".to_string().into()),
        theirs: Some("theirs only\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            choice: crate::msg::ConflictRegionChoice::Base,
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
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
    let file = two_region_marker_conflict_file("file.txt", current);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
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
            path: PathBuf::from("file.txt").into(),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictResetResolutions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(
        session.regions[1].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours one\n".to_vec().into()),
        theirs_bytes: Some(b"theirs one\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".into()),
        ours: Some("ours one\n".into()),
        theirs: Some("theirs one\n".into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictResetResolutions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
}

#[test]
fn conflict_file_loaded_uses_plan_default_for_identical_sides() {
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"same content\n".to_vec().into()),
        theirs_bytes: Some(b"same content\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".to_string().into()),
        ours: Some("same content\n".to_string().into()),
        theirs: Some("same content\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    // Identical contributor changes are a KDiff3 default selection, so they
    // never become an original conflict region or require an autosolve pass.
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 0);
    assert!(session.regions.is_empty());
    assert_eq!(
        session
            .merge_plan
            .as_ref()
            .expect("merge plan")
            .unresolved_count(),
        0
    );

    // A later explicit Safe dispatch has nothing left to solve: no rev bump.
    let before_rev = repo_state.conflict_state.conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictApplyAutosolve {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            mode: crate::msg::ConflictAutosolveMode::Safe,
            whitespace_normalize: false,
        },
    );
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
}

#[test]
fn conflict_file_loaded_syncs_on_open_whitespace_autosolve_into_plan() {
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
    let current = concat!(
        "<<<<<<< ours\n",
        "value=1\n",
        "||||||| base\n",
        "value = 1\n",
        "=======\n",
        "value  =  1\n",
        ">>>>>>> theirs\n",
    );
    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"value = 1\n".to_vec().into()),
        ours_bytes: Some(b"value=1\n".to_vec().into()),
        theirs_bytes: Some(b"value  =  1\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("value = 1\n".into()),
        ours: Some("value=1\n".into()),
        theirs: Some("value  =  1\n".into()),
        current: Some(current.into()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 0);
    assert!(matches!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::AutoResolved { .. }
    ));
    assert_eq!(
        session
            .merge_plan
            .as_ref()
            .expect("merge plan")
            .unresolved_count(),
        0,
        "on-open autosolve decisions must be reflected in the shared plan",
    );
}

fn history_conflict_file() -> ConflictFile {
    let current = "\
<<<<<<< ours\n\
## Changelog\n\
- entry a\n\
- entry b\n\
||||||| base\n\
## Changelog\n\
- entry a\n\
=======\n\
## Changelog\n\
- entry a\n\
- entry c\n\
>>>>>>> theirs\n\
";
    ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"## Changelog\n- entry a\n".to_vec().into()),
        ours_bytes: Some(b"## Changelog\n- entry a\n- entry b\n".to_vec().into()),
        theirs_bytes: Some(b"## Changelog\n- entry a\n- entry c\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("## Changelog\n- entry a\n".to_string().into()),
        ours: Some("## Changelog\n- entry a\n- entry b\n".to_string().into()),
        theirs: Some("## Changelog\n- entry a\n- entry c\n".to_string().into()),
        current: Some(current.to_string().into()),
    }
}

#[test]
fn conflict_apply_autosolve_history_stays_manual_and_updates_session() {
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(history_conflict_file()))),
            conflict_session: None,
        }),
    );

    // The Low tier (history merge) never applies on open.
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 1);
    let before_rev = repo_state.conflict_state.conflict_rev;

    // The explicit History dispatch resolves it and bumps the rev.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictApplyAutosolve {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            mode: crate::msg::ConflictAutosolveMode::History,
            whitespace_normalize: false,
        },
    );
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 0);
    assert!(matches!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::AutoResolved { .. }
    ));
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
}

#[test]
fn conflict_file_reload_keeps_identical_plan_clean() {
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
    let make_file = || ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"same content\n".to_vec().into()),
        theirs_bytes: Some(b"same content\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".to_string().into()),
        ours: Some("same content\n".to_string().into()),
        theirs: Some("same content\n".to_string().into()),
        current: Some(current.to_string().into()),
    };

    let load = |repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
                state: &mut AppState,
                file: ConflictFile| {
        reduce(
            repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id,
                path: PathBuf::from("file.txt"),
                result: Box::new(Ok(Some(file))),
                conflict_session: None,
            }),
        );
    };

    load(&mut repos, &mut state, make_file());

    // A reload rebuilds the same clean automatic plan without manufacturing a
    // marker region from the stale worktree text.
    load(&mut repos, &mut state, make_file());

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(session.unsolved_count(), 0);
    assert!(session.regions.is_empty());
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
    let file = two_region_marker_conflict_file("file.txt", current);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            updates: vec![
                crate::msg::ConflictRegionResolutionUpdate {
                    region_index: 0,
                    resolution:
                        gitcomet_core::conflict_session::ConflictRegionResolution::ManualEdit(
                            "custom merged one\n".into(),
                        ),
                },
                crate::msg::ConflictRegionResolutionUpdate {
                    region_index: 1,
                    resolution:
                        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs,
                },
            ],
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::ManualEdit(
            "custom merged one\n".into()
        )
    );
    assert_eq!(
        session.regions[1].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours one\n".to_vec().into()),
        theirs_bytes: Some(b"theirs one\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".to_string().into()),
        ours: Some("ours one\n".to_string().into()),
        theirs: Some("theirs one\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            updates: vec![crate::msg::ConflictRegionResolutionUpdate {
                region_index: 0,
                resolution: gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved,
            }],
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
    assert_eq!(
        repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session exists")
            .regions[0]
            .resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
}

#[test]
fn repo_command_finished_checkout_conflict_side_syncs_all_session_regions() {
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
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"ours\n".to_vec().into()),
        theirs_bytes: Some(b"theirs\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".to_string().into()),
        ours: Some("ours\n".to_string().into()),
        theirs: Some("theirs\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("file.txt"),
                side: ConflictSide::Theirs,
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout --theirs -- file.txt",
            )),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert!(session.regions.iter().all(|region| matches!(
        region.resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs
    )));
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
}

#[test]
fn repo_command_finished_checkout_conflict_base_syncs_regions_with_base() {
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
||||||| base\n\
base one\n\
=======\n\
theirs one\n\
>>>>>>> theirs\n\
middle\n\
<<<<<<< ours\n\
ours two\n\
||||||| base\n\
base two\n\
=======\n\
theirs two\n\
>>>>>>> theirs\n\
";
    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base one\nbase two\n".to_vec().into()),
        ours_bytes: Some(b"ours one\nours two\n".to_vec().into()),
        theirs_bytes: Some(b"theirs one\ntheirs two\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base one\nbase two\n".to_string().into()),
        ours: Some("ours one\nours two\n".to_string().into()),
        theirs: Some("theirs one\ntheirs two\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:file.txt -- file.txt",
            )),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert!(session.regions.iter().all(|region| matches!(
        region.resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickBase
    )));
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
}

#[test]
fn repo_command_finished_accept_conflict_deletion_syncs_two_way_region_resolution() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::AddedByUs,
    );

    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: Some(b"ours only\n".to_vec().into()),
        theirs_bytes: None,
        current_bytes: Some(b"ours only\n".to_vec().into()),
        base: None,
        ours: Some("ours only\n".to_string().into()),
        theirs: None,
        current: Some("ours only\n".to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AcceptConflictDeletion {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success("git rm -- file.txt")),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let session = repo_state
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickTheirs
    );
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev + 1);
}

#[test]
fn repo_command_finished_launch_mergetool_clears_conflict_context() {
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            hide_resolved: true,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LaunchMergetool {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success("mergetool (dummy)")),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_file_path, None);
    assert!(matches!(
        repo_state.conflict_state.conflict_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.conflict_state.conflict_session.is_none());
    assert!(!repo_state.conflict_state.conflict_hide_resolved);
    assert!(repo_state.conflict_state.conflict_rev > before_rev);
}

#[test]
fn repo_command_finished_checkout_conflict_side_clears_binary_conflict_context() {
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

    let file = ConflictFile {
        path: PathBuf::from("image.png").into(),
        base_bytes: Some(vec![0x89, 0x50, 0x4E, 0x47].into()),
        ours_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        theirs_bytes: Some(vec![0x89, 0x50, 0x4E, 0x49].into()),
        current_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        base: None,
        ours: None,
        theirs: None,
        current: None,
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("image.png"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("image.png").into(),
            hide_resolved: true,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("image.png"),
                side: ConflictSide::Theirs,
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout --theirs -- image.png",
            )),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_file_path, None);
    assert!(matches!(
        repo_state.conflict_state.conflict_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.conflict_state.conflict_session.is_none());
    assert!(!repo_state.conflict_state.conflict_hide_resolved);
    assert!(repo_state.conflict_state.conflict_rev > before_rev);
}

#[test]
fn repo_command_finished_checkout_conflict_base_clears_binary_conflict_context() {
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

    let file = ConflictFile {
        path: PathBuf::from("image.png").into(),
        base_bytes: Some(vec![0x89, 0x50, 0x4E, 0x47].into()),
        ours_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        theirs_bytes: Some(vec![0x89, 0x50, 0x4E, 0x49].into()),
        current_bytes: Some(vec![0x89, 0x50, 0x4E, 0x48].into()),
        base: None,
        ours: None,
        theirs: None,
        current: None,
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("image.png"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetHideResolved {
            repo_id,
            path: PathBuf::from("image.png").into(),
            hide_resolved: true,
        },
    );

    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("image.png"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:image.png -- image.png",
            )),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_file_path, None);
    assert!(matches!(
        repo_state.conflict_state.conflict_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.conflict_state.conflict_session.is_none());
    assert!(!repo_state.conflict_state.conflict_hide_resolved);
    assert!(repo_state.conflict_state.conflict_rev > before_rev);
}

#[test]
fn repo_command_finished_conflict_sync_noops_when_paths_or_session_do_not_match() {
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

    // No session yet: should no-op.
    let before_no_session_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:file.txt -- file.txt",
            )),
        }),
    );
    assert_eq!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_state
            .conflict_rev,
        before_no_session_rev
    );

    // Load a normal session.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );

    // Tracked conflict path mismatch should no-op.
    let before_tracked_mismatch_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("other.txt"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:other.txt -- other.txt",
            )),
        }),
    );
    assert_eq!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_state
            .conflict_rev,
        before_tracked_mismatch_rev
    );

    // Session path mismatch should also no-op.
    state
        .repos
        .iter_mut()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_session
        .as_mut()
        .expect("session")
        .path = PathBuf::from("different-session-path.txt");
    let before_session_mismatch_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:file.txt -- file.txt",
            )),
        }),
    );
    assert_eq!(
        state
            .repos
            .iter()
            .find(|r| r.id == repo_id)
            .unwrap()
            .conflict_state
            .conflict_rev,
        before_session_mismatch_rev
    );
}

#[test]
fn repo_command_finished_checkout_conflict_side_ours_syncs_region_resolution() {
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("file.txt"),
                side: ConflictSide::Ours,
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout --ours -- file.txt",
            )),
        }),
    );

    let session = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session");
    assert!(session.regions.iter().all(|region| matches!(
        region.resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs
    )));
}

#[test]
fn repo_command_finished_accept_conflict_deletion_maps_added_by_them_to_pick_ours() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::AddedByThem,
    );

    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: Some(b"theirs only\n".to_vec().into()),
        current_bytes: Some(b"theirs only\n".to_vec().into()),
        base: None,
        ours: None,
        theirs: Some("theirs only\n".to_string().into()),
        current: Some("theirs only\n".to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AcceptConflictDeletion {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success("git rm -- file.txt")),
        }),
    );

    let session = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs
    );
}

#[test]
fn repo_command_finished_accept_conflict_deletion_maps_both_modified_to_pick_ours() {
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
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(sample_marker_conflict_file("file.txt")))),
            conflict_session: None,
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AcceptConflictDeletion {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success("git rm -- file.txt")),
        }),
    );

    let session = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_session
        .as_ref()
        .expect("session exists");
    assert_eq!(
        session.regions[0].resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs
    );
}

#[test]
fn repo_command_finished_checkout_conflict_base_noops_for_regions_without_base() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = setup_repo_with_conflict(
        &mut state,
        &mut repos,
        &id_alloc,
        "file.txt",
        FileConflictKind::AddedByThem,
    );

    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: Some(b"theirs only\n".to_vec().into()),
        current_bytes: Some(b"theirs only\n".to_vec().into()),
        base: None,
        ours: None,
        theirs: Some("theirs only\n".to_string().into()),
        current: Some("theirs only\n".to_string().into()),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    let before_rev = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .unwrap()
        .conflict_state
        .conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success(
                "git checkout :1:file.txt -- file.txt",
            )),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(repo_state.conflict_state.conflict_rev, before_rev);
    assert_eq!(
        repo_state
            .conflict_state
            .conflict_session
            .as_ref()
            .expect("session exists")
            .regions[0]
            .resolution,
        gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
    );
}

/// Load a two-conflict `file.txt` where the first block has two lines per
/// side (so it can be split) and the second block has one. Returns the repo id.
fn setup_two_conflict_file(
    state: &mut AppState,
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
) -> RepoId {
    let repo_id = setup_repo_with_conflict(
        state,
        repos,
        id_alloc,
        "file.txt",
        FileConflictKind::BothModified,
    );

    let current = concat!(
        "<<<<<<< ours\n",
        "ours one\n",
        "ours two\n",
        "=======\n",
        "theirs one\n",
        "theirs two\n",
        ">>>>>>> theirs\n",
        "middle\n",
        "<<<<<<< ours\n",
        "ours three\n",
        "=======\n",
        "theirs three\n",
        ">>>>>>> theirs\n",
    );
    let file = ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base one\nbase two\nmiddle\nbase three\n".to_vec().into()),
        ours_bytes: Some(b"ours one\nours two\nmiddle\nours three\n".to_vec().into()),
        theirs_bytes: Some(
            b"theirs one\ntheirs two\nmiddle\ntheirs three\n"
                .to_vec()
                .into(),
        ),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base one\nbase two\nmiddle\nbase three\n".into()),
        ours: Some("ours one\nours two\nmiddle\nours three\n".into()),
        theirs: Some("theirs one\ntheirs two\nmiddle\ntheirs three\n".into()),
        current: Some(current.to_string().into()),
    };
    reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    repo_id
}

#[test]
fn conflict_split_region_stays_in_memory_and_carries_over_resolutions() {
    use gitcomet_core::conflict_session::{
        ConflictRegionResolution, ConflictRegionSplitBoundaries,
    };

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);

    // Split parts preserve the first conflict's selection. The second
    // conflict's choice must also survive its index shift from 1 to 2.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
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
            path: PathBuf::from("file.txt").into(),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );

    let before_rev = state.repos[0].conflict_state.conflict_rev;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: before_rev,
        },
    );

    assert!(
        effects.is_empty(),
        "split defers all disk writes until Save"
    );

    let repo_state = &state.repos[0];
    let session = repo_state.conflict_state.conflict_session.as_ref().unwrap();
    assert_eq!(session.regions.len(), 3, "one block became two");
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::PickOurs
    );
    assert_eq!(
        session.regions[1].resolution,
        ConflictRegionResolution::PickOurs
    );
    assert_eq!(
        session.regions[2].resolution,
        ConflictRegionResolution::PickTheirs,
        "second block resolution carried over to its shifted index"
    );
    assert_eq!(session.region_plan_blocks.len(), 3);
    assert_ne!(
        session.region_plan_blocks[0], session.region_plan_blocks[1],
        "split parts must remain independently selectable plan blocks",
    );
    assert_eq!(
        session
            .merge_plan
            .as_ref()
            .expect("shared merge plan")
            .unresolved_count(),
        0,
    );
    assert_eq!(
        repo_state.conflict_state.conflict_rev,
        before_rev.wrapping_add(1),
        "one structural edit publishes exactly one conflict revision",
    );

    assert_eq!(
        session
            .marker_projection_text()
            .expect("session marker text")
            .matches("<<<<<<<")
            .count(),
        3,
    );
    assert!(session.has_pending_structural_edits);

    // Structural edits are in-memory: the loaded worktree snapshot remains
    // untouched until an explicit save.
    if let crate::model::Loadable::Ready(Some(file)) = &repo_state.conflict_state.conflict_file {
        assert_eq!(file.current.as_ref().unwrap().matches("<<<<<<<").count(), 2);
        assert!(file.current_bytes.is_some());
    } else {
        panic!("conflict file should still be loaded");
    }
}

#[test]
fn conflict_split_partitions_source_backed_autosolved_content() {
    use gitcomet_core::conflict_session::{
        AutosolveRule, ConflictRegionResolution, ConflictRegionSplitBoundaries,
    };

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let rule = AutosolveRule::OnlyOursChanged;
    state.repos[0]
        .conflict_state
        .conflict_session
        .as_mut()
        .unwrap()
        .regions[0]
        .resolution = ConflictRegionResolution::AutoResolved {
        rule,
        confidence: rule.confidence(),
        content: "ours one\nours two\n".to_string(),
    };
    let before_rev = state.repos[0].conflict_state.conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: before_rev,
        },
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(session.regions.len(), 3);
    for (region, expected) in session.regions[..2]
        .iter()
        .zip(["ours one\n", "ours two\n"])
    {
        assert_eq!(
            region.resolution,
            ConflictRegionResolution::AutoResolved {
                rule,
                confidence: rule.confidence(),
                content: expected.to_string(),
            }
        );
    }
}

#[test]
fn conflict_split_preserves_arbitrary_manual_content_by_refusing_the_split() {
    use gitcomet_core::conflict_session::{
        ConflictRegionResolution, ConflictRegionSplitBoundaries,
    };

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_mut()
        .unwrap();
    session.regions[0].resolution =
        ConflictRegionResolution::ManualEdit("custom merged output\n".to_string());
    let projection_before = session.marker_projection.clone();
    let before_rev = state.repos[0].conflict_state.conflict_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: before_rev,
        },
    );

    assert!(effects.is_empty());
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(state.repos[0].conflict_state.conflict_rev, before_rev);
    assert_eq!(session.marker_projection, projection_before);
    assert_eq!(session.regions.len(), 2);
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::ManualEdit("custom merged output\n".to_string())
    );
}

#[test]
fn conflict_split_geometry_survives_same_path_reload() {
    use gitcomet_core::conflict_session::ConflictRegionSplitBoundaries;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let loaded_file = match &state.repos[0].conflict_state.conflict_file {
        Loadable::Ready(Some(file)) => file.clone(),
        other => panic!("expected loaded conflict file, got {other:?}"),
    };
    let rev = state.repos[0].conflict_state.conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: rev,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(loaded_file))),
            conflict_session: None,
        }),
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(session.regions.len(), 3);
    assert_eq!(
        session
            .marker_projection_text()
            .unwrap()
            .matches("<<<<<<<")
            .count(),
        3,
    );
    assert!(session.has_pending_structural_edits);
}

#[test]
fn conflict_join_regions_merges_blocks_without_writing() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let before_rev = state.repos[0].conflict_state.conflict_rev;

    let stale_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictJoinRegions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            expected_conflict_rev: before_rev.wrapping_add(1),
        },
    );
    assert!(
        stale_effects.is_empty(),
        "stale join must be a reducer no-op"
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions
            .len(),
        2,
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictJoinRegions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            expected_conflict_rev: before_rev,
        },
    );

    assert!(effects.is_empty(), "join defers all disk writes until Save");
    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(
        state.repos[0].conflict_state.conflict_rev,
        before_rev.wrapping_add(1),
        "one structural edit publishes exactly one conflict revision",
    );
    assert_eq!(session.regions.len(), 1, "two blocks became one");
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::Unresolved
    );
    assert_eq!(session.region_plan_blocks.len(), 1);
    let plan = session.merge_plan.as_ref().expect("shared merge plan");
    assert_eq!(plan.original_conflict_block_indices().len(), 1);
    assert_eq!(plan.unresolved_count(), 1);
    assert!(session.has_pending_structural_edits);
    // "middle" context between the blocks was absorbed into both sides.
    assert!(session.regions[0].ours.as_str().contains("middle"));
    assert!(session.regions[0].theirs.as_str().contains("middle"));
}

#[test]
fn conflict_join_geometry_survives_same_path_reload() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let loaded_file = match &state.repos[0].conflict_state.conflict_file {
        Loadable::Ready(Some(file)) => file.clone(),
        other => panic!("expected loaded conflict file, got {other:?}"),
    };
    let rev = state.repos[0].conflict_state.conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictJoinRegions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            expected_conflict_rev: rev,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(loaded_file))),
            conflict_session: None,
        }),
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(session.regions.len(), 1);
    assert_eq!(
        session
            .marker_projection_text()
            .unwrap()
            .matches("<<<<<<<")
            .count(),
        1,
    );
    assert!(session.has_pending_structural_edits);
}

#[test]
fn conflict_join_regions_carries_following_resolution_to_shifted_index() {
    use gitcomet_core::conflict_session::{
        ConflictRegionResolution, ConflictRegionSplitBoundaries,
    };

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let before_split_rev = state.repos[0].conflict_state.conflict_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: before_split_rev,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 2,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );
    let before_join_rev = state.repos[0].conflict_state.conflict_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictJoinRegions {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            expected_conflict_rev: before_join_rev,
        },
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(session.regions.len(), 2);
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::Unresolved,
    );
    assert_eq!(
        session.regions[1].resolution,
        ConflictRegionResolution::PickTheirs,
        "the untouched trailing region keeps its resolution after its index shifts",
    );
    assert_eq!(session.region_plan_blocks.len(), 2);
    assert_eq!(
        session
            .merge_plan
            .as_ref()
            .expect("shared merge plan")
            .unresolved_count(),
        1,
        "only the newly joined block should remain unresolved",
    );
}

#[test]
fn conflict_split_noops_on_degenerate_selection() {
    use gitcomet_core::conflict_session::ConflictRegionSplitBoundaries;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let before_rev = state.repos[0].conflict_state.conflict_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            // Whole block in one part -> None.
            boundaries: ConflictRegionSplitBoundaries {
                ours: [0, 2],
                theirs: [0, 2],
                base: Some([0, 2]),
            },
            expected_conflict_rev: before_rev,
        },
    );
    assert!(effects.is_empty(), "degenerate split emits no effects");
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions
            .len(),
        2
    );
}

#[test]
fn conflict_split_region_rejects_stale_revision() {
    use gitcomet_core::conflict_session::ConflictRegionSplitBoundaries;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    let current_rev = state.repos[0].conflict_state.conflict_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSplitRegion {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            boundaries: ConflictRegionSplitBoundaries {
                ours: [1, 1],
                theirs: [1, 1],
                base: Some([1, 1]),
            },
            expected_conflict_rev: current_rev.wrapping_add(1),
        },
    );

    assert!(effects.is_empty());
    assert_eq!(state.repos[0].conflict_state.conflict_rev, current_rev);
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions
            .len(),
        2,
    );
}

#[test]
fn conflict_reload_via_stash_keeps_ordered_resolution() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;
    use gitcomet_core::domain::{DiffArea, DiffTarget};

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
    state.active_repo = Some(repo_id);

    let current = "<<<<<<< ours\nlocal\n=======\nremote\n>>>>>>> theirs\n";
    let make_file = || ConflictFile {
        path: PathBuf::from("file.txt").into(),
        base_bytes: Some(b"base\n".to_vec().into()),
        ours_bytes: Some(b"local\n".to_vec().into()),
        theirs_bytes: Some(b"remote\n".to_vec().into()),
        current_bytes: Some(current.as_bytes().to_vec().into()),
        base: Some("base\n".to_string().into()),
        ours: Some("local\n".to_string().into()),
        theirs: Some("remote\n".to_string().into()),
        current: Some(current.to_string().into()),
    };
    let load = |repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>, state: &mut AppState| {
        reduce(
            repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
                repo_id,
                path: PathBuf::from("file.txt"),
                result: Box::new(Ok(Some(make_file()))),
                conflict_session: None,
            }),
        );
    };

    load(&mut repos, &mut state);
    // The user's selection is the state that must survive the reload.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 0,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::PickTheirs
    );

    // A real reload trigger (selecting the conflict diff) clears the live
    // session and stashes it for restore.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("file.txt"),
                area: DiffArea::Unstaged,
            },
        },
    );
    assert!(
        state.repos[0].conflict_state.conflict_session.is_none(),
        "reload clears the live session"
    );
    assert!(
        state.repos[0]
            .conflict_state
            .session_pending_restore
            .is_some(),
        "reload stashes the session for restore"
    );

    // The stash restores the selection by stable plan-block identity.
    load(&mut repos, &mut state);

    let repo_state = &state.repos[0];
    assert!(
        repo_state.conflict_state.session_pending_restore.is_none(),
        "stash consumed on load"
    );
    let session = repo_state.conflict_state.conflict_session.as_ref().unwrap();
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::PickTheirs,
        "reload restored the user's source selection"
    );
}

#[test]
fn same_path_explicit_full_load_restores_session_resolutions() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );
    let file = match &state.repos[0].conflict_state.conflict_file {
        Loadable::Ready(Some(file)) => file.clone(),
        other => panic!("expected loaded conflict file, got {other:?}"),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    let repo = &state.repos[0];
    assert!(repo.conflict_state.conflict_session.is_none());
    assert_eq!(
        repo.conflict_state
            .session_pending_restore
            .as_ref()
            .unwrap()
            .regions[1]
            .resolution,
        ConflictRegionResolution::PickTheirs,
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    let repo = &state.repos[0];
    assert!(repo.conflict_state.session_pending_restore.is_none());
    assert_eq!(
        repo.conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[1]
            .resolution,
        ConflictRegionResolution::PickTheirs,
    );
}

#[test]
fn failed_same_path_reload_keeps_stash_for_retry() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 1,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );
    let file = match &state.repos[0].conflict_state.conflict_file {
        Loadable::Ready(Some(file)) => file.clone(),
        other => panic!("expected loaded conflict file, got {other:?}"),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Err(Error::new(ErrorKind::Backend("transient".into())))),
            conflict_session: None,
        }),
    );
    assert!(state.repos[0].conflict_state.conflict_session.is_none());
    assert!(
        state.repos[0]
            .conflict_state
            .session_pending_restore
            .is_some(),
        "a failed reload must retain the resolution stash",
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(file))),
            conflict_session: None,
        }),
    );
    assert_eq!(
        state.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[1]
            .resolution,
        ConflictRegionResolution::PickTheirs,
    );
}

#[test]
fn resolution_restore_finds_unique_regions_after_large_prefix_deletion() {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    fn stage_text(indices: std::ops::Range<usize>, side: &str) -> String {
        indices
            .flat_map(|index| [format!("{side} {index}\n"), format!("anchor {index}\n")])
            .collect()
    }

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
    let make_file = |indices: std::ops::Range<usize>| {
        let base = stage_text(indices.clone(), "base");
        let ours = stage_text(indices.clone(), "ours");
        let theirs = stage_text(indices, "theirs");
        ConflictFile {
            path: PathBuf::from("file.txt").into(),
            base_bytes: Some(base.as_bytes().to_vec().into()),
            ours_bytes: Some(ours.as_bytes().to_vec().into()),
            theirs_bytes: Some(theirs.as_bytes().to_vec().into()),
            current_bytes: Some(Vec::new().into()),
            base: Some(base.into()),
            ours: Some(ours.into()),
            theirs: Some(theirs.into()),
            current: Some("".into()),
        }
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(make_file(0..40)))),
            conflict_session: None,
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConflictSetRegionChoice {
            repo_id,
            path: PathBuf::from("file.txt").into(),
            region_index: 39,
            choice: crate::msg::ConflictRegionChoice::Theirs,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::CurrentOnly,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path: PathBuf::from("file.txt"),
            result: Box::new(Ok(Some(make_file(33..40)))),
            conflict_session: None,
        }),
    );

    let session = state.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(session.regions.len(), 7);
    assert_eq!(
        session.regions[6].resolution,
        ConflictRegionResolution::PickTheirs,
        "the old region at offset 39 should align after deleting 33 predecessors",
    );
}

#[test]
fn clearing_conflict_context_drops_pending_restore_session() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = setup_two_conflict_file(&mut state, &mut repos, &id_alloc);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadConflictFile {
            repo_id,
            path: PathBuf::from("file.txt"),
            mode: crate::model::ConflictFileLoadMode::Full,
        },
    );
    assert!(
        state.repos[0]
            .conflict_state
            .session_pending_restore
            .is_some()
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LaunchMergetool {
                path: PathBuf::from("file.txt"),
            },
            result: Ok(CommandOutput::empty_success("git mergetool file.txt")),
        }),
    );
    assert!(state.repos[0].conflict_state.conflict_file_path.is_none());
    assert!(
        state.repos[0]
            .conflict_state
            .session_pending_restore
            .is_none()
    );
}
