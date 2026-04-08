use super::*;

fn mark_repo_switch_secondary_metadata_ready(repo: &mut RepoState) {
    repo.branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remotes = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
}

fn has_secondary_refresh_effects(effects: &[Effect], repo_id: RepoId) -> bool {
    effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::LoadBranches { repo_id: candidate }
                | Effect::LoadTags { repo_id: candidate }
                | Effect::LoadRemoteTags { repo_id: candidate }
                | Effect::LoadRemotes { repo_id: candidate }
                | Effect::LoadRemoteBranches { repo_id: candidate }
                if *candidate == repo_id
        )
    })
}

#[test]
fn open_repo_sets_opening_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo_state = state.repos.first().expect("repo state to be set");
    assert_eq!(repo_state.id.0, 1);
    assert!(repo_state.open.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(1)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
}

#[test]
fn open_repo_focuses_existing_repo_instead_of_opening_duplicate() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadStatus { repo_id } if *repo_id == RepoId(1))),
        "expected status refresh when focusing an already open repo"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo1 = super::reducer::normalize_repo_path(PathBuf::from("/tmp/repo1"));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == repo1)
            .count(),
        1
    );
}

#[test]
fn open_repo_allows_same_basename_in_different_folders() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-open-repo-same-basename-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let repo_a = dir.join("a").join("repo");
    let repo_b = dir.join("b").join("repo");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_a.clone()),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_b.clone()),
    );

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_a.clone()),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadStatus { repo_id } if *repo_id == RepoId(1))),
        "expected status refresh when re-focusing repo by path"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_a.clone()))
            .count(),
        1
    );
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_b.clone()))
            .count(),
        1
    );
}

#[test]
fn open_repo_refreshes_when_repo_is_already_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    state.repos[0].missing_on_disk = true;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadStatus { repo_id } if *repo_id == RepoId(1))),
        "expected status refresh when re-opening active repo"
    );
}

#[test]
fn clone_repo_sets_running_state_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

#[test]
fn close_repo_removes_and_moves_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(10);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(11)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(11),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(10)));
}

#[test]
fn close_repo_selects_right_neighbor_when_closing_first_active_tab() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(20);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: RepoId(20),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(20),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(21)));
}

#[test]
fn reorder_repo_tabs_moves_repo_and_keeps_active() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );

    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: Some(RepoId(1)),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(3), RepoId(1), RepoId(2)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));
}

#[test]
fn reorder_repo_tabs_noops_for_invalid_or_already_stable_ordering() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(999),
            insert_before: Some(RepoId(1)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(2),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );
}

#[test]
fn remote_branches_loaded_sets_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(vec![RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: CommitId("deadbeef".into()),
            }]),
        }),
    );

    let repo = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    match &repo.remote_branches {
        Loadable::Ready(branches) => {
            assert_eq!(branches.len(), 1);
            assert_eq!(branches[0].remote, "origin");
            assert_eq!(branches[0].name, "main");
        }
        other => panic!("expected Ready remote_branches, got {other:?}"),
    }
}

#[test]
fn restore_session_opens_all_and_selects_active_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-restore-session-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a.clone(), repo_b],
            active_repo: Some(repo_a.clone()),
        },
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::OpenRepo { .. }))
            .count(),
        2
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::PersistSession { .. }))
            .count(),
        1
    );

    let active_repo_id = state.active_repo.expect("active repo is set");
    let active_workdir = state
        .repos
        .iter()
        .find(|r| r.id == active_repo_id)
        .expect("active repo exists")
        .spec
        .workdir
        .clone();

    assert_eq!(active_workdir, super::reducer::normalize_repo_path(repo_a));
}

#[test]
fn set_active_repo_refreshes_repo_state_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));

    let has_status = effects
        .iter()
        .any(|e| matches!(e, Effect::LoadStatus { repo_id } if *repo_id == repo1));
    let has_log = effects.iter().any(|e| {
        matches!(e, Effect::LoadLog { repo_id, scope: _, limit: _, cursor: _ } if *repo_id == repo1)
    });
    let has_selected_diff_reload = effects.iter().any(|e| {
        matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: false,
                preview_text_side: None,
            } if *repo_id == repo1
        )
    });
    let has_persist = effects
        .iter()
        .any(|e| matches!(e, Effect::PersistSession { .. }));

    assert!(has_status, "expected status refresh on activation");
    assert!(has_log, "expected log refresh on activation");
    assert!(
        has_selected_diff_reload,
        "expected combined selected-diff reload on activation"
    );
    assert!(
        matches!(
            state
                .repos
                .iter()
                .find(|repo| repo.id == repo1)
                .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            Some(DiffTarget::WorkingTree { path, .. }) if path == &PathBuf::from("src/lib.rs")
        ),
        "expected the selected diff target to remain available on repo state for scheduling"
    );
    assert!(
        has_persist,
        "expected session persist when active repo changes"
    );
}

#[test]
fn set_active_repo_reloads_selected_image_diff_via_image_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("icon.png"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff: true,
            load_file_text: false,
            load_file_image: true,
            preview_text_side: None,
        } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_png_diff_enqueues_image_preview_only() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("image.png"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: false,
                load_file_image: true,
                ..
            } if *repo_id == repo1
        )),
        "expected combined selected-diff reload with image preview only for png target"
    );
}

#[test]
fn set_active_repo_svg_diff_enqueues_image_and_text_previews() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("vector.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: true,
                ..
            } if *repo_id == repo1
        )),
        "expected combined selected-diff reload with both image and text previews for svg target"
    );
}

#[test]
fn set_active_repo_selected_conflict_target_reuses_existing_conflict_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let conflict_path = PathBuf::from("src/conflict.rs");
    let before_rev = {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|r| r.id == repo1)
            .expect("repo1 exists");
        repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: conflict_path.clone(),
            area: gitcomet_core::domain::DiffArea::Unstaged,
        });
        repo1_state.conflict_state.conflict_file_path = Some(conflict_path.clone());
        let content: Arc<str> = Arc::from("conflict contents");
        repo1_state.conflict_state.conflict_file =
            Loadable::Ready(Some(crate::model::ConflictFile {
                path: conflict_path.clone().into(),
                base_bytes: None,
                ours_bytes: None,
                theirs_bytes: None,
                current_bytes: None,
                base: Some(Arc::clone(&content)),
                ours: Some(Arc::clone(&content)),
                theirs: Some(Arc::clone(&content)),
                current: Some(content),
            }));
        repo1_state.conflict_state.conflict_rev = 41;
        repo1_state.conflict_state.conflict_rev
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|r| r.id == repo1)
        .expect("repo1 exists");
    assert_eq!(
        repo1_state.conflict_state.conflict_file_path.as_ref(),
        Some(&conflict_path)
    );
    assert!(repo1_state.conflict_state.conflict_file.is_loading());
    assert!(repo1_state.conflict_state.conflict_session.is_none());
    assert_eq!(repo1_state.conflict_state.conflict_rev, before_rev + 1);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedConflictFile {
            repo_id,
            mode: crate::model::ConflictFileLoadMode::CurrentOnly,
        } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_hot_switch_skips_secondary_refresh_when_metadata_is_ready() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.last_active_at = Some(SystemTime::now());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        !has_secondary_refresh_effects(&effects, repo1),
        "hot repo switches with ready metadata should stay on the primary refresh path"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadStatus { repo_id } if *repo_id == repo1))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { repo_id, .. } if *repo_id == repo1))
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_uses_full_refresh_when_hot_switch_metadata_is_incomplete() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.tags = Loadable::NotLoaded;
    repo1_state.last_active_at = Some(SystemTime::now());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        has_secondary_refresh_effects(&effects, repo1),
        "missing secondary metadata should force the full refresh path"
    );
}

#[test]
fn set_active_repo_uses_full_refresh_when_hot_switch_window_expires() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.last_active_at = Some(SystemTime::now() - Duration::from_secs(6));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        has_secondary_refresh_effects(&effects, repo1),
        "stale repo switches should fall back to the full refresh path"
    );
}

#[test]
fn set_fetch_prune_deleted_remote_tracking_branches_updates_and_noops() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let initial = state.repos[0].fetch_prune_deleted_remote_tracking_branches;
    let target = !initial;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(1),
            enabled: target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(1),
            enabled: target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFetchPruneDeletedRemoteTrackingBranches {
            repo_id: RepoId(999),
            enabled: !target,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].fetch_prune_deleted_remote_tracking_branches,
        target
    );
}

#[test]
fn repo_opened_ok_sets_loading_and_emits_refresh_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    state.repos[0].missing_on_disk = true;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let repo_state = state.repos.first().unwrap();
    assert!(matches!(repo_state.open, Loadable::Ready(())));
    assert!(!repo_state.missing_on_disk);
    assert!(repo_state.head_branch.is_loading());
    assert!(repo_state.branches.is_loading());
    assert!(repo_state.tags.is_loading());
    assert!(repo_state.remote_tags.is_loading());
    assert!(repo_state.remotes.is_loading());
    assert!(repo_state.remote_branches.is_loading());
    assert!(repo_state.status.is_loading());
    assert!(repo_state.log.is_loading());
    assert!(matches!(repo_state.stashes, Loadable::NotLoaded));
    assert!(matches!(repo_state.reflog, Loadable::NotLoaded));
    assert!(repo_state.upstream_divergence.is_loading());
    assert!(repo_state.rebase_in_progress.is_loading());
    assert!(repo_state.merge_commit_message.is_loading());
    assert!(matches!(
        repo_state.history_state.file_history,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.history_state.blame,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::LoadHeadBranch { .. },
            Effect::LoadUpstreamDivergence { .. },
            Effect::LoadStatus { .. },
            Effect::LoadLog { .. },
            Effect::LoadBranches { .. },
            Effect::LoadTags { .. },
            Effect::LoadRemoteTags { .. },
            Effect::LoadRemotes { .. },
            Effect::LoadRemoteBranches { .. },
            Effect::LoadRebaseAndMergeState { .. },
        ]
    ));
}

#[test]
fn repo_action_finished_clears_error_and_refreshes() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    state.repos[0].last_error = Some("boom".to_string());
    state.banner_error = Some(crate::model::BannerErrorState {
        repo_id: Some(RepoId(1)),
        message: "boom".to_string(),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].last_error.is_none());
    assert!(state.banner_error.is_none());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadStatus { repo_id: RepoId(1) }))
    );
}

#[test]
fn repo_action_finished_err_records_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let error = Error::new(ErrorKind::Backend("boom".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            result: Err(error),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(
        repo_state
            .last_error
            .as_deref()
            .is_some_and(|s| s.contains("boom"))
    );
    assert!(
        repo_state
            .diagnostics
            .iter()
            .any(|d| d.message.contains("boom"))
    );
}

#[test]
fn repo_opened_err_records_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let error = Error::new(ErrorKind::Backend("nope".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            error,
        }),
    );

    let repo_state = &state.repos[0];
    assert!(
        repo_state
            .last_error
            .as_deref()
            .is_some_and(|s| s.contains("nope"))
    );
    assert!(
        repo_state
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nope"))
    );
    assert!(!repo_state.missing_on_disk);
}

#[test]
fn repo_opened_err_not_found_marks_repo_missing_without_banner_error() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/missing-repo")),
    );

    let error = Error::new(ErrorKind::Io(std::io::ErrorKind::NotFound));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/missing-repo"),
            },
            error,
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.missing_on_disk);
    assert!(repo_state.last_error.is_none());
    assert!(repo_state.diagnostics.is_empty());
    assert!(matches!(repo_state.open, Loadable::Error(_)));
}

#[test]
fn repo_opened_err_not_a_repository_shows_notification_and_does_not_add_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/not-a-repo")),
    );

    let error = Error::new(ErrorKind::NotARepository);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error,
        }),
    );

    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("not a git repository"))
    );
}

#[test]
fn repo_opened_err_not_a_repository_allows_opening_another_repo_afterwards() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/not-a-repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error: Error::new(ErrorKind::NotARepository),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.repos[0].id, RepoId(2));
    assert_eq!(
        state.repos[0].spec.workdir,
        super::reducer::normalize_repo_path(PathBuf::from("/tmp/repo"))
    );
    assert!(state.repos[0].open.is_loading());
    assert_eq!(state.active_repo, Some(RepoId(2)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
}

#[test]
fn set_active_repo_ignores_unknown_repo() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: RepoId(999),
        },
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn diagnostics_are_capped() {
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );

    for i in 0..205 {
        super::reducer::push_diagnostic(&mut repo_state, DiagnosticKind::Error, format!("err-{i}"));
    }

    assert_eq!(repo_state.diagnostics.len(), 200);
    assert_eq!(repo_state.diagnostics[0].message, "err-5");
    assert_eq!(repo_state.diagnostics.last().unwrap().message, "err-204");
}

#[test]
fn session_persist_error_reports_notification_and_repo_diagnostic() {
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(1)),
        "opening a repository",
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("permission denied"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("permission denied"))
    );
}

#[test]
fn session_persist_error_without_repo_still_reports_notification() {
    let mut state = AppState::default();

    super::reducer::handle_session_persist_result(
        &mut state,
        Some(RepoId(999)),
        "closing a repository",
        Err(std::io::Error::other("disk full")),
    );

    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(state.repos.is_empty());
}

#[test]
fn session_persist_failed_msg_reports_notification_and_repo_diagnostic() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id: Some(RepoId(1)),
            action: "opening a repository",
            error: "disk full".to_string(),
        }),
    );

    assert!(effects.is_empty());
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("Failed to persist session state"))
    );
    assert!(
        state
            .notifications
            .iter()
            .any(|n| n.message.contains("disk full"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("disk full"))
    );
}
