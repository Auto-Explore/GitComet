use super::*;

fn test_force_push_lease() -> gitcomet_core::services::ForcePushLease {
    gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: CommitId("1111111111111111111111111111111111111111".into()),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    }
}

fn test_recent_commit_message() -> gitcomet_core::domain::RecentCommitMessage {
    test_recent_commit_message_with_summary(
        "1111111111111111111111111111111111111111",
        "old message",
    )
}

fn test_recent_commit_message_with_summary(
    id: &str,
    summary: &str,
) -> gitcomet_core::domain::RecentCommitMessage {
    gitcomet_core::domain::RecentCommitMessage {
        id: CommitId(id.into()),
        summary: Arc::from(summary),
        message: format!("{summary}\n\nbody"),
    }
}

fn repo_with_head_dependent_cached_state(repo_id: RepoId) -> RepoState {
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.pending_force_push_lease = Some(test_force_push_lease());
    repo_state.set_recent_commit_messages(Loadable::Ready(vec![test_recent_commit_message()]));
    repo_state
}

#[test]
fn pull_and_push_mark_in_flight_until_command_finished() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    let workdir = PathBuf::from("/tmp/repo");
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.clone(),
        },
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);

    reduce(&mut repos, &id_alloc, &mut state, Msg::FetchAll { repo_id });
    assert_eq!(state.repos[0].pull_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneMergedBranches { repo_id },
    );
    assert_eq!(state.repos[0].pull_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneLocalTags { repo_id },
    );
    assert_eq!(state.repos[0].pull_in_flight, 4);

    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert_eq!(state.repos[0].push_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushTag {
            repo_id,
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteTag {
            repo_id,
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 4);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "main".to_string(),
        },
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchAll,
            result: Ok(CommandOutput::empty_success("git fetch --all")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Ok(CommandOutput::empty_success("git pull")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneMergedBranches,
            result: Ok(CommandOutput::empty_success("git prune merged branches")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneLocalTags,
            result: Ok(CommandOutput::empty_success("git prune local tags")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 0);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Ok(CommandOutput::empty_success("git push")),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteRemoteBranch {
                remote: "origin".to_string(),
                branch: "feature".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin --delete feature",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PushTag {
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin refs/tags/v1.0.0",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteRemoteTag {
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin --delete refs/tags/v1.0.0",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 0);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UnsetUpstreamBranch {
                branch: "main".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git branch --unset-upstream main",
            )),
        }),
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn pull_and_push_do_not_mark_in_flight_before_repo_is_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    reduce(&mut repos, &id_alloc, &mut state, Msg::FetchAll { repo_id });
    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });

    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert_eq!(state.repos[0].push_in_flight, 0);
}

#[test]
fn pull_error_is_formatted_as_command_and_output() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let message = "git pull --no-rebase origin main failed: From https://example.com\n * branch main -> FETCH_HEAD\nfatal: refusing to merge unrelated histories".to_string();
    let error = Error::new(ErrorKind::Backend(message));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PullBranch {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            },
            result: Err(error),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.diagnostics.is_empty());
    assert_eq!(repo_state.command_log.len(), 1);

    let summary = &repo_state.command_log[0].summary;
    assert!(summary.starts_with("Pull failed:\n\n    git pull --no-rebase origin main"));
    assert!(summary.contains(
        "\n\n    From https://example.com\n     * branch main -> FETCH_HEAD\n    fatal: refusing to merge unrelated histories"
    ));
    assert!(!summary.contains("\\n"));
    assert_eq!(repo_state.last_error.as_deref(), Some(summary.as_str()));
}

#[test]
fn fetch_all_emits_effect_with_repo_prune_setting() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.fetch_prune_deleted_remote_tracking_branches = false;
    state.repos.push(repo_state);

    let fetch_without_prune = reduce(&mut repos, &id_alloc, &mut state, Msg::FetchAll { repo_id });
    assert!(matches!(
        fetch_without_prune.as_slice(),
        [Effect::FetchAll {
            repo_id: RepoId(1),
            prune: false,
            ..
        }]
    ));
    assert_eq!(state.repos[0].pull_in_flight, 1);

    state.repos[0].fetch_prune_deleted_remote_tracking_branches = true;
    let fetch_with_prune = reduce(&mut repos, &id_alloc, &mut state, Msg::FetchAll { repo_id });
    assert!(matches!(
        fetch_with_prune.as_slice(),
        [Effect::FetchAll {
            repo_id: RepoId(1),
            prune: true,
            ..
        }]
    ));
    assert_eq!(state.repos[0].pull_in_flight, 2);
}

#[test]
fn commit_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Commit {
            repo_id: RepoId(1),
            message: "hello".to_string(),
            push_after_commit: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::Commit { repo_id: RepoId(1), message, .. } ] if message == "hello"
    ));
}

#[test]
fn checkout_conflict_base_emits_effect() {
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

    let path = PathBuf::from("conflicted.bin");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutConflictBase {
            repo_id: RepoId(1),
            path: path.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutConflictBase { repo_id: RepoId(1), path: effect_path }] if effect_path == &path
    ));
}

#[test]
fn accept_conflict_deletion_emits_effect() {
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

    let path = PathBuf::from("conflicted.bin");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcceptConflictDeletion {
            repo_id: RepoId(1),
            path: path.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::AcceptConflictDeletion { repo_id: RepoId(1), path: effect_path }] if effect_path == &path
    ));
}

#[test]
fn reset_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Reset {
            repo_id: RepoId(1),
            target: "HEAD~1".to_string(),
            mode: gitcomet_core::services::ResetMode::Hard,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::Reset { repo_id: RepoId(1), target, mode: gitcomet_core::services::ResetMode::Hard }]
            if target == "HEAD~1"
    ));
}

#[test]
fn revert_commit_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevertCommit {
            repo_id: RepoId(1),
            commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RevertCommit {
            repo_id: RepoId(1),
            commit_id: _
        }]
    ));
}

#[test]
fn commit_amend_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CommitAmend {
            repo_id: RepoId(1),
            message: "amended".to_string(),
            push_after_commit: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CommitAmend { repo_id: RepoId(1), message, .. }] if message == "amended"
    ));
}

#[test]
fn worktree_commands_reload_worktrees_on_success() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_worktrees(Loadable::Ready(Vec::new()));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddWorktree {
                path: PathBuf::from("/tmp/worktree"),
                reference: None,
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree add /tmp/worktree",
            )),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id: id } if *id == repo_id))
    );

    state.repos[0].set_worktrees(Loadable::Ready(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree remove /tmp/worktree",
            )),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id: id } if *id == repo_id))
    );
}

#[test]
fn worktree_remove_closes_tab_for_removed_worktree() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos.push(RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: PathBuf::from("/tmp/worktree"),
        },
    ));
    state.active_repo = Some(RepoId(2));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree remove /tmp/worktree",
            )),
        }),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.repos[0].id, RepoId(1));
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id } if *repo_id == RepoId(1)))
    );
}

#[test]
fn submodule_commands_reload_submodules_on_success() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    state.repos[0].submodule_add_in_flight = Some(crate::model::SubmoduleAddProgressState {
        url: "https://example.com/sub.git".to_string(),
        path: PathBuf::from("submodule"),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("submodule"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            result: Ok(CommandOutput::empty_success("git submodule add")),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(state.repos[0].submodule_add_in_flight.is_none());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .finish(crate::model::RepoLoadsInFlight::SUBMODULES)
    );

    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            result: Ok(CommandOutput::empty_success(
                "git submodule update --init --recursive",
            )),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .finish(crate::model::RepoLoadsInFlight::SUBMODULES)
    );

    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("submodule"),
            },
            result: Ok(CommandOutput::empty_success(
                "git submodule deinit -f submodule",
            )),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
}

#[test]
fn selected_submodule_command_reloads_selected_summary() {
    use gitcomet_core::domain::{
        Diff, FileStatus, FileStatusKind, Submodule, SubmoduleDiffSummary,
        SubmoduleDiffSummaryMode, SubmoduleStatus,
    };

    fn seeded_state(
        repo_id: RepoId,
        command_path: &std::path::Path,
    ) -> (AppState, gitcomet_core::domain::DiffTarget) {
        let mut state = AppState::default();
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
            unstaged: vec![FileStatus {
                path: command_path.to_path_buf(),
                kind: FileStatusKind::Modified,
                conflict: None,
            }],
            staged: Vec::new(),
        })));
        repo.set_submodules(Loadable::Ready(vec![Submodule {
            path: command_path.to_path_buf(),
            recorded_head: CommitId("old-recorded".into()),
            checked_out_head: None,
            status: SubmoduleStatus::NotInitialized,
        }]));
        let target = DiffTarget::WorkingTree {
            path: command_path.to_path_buf(),
            area: DiffArea::Unstaged,
        };
        repo.diff_state.diff_target = Some(target.clone());
        repo.diff_state.submodule_summary = Loadable::Ready(Arc::new(SubmoduleDiffSummary {
            path: command_path.to_path_buf(),
            mode: SubmoduleDiffSummaryMode::Worktree,
            status: Some(SubmoduleStatus::NotInitialized),
            commit_id: None,
            parent_commit_id: None,
            checked_out_head: None,
            ranges: Vec::new(),
            live_staged: Vec::new(),
            live_unstaged: Vec::new(),
        }));
        let inline_target = DiffTarget::WorkingTree {
            path: PathBuf::from("inner.rs"),
            area: DiffArea::Unstaged,
        };
        repo.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/lib"),
            parent_submodule_path: command_path.to_path_buf(),
            entries: vec![crate::model::InlineSubmoduleDiffEntry {
                path: PathBuf::from("inner.rs"),
                kind: FileStatusKind::Modified,
                target: inline_target.clone(),
                section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
            }],
            selected_ix: 0,
            target: inline_target,
            rev: 1,
            diff: Loadable::Ready(Arc::new(Diff {
                target: target.clone(),
                lines: Vec::new(),
            })),
            diff_rev: 1,
            diff_file_rev: 1,
            diff_file: Loadable::NotLoaded,
            diff_file_image: Loadable::NotLoaded,
        });
        state.repos.push(repo);
        (state, target)
    }

    let repo_id = RepoId(1);
    let path = PathBuf::from("vendor/lib");
    for command in [
        RepoCommandKind::LoadSubmodule {
            path: path.clone(),
            approved_sources: Vec::new(),
        },
        RepoCommandKind::UpdateSubmodules {
            approved_sources: Vec::new(),
        },
        RepoCommandKind::ChangeSubmodulePointer {
            path: path.clone(),
            reference: "main".to_string(),
        },
    ] {
        let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
        let id_alloc = AtomicU64::new(1);
        let (mut state, target) = seeded_state(repo_id, path.as_path());

        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git submodule command")),
            }),
        );

        assert!(state.repos[0].diff_state.submodule_summary.is_loading());
        assert!(state.repos[0].diff_state.inline_submodule_diff.is_none());
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmoduleSummary {
                repo_id: id,
                target: effect_target,
            } if *id == repo_id && *effect_target == target
        )));
    }
}

#[test]
fn merge_ref_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MergeRef {
            repo_id: RepoId(1),
            reference: "feature".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::MergeRef { repo_id: RepoId(1), reference }] if reference == "feature"
    ));
}

#[test]
fn squash_ref_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SquashRef {
            repo_id: RepoId(1),
            reference: "feature".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::SquashRef { repo_id: RepoId(1), reference }] if reference == "feature"
    ));
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
}

#[test]
fn rebase_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Rebase {
            repo_id: RepoId(1),
            onto: "master".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::Rebase { repo_id: RepoId(1), onto }] if onto == "master"
    ));
}

#[test]
fn create_rename_and_delete_branch_emit_effects() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
            target: "HEAD".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranch {
            repo_id: RepoId(1),
            name,
            target,
        }] if name == "feature" && target == "HEAD"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RenameBranch {
            repo_id: RepoId(1),
            old_name: "feature".to_string(),
            new_name: "renamed-feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RenameBranch {
            repo_id: RepoId(1),
            old_name,
            new_name,
        }] if old_name == "feature" && new_name == "renamed-feature"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteBranch { repo_id: RepoId(1), name }] if name == "feature"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForceDeleteBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ForceDeleteBranch { repo_id: RepoId(1), name }] if name == "feature"
    ));
}

#[test]
fn create_and_delete_tag_emit_effects() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
            target: "HEAD".to_string(),
            message: None,
            annotated: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateTag { repo_id: RepoId(1), name, target, message: None, annotated: false }] if name == "v1.0.0" && target == "HEAD"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteTag { repo_id: RepoId(1), name }] if name == "v1.0.0"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushTag {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::PushTag { repo_id: RepoId(1), remote, name, .. }] if remote == "origin" && name == "v1.0.0"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteTag {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteRemoteTag { repo_id: RepoId(1), remote, name, .. }] if remote == "origin" && name == "v1.0.0"
    ));
}

#[test]
fn apply_pop_and_drop_stash_emit_effects() {
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

    let apply = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        apply.as_slice(),
        [Effect::ApplyStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));

    let pop = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PopStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        pop.as_slice(),
        [Effect::PopStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));

    let drop = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DropStash {
            repo_id: RepoId(1),
            index: 0,
        },
    );
    assert!(matches!(
        drop.as_slice(),
        [Effect::DropStash {
            repo_id: RepoId(1),
            index: 0
        }]
    ));
}

#[test]
fn checkout_commit_emits_effect() {
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

    let commit_id = gitcomet_core::domain::CommitId("deadbeef".into());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutCommit {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutCommit {
            repo_id: RepoId(1),
            commit_id: _
        }]
    ));

    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo should exist");
    assert_eq!(repo.detached_head_commit, Some(commit_id));
}

#[test]
fn discard_worktree_changes_path_emits_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DiscardWorktreeChangesPath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::DiscardWorktreeChangesPath {
            repo_id: RepoId(1),
            path: _
        }]
    ));
}

#[test]
fn repo_operations_emit_effects() {
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

    let stage = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StagePath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );
    assert!(matches!(
        stage.as_slice(),
        [Effect::StagePath {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let unstage = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnstagePath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );
    assert!(matches!(
        unstage.as_slice(),
        [Effect::UnstagePath {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let commit = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Commit {
            repo_id: RepoId(1),
            message: "m".to_string(),
            push_after_commit: false,
        },
    );
    assert!(matches!(
        commit.as_slice(),
        [Effect::Commit {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let pull = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id: RepoId(1),
            mode: PullMode::Rebase,
        },
    );
    assert!(matches!(
        pull.as_slice(),
        [Effect::Pull {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let prune_branches = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneMergedBranches { repo_id: RepoId(1) },
    );
    assert!(matches!(
        prune_branches.as_slice(),
        [Effect::PruneMergedBranches { repo_id: RepoId(1) }]
    ));

    let prune_tags = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneLocalTags { repo_id: RepoId(1) },
    );
    assert!(matches!(
        prune_tags.as_slice(),
        [Effect::PruneLocalTags { repo_id: RepoId(1) }]
    ));

    let push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Push { repo_id: RepoId(1) },
    );
    assert!(matches!(
        push.as_slice(),
        [Effect::Push {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let force_push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id: RepoId(1) },
    );
    assert!(matches!(
        force_push.as_slice(),
        [Effect::ForcePush {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let push_set_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id: RepoId(1),
            remote: "origin".to_string(),
            branch: "feature/foo".to_string(),
        },
    );
    assert!(matches!(
        push_set_upstream.as_slice(),
        [Effect::PushSetUpstream {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let set_upstream_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch: "feature/local".to_string(),
            upstream: "origin/feature/foo".to_string(),
        },
    );
    assert!(matches!(
        set_upstream_branch.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        }] if branch == "feature/local" && upstream == "origin/feature/foo"
    ));

    let unset_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch: "feature/foo".to_string(),
        },
    );
    assert!(matches!(
        unset_upstream.as_slice(),
        [Effect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        }] if branch == "feature/foo"
    ));

    let stash = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Stash {
            repo_id: RepoId(1),
            message: "wip".to_string(),
            include_untracked: true,
        },
    );
    assert!(matches!(
        stash.as_slice(),
        [Effect::Stash {
            repo_id: RepoId(1),
            ..
        }]
    ));
}

// --- Revision counter regression tests ---

#[test]
fn pull_push_bump_ops_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let ops_before = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after Pull"
    );
    let ops_after_pull = state.repos[0].ops_rev;

    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert!(
        state.repos[0].ops_rev > ops_after_pull,
        "ops_rev should bump after Push"
    );
    let ops_after_push = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Ok(CommandOutput::empty_success("git pull")),
        }),
    );
    assert!(
        state.repos[0].ops_rev > ops_after_push,
        "ops_rev should bump when command finishes"
    );
}

#[test]
fn pull_branch_and_extended_push_commands_bump_in_flight_and_ops_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let ops_before = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PullBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after PullBranch"
    );
    let ops_after_pull_branch = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id },
    );
    assert_eq!(state.repos[0].push_in_flight, 1);
    assert!(
        state.repos[0].ops_rev > ops_after_pull_branch,
        "ops_rev should bump after ForcePush"
    );
    let ops_after_force_push = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature/test".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_force_push,
        "ops_rev should bump after PushSetUpstream"
    );
    let ops_after_push_set_upstream = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/test".to_string(),
            upstream: "origin/feature/test".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_push_set_upstream,
        "ops_rev should bump after SetUpstreamBranch"
    );
    let ops_after_set_upstream_branch = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/test".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_set_upstream_branch,
        "ops_rev should bump after UnsetUpstreamBranch"
    );
}

#[test]
fn commit_bumps_ops_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let ops_before = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Commit {
            repo_id,
            message: "test commit".to_string(),
            push_after_commit: false,
        },
    );
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after Commit"
    );
}

#[test]
fn pull_push_do_not_bump_unrelated_revs() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let status_before = state.repos[0].status_rev;
    let log_before = state.repos[0].history_state.log_rev;
    let selected_before = state.repos[0].history_state.selected_commit_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );

    assert_eq!(state.repos[0].status_rev, status_before);
    assert_eq!(state.repos[0].history_state.log_rev, log_before);
    assert_eq!(
        state.repos[0].history_state.selected_commit_rev,
        selected_before
    );
}

#[test]
fn commit_finished_clears_commit_state_and_requests_primary_refreshes() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("README.md"),
        area: DiffArea::Unstaged,
    });
    state.repos[0].diff_state.diff = Loadable::Loading;
    state.repos[0].diff_state.diff_file = Loadable::Loading;
    state.repos[0].diff_state.diff_file_image = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    let expected_scope = state.repos[0].history_state.history_scope;

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].commit_in_flight, 0);
    assert_eq!(state.repos[0].diff_state.diff_target, None);
    assert!(matches!(
        state.repos[0].diff_state.diff,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state.repos[0].diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { repo_id: id } if *id == repo_id))
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadLog {
            repo_id: id,
            scope,
            ..
        } if *id == repo_id && *scope == expected_scope
    )));
}

#[test]
fn repo_command_finished_stage_hunk_triggers_diff_reload_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(state.repos[0].diff_state.diff.is_loading());
    assert!(state.repos[0].diff_state.diff_file.is_loading());
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("src/lib.rs")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("src/lib.rs")
    )));
}

fn ready_working_tree_blame() -> Loadable<std::sync::Arc<Vec<gitcomet_core::services::BlameLine>>> {
    Loadable::Ready(std::sync::Arc::new(vec![
        gitcomet_core::services::BlameLine {
            commit_id: Arc::from("1111111111111111111111111111111111111111"),
            author: Arc::from("Ada"),
            author_time_unix: Some(1_700_000_000),
            summary: Arc::from("initial"),
            body: None,
            line: "let x = 1;".to_string(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        },
    ]))
}

#[test]
fn repo_command_finished_stage_hunk_invalidates_loaded_blame() {
    // Regression: staging recomputes the diff, and the blame annotation column is
    // derived from the same content. Leaving blame `Ready` would make the view
    // skip a reload (same target, already attempted) and paint stale attribution.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    state.repos[0].history_state.blame_source = Some(
        gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged),
    );
    state.repos[0].history_state.blame = ready_working_tree_blame();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert!(
        matches!(state.repos[0].history_state.blame, Loadable::NotLoaded),
        "blame must be invalidated so the annotation column reloads after staging"
    );
    // The target is preserved so the reload re-blames the same file/source.
    assert_eq!(
        state.repos[0].history_state.blame_path.as_deref(),
        Some(std::path::Path::new("src/lib.rs"))
    );
    assert_eq!(
        state.repos[0].history_state.blame_source,
        Some(gitcomet_core::domain::BlameSource::WorkingTree(
            DiffArea::Unstaged
        ))
    );
}

#[test]
fn commit_finished_invalidates_loaded_blame() {
    // Regression: committing changes which lines are committed, so a stale blame
    // would mislabel them. Blame must be dropped along with the diff.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    state.repos[0].history_state.blame_source = Some(
        gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Staged),
    );
    state.repos[0].history_state.blame = ready_working_tree_blame();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );

    assert!(
        matches!(state.repos[0].history_state.blame, Loadable::NotLoaded),
        "blame must be invalidated after a commit so the annotation column reloads"
    );
}

#[test]
fn repo_command_finished_stage_hunk_with_svg_diff_triggers_text_and_image_reload_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: DiffArea::Unstaged,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(state.repos[0].diff_state.diff.is_loading());
    assert!(state.repos[0].diff_state.diff_file.is_loading());
    assert!(state.repos[0].diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFileImage {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: id,
            target: DiffTarget::WorkingTree { path, .. },
        } if *id == repo_id && path == &PathBuf::from("icon.svg")
    )));
}

#[test]
fn additional_routing_messages_emit_effects_and_update_counters() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyWorktreePatch {
            repo_id,
            patch: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
            reverse: true,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ApplyWorktreePatch {
            repo_id: RepoId(1),
            reverse: true,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
            local_branch: "feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutRemoteBranch {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CherryPickCommit {
            repo_id,
            commit_id: CommitId("deadbeef".into()),
            commit: true,
            mainline: Some(2),
            summary: "pick me".into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CherryPickCommit {
            repo_id: RepoId(1),
            commit: true,
            mainline: Some(2),
            summary,
            ..
        }] if summary == "pick me"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateBranchAndCheckout {
            repo_id,
            name: "feature/new".to_string(),
            target: "HEAD".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranchAndCheckout {
            repo_id: RepoId(1),
            target,
            ..
        }] if target == "HEAD"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::StagePaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")].into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::StagePaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnstagePaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")].into(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::UnstagePaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DiscardWorktreeChangesPaths {
            repo_id,
            paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DiscardWorktreeChangesPaths {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
            upstream: "origin/feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        }] if branch == "feature/current" && upstream == "origin/feature/current"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        }] if branch == "feature/current"
    ));

    assert_eq!(
        state.repos[0].local_actions_in_flight, 9,
        "expected begin_local_action for all routed local-action messages"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExportPatch {
            repo_id,
            commit_id: CommitId("cafebabe".into()),
            dest: PathBuf::from("out.patch"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ExportPatch {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyPatch {
            repo_id,
            patch: PathBuf::from("input.patch"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ApplyPatch {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddWorktree {
            repo_id,
            path: PathBuf::from("/tmp/worktree"),
            reference: Some("main".to_string()),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::AddWorktree {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert_eq!(state.repos[0].worktrees_in_flight, 1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveWorktree {
            repo_id,
            path: PathBuf::from("nested/worktree"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RemoveWorktree {
            repo_id: RepoId(1),
            path
        }] if path == &PathBuf::from("/tmp/repo/nested/worktree")
    ));
    assert_eq!(state.repos[0].worktrees_in_flight, 2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SaveWorktreeFile {
            repo_id,
            path: PathBuf::from("src/lib.rs"),
            contents: "fn main() {}".to_string(),
            stage: true,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveWorktreeFile {
            repo_id: RepoId(1),
            stage: true,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AppendGitignorePatterns {
            repo_id,
            patterns: vec!["/build/out.log".to_string(), "*.tmp".to_string()],
        },
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::AppendGitignorePatterns {
                repo_id: RepoId(1),
                patterns,
            }] if patterns.as_slice() == ["/build/out.log", "*.tmp"]
        ),
        "the patterns must reach the effect verbatim: the reducer is not allowed \
         to re-derive or reorder them"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RebaseContinue { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RebaseContinue {
            repo_id: RepoId(1),
            auth: None,
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RebaseAbort { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RebaseAbort { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::MergeAbort { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::MergeAbort { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddRemote {
            repo_id,
            name: "origin".to_string(),
            url: "https://example.com/repo.git".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::AddRemote {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveRemote {
            repo_id,
            name: "origin".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RemoveRemote {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetRemoteUrl {
            repo_id,
            name: "origin".to_string(),
            url: "https://example.com/alt.git".to_string(),
            kind: gitcomet_core::services::RemoteUrlKind::Push,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SetRemoteUrl {
            repo_id: RepoId(1),
            kind: gitcomet_core::services::RemoteUrlKind::Push,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
            upstream: "origin/feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        }] if branch == "feature/current" && upstream == "origin/feature/current"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/current".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        }] if branch == "feature/current"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutConflictSide {
            repo_id,
            path: PathBuf::from("conflicted.txt"),
            side: gitcomet_core::services::ConflictSide::Theirs,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutConflictSide {
            repo_id: RepoId(1),
            side: gitcomet_core::services::ConflictSide::Theirs,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LaunchMergetool {
            repo_id,
            path: PathBuf::from("conflicted.txt"),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LaunchMergetool {
            repo_id: RepoId(1),
            ..
        }]
    ));
}

#[test]
fn repo_command_finished_error_summaries_cover_additional_labels() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

    let cases: Vec<(RepoCommandKind, &str)> = vec![
        (
            RepoCommandKind::AddRemote {
                name: "origin".to_string(),
                url: "https://example.com/repo.git".to_string(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::RemoveRemote {
                name: "origin".to_string(),
            },
            "Remote",
        ),
        (
            RepoCommandKind::SetRemoteUrl {
                name: "origin".to_string(),
                url: "https://example.com/push.git".to_string(),
                kind: gitcomet_core::services::RemoteUrlKind::Push,
            },
            "Remote",
        ),
        (
            RepoCommandKind::SetUpstreamBranch {
                branch: "feature/current".to_string(),
                upstream: "origin/feature/current".to_string(),
            },
            "Set as tracking upstream",
        ),
        (
            RepoCommandKind::UnsetUpstreamBranch {
                branch: "feature/current".to_string(),
            },
            "Unlink upstream branch",
        ),
        (
            RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("conflicted.txt"),
                side: gitcomet_core::services::ConflictSide::Ours,
            },
            "Checkout ours",
        ),
        (
            RepoCommandKind::CheckoutConflict {
                path: PathBuf::from("conflicted.txt"),
                side: gitcomet_core::services::ConflictSide::Theirs,
            },
            "Checkout theirs",
        ),
        (
            RepoCommandKind::AcceptConflictDeletion {
                path: PathBuf::from("conflicted.txt"),
            },
            "Accept deletion",
        ),
        (
            RepoCommandKind::CheckoutConflictBase {
                path: PathBuf::from("conflicted.txt"),
            },
            "Checkout base",
        ),
        (
            RepoCommandKind::LaunchMergetool {
                path: PathBuf::from("conflicted.txt"),
            },
            "Mergetool",
        ),
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: false,
            },
            "Save file",
        ),
        (
            RepoCommandKind::ExportPatch {
                commit_id: CommitId("deadbeef".into()),
                dest: PathBuf::from("out.patch"),
            },
            "Patch",
        ),
        (
            RepoCommandKind::ApplyPatch {
                patch: PathBuf::from("in.patch"),
            },
            "Patch",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("/tmp/worktree"),
                reference: None,
            },
            "Worktree",
        ),
        (
            RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
            },
            "Worktree",
        ),
        (
            RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            "Submodule",
        ),
        (
            RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            "Submodule",
        ),
        (
            RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("mods/sub"),
            },
            "Submodule",
        ),
        (RepoCommandKind::StageHunk, "Hunk"),
        (RepoCommandKind::UnstageHunk, "Hunk"),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: true },
            "Discard",
        ),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: false },
            "Patch",
        ),
    ];

    for (command, label) in cases {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
            }),
        );

        let summary = state.repos[0]
            .command_log
            .last()
            .expect("command log entry")
            .summary
            .clone();
        assert!(
            summary.starts_with(&format!("{label} failed:")),
            "unexpected summary for label {label}: {summary}"
        );
    }
}

#[test]
fn repo_command_finished_success_summaries_cover_additional_commands() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

    let cases: Vec<(RepoCommandKind, &str)> = vec![
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: true,
            },
            "Saved and staged → a.txt",
        ),
        (
            RepoCommandKind::SaveWorktreeFile {
                path: PathBuf::from("a.txt"),
                stage: false,
            },
            "Saved → a.txt",
        ),
        (
            RepoCommandKind::ExportPatch {
                commit_id: CommitId("deadbeef".into()),
                dest: PathBuf::from("out.patch"),
            },
            "Patch exported → out.patch",
        ),
        (
            RepoCommandKind::ApplyPatch {
                patch: PathBuf::from("in.patch"),
            },
            "Patch applied → in.patch",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("../wt"),
                reference: Some("main".to_string()),
            },
            "Worktree added → ../wt (main)",
        ),
        (
            RepoCommandKind::AddWorktree {
                path: PathBuf::from("../wt"),
                reference: None,
            },
            "Worktree added → ../wt",
        ),
        (
            RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("../wt"),
            },
            "Worktree removed → ../wt",
        ),
        (
            RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            "Submodule added → mods/sub",
        ),
        (
            RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            "Submodules: Updated",
        ),
        (
            RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("mods/sub"),
            },
            "Submodule removed → mods/sub",
        ),
        (RepoCommandKind::StageHunk, "Hunk staged"),
        (RepoCommandKind::UnstageHunk, "Hunk unstaged"),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: true },
            "Changes discarded",
        ),
        (
            RepoCommandKind::ApplyWorktreePatch { reverse: false },
            "Patch applied",
        ),
    ];

    for (command, expected_summary) in cases {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git command")),
            }),
        );

        let summary = state.repos[0]
            .command_log
            .last()
            .expect("command log entry")
            .summary
            .clone();
        assert_eq!(summary, expected_summary);
    }
}

#[test]
fn apply_worktree_patch_command_finished_reloads_png_diff_preview() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("image.png"),
        area: DiffArea::Unstaged,
    };
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ApplyWorktreePatch { reverse: true },
            result: Ok(CommandOutput::empty_success("git apply -R")),
        }),
    );

    let repo_state = state.repos.first().expect("repo");
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiffFileImage {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::LoadDiffFile { .. })),
        "png reload should request image preview only"
    );
}

#[test]
fn checkout_branch_and_submodule_messages_emit_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == RepoId(1)) {
        repo.set_detached_head_commit(Some(CommitId("deadbeef".into())));
    }
    state.active_repo = Some(RepoId(1));

    let checkout = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id: RepoId(1),
            name: "feature/x".to_string(),
        },
    );
    assert!(matches!(
        checkout.as_slice(),
        [Effect::CheckoutBranch {
            repo_id: RepoId(1),
            name
        }] if name == "feature/x"
    ));
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo should exist");
    assert!(repo.detached_head_commit.is_none());

    let add_submodule = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddSubmodule {
            repo_id: RepoId(1),
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: Some("feature".to_string()),
            name: None,
            force: false,
        },
    );
    assert!(matches!(
        add_submodule.as_slice(),
        [Effect::CheckSubmoduleAddTrust {
            repo_id: RepoId(1),
            url,
            path,
            branch,
            ..
        }] if url == "https://example.com/sub.git"
            && path == &PathBuf::from("mods/sub")
            && branch.as_deref() == Some("feature")
    ));

    let update_submodules = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UpdateSubmodules { repo_id: RepoId(1) },
    );
    assert!(matches!(
        update_submodules.as_slice(),
        [Effect::CheckSubmoduleUpdateTrust { repo_id: RepoId(1) }]
    ));

    let remove_submodule = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RemoveSubmodule {
            repo_id: RepoId(1),
            path: PathBuf::from("mods/sub"),
        },
    );
    assert!(matches!(
        remove_submodule.as_slice(),
        [Effect::RemoveSubmodule {
            repo_id: RepoId(1),
            path
        }] if path == &PathBuf::from("mods/sub")
    ));
}

#[test]
fn local_submodule_add_trust_prompt_confirms_into_add_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let source = gitcomet_core::services::SubmoduleTrustTarget {
        submodule_path: PathBuf::from("mods/sub"),
        display_source: "../local-sub".to_string(),
        local_source_path: PathBuf::from("/tmp/local-sub"),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: Some("feature".to_string()),
            name: None,
            force: false,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![source.clone()],
            }),
        }),
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.submodule_trust_prompt,
        Some(crate::model::SubmoduleTrustPromptState {
            repo_id,
            operation: crate::model::SubmoduleTrustPromptOperation::Add {
                url: "../local-sub".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: Some("feature".to_string()),
                name: None,
                force: false,
            },
            sources: vec![source.clone()],
        })
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConfirmSubmoduleTrustPrompt,
    );
    assert!(state.submodule_trust_prompt.is_none());
    assert!(matches!(
        effects.as_slice(),
        [Effect::AddSubmodule {
            repo_id: RepoId(1),
            url,
            path,
            branch,
            approved_sources,
            ..
        }] if url == "../local-sub"
            && path == &PathBuf::from("mods/sub")
            && branch.as_deref() == Some("feature")
            && approved_sources == &vec![source]
    ));
    assert_eq!(
        state.repos[0].submodule_add_in_flight,
        Some(crate::model::SubmoduleAddProgressState {
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
        })
    );
}

#[test]
fn submodule_trust_check_pending_marks_and_clears_around_the_check() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    // Triggering the add marks a pending check so the UI can show a spinner.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddSubmodule {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: None,
            force: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckSubmoduleAddTrust {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert_eq!(
        state.submodule_trust_check_pending,
        Some(crate::model::SubmoduleTrustCheckState {
            repo_id,
            operation: crate::model::SubmoduleTrustCheckOperation::Add,
        })
    );

    // The check resolving (here, into a prompt) clears the pending marker.
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: None,
            force: false,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![gitcomet_core::services::SubmoduleTrustTarget {
                    submodule_path: PathBuf::from("mods/sub"),
                    display_source: "../local-sub".to_string(),
                    local_source_path: PathBuf::from("/tmp/local-sub"),
                }],
            }),
        }),
    );
    assert!(state.submodule_trust_check_pending.is_none());
    assert!(state.submodule_trust_prompt.is_some());
}

#[test]
fn submodule_add_progress_starts_when_trust_check_proceeds() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: Some("deps/sub".to_string()),
            force: true,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed),
        }),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::AddSubmodule {
            repo_id: RepoId(1),
            url,
            path,
            name,
            force,
            ..
        }] if url == "https://example.com/sub.git"
            && path == &PathBuf::from("mods/sub")
            && name.as_deref() == Some("deps/sub")
            && *force
    ));
    assert_eq!(
        state.repos[0].submodule_add_in_flight,
        Some(crate::model::SubmoduleAddProgressState {
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
        })
    );
}

#[test]
fn submodule_add_progress_clears_on_failed_add_command() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].submodule_add_in_flight = Some(crate::model::SubmoduleAddProgressState {
        url: "https://example.com/sub.git".to_string(),
        path: PathBuf::from("mods/sub"),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("submodule add failed".to_string()),
            )),
        }),
    );

    assert!(state.repos[0].submodule_add_in_flight.is_none());
    assert!(
        !effects.iter().any(
            |effect| matches!(effect, Effect::LoadSubmodules { repo_id: id } if *id == repo_id)
        )
    );
}

#[test]
fn local_submodule_update_trust_prompt_cancels_cleanly() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let source = gitcomet_core::services::SubmoduleTrustTarget {
        submodule_path: PathBuf::from("mods/sub"),
        display_source: "../local-sub".to_string(),
        local_source_path: PathBuf::from("/tmp/local-sub"),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked {
            repo_id,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![source],
            }),
        }),
    );
    assert!(matches!(
        state.submodule_trust_prompt,
        Some(crate::model::SubmoduleTrustPromptState {
            repo_id: RepoId(1),
            operation: crate::model::SubmoduleTrustPromptOperation::Update,
            ..
        })
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CancelSubmoduleTrustPrompt,
    );
    assert!(effects.is_empty());
    assert!(state.submodule_trust_prompt.is_none());
}

#[test]
fn pull_branch_and_push_variants_mark_in_flight_when_repo_is_opened() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let pull_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PullBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
    );
    assert!(matches!(
        pull_branch.as_slice(),
        [Effect::PullBranch {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        }] if remote == "origin" && branch == "main"
    ));
    assert_eq!(state.repos[0].pull_in_flight, 1);

    let force_push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id },
    );
    assert!(matches!(
        force_push.as_slice(),
        [Effect::ForcePush {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert_eq!(state.repos[0].push_in_flight, 1);

    let push_set_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature/xyz".to_string(),
        },
    );
    assert!(matches!(
        push_set_upstream.as_slice(),
        [Effect::PushSetUpstream {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        }] if remote == "origin" && branch == "feature/xyz"
    ));
    assert_eq!(state.repos[0].push_in_flight, 2);

    let set_upstream_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/local".to_string(),
            upstream: "origin/feature/xyz".to_string(),
        },
    );
    assert!(matches!(
        set_upstream_branch.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream
        }] if branch == "feature/local" && upstream == "origin/feature/xyz"
    ));
    assert_eq!(state.repos[0].push_in_flight, 2);
}

#[test]
fn commit_and_amend_finished_cover_success_error_and_unknown_repo_paths() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    {
        let repo = &mut state.repos[0];
        repo.local_actions_in_flight = 1;
        repo.commit_in_flight = 1;
        repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        });
        repo.diff_state.diff = Loadable::Loading;
        repo.diff_state.diff_file = Loadable::Loading;
        repo.diff_state.diff_file_image = Loadable::Loading;
    }

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(!effects.is_empty());
    let repo = &state.repos[0];
    assert_eq!(repo.local_actions_in_flight, 0);
    assert_eq!(repo.commit_in_flight, 0);
    assert!(repo.last_error.is_none());
    assert!(repo.diff_state.diff_target.is_none());
    assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(matches!(
        repo.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert_eq!(
        repo.command_log.last().map(|entry| entry.summary.as_str()),
        Some("Commit: Completed")
    );

    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("commit boom".to_string()))),
        }),
    );
    assert!(
        state.repos[0]
            .last_error
            .as_deref()
            .unwrap_or_default()
            .starts_with("Commit failed:")
    );

    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert_eq!(
        state.repos[0]
            .command_log
            .last()
            .map(|entry| entry.summary.as_str()),
        Some("Amend: Completed")
    );
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("amend boom".to_string()))),
        }),
    );
    assert!(
        state.repos[0]
            .last_error
            .as_deref()
            .unwrap_or_default()
            .starts_with("Amend failed:")
    );

    let missing_commit = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id: RepoId(999),
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(missing_commit.is_empty());
    let missing_amend = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id: RepoId(999),
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(missing_amend.is_empty());
}

#[test]
fn commit_finished_push_after_commit_enqueues_safe_push_only_on_success() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].pending_commit_retry = Some(crate::model::PendingCommitRetry {
        message: "ship".to_string(),
        amend: false,
        push_after_commit: true,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome {
                local_branch: Some("main".to_string()),
                pre_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            }),
        }),
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::SafePushAfterCommit { repo_id: id, context, .. } if *id == repo_id
                && !context.amend
                && context.local_branch.as_deref() == Some("main")
                && context.pre_head.as_ref().is_some_and(|id| id.as_ref() == "1111111111111111111111111111111111111111")
                && context.post_head.as_ref().is_some_and(|id| id.as_ref() == "2222222222222222222222222222222222222222")))
    );
    assert_eq!(state.repos[0].push_in_flight, 0);
    assert!(state.repos[0].pending_commit_retry.is_none());

    state.repos[0].push_in_flight = 0;
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].pending_commit_retry = Some(crate::model::PendingCommitRetry {
        message: "ship".to_string(),
        amend: false,
        push_after_commit: true,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("commit boom".to_string()))),
        }),
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::SafePushAfterCommit { .. }))
    );
    assert_eq!(state.repos[0].push_in_flight, 0);
}

#[test]
fn safe_push_after_commit_decision_push_enqueues_checked_push() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let auth = gitcomet_core::auth::StagedGitAuth {
        kind: gitcomet_core::auth::GitAuthKind::UsernamePassword,
        username: Some("alice".to_string()),
        secret: "token".to_string(),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: false,
                local_branch: Some("main".to_string()),
                pre_head: None,
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: Some(auth.clone()),
            result: Ok(gitcomet_core::services::SafePushAfterCommitDecision::Push {
                target: target.clone(),
            }),
        }),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PushAfterCommit {
            repo_id: id,
            target: effect_target,
            set_upstream: false,
            auth: Some(effect_auth),
        } if *id == repo_id && effect_target == &target && effect_auth == &auth
    )));
    assert_eq!(state.repos[0].push_in_flight, 1);
}

#[test]
fn safe_push_after_commit_decision_push_set_upstream_enqueues_checked_push() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "feature".to_string(),
        local_branch: "feature".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: false,
                local_branch: Some("feature".to_string()),
                pre_head: None,
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::PushSetUpstream {
                    target: target.clone(),
                },
            ),
        }),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PushAfterCommit {
            repo_id: id,
            target: effect_target,
            set_upstream: true,
            ..
        } if *id == repo_id && effect_target == &target
    )));
    assert_eq!(state.repos[0].push_in_flight, 1);
}

#[test]
fn safe_push_after_commit_published_amend_block_stores_lease_offer() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let expected = CommitId("1111111111111111111111111111111111111111".into());
    let lease = gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: expected.clone(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: true,
                local_branch: Some("main".to_string()),
                pre_head: Some(expected.clone()),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::Blocked {
                    summary: "published amend".to_string(),
                    lease: Some(lease.clone()),
                },
            ),
        }),
    );

    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Push { .. } | Effect::PushAfterCommit { .. } | Effect::PushSetUpstream { .. }
    )));
    assert_eq!(state.repos[0].pending_force_push_lease, Some(lease));
    assert!(
        state.repos[0]
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("Push after commit blocked")
    );
}

#[test]
fn safe_push_after_commit_published_amend_lease_survives_followup_git_state_refresh() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let expected = CommitId("1111111111111111111111111111111111111111".into());
    let lease = gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: expected.clone(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: true,
                local_branch: Some("main".to_string()),
                pre_head: Some(expected),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::Blocked {
                    summary: "published amend".to_string(),
                    lease: Some(lease.clone()),
                },
            ),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert_eq!(state.repos[0].pending_force_push_lease, Some(lease));
}

#[test]
fn checkout_branch_clears_stale_force_push_lease_and_recent_messages() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));
    let recent_rev = state.repos[0].recent_commit_messages_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id,
            name: "feature".to_string(),
        },
    );

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::CheckoutBranch { repo_id: id, name }
                if *id == repo_id && name == "feature"
        )
    }));
    assert_eq!(state.repos[0].pending_force_push_lease, None);
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert!(state.repos[0].recent_commit_messages_rev > recent_rev);
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
}

#[test]
fn cherry_pick_clears_recent_messages_from_previous_head() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CherryPickCommit {
            repo_id,
            commit_id: CommitId("3333333333333333333333333333333333333333".into()),
            commit: true,
            mainline: None,
            summary: "pick me".into(),
        },
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert_eq!(state.repos[0].pending_force_push_lease, None);
}

#[test]
fn interactive_cherry_pick_finished_clears_stale_force_push_lease() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::InteractiveCherryPick {
                entries: vec![gitcomet_core::services::InteractiveRebaseEntry {
                    action: gitcomet_core::services::InteractiveRebaseAction::Pick,
                    commit_id: "3333333333333333333333333333333333333333".to_string(),
                    summary: "pick me".to_string(),
                    message: "pick me".to_string(),
                    new_message: None,
                }],
            },
            result: Ok(CommandOutput::empty_success("git cherry-pick")),
        }),
    );

    assert_eq!(state.repos[0].pending_force_push_lease, None);
}

#[test]
fn head_changing_repo_action_finish_invalidates_data_loaded_while_in_flight() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.local_actions_in_flight = 1;
    state.repos.push(repo_state);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::CherryPickCommit,
            result: Ok(()),
        }),
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert_eq!(state.repos[0].pending_force_push_lease, None);
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn stale_recent_commit_messages_loaded_after_head_change_is_ignored() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);

    let first_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRecentCommitMessages { repo_id, limit: 10 },
    );
    let first_request_rev = match first_effects.as_slice() {
        [
            Effect::LoadRecentCommitMessages {
                repo_id: effect_repo_id,
                limit,
                request_rev,
            },
        ] if *effect_repo_id == repo_id && *limit == 10 => *request_rev,
        effects => panic!("expected recent commit message load effect, got {effects:?}"),
    };
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::Loading
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));

    let second_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRecentCommitMessages { repo_id, limit: 10 },
    );
    let second_request_rev = match second_effects.as_slice() {
        [
            Effect::LoadRecentCommitMessages {
                repo_id: effect_repo_id,
                limit,
                request_rev,
            },
        ] if *effect_repo_id == repo_id && *limit == 10 => *request_rev,
        effects => panic!("expected second recent commit message load effect, got {effects:?}"),
    };
    assert!(second_request_rev > first_request_rev);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev: first_request_rev,
            result: Ok(vec![test_recent_commit_message_with_summary(
                "2222222222222222222222222222222222222222",
                "stale message",
            )]),
        }),
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::Loading
    ));
    assert_eq!(
        state.repos[0].recent_commit_messages_rev,
        second_request_rev
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev: second_request_rev,
            result: Ok(vec![test_recent_commit_message_with_summary(
                "3333333333333333333333333333333333333333",
                "current message",
            )]),
        }),
    );

    match &state.repos[0].recent_commit_messages {
        Loadable::Ready(messages) => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].summary.as_ref(), "current message");
        }
        other => panic!("expected current recent messages, got {other:?}"),
    }
}

#[test]
fn repo_command_finished_reset_clears_diff_state_and_unknown_repo_is_noop() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("a.txt"),
        area: DiffArea::Staged,
    });
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Reset {
                mode: gitcomet_core::services::ResetMode::Mixed,
                target: "HEAD~1".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git reset --mixed HEAD~1")),
        }),
    );
    assert!(!effects.is_empty());
    let repo = &state.repos[0];
    assert!(repo.diff_state.diff_target.is_none());
    assert_eq!(repo.pending_force_push_lease, None);
    assert!(matches!(&repo.recent_commit_messages, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(matches!(
        repo.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));

    let no_repo_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(999),
            command: RepoCommandKind::Reset {
                mode: gitcomet_core::services::ResetMode::Hard,
                target: "HEAD".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git reset --hard HEAD")),
        }),
    );
    assert!(no_repo_effects.is_empty());
}

#[test]
fn hunk_staging_is_logged_without_announcing_success() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    for command in [RepoCommandKind::StageHunk, RepoCommandKind::UnstageHunk] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::default()),
            }),
        );
    }

    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    assert_eq!(
        repo_state.command_log.len(),
        2,
        "staging a hunk still belongs in the command log"
    );
    assert!(
        repo_state
            .command_log
            .iter()
            .all(|entry| !entry.announce_success),
        "a staged hunk shows itself in the diff; it must not raise a toast"
    );

    // A failure is still surfaced.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("patch does not apply".into()),
            )),
        }),
    );
    let repo_state = state.repos.iter().find(|r| r.id == repo_id).unwrap();
    let last = repo_state.command_log.last().unwrap();
    assert!(!last.ok, "the failure must be recorded as such");
}

#[test]
fn stage_hunk_command_finished_reloads_commit_png_image_preview_only() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    let target = DiffTarget::Commit {
        commit_id: CommitId("abc123".into()),
        path: Some(PathBuf::from("assets/icon.png")),
    };
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(target.clone());
    state.repos.push(repo_state);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::empty_success("git apply --cached")),
        }),
    );

    let repo = state.repos.first().expect("repo");
    assert!(repo.diff_state.diff.is_loading());
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(repo.diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiffFileImage {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::LoadDiffFile { .. })),
        "png reload should not request text diff"
    );
}

fn repo_state_with_tags_loaded(repo_id: RepoId) -> RepoState {
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.local_actions_in_flight = 1;
    repo_state.set_tags(Loadable::Ready(vec![gitcomet_core::domain::Tag {
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));
    repo_state
}

#[test]
fn create_tag_command_finished_reloads_tags() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CreateTag {
                name: "v2.0.0".to_string(),
                target: "HEAD".to_string(),
                message: None,
                annotated: false,
            },
            result: Ok(CommandOutput::empty_success(
                "git tag -c tag.gpgsign=false -- v2.0.0 HEAD",
            )),
        }),
    );

    assert!(
        matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should be reset to NotLoaded after CreateTag"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTags { repo_id: id } if *id == repo_id)),
        "expected LoadTags effect after CreateTag"
    );
}

#[test]
fn delete_tag_command_finished_reloads_tags() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteTag {
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git tag -d v1.0.0")),
        }),
    );

    assert!(
        matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should be reset to NotLoaded after DeleteTag"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTags { repo_id: id } if *id == repo_id)),
        "expected LoadTags effect after DeleteTag"
    );
}

#[test]
fn prune_local_tags_command_finished_reloads_tags() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));
    state.repos[0].local_actions_in_flight = 0;
    state.repos[0].pull_in_flight = 1;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneLocalTags,
            result: Ok(CommandOutput::empty_success("git tag prune")),
        }),
    );

    assert!(
        matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should be reset to NotLoaded after PruneLocalTags"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTags { repo_id: id } if *id == repo_id)),
        "expected LoadTags effect after PruneLocalTags"
    );
}

#[test]
fn create_tag_failed_does_not_reload_tags() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(repo_state_with_tags_loaded(repo_id));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::CreateTag {
                name: "v2.0.0".to_string(),
                target: "HEAD".to_string(),
                message: None,
                annotated: false,
            },
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("tag already exists".to_string()),
            )),
        }),
    );

    assert!(
        !matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should not be reset when CreateTag fails"
    );
}

#[test]
fn create_tag_with_message_propagates_to_effect() {
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

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateTag {
            repo_id: RepoId(1),
            name: "v1.0.0".to_string(),
            target: "HEAD".to_string(),
            message: Some("Release 1.0".to_string()),
            annotated: true,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateTag { repo_id: RepoId(1), name, target, message: Some(msg), annotated: true }]
            if name == "v1.0.0" && target == "HEAD" && msg == "Release 1.0"
    ));
}

#[test]
fn cherry_pick_setup_loads_full_messages_and_patches_entries() {
    const SHA: &str = "1111111111111111111111111111111111111111";
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    // Opening the setup with subject-only seeds schedules the full-message
    // load for the selected commits.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![gitcomet_core::services::InteractiveRebaseEntry {
                action: gitcomet_core::services::InteractiveRebaseAction::Pick,
                commit_id: SHA.to_string(),
                summary: "subject".to_string(),
                message: "subject".to_string(),
                new_message: None,
            }],
            source_colors: vec![],
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadInteractiveCherryPickMessages { repo_id: RepoId(1), ids }]
            if ids.as_slice() == [SHA.to_string()]
    ));
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup opens in a loading state");
    assert!(matches!(setup.full_messages, Loadable::Loading));

    // Only after every full message arrives does the setup become editable;
    // the full body replaces the subject-only seed while the summary stays.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![SHA.to_string()],
                result: Ok(vec![(SHA.to_string(), "subject\n\nfull body".to_string())]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open");
    assert_eq!(setup.entries[0].message, "subject\n\nfull body");
    assert_eq!(setup.entries[0].summary, "subject");
    assert!(matches!(setup.full_messages, Loadable::Ready(())));
}

#[test]
fn cherry_pick_setup_applies_repository_topological_order() {
    const DESCENDANT: &str = "3333333333333333333333333333333333333333";
    const ANCESTOR: &str = "1111111111111111111111111111111111111111";
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let entry = |commit_id: &str, summary: &str| gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: commit_id.to_string(),
        summary: summary.to_string(),
        message: summary.to_string(),
        new_message: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(DESCENDANT, "descendant"), entry(ANCESTOR, "ancestor")],
            source_colors: vec![],
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![DESCENDANT.to_string(), ANCESTOR.to_string()],
                result: Ok(vec![
                    (ANCESTOR.to_string(), "ancestor full".to_string()),
                    (DESCENDANT.to_string(), "descendant full".to_string()),
                ]),
            },
        ),
    );

    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open");
    assert_eq!(
        setup
            .entries
            .iter()
            .map(|entry| entry.commit_id.as_str())
            .collect::<Vec<_>>(),
        [ANCESTOR, DESCENDANT]
    );
    assert_eq!(setup.entries[0].message, "ancestor full");
    assert_eq!(setup.entries[1].message, "descendant full");
    assert!(matches!(setup.full_messages, Loadable::Ready(())));
}

#[test]
fn cherry_pick_setup_never_enables_rewording_after_partial_or_stale_message_load() {
    const OLD_SHA: &str = "1111111111111111111111111111111111111111";
    const NEW_SHA: &str = "2222222222222222222222222222222222222222";
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let entry = |commit_id: &str| gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: commit_id.to_string(),
        summary: "subject".to_string(),
        message: "subject".to_string(),
        new_message: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(OLD_SHA)],
            source_colors: vec![],
        },
    );
    // Replace the selection before the detached response for the first one
    // arrives. That stale body must not unlock or alter the new setup.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(NEW_SHA)],
            source_colors: vec![],
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![OLD_SHA.to_string()],
                result: Ok(vec![(OLD_SHA.to_string(), "old full body".to_string())]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("new setup stays open");
    assert_eq!(setup.entries[0].commit_id, NEW_SHA);
    assert_eq!(setup.entries[0].message, "subject");
    assert!(matches!(setup.full_messages, Loadable::Loading));

    // A response missing even one requested full message becomes an error,
    // never a subject-only Ready state.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![NEW_SHA.to_string()],
                result: Ok(vec![]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open with its load error");
    assert!(matches!(setup.full_messages, Loadable::Error(_)));
    assert_eq!(setup.entries[0].message, "subject");
}
