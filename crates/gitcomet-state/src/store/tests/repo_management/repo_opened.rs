use super::*;

#[test]
fn set_fetch_prune_deleted_remote_tracking_branches_updates_and_noops() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    assert!(repo_state.worktree_status_is_loading());
    assert!(repo_state.staged_status_is_loading());
    assert!(repo_state.log.is_loading());
    assert!(matches!(repo_state.stashes, Loadable::NotLoaded));
    assert!(matches!(repo_state.reflog, Loadable::NotLoaded));
    assert!(repo_state.upstream_divergence.is_loading());
    assert!(repo_state.rebase_in_progress.is_loading());
    assert!(repo_state.merge_commit_message.is_loading());
    assert!(repo_state.worktrees.is_loading());
    assert!(repo_state.submodules.is_loading());
    assert!(matches!(
        repo_state.history_state.file_history,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.history_state.blame,
        Loadable::NotLoaded
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadHeadBranch { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadUpstreamDivergence {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadLog { repo_id: candidate, .. } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadBranches { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(effect, Effect::LoadRemotes { repo_id: candidate } if *candidate == repo_id)
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadRemoteBranches {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_effect_for_repo(
        &effects,
        RepoId(1),
        |effect, repo_id| {
            matches!(
                effect,
                Effect::LoadRebaseAndMergeState {
                    repo_id: candidate
                } if *candidate == repo_id
            )
        }
    ));
    assert!(has_worktree_refresh_effect(&effects, RepoId(1)));
    assert!(has_submodule_load_effect(&effects, RepoId(1)));
}

#[test]
fn repo_opened_ok_auto_loads_tags_when_enabled() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState {
        git_log_settings: GitLogSettings {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
        },
        ..AppState::default()
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

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
    assert!(repo_state.tags.is_loading());
    assert!(repo_state.remote_tags.is_loading());
    assert!(repo_state.submodules.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSubmodules { repo_id } if *repo_id == RepoId(1)
    )));
}

#[test]
fn repo_opened_ok_for_closed_repo_is_ignored() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(1) },
    );

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

    assert!(effects.is_empty());
    assert!(state.repos.is_empty());
    assert!(!repos.contains_key(&RepoId(1)));
}

#[test]
fn repo_opened_err_for_closed_repo_is_ignored() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
        Msg::CloseRepo { repo_id: RepoId(1) },
    );

    let effects = reduce(
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

    assert!(effects.is_empty());
    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert!(
        state.notifications.is_empty(),
        "stale open errors for a closed repo must not surface notifications"
    );
    assert!(!repos.contains_key(&RepoId(1)));
}

#[test]
fn repo_action_finished_clears_error_and_refreshes() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
            action: RepoActionKind::CheckoutBranch,
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].last_error.is_none());
    assert!(state.banner_error.is_none());
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
}

#[test]
fn repo_action_finished_err_records_diagnostic() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
            action: RepoActionKind::CheckoutBranch,
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
fn cherry_pick_error_completion_refreshes_status_log_and_sequencer_state() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].local_actions_in_flight = 1;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::CherryPickCommit,
            result: Err(Error::new(ErrorKind::Backend("conflict".to_string()))),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(
        state.repos[0]
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("conflict"))
    );
    assert!(
        has_status_refresh_effects(&effects, repo_id),
        "cherry-pick errors should refresh status so conflicts are visible, got {effects:?}"
    );
    assert!(
        effects.iter().any(
            |effect| matches!(effect, Effect::LoadLog { repo_id: candidate, .. } if *candidate == repo_id)
        ),
        "cherry-pick errors should refresh the log, got {effects:?}"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadRebaseAndMergeState { repo_id: candidate } if *candidate == repo_id
        )),
        "cherry-pick errors should refresh merge/rebase/cherry-pick state, got {effects:?}"
    );
}

#[test]
fn repo_action_finished_bumps_load_epoch_and_forces_fresh_status_load_when_stale_in_flight() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STAGED_STATUS);
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        state.repos[0].load_epoch > old_epoch,
        "load_epoch should be bumped to invalidate stale load results"
    );
    assert!(
        has_status_refresh_effects(&effects, repo_id),
        "a fresh status load should be dispatched even when a stale one was in-flight"
    );
    assert!(
        has_cancel_repo_loads_effect(&effects, repo_id, old_epoch),
        "the stale in-flight loads should be cancelled at the pre-bump epoch"
    );
}

#[test]
fn repo_action_finished_reissues_inflight_non_status_loads() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // A primary refresh plus a branch refresh are in flight when the action completes. The epoch
    // bump invalidates all of them, so they must be re-issued, not left stuck in `in_flight`.
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STAGED_STATUS);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::HEAD_BRANCH);
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES);
    state.repos[0].branches = Loadable::Loading;
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].load_epoch > old_epoch);
    assert!(has_cancel_repo_loads_effect(&effects, repo_id, old_epoch));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { repo_id: r } if *r == repo_id)),
        "head branch should be re-loaded, not stranded in flight"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id: r } if *r == repo_id)),
        "branch list should be re-loaded, not stranded in flight"
    );
    assert!(has_status_refresh_effects(&effects, repo_id));
}

#[test]
fn repo_action_finished_reissues_inflight_sidebar_data_loads() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].open = Loadable::Ready(());

    state.repos[0].set_sidebar_data_request(SidebarDataRequest {
        worktrees: true,
        submodules: true,
        stashes: true,
    });

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREES);
    state.repos[0].worktrees = Loadable::Loading;

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::SUBMODULES);
    state.repos[0].submodules = Loadable::Loading;

    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::STASHES);
    state.repos[0].stashes = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        has_worktree_refresh_effect(&effects, repo_id),
        "worktrees should be re-loaded after a repo action, not stranded in NotLoaded"
    );
    assert!(
        has_submodule_load_effect(&effects, repo_id),
        "submodules should be re-loaded after a repo action, not stranded in NotLoaded"
    );
    assert!(
        has_stash_load_effect(&effects, repo_id),
        "stashes should be re-loaded after a repo action, not stranded in NotLoaded"
    );
}

#[test]
fn repo_action_finished_reissues_inflight_blame_and_commit_details() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // The user has a blame and a commit-details view open and still loading.
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/main.rs"));
    state.repos[0].history_state.blame_source = Some(gitcomet_core::domain::BlameSource::Revision(
        Some("HEAD".to_string()),
    ));
    state.repos[0].history_state.blame = Loadable::Loading;
    state.repos[0].history_state.selected_commit = Some(CommitId("abc123".into()));
    state.repos[0].history_state.commit_details = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        state.repos[0].history_state.blame.is_loading(),
        "blame should be reset and re-loaded, not stranded on a spinner"
    );
    assert!(
        state.repos[0].history_state.commit_details.is_loading(),
        "commit details should be reset and re-loaded, not stranded on a spinner"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBlame { repo_id: r, .. } if *r == repo_id)),
        "a fresh blame load should be dispatched"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadCommitDetails { repo_id: r, .. } if *r == repo_id)),
        "a fresh commit-details load should be dispatched"
    );
}

#[test]
fn repo_action_finished_reissues_selected_commit_diff() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // A historical commit's diff (a non-WorkingTree target) is open and loading. The old code only
    // re-issued WorkingTree diffs, leaving this one stranded.
    state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: CommitId("abc123".into()),
        path: Some(PathBuf::from("src/main.rs")),
    });
    state.repos[0].diff_state.diff = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: r,
                target: DiffTarget::Commit { .. }
            } if *r == repo_id
        )),
        "a commit diff in flight should be re-loaded when its action completes"
    );
}

#[test]
fn repo_action_finished_invalidates_but_does_not_reissue_views_for_non_active_repo() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let background = RepoId(1);
    let active = RepoId(2);
    state.repos.push(RepoState::new_opening(
        background,
        RepoSpec {
            workdir: PathBuf::from("/tmp/bg"),
        },
    ));
    state.repos.push(RepoState::new_opening(
        active,
        RepoSpec {
            workdir: PathBuf::from("/tmp/active"),
        },
    ));
    state.active_repo = Some(active);

    // The background repo had a branch load and a blame load in flight when its action completed.
    state.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::BRANCHES);
    state.repos[0].branches = Loadable::Loading;
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/main.rs"));
    state.repos[0].history_state.blame_source = Some(gitcomet_core::domain::BlameSource::Revision(
        Some("HEAD".to_string()),
    ));
    state.repos[0].history_state.blame = Loadable::Loading;
    let old_epoch = state.repos[0].load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: background,
            action: RepoActionKind::StagePaths,
            result: Ok(()),
        }),
    );

    // The background repo's stale loads are still invalidated, so nothing is left stranded ...
    assert!(state.repos[0].load_epoch > old_epoch);
    assert!(has_cancel_repo_loads_effect(
        &effects, background, old_epoch
    ));
    assert!(matches!(state.repos[0].branches, Loadable::NotLoaded));
    assert!(matches!(
        state.repos[0].history_state.blame,
        Loadable::NotLoaded
    ));
    // ... but its view-specific data is not eagerly re-issued; it reloads when next activated.
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBlame { repo_id: r, .. } if *r == background)),
        "a non-active repo should not eagerly re-load blame"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id: r } if *r == background)),
        "a non-active repo should not eagerly re-load its branch list"
    );
}

#[test]
fn repo_opened_err_records_diagnostic() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let invalid_repo = PathBuf::from("/tmp/not-a-repo");
    let normalized_invalid_repo = crate::store::reducer::normalize_repo_path(invalid_repo.clone());
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(invalid_repo.clone()),
    );

    let error = Error::new(ErrorKind::NotARepository);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: invalid_repo.clone(),
            },
            error,
        }),
    );

    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert!(state.notifications.iter().any(|notification| {
        notification.kind == AppNotificationKind::Warning
            && notification.message
                == format!(
                    "No valid Git repository was found at {}.",
                    normalized_invalid_repo.display()
                )
    }));
}

#[test]
fn repo_opened_err_not_a_repository_opens_restored_fallback_tab() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let invalid_repo = dir.path().join("invalid");
    let fallback_repo = dir.path().join("fallback");
    std::fs::create_dir_all(&invalid_repo).expect("create invalid repo dir");
    std::fs::create_dir_all(&fallback_repo).expect("create fallback repo dir");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![invalid_repo.clone(), fallback_repo],
            active_repo: Some(invalid_repo.clone()),
        },
    );
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(matches!(state.repos[1].open, Loadable::NotLoaded));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: invalid_repo,
            },
            error: Error::new(ErrorKind::NotARepository),
        }),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(2)));
    assert_eq!(state.repos[0].id, RepoId(2));
    assert!(matches!(state.repos[0].open, Loadable::Loading));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)
    )));
}

#[test]
fn repo_opened_err_not_a_repository_allows_opening_another_repo_afterwards() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
fn repo_opened_ok_loads_file_browser_for_active_repo_in_files_mode() {
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

    // The repo was activated before its open completed; the open completing
    // must kick the file browser listing for the Files sidebar.
    let spec = state.repos[0].spec.clone();
    let workdir = spec.workdir.to_string_lossy().into_owned();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: repo1,
            spec,
            repo: Arc::new(DummyRepo::new(&workdir)),
        }),
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadFileBrowser { repo_id, .. } if *repo_id == repo1
        )),
        "expected RepoOpenedOk to load the file browser, got {effects:?}"
    );
}
