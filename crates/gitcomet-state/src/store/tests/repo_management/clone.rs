use super::*;

#[test]
fn clone_repo_sets_running_state_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: PathBuf::from("/tmp/example"),
        },
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Running));
    assert_eq!(op.progress.stage, CloneProgressStage::Loading);
    assert_eq!(op.progress.percent, 0);
    assert_eq!(op.seq, 0);
    assert!(matches!(effects.as_slice(), [Effect::CloneRepo { .. }]));
}

#[test]
fn clone_repo_progress_trims_tail_and_skips_blank_lines() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "   ".to_string(),
        }),
    );
    for i in 0..84 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
                dest: Arc::new(dest.clone()),
                line: format!("line-{i}"),
            }),
        );
    }

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(op.seq, 85);
    assert_eq!(op.output_tail.len(), 80);
    assert_eq!(op.output_tail.front().map(String::as_str), Some("line-4"));
    assert_eq!(op.output_tail.back().map(String::as_str), Some("line-83"));
    assert_eq!(op.progress.stage, CloneProgressStage::Loading);
    assert_eq!(op.progress.percent, 0);
}

#[test]
fn clone_repo_progress_tracks_loading_and_remote_object_phases() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "Receiving objects:  42% (52/123), 1.23 MiB | 2.00 MiB/s".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.progress.stage, CloneProgressStage::Loading);
        assert_eq!(op.progress.percent, 42);
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest),
            line: "Resolving deltas:  17% (5/29)".to_string(),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(op.progress.stage, CloneProgressStage::RemoteObjects);
    assert_eq!(op.progress.percent, 17);
}

#[test]
fn clone_repo_progress_ignores_mismatched_or_non_running_operation() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(PathBuf::from("/tmp/other")),
            line: "ignored".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.seq, 0);
        assert!(op.output_tail.is_empty());
    }

    if let Some(op) = state.clone.as_mut() {
        op.status = CloneOpStatus::FinishedOk;
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest.clone()),
            line: "ignored-too".to_string(),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(op.seq, 0);
        assert!(op.output_tail.is_empty());
    }

    state.clone = None;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress {
            dest: Arc::new(dest),
            line: "no-op".to_string(),
        }),
    );
    assert!(state.clone.is_none());
}

#[test]
fn abort_clone_repo_marks_operation_cancelling_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Cancelling));
    assert_eq!(op.seq, 1);
    assert!(
        matches!(effects.as_slice(), [Effect::AbortCloneRepo { dest: effect_dest }] if effect_dest == &dest)
    );
}

#[test]
fn clone_repo_finished_updates_existing_operation_for_success_and_error() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/success.git".to_string(),
            dest: dest.clone(),
            result: Ok(CommandOutput::empty_success("git clone")),
        }),
    );
    {
        let op = state.clone.as_ref().expect("clone op set");
        assert_eq!(&*op.url, "file:///tmp/success.git");
        assert_eq!(op.dest.as_ref(), &dest);
        assert!(matches!(op.status, CloneOpStatus::FinishedOk));
        assert_eq!(op.seq, 1);
    }

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/failure.git".to_string(),
            dest: PathBuf::from("/tmp/example"),
            result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
        }),
    );
    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(&*op.url, "file:///tmp/failure.git");
    assert_eq!(op.seq, 2);
    match &op.status {
        CloneOpStatus::FinishedErr(message) => {
            assert!(message.contains("Clone failed"));
            assert!(message.contains("boom"));
        }
        other => panic!("expected clone error status, got {other:?}"),
    }
}

#[test]
fn clone_repo_finished_maps_cancelling_error_to_cancelled() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/example.git".to_string(),
            dest,
            result: Err(Error::new(ErrorKind::Backend("clone aborted".to_string()))),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert!(matches!(op.status, CloneOpStatus::Cancelled));
    assert_eq!(op.seq, 2);
}

#[test]
fn clone_repo_finished_preserves_cleanup_failure_when_cancelling() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dest = PathBuf::from("/tmp/example");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/example.git".to_string(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/example.git".to_string(),
            dest,
            result: Err(Error::new(ErrorKind::Backend(
                "clone aborted, but failed to remove partially created destination `/tmp/example`: permission denied"
                    .to_string(),
            ))),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    match &op.status {
        CloneOpStatus::FinishedErr(message) => {
            assert!(message.contains("Clone failed"));
            assert!(message.contains("failed to remove partially created destination"));
        }
        other => panic!("expected cleanup failure to remain visible, got {other:?}"),
    }
    assert_eq!(op.seq, 2);
}

#[test]
fn clone_repo_finished_replaces_state_when_destination_differs() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "file:///tmp/original.git".to_string(),
            dest: PathBuf::from("/tmp/original"),
        },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: "file:///tmp/replacement.git".to_string(),
            dest: PathBuf::from("/tmp/replacement"),
            result: Ok(CommandOutput::empty_success("git clone")),
        }),
    );

    let op = state.clone.as_ref().expect("clone op set");
    assert_eq!(&*op.url, "file:///tmp/replacement.git");
    assert_eq!(op.dest.as_ref(), &PathBuf::from("/tmp/replacement"));
    assert!(matches!(op.status, CloneOpStatus::FinishedOk));
    assert_eq!(op.seq, 1);
    assert!(op.output_tail.is_empty());
}
