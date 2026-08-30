use super::*;

#[test]
fn set_active_repo_waits_for_repo_open_before_refreshing() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo1
    )));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadWorktreeStatus { .. }
                | Effect::LoadStagedStatus { .. }
                | Effect::LoadBranches { .. }
                | Effect::LoadWorktrees { .. }
                | Effect::LoadSelectedDiff { .. }
        )),
        "expected no handle-dependent refreshes before RepoOpenedOk"
    );
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists")
            .worktrees,
        Loadable::NotLoaded
    ));
}

#[test]
fn switching_away_from_opening_repo_cancels_loading_and_restarts_on_return() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));
    assert!(state.repos[1].open.is_loading());

    let old_epoch = state.repos[1].load_epoch;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let repo2_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists");
    assert_eq!(repo2_state.load_epoch, old_epoch.wrapping_add(1));
    assert!(matches!(repo2_state.open, Loadable::NotLoaded));
    assert!(has_cancel_repo_loads_effect(&effects, repo2, old_epoch));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );

    let repo2_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists");
    assert!(repo2_state.open.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo2
    )));
}

#[test]
fn opening_another_repo_cancels_previous_active_repo_loads() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo1 = RepoId(1);
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(matches!(repo1_state.open, Loadable::NotLoaded));
    assert_eq!(repo1_state.load_epoch, old_epoch.wrapping_add(1));
    assert!(has_cancel_repo_loads_effect(&effects, repo1, old_epoch));
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn closing_active_repo_refreshes_open_neighbor_with_cancelled_loads() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        repo1_state.set_branches(Loadable::Loading);
        repo1_state.set_status(Loadable::Loading);
        repo1_state.set_log(Loadable::Loading);
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::BRANCHES)
        );
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::WORKTREE_STATUS)
        );
        assert!(
            repo1_state
                .loads_in_flight
                .request(RepoLoadsInFlight::STAGED_STATUS)
        );
        let log_request = crate::model::PendingLogLoad {
            scope: repo1_state.history_state.history_scope,
            author: None,
            limit: 50,
            cursor: None,
        };
        assert!(
            repo1_state
                .loads_in_flight
                .request_log(log_request)
                .is_some()
        );
    }

    let repo2 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(matches!(repo1_state.open, Loadable::Ready(())));
    assert!(matches!(repo1_state.branches, Loadable::NotLoaded));
    assert!(matches!(repo1_state.status, Loadable::NotLoaded));
    assert!(matches!(repo1_state.log, Loadable::NotLoaded));
    assert!(!repo1_state.loads_in_flight.any_in_flight());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: repo2 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(
        has_status_refresh_effects(&effects, repo1),
        "expected status refresh when close selects already-open repo"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { repo_id, .. } if *repo_id == repo1)),
        "expected log refresh when close selects already-open repo"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBranches { repo_id } if *repo_id == repo1)),
        "expected branch refresh when close selects already-open repo"
    );
}

#[test]
fn stale_open_result_after_cancel_is_ignored() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let repo1 = RepoId(1);
    let old_epoch = state.repos[0].load_epoch;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id: repo1,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::RepoOpenedOk {
                repo_id: repo1,
                spec: RepoSpec {
                    workdir: PathBuf::from("/tmp/repo1"),
                },
                repo: Arc::new(DummyRepo::new("/tmp/repo1")),
            }),
        }),
    );

    assert!(effects.is_empty());
    assert!(!repos.contains_key(&repo1));
    assert!(matches!(state.repos[0].open, Loadable::NotLoaded));
}

#[test]
fn stale_load_result_after_cancel_is_ignored() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo1 = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_status(Loadable::Loading);
    assert!(
        repo1_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_STATUS)
    );
    let old_epoch = repo1_state.load_epoch;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id: repo1,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::StatusLoaded {
                repo_id: repo1,
                result: Ok(RepoStatus::default()),
            }),
        }),
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(effects.is_empty());
    assert!(matches!(repo1_state.status, Loadable::NotLoaded));
    assert!(!repo1_state.loads_in_flight.any_in_flight());
}

#[test]
fn inactive_open_result_does_not_schedule_refresh_or_tags() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let inactive_repo = RepoId(1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: inactive_repo,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo1"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo1")),
        }),
    );

    assert!(effects.is_empty());
    assert!(repos.contains_key(&inactive_repo));
    let repo_state = state
        .repos
        .iter()
        .find(|repo| repo.id == inactive_repo)
        .expect("inactive repo exists");
    assert!(matches!(repo_state.open, Loadable::Ready(())));
    assert!(matches!(repo_state.tags, Loadable::NotLoaded));
    assert!(matches!(repo_state.remote_tags, Loadable::NotLoaded));
    assert!(!repo_state.loads_in_flight.any_in_flight());
}

#[test]
fn closing_loading_active_repo_cancels_and_opens_neighbor() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let active_repo = state.active_repo.expect("active repo exists");
    let neighbor_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != active_repo)
        .expect("neighbor repo exists")
        .id;
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == active_repo)
        .expect("active repo exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: active_repo,
        },
    );

    assert!(state.repos.iter().all(|repo| repo.id != active_repo));
    assert_eq!(state.active_repo, Some(neighbor_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        active_repo,
        old_epoch
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == neighbor_repo
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == neighbor_repo)
            .expect("neighbor exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn closing_loading_inactive_repo_cancels_without_changing_active_repo() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let active_repo = state.active_repo.expect("active repo exists");
    let inactive_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != active_repo)
        .expect("inactive repo exists")
        .id;
    let inactive_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == inactive_repo)
        .expect("inactive repo exists");
    inactive_state.set_open(Loadable::Loading);
    let old_epoch = inactive_state.load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: inactive_repo,
        },
    );

    assert_eq!(state.active_repo, Some(active_repo));
    assert!(state.repos.iter().all(|repo| repo.id != inactive_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        inactive_repo,
        old_epoch
    ));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == active_repo
    )));
}

#[test]
fn pre_open_worktree_lazy_load_retries_after_repo_opened() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadWorktrees { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].worktrees, Loadable::NotLoaded));
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::WORKTREES)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: rid } if *rid == repo_id)
        )
    );
}

#[test]
fn load_ref_metadata_emits_effect_and_result_builds_the_lookup_map() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRefMetadata { repo_id },
    );
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::LoadRefMetadata { repo_id: rid } if *rid == repo_id)
    ));
    assert!(state.repos[0].ref_metadata.is_loading());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Ok(vec![(
                "main".to_string(),
                gitcomet_core::domain::RefMetadata {
                    author: "Ada".to_string(),
                    committed_at: 1_754_870_400,
                    summary: "first".to_string(),
                },
            )]),
        }),
    );

    let Loadable::Ready(map) = &state.repos[0].ref_metadata else {
        panic!("expected ref metadata to be ready");
    };
    assert_eq!(map.get("main").map(|m| m.summary.as_str()), Some("first"));
}

#[test]
fn ref_metadata_load_failure_records_no_diagnostic() {
    // Decorative data: a backend that cannot supply it must not raise an error
    // banner every time a picker opens.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    let diagnostics_before = state.repos[0].feedback.diagnostics.len();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("git blew up".to_string()),
            )),
        }),
    );

    assert!(matches!(state.repos[0].ref_metadata, Loadable::Error(_)));
    assert_eq!(
        state.repos[0].feedback.diagnostics.len(),
        diagnostics_before
    );
}

#[test]
fn unsupported_ref_metadata_latches_instead_of_retrying_forever() {
    // Callers refetch on `Error`, so storing `Error` for a backend that can
    // never supply this would re-schedule a doomed load on every picker open.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Unsupported("nope"),
            )),
        }),
    );

    let Loadable::Ready(map) = &state.repos[0].ref_metadata else {
        panic!("expected Unsupported to latch as an empty Ready map");
    };
    assert!(map.is_empty());
}

#[test]
fn transient_ref_metadata_failure_stays_retryable() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("git blew up".to_string()),
            )),
        }),
    );

    assert!(
        matches!(state.repos[0].ref_metadata, Loadable::Error(_)),
        "a transient failure must stay retryable"
    );
}

#[test]
fn branch_change_during_an_in_flight_metadata_load_schedules_a_refetch() {
    // Otherwise the in-flight result (read from the *old* refs) lands as
    // `Ready` and, since callers only refetch on NotLoaded/Error, is never
    // corrected.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    let repo_id = RepoId(1);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    // Start a metadata load, then let the branch list change underneath it.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRefMetadata { repo_id },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![]),
        }),
    );

    // The stale result arrives; it must trigger another load rather than stick.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RefMetadataLoaded {
            repo_id,
            result: Ok(vec![]),
        }),
    );

    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadRefMetadata { repo_id: rid } if *rid == repo_id)
        ),
        "expected a refetch to be scheduled, got {effects:?}"
    );
}

#[test]
fn pre_open_submodule_load_auto_starts_after_repo_opened() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadSubmodules { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].submodules, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::LoadSubmodules { repo_id: rid } if *rid == repo_id)
    ));
    assert!(state.repos[0].submodules.is_loading());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadSubmodules { repo_id },
    );
    assert!(effects.is_empty());
    assert!(state.repos[0].submodules.is_loading());
}

#[test]
fn pre_open_stash_lazy_load_can_retry_after_repo_opened() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadStashes { repo_id },
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::STASHES)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadStashes {
            repo_id: rid,
            limit: 50
        } if *rid == repo_id
    )));
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadStashes { repo_id },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadStashes {
            repo_id: rid,
            limit: 50
        } if *rid == repo_id
    )));
    assert!(state.repos[0].stashes.is_loading());
}

#[test]
fn ensure_sidebar_data_retries_requested_sections_after_repo_opened() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    let repo_id = RepoId(1);
    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::EnsureSidebarData { repo_id, request },
    );
    assert!(effects.is_empty());
    assert_eq!(state.repos[0].sidebar_data_request, request);
    assert!(matches!(state.repos[0].worktrees, Loadable::NotLoaded));
    assert!(matches!(state.repos[0].submodules, Loadable::NotLoaded));
    assert!(matches!(state.repos[0].stashes, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    assert!(has_worktree_refresh_effect(&effects, repo_id));
    assert!(has_submodule_load_effect(&effects, repo_id));
    assert!(has_stash_load_effect(&effects, repo_id));
    assert!(state.repos[0].worktrees.is_loading());
    assert!(state.repos[0].submodules.is_loading());
    assert!(state.repos[0].stashes.is_loading());
}

#[test]
fn set_active_repo_replays_stored_sidebar_data_request() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_sidebar_data_request(request);
    repo1_state.set_worktrees(Loadable::NotLoaded);
    repo1_state.set_submodules(Loadable::NotLoaded);
    repo1_state.set_stashes(Loadable::NotLoaded);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(has_worktree_refresh_effect(&effects, repo1));
    assert!(has_submodule_load_effect(&effects, repo1));
    assert!(has_stash_load_effect(&effects, repo1));
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.worktrees.is_loading());
    assert!(repo1_state.submodules.is_loading());
    assert!(repo1_state.stashes.is_loading());
}

#[test]
fn set_active_repo_full_refresh_with_sidebar_request_and_selected_diff_does_not_panic() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    assert_eq!(state.active_repo, Some(repo2));

    let request = SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    };
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_sidebar_data_request(request);
    repo1_state.set_worktrees(Loadable::NotLoaded);
    repo1_state.set_submodules(Loadable::NotLoaded);
    repo1_state.set_stashes(Loadable::NotLoaded);
    repo1_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert_eq!(state.active_repo, Some(repo1));
    assert!(
        has_full_refresh_only_effects(&effects, repo1),
        "expected cold repo switch to use full refresh"
    );
    assert!(has_worktree_refresh_effect(&effects, repo1));
    assert!(has_submodule_load_effect(&effects, repo1));
    assert!(has_stash_load_effect(&effects, repo1));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff: true,
            load_file_text: true,
            load_file_image: false,
            load_submodule_summary: false,
            preview_text_side: None,
        } if *repo_id == repo1
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistSession { repo_id, .. } if *repo_id == Some(repo1)
    )));

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.worktrees.is_loading());
    assert!(repo1_state.submodules.is_loading());
    assert!(repo1_state.stashes.is_loading());
}

#[test]
fn set_active_repo_refreshes_repo_state_and_selected_diff() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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

    let has_status = has_status_refresh_effects(&effects, repo1);
    let has_log = effects
        .iter()
        .any(|e| matches!(e, Effect::LoadLog { repo_id, .. } if *repo_id == repo1));
    let has_selected_diff_reload = effects.iter().any(|e| {
        matches!(
            e,
            Effect::LoadSelectedDiff {
                repo_id,
                load_patch_diff: true,
                load_file_text: true,
                load_file_image: false,
                load_submodule_summary: false,
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
fn set_active_repo_plans_retained_commit_submodule_diff_before_clearing_details() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    for repo in &mut state.repos {
        mark_repo_switch_secondary_metadata_ready(repo);
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let commit_id = CommitId("submodule-commit".into());
    let path = PathBuf::from("vendor/dependency");
    let target = DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    repo1_state.set_selected_commit(Some(commit_id.clone()));
    repo1_state.set_commit_details(Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id,
        message: "update dependency".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: vec![CommitFileChange {
            path,
            kind: FileStatusKind::Modified,
            is_submodule: true,
            additions: None,
            deletions: None,
        }],
    })));
    repo1_state.set_diff_target(Some(target));
    repo1_state.diff_state.submodule_summary = Loadable::Loading;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(matches!(
        repo1_state.diff_state.submodule_summary,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo1_state.history_state.commit_details,
        Loadable::Ready(_)
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedDiff {
            repo_id,
            load_patch_diff: false,
            load_file_text: false,
            preview_text_side: None,
            load_submodule_summary: true,
            load_file_image: false,
        } if *repo_id == repo1
    )));
    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.history_state.selected_commit.is_none());
    assert!(matches!(
        repo1_state.history_state.commit_details,
        Loadable::NotLoaded
    ));
}

#[test]
fn set_active_repo_resets_the_activated_tabs_history_selection_only_on_change() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let stale_commit = CommitId("stale".into());
    let target = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    target.history_state.selected_commit = Some(stale_commit.clone());
    target.history_state.multi_selection = CommitMultiSelection {
        commits: vec![stale_commit.clone(), CommitId("older".into())],
        anchor: Some(stale_commit.clone()),
        anchor_index: Some(0),
        anchor_log_rev: Some(target.history_state.log_rev),
    };
    target.history_state.range_selection = Some(RangeSelection {
        from: CommitId("older".into()),
        to: Some(stale_commit),
        from_label: "older".to_string(),
        to_label: "stale".to_string(),
    });
    target.history_state.worktree_selection = Some(PathBuf::from("/tmp/repo1-linked"));
    target.history_state.commit_details = Loadable::Error("stale details".to_string());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(target.history_state.selected_commit.is_none());
    assert!(target.history_state.multi_selection.commits.is_empty());
    assert!(target.history_state.range_selection.is_none());
    assert!(target.history_state.worktree_selection.is_none());
    assert!(matches!(
        target.history_state.commit_details,
        Loadable::NotLoaded
    ));

    let fresh_commit = CommitId("fresh".into());
    let target = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    target.set_selected_commit(Some(fresh_commit.clone()));
    target.set_commit_details(Loadable::Error("keep details".to_string()));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert_eq!(
        target.history_state.selected_commit.as_ref(),
        Some(&fresh_commit),
        "re-focusing the active tab must preserve an explicit selection"
    );
    assert!(matches!(
        &target.history_state.commit_details,
        Loadable::Error(error) if error == "keep details"
    ));
}

#[test]
fn set_active_repo_inline_retires_the_activated_worktrees_orphaned_diff() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let target_repo = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let worktree = PathBuf::from("/tmp/repo1-linked");
    let inline_target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
    let target = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    target.history_state.worktree_selection = Some(worktree.clone());
    target.diff_state.inline_submodule_diff = Some(InlineSubmoduleDiffState {
        origin: ForeignDiffOrigin::Worktree {
            branch: Some("feature".to_string()),
            detached: false,
        },
        submodule_repo_path: worktree.clone(),
        parent_submodule_path: worktree,
        entries: Vec::new(),
        selected_ix: 0,
        target: inline_target,
        rev: 1,
        diff_rev: 1,
        diff: Loadable::NotLoaded,
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });

    let mut effects = crate::store::reducer::SetActiveRepoEffects::new();
    crate::store::reducer::fill_set_active_repo_inline(
        &repos,
        &mut state,
        target_repo,
        &mut effects,
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert!(target.history_state.worktree_selection.is_none());
    assert!(
        target.diff_state.inline_submodule_diff.is_none(),
        "the production inline activation path must not leave a linked-worktree diff visible after its row selection is reset"
    );
}

#[test]
fn set_active_repo_inline_folds_the_reset_selection_into_navigation_tail() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let target_repo = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let older_commit = CommitId("older".into());
    let stale_commit = CommitId("stale".into());
    let snapshot = crate::store::tests::snapshot_with_commit;
    let target = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    target.history_state.selected_commit = Some(stale_commit.clone());
    target.navigation.main_history.clear();
    target
        .navigation
        .main_history
        .record(snapshot(Some(older_commit.clone())));
    target
        .navigation
        .main_history
        .record(snapshot(Some(stale_commit.clone())));

    let mut effects = crate::store::reducer::SetActiveRepoEffects::new();
    crate::store::reducer::fill_set_active_repo_inline(
        &repos,
        &mut state,
        target_repo,
        &mut effects,
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert!(target.history_state.selected_commit.is_none());
    assert_eq!(target.navigation.main_history.cursor, 1);
    assert_eq!(
        target.navigation.main_history.entries.get(1),
        Some(&snapshot(None)),
        "activation must replace the stale live tail with the reset workspace view"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack {
            repo_id: target_repo,
        },
    );
    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert_eq!(
        target.history_state.selected_commit.as_ref(),
        Some(&older_commit),
        "Back immediately after activation must reach the preceding view"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavForward {
            repo_id: target_repo,
        },
    );
    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert!(
        target.history_state.selected_commit.is_none(),
        "Forward must return to the reset workspace view, not resurrect the stale selection"
    );
    assert_ne!(
        target.history_state.selected_commit.as_ref(),
        Some(&stale_commit)
    );
}

#[test]
fn set_active_repo_inline_realigns_a_mid_stack_reset_before_new_navigation() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let target_repo = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let older_commit = CommitId("older".into());
    let stale_commit = CommitId("stale".into());
    let forward_commit = CommitId("forward".into());
    let new_commit = CommitId("new".into());
    let snapshot = crate::store::tests::snapshot_with_commit;
    let target = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    target.history_state.selected_commit = Some(stale_commit.clone());
    target.navigation.main_history.clear();
    target
        .navigation
        .main_history
        .record(snapshot(Some(older_commit.clone())));
    target
        .navigation
        .main_history
        .record(snapshot(Some(stale_commit.clone())));
    target
        .navigation
        .main_history
        .record(snapshot(Some(forward_commit.clone())));
    assert_eq!(
        target.navigation.main_history.step(ViewNavDir::Back),
        Some(snapshot(Some(stale_commit.clone())))
    );

    let mut effects = crate::store::reducer::SetActiveRepoEffects::new();
    crate::store::reducer::fill_set_active_repo_inline(
        &repos,
        &mut state,
        target_repo,
        &mut effects,
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert!(target.history_state.selected_commit.is_none());
    assert_eq!(target.navigation.main_history.cursor, 1);
    assert_eq!(
        target.navigation.main_history.entries.get(1),
        Some(&snapshot(None))
    );
    assert_eq!(
        target.navigation.main_history.entries.get(2),
        Some(&snapshot(Some(forward_commit))),
        "activation must preserve forward history while replacing the stale current entry"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectCommit {
            repo_id: target_repo,
            commit_id: new_commit,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack {
            repo_id: target_repo,
        },
    );

    let target = state
        .repos
        .iter()
        .find(|repo| repo.id == target_repo)
        .expect("target repo exists");
    assert!(
        target.history_state.selected_commit.is_none(),
        "Back after a new destination must return to the reset activation view"
    );
    assert_ne!(
        target.history_state.selected_commit.as_ref(),
        Some(&stale_commit),
        "the stale mid-stack selection must not survive as the Back origin"
    );
}

#[test]
fn set_active_repo_reloads_cancelled_history_panes_but_resets_commit_selection() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    let history_path = PathBuf::from("src/lib.rs");
    let blame_path = PathBuf::from("src/main.rs");
    let selected_commit = CommitId("deadbeef".into());

    mark_repo_switch_secondary_metadata_ready(
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists"),
    );
    mark_repo_switch_secondary_metadata_ready(
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo2)
            .expect("repo2 exists"),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    {
        let repo1_state = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        repo1_state.history_state.file_history_path = Some(history_path.clone());
        repo1_state.history_state.file_history = Loadable::Loading;
        repo1_state.history_state.blame_path = Some(blame_path.clone());
        repo1_state.history_state.blame_source = Some(
            gitcomet_core::domain::BlameSource::Revision(Some("HEAD~1".to_string())),
        );
        repo1_state.history_state.blame = Loadable::Loading;
        repo1_state.set_selected_commit(Some(selected_commit.clone()));
        repo1_state.set_commit_details(Loadable::Loading);
    }
    let repo1_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists")
        .load_epoch;

    let deactivate_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        has_cancel_repo_loads_effect(&deactivate_effects, repo1, repo1_epoch),
        "expected repo switch to cancel in-flight repo1 loads"
    );
    {
        let repo1_state = state
            .repos
            .iter()
            .find(|repo| repo.id == repo1)
            .expect("repo1 exists");
        assert!(matches!(
            repo1_state.history_state.file_history,
            Loadable::NotLoaded
        ));
        assert!(matches!(
            repo1_state.history_state.blame,
            Loadable::NotLoaded
        ));
        assert!(matches!(
            repo1_state.history_state.commit_details,
            Loadable::NotLoaded
        ));
        assert_eq!(
            repo1_state.history_state.file_history_path.as_ref(),
            Some(&history_path)
        );
        assert_eq!(
            repo1_state.history_state.blame_path.as_ref(),
            Some(&blame_path)
        );
        assert_eq!(
            repo1_state.history_state.selected_commit.as_ref(),
            Some(&selected_commit)
        );
    }

    let reactivate_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(reactivate_effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadFileHistory {
            repo_id,
            path,
            limit: 200,
        } if *repo_id == repo1 && path == &history_path
    )));
    assert!(reactivate_effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadBlame { repo_id, path, source: gitcomet_core::domain::BlameSource::Revision(Some(rev)) }
            if *repo_id == repo1
                && path == &blame_path
                && rev == "HEAD~1"
    )));
    assert!(
        !reactivate_effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadCommitDetails { .. })),
        "a tab switch must not reload the commit selection it just reset"
    );

    let repo1_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    assert!(repo1_state.history_state.file_history.is_loading());
    assert!(repo1_state.history_state.blame.is_loading());
    assert!(repo1_state.history_state.selected_commit.is_none());
    assert!(matches!(
        repo1_state.history_state.commit_details,
        Loadable::NotLoaded
    ));
}

#[test]
fn set_active_repo_reloads_selected_image_diff_via_image_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
            load_submodule_summary: false,
            preview_text_side: None,
        } if *repo_id == repo1
    )));
}

#[test]
fn set_active_repo_png_diff_enqueues_image_preview_only() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
        !has_full_refresh_only_effects(&effects, repo1),
        "hot repo switches with ready metadata should stay on the primary refresh path"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBranches { repo_id } if *repo_id == repo1)),
        "expected local branches refresh on activation"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh on activation"
    );
    assert!(has_status_refresh_effects(&effects, repo1));
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    let repo1 = RepoId(1);
    let repo1_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo1)
        .expect("repo1 exists");
    mark_repo_switch_secondary_metadata_ready(repo1_state);
    repo1_state.remotes = Loadable::NotLoaded;
    repo1_state.last_active_at = Some(SystemTime::now());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    assert!(
        has_full_refresh_only_effects(&effects, repo1),
        "missing secondary metadata should force the full refresh path"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh even on the full refresh path"
    );
}

#[test]
fn set_active_repo_uses_full_refresh_when_hot_switch_window_expires() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

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
        has_full_refresh_only_effects(&effects, repo1),
        "stale repo switches should fall back to the full refresh path"
    );
    assert!(
        has_worktree_refresh_effect(&effects, repo1),
        "expected worktrees refresh even when the hot-switch window expires"
    );
}

#[test]
fn stale_status_result_after_repo_action_finished_is_dropped() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo");
    let repo_state = state
        .repos
        .iter_mut()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    repo_state.set_status(Loadable::Loading);
    assert!(
        repo_state
            .loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_STATUS)
    );
    let old_epoch = repo_state.load_epoch;

    // The action completes and bumps the epoch, invalidating the in-flight status load.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    // The stale (pre-action) status result then arrives stamped with the old epoch.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch: old_epoch,
            message: Box::new(crate::msg::InternalMsg::StatusLoaded {
                repo_id,
                result: Ok(RepoStatus::default()),
            }),
        }),
    );

    let repo_state = state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .expect("repo exists");
    // It is dropped by the epoch gate: no effects, and it does not clobber the reset status ...
    assert!(effects.is_empty());
    assert!(matches!(repo_state.status, Loadable::NotLoaded));
    assert_ne!(repo_state.load_epoch, old_epoch);
    // ... while the fresh post-action status load is live (its flag belongs to the new epoch).
    assert!(
        repo_state
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS)
    );
}

#[test]
fn set_active_repo_ignores_unknown_repo() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
fn set_active_repo_loads_file_browser_when_files_mode_is_active() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    mark_repo_open_ready(&mut repos, &mut state, repo1);
    mark_repo_open_ready(&mut repos, &mut state, repo2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSidebarMode {
            mode: SidebarMode::Files,
        },
    );

    // Activating a repo whose listing never loaded must kick the file
    // browser while the Files sidebar is showing, or the tree is stuck on
    // "Loading files..." until the user toggles the sidebar tabs.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser { repo_id, .. } if *repo_id == repo2
        )),
        "expected activation to load the file browser, got {effects:?}"
    );

    // An already-loaded listing must not reload on every activation.
    state
        .repos
        .iter_mut()
        .find(|r| r.id == repo1)
        .expect("repo1 exists")
        .file_browser
        .entries = Loadable::Ready(Arc::new(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadFileBrowser { .. })),
        "expected no file browser reload for a loaded listing, got {effects:?}"
    );
}

#[test]
fn set_active_repo_skips_file_browser_load_in_branches_mode() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    mark_repo_open_ready(&mut repos, &mut state, repo1);
    mark_repo_open_ready(&mut repos, &mut state, repo2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo1 },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadFileBrowser { .. })),
        "expected no file browser load while the Branches sidebar is showing"
    );
}
