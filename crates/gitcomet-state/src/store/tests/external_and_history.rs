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
    gitcomet_core::domain::RecentCommitMessage {
        id: CommitId("1111111111111111111111111111111111111111".into()),
        summary: Arc::from("old message"),
        message: "old message\n\nbody".to_string(),
    }
}

#[test]
fn repo_activated_is_reducer_noop_by_itself() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_open(Loadable::Ready(()));
    state.active_repo = Some(repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoActivated { repo_id },
    );

    assert!(effects.is_empty());
    assert!(!state.repos[0].status.is_loading());
    assert!(!state.repos[0].log.is_loading());
}

#[test]
fn repo_load_trace_names_repo_activation_and_refresh_messages() {
    let repo_id = RepoId(1);

    assert_eq!(
        repo_load_trace::msg_name(&Msg::RepoActivated { repo_id }),
        "RepoActivated"
    );
    assert_eq!(
        repo_load_trace::msg_name(&Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        }),
        "RepoExternallyChanged"
    );
    assert_eq!(
        repo_load_trace::msg_name(&Msg::ReloadRepo { repo_id }),
        "ReloadRepo"
    );
    assert_eq!(
        repo_load_trace::msg_repo_id(&Msg::RepoActivated { repo_id }),
        Some(repo_id)
    );
    assert_eq!(
        repo_load_trace::msg_external_change(&Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        }),
        Some(crate::msg::RepoExternalChange::GitState)
    );
}

#[test]
fn external_worktree_change_refreshes_status_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            },
        },
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );

    assert!(
        has_worktree_status_effect(&effects, RepoId(1)),
        "expected status refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id, .. } if *repo_id == RepoId(1))),
        "expected diff refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadDiffFile { repo_id, .. } if *repo_id == RepoId(1))
        }),
        "expected diff-file refresh"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::LoadLog { .. })),
        "did not expect history refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { .. })),
        "did not expect head-branch refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadUpstreamDivergence { .. })),
        "did not expect upstream divergence refresh on pure worktree changes"
    );
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::LoadBranches { .. } | Effect::LoadRemoteBranches { .. }
        )),
        "did not expect branch refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseState { .. })),
        "did not expect rebase state refresh on pure worktree changes"
    );
}

#[test]
fn external_index_change_refreshes_both_staged_and_unstaged_lanes() {
    // An external `git add` / `git reset` / `git restore --staged` rewrites `.git/index` without
    // touching any worktree file, so the monitor reports an index-only change. The index is one
    // side of BOTH the staged (HEAD↔index) and unstaged (index↔worktree) diffs, so both lanes
    // must refresh; otherwise a file that moved between the staged and unstaged sections lingers
    // (stale) in the lane that was not reloaded.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);

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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Index,
        },
    );

    assert!(
        has_status_refresh_effects(&effects, repo_id),
        "an index-only external change must refresh both the staged and unstaged lanes, got {effects:?}"
    );
}

#[test]
fn external_index_change_reloads_open_working_tree_diff() {
    // With a staged file's diff open, an external `git add` / `git reset` (index-only change)
    // must reload that working-tree diff so it reflects the new index content rather than showing
    // a stale diff.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);

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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Staged,
            },
        },
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Index,
        },
    );

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id: rid, .. } if *rid == repo_id)),
        "an index change must reload the open staged working-tree diff, got {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiffFile { repo_id: rid, .. } if *rid == repo_id)),
        "an index change must reload the open diff's file content, got {effects:?}"
    );
}

#[test]
fn external_index_change_must_not_refresh_only_the_staged_lane() {
    // Regression test that fails against the previous behavior: an index-only external change
    // (`git add` / `git reset` / `git restore --staged`) used to emit exactly
    // `[LoadStagedStatus]`, refreshing only the staged lane and leaving a moved file stale in the
    // unstaged section. The change must also pursue the unstaged (worktree) lane.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);

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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );
    // Settle the initial refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Index,
        },
    );

    // The unstaged lane must be refreshed — either by the combined status load or a direct
    // worktree load.
    assert!(
        has_combined_status_effect(&effects, repo_id) || has_worktree_status_effect(&effects, repo_id),
        "an index-only change must refresh the unstaged lane, got {effects:?}"
    );
    // The exact old-behavior shape (staged lane only) must not occur.
    let staged_lane_only = has_staged_status_effect(&effects, repo_id)
        && !has_combined_status_effect(&effects, repo_id)
        && !has_worktree_status_effect(&effects, repo_id);
    assert!(
        !staged_lane_only,
        "an index-only change must not refresh only the staged lane, got {effects:?}"
    );
}

#[test]
fn external_git_state_change_preserves_pending_force_push_lease_and_clears_recent_messages() {
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
    repo_state.pending_force_push_lease = Some(test_force_push_lease());
    repo_state.set_recent_commit_messages(Loadable::Ready(vec![test_recent_commit_message()]));
    let recent_rev = repo_state.recent_commit_messages_rev;
    state.repos.push(repo_state);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert_eq!(
        state.repos[0].pending_force_push_lease,
        Some(test_force_push_lease())
    );
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert!(state.repos[0].recent_commit_messages_rev > recent_rev);
}

#[test]
fn external_git_state_change_refreshes_history_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            },
        },
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id: RepoId(1),
            result: Ok("main".to_string()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
            repo_id: RepoId(1),
            result: Ok(false),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    let history_scope = state.repos[0].history_state.history_scope;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert!(
        effects
            .iter()
            .any(|e| { matches!(e, Effect::LoadLog { repo_id, .. } if *repo_id == RepoId(1)) }),
        "expected history refresh"
    );
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { repo_id } if *repo_id == RepoId(1))),
        "expected head-branch refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadUpstreamDivergence { repo_id } if *repo_id == RepoId(1))
        }),
        "expected upstream divergence refresh"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == RepoId(1)
        )),
        "expected rebase state refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id } if *repo_id == RepoId(1))),
        "expected local branches refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadRemoteBranches { repo_id } if *repo_id == RepoId(1))
        }),
        "expected remote branches refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(
                e,
                Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == RepoId(1)
            )
        }),
        "expected merge commit message refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id, .. } if *repo_id == RepoId(1))),
        "expected diff refresh"
    );
}

#[test]
fn external_git_state_refresh_is_coalesced_and_replayed_once() {
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

    let effects1 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadUpstreamDivergence { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseAndMergeState { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseAndMergeState { .. }))
    );
    assert!(has_status_refresh_effects(&effects1, RepoId(1)));
    assert!(effects1.iter().any(|e| matches!(e, Effect::LoadLog { .. })));

    // Second refresh request while the first one is in flight is coalesced into a single pending
    // refresh per load kind (no immediate duplicate effects).
    let effects2 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(
        effects2.is_empty(),
        "expected coalescing/backpressure, got {effects2:?}"
    );

    // Completing each in-flight load replays exactly one more load for that kind.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id: RepoId(1),
            result: Ok("main".to_string()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadHeadBranch { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadUpstreamDivergence { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
            repo_id: RepoId(1),
            result: Ok(false),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadRebaseState { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadMergeCommitMessage { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadWorktreeStatus { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadStagedStatus { repo_id: RepoId(1) }]
    ));

    let history_scope = state.repos[0].history_state.history_scope;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadLog {
            repo_id: RepoId(1),
            scope,
            limit: 200,
            cursor: None
        }] if *scope == history_scope
    ));
}

#[test]
fn external_worktree_refresh_replays_coalesced_change_then_settles() {
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
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects, repo_id),
        "expected first worktree event to request status refresh"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        effects.is_empty(),
        "expected in-flight coalescing while status load is running, got {effects:?}"
    );

    // The in-flight load completes with an unchanged payload, but a second worktree event was
    // coalesced while it ran. That event is a genuine external change the load may have read just
    // before it landed, so the coalesced refresh must be replayed (not dropped) — otherwise the
    // uncommitted view keeps showing stale entries.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    assert!(
        has_worktree_status_effect(&effects, repo_id),
        "coalesced external change must replay a status load even when the payload is unchanged, got {effects:?}"
    );
    assert!(
        state.repos[0]
            .loads_in_flight
            .is_in_flight(crate::model::RepoLoadsInFlight::WORKTREE_STATUS),
        "the replayed load should re-arm the worktree status lane"
    );

    // The replayed load completes and nothing new is pending, so the lane settles instead of
    // looping forever (status reads are read-only and cannot manufacture their own events).
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::LoadWorktreeStatus { repo_id: rid } if *rid == repo_id)),
        "with no pending change the lane should stop replaying, got {effects:?}"
    );
    assert!(
        !state.repos[0].loads_in_flight.any_in_flight(),
        "in-flight flags should settle once no refresh is pending"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects, repo_id),
        "subsequent real worktree events should still trigger status refresh"
    );
}

#[test]
fn external_worktree_refresh_coalesces_status_while_status_is_in_flight() {
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

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs"),
                area: DiffArea::Unstaged,
            },
        },
    );

    let effects1 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects1, RepoId(1)),
        "expected first refresh to request status"
    );
    assert!(
        effects1.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                ..
            }
        )),
        "expected first refresh to request diff reload"
    );

    let effects2 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        !has_worktree_status_effect(&effects2, RepoId(1)),
        "coalesced worktree refresh should not emit duplicate status effects, got {effects2:?}"
    );
    assert!(
        effects2.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                ..
            }
        )),
        "selected diff should still refresh on subsequent worktree changes"
    );
    assert!(
        effects2.iter().any(|e| matches!(
            e,
            Effect::LoadDiffFile {
                repo_id: RepoId(1),
                ..
            }
        )),
        "selected diff file should still refresh on subsequent worktree changes"
    );
}

#[test]
fn reload_repo_sets_sections_loading_and_emits_refresh_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_open(Loadable::Ready(()));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReloadRepo { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
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
    assert!(!repo_state.history_state.log_loading_more);
    assert!(repo_state.merge_commit_message.is_loading());
    assert!(repo_state.submodules.is_loading());
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadTags { repo_id } if *repo_id == RepoId(1)
        )),
        "tags should auto-load in the background on repo reload"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
        )),
        "remote tags should auto-load in the background on repo reload"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmodules { repo_id } if *repo_id == RepoId(1)
        )),
        "submodules should auto-load in the background on repo reload"
    );
}

#[test]
fn repo_externally_changed_invalidates_loaded_blame() {
    // Regression: an external edit/stage reloads the working-tree diff, and the
    // blame annotation column is derived from that same content. Leaving blame
    // `Ready` would make `request_blame_for_current_target` treat the target as
    // already attempted and keep painting stale attribution over the new lines.
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
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
    state.repos[0].diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });
    state.repos[0].history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    state.repos[0].history_state.blame_source = Some(
        gitcomet_core::domain::BlameSource::WorkingTree(DiffArea::Unstaged),
    );
    state.repos[0].history_state.blame = Loadable::Ready(std::sync::Arc::new(vec![
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
    ]));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );

    // The working-tree diff reloads against the new content...
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id: id, .. } if *id == repo_id)),
        "external worktree change must reload the diff"
    );
    // ...and blame is dropped so it reloads too, with the target preserved.
    assert!(
        matches!(state.repos[0].history_state.blame, Loadable::NotLoaded),
        "blame must be invalidated when the working-tree diff reloads externally"
    );
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
fn reload_repo_clears_stale_navigation_history() {
    // Regression: a full reload may rewrite history (rebase/amend), so saved
    // back/forward snapshots can reference commits that no longer resolve. The
    // nav stacks must start fresh rather than letting Back restore a dead view.
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
    state.repos[0].set_open(Loadable::Ready(()));
    state.active_repo = Some(repo_id);

    let commit_a = CommitId("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let commit_b = CommitId("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    let snap = |c: &CommitId| crate::model::MainViewSnapshot {
        diff_target: Some(DiffTarget::Commit {
            commit_id: c.clone(),
            path: Some(PathBuf::from("src/lib.rs")),
        }),
        content_preview: false,
        selected_commit: Some(c.clone()),
    };
    state.repos[0].nav_history.record(snap(&commit_a));
    state.repos[0].nav_history.record(snap(&commit_b));
    state.repos[0]
        .view_history
        .record(crate::model::ViewHistoryEntry {
            source: gitcomet_core::domain::FileSource::Commit(commit_a.clone()),
            path: PathBuf::from("src/lib.rs"),
        });
    // Make the live view match the nav tail so the reduce-wrapper's reconcile is
    // a no-op and the stack survives intact up to the point ReloadRepo runs.
    state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: commit_b.clone(),
        path: Some(PathBuf::from("src/lib.rs")),
    });
    state.repos[0].set_selected_commit(Some(commit_b.clone()));
    assert_eq!(state.repos[0].nav_history.entries.len(), 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReloadRepo { repo_id },
    );

    // The stale back-stack entry (commit A) must be gone — without the clear it
    // would survive as `entries[0]` while only the tail gets folded over.
    assert!(
        !state.repos[0]
            .nav_history
            .entries
            .iter()
            .any(|s| s.selected_commit.as_ref() == Some(&commit_a)),
        "stale nav back-stack entry must be cleared on reload"
    );
    assert!(
        state.repos[0].nav_history.entries.len() <= 1,
        "only the post-reload current view may remain in nav_history"
    );
    assert!(
        state.repos[0].view_history.entries.is_empty(),
        "view_history must be cleared on reload"
    );
}

#[test]
fn load_more_history_emits_paginated_load_log_effect() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    }));
    repo_state.history_state.log_loading_more = false;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadMoreHistory { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.history_state.log_loading_more);
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadLog {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            limit: 200,
            cursor: Some(_)
        }]
    ));
}

#[test]
fn set_history_scope_emits_load_log_effect_for_every_history_mode() {
    for target_scope in [
        LogScope::FullReachable,
        LogScope::FirstParent,
        LogScope::NoMerges,
        LogScope::MergesOnly,
        LogScope::AllBranches,
    ] {
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

        let repo_state = &mut state.repos[0];
        repo_state.history_state.history_scope = if target_scope == LogScope::FullReachable {
            LogScope::FirstParent
        } else {
            LogScope::FullReachable
        };
        repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("old".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "old".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })));

        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SetHistoryScope {
                repo_id: RepoId(1),
                scope: target_scope,
            },
        );

        let repo_state = &state.repos[0];
        assert_eq!(repo_state.history_state.history_scope, target_scope);
        assert!(repo_state.log.is_loading());
        assert!(
            repo_state
                .history_state
                .retained_log_while_loading
                .is_some(),
            "expected retained history page while switching to {target_scope:?}"
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::LoadLog {
                    repo_id: RepoId(1),
                    scope,
                    cursor: None,
                    ..
                } if *scope == target_scope
            )),
            "expected LoadLog({target_scope:?}) effect, got {effects:?}"
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::PersistRepoHistoryMode {
                    repo_id: Some(RepoId(1)),
                    mode,
                    ..
                } if *mode == target_scope
            )),
            "expected async history mode persist effect for {target_scope:?}, got {effects:?}"
        );
    }
}

#[test]
fn set_history_scope_retains_ready_log_while_loading() {
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

    let retained_page = Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    });
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::clone(&retained_page)));

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.log.is_loading());
    let retained = repo_state
        .history_state
        .retained_log_while_loading
        .as_ref()
        .expect("scope switch should retain the previous ready log while loading");
    assert!(Arc::ptr_eq(retained, &retained_page));
}

#[test]
fn stale_log_loaded_result_replays_latest_pending_scope_switch() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::FullReachable;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("old".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "old".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    })));
    assert!(
        repo_state
            .loads_in_flight
            .request_log(LogScope::FullReachable, 200, None)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistRepoHistoryMode {
            repo_id: Some(RepoId(1)),
            mode: LogScope::AllBranches,
            ..
        }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::NoMerges,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistRepoHistoryMode {
            repo_id: Some(RepoId(1)),
            mode: LogScope::NoMerges,
            ..
        }]
    ));
    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::NoMerges
    );
    assert!(state.repos[0].log.is_loading());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::FullReachable,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![],
                next_cursor: None,
            }),
        }),
    );

    assert!(state.repos[0].log.is_loading());
    assert!(!state.repos[0].history_state.log_loading_more);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadLog {
                repo_id: RepoId(1),
                scope: LogScope::NoMerges,
                limit: 200,
                cursor: None,
            }]
        ),
        "expected stale result to replay the latest pending scope switch, got {effects:?}"
    );
}

#[test]
fn load_more_history_noops_when_no_next_cursor() {
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

    let repo_state = &mut state.repos[0];
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    }));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadMoreHistory { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
    assert!(!repo_state.history_state.log_loading_more);
    assert!(effects.is_empty());
}

#[test]
fn log_loaded_appends_when_loading_more() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    }));
    repo_state.history_state.log_loading_more = true;
    let log_before = (repo_state.log_rev, repo_state.history_state.log_rev);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c2".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s2".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(!repo_state.history_state.log_loading_more);
    assert!(repo_state.log_rev > log_before.0);
    assert!(repo_state.history_state.log_rev > log_before.1);
    let Loadable::Ready(page) = &repo_state.log else {
        panic!("expected log ready");
    };
    assert_eq!(page.commits.len(), 2);
    assert_eq!(page.commits[0].id.as_ref(), "c1");
    assert_eq!(page.commits[1].id.as_ref(), "c2");
    assert_eq!(page.next_cursor, None);
}

#[test]
fn log_loaded_appends_when_loading_more_re_shares_history_log_arc() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    })));
    repo_state.history_state.log_loading_more = true;

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c2".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s2".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: Some(LogCursor {
                    last_seen: CommitId("c2".into()),
                    resume_from: None,
                    resume_token: None,
                }),
            }),
        }),
    );

    let repo_state = &state.repos[0];
    let Loadable::Ready(repo_log) = &repo_state.log else {
        panic!("expected repo log ready");
    };
    let Loadable::Ready(history_log) = &repo_state.history_state.log else {
        panic!("expected history log ready");
    };

    assert!(Arc::ptr_eq(repo_log, history_log));
    assert_eq!(repo_log.commits.len(), 2);
    assert_eq!(repo_log.commits[1].id.as_ref(), "c2");
    assert_eq!(
        repo_log
            .next_cursor
            .as_ref()
            .and_then(|cursor| cursor.last_seen.as_ref().strip_prefix('c')),
        Some("2")
    );
}

#[test]
fn log_loaded_clears_retained_scope_switch_log() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("old".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "old".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    })));

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );

    assert!(
        state.repos[0]
            .history_state
            .retained_log_while_loading
            .is_some()
    );

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("new".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "new".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.log, Loadable::Ready(_)));
    assert!(
        repo_state
            .history_state
            .retained_log_while_loading
            .is_none()
    );
}

#[test]
fn log_loaded_initial_paginated_page_keeps_append_slack() {
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

    let commits: Vec<Commit> = (0..600)
        .map(|ix| Commit {
            id: CommitId(format!("{ix:040x}").into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: format!("s{ix}").into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        })
        .collect();
    let last_seen = commits.last().expect("last commit").id.clone();
    let history_scope = state.repos[0].history_state.history_scope;

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits,
                next_cursor: Some(LogCursor {
                    last_seen,
                    resume_from: None,
                    resume_token: None,
                }),
            }),
        }),
    );

    let Loadable::Ready(page) = &state.repos[0].log else {
        panic!("expected log ready");
    };
    assert!(page.commits.capacity() >= page.commits.len() + 512);
}

// --- Revision counter regression tests ---

#[test]
fn log_loaded_bumps_log_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = (state.repos[0].log_rev, state.repos[0].history_state.log_rev);
    let history_scope = state.repos[0].history_state.history_scope;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c1".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s1".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    assert!(
        state.repos[0].log_rev > log_before.0,
        "repo log_rev should bump after LogLoaded"
    );
    assert!(
        state.repos[0].history_state.log_rev > log_before.1,
        "log_rev should bump after LogLoaded"
    );
}

#[test]
fn detached_head_target_tracks_current_branch_log_head() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].history_state.history_scope = LogScope::CurrentBranch;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("HEAD".to_string()),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![
                    Commit {
                        id: CommitId("c1".into()),
                        parent_ids: smallvec::smallvec![CommitId("c0".into())],
                        summary: "s1".into(),
                        author: "a".into(),
                        time: SystemTime::UNIX_EPOCH,
                    },
                    Commit {
                        id: CommitId("c0".into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "s0".into(),
                        author: "a".into(),
                        time: SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }),
        }),
    );

    assert_eq!(
        state.repos[0].detached_head_commit,
        Some(CommitId("c1".into()))
    );
}

#[test]
fn filtered_current_branch_logs_do_not_backfill_detached_head_target() {
    for (scope, commits, expected_first_visible) in [
        (
            LogScope::NoMerges,
            vec![Commit {
                id: CommitId("visible-non-merge".into()),
                parent_ids: smallvec::smallvec![CommitId("hidden-head".into())],
                summary: "visible".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            CommitId("visible-non-merge".into()),
        ),
        (
            LogScope::MergesOnly,
            vec![Commit {
                id: CommitId("visible-merge".into()),
                parent_ids: smallvec::smallvec![CommitId("p0".into()), CommitId("p1".into())],
                summary: "merge".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            CommitId("visible-merge".into()),
        ),
    ] {
        let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = AppState::default();
        let repo_id = RepoId(1);
        repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(repo_id);
        state.repos[0].history_state.history_scope = scope;

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Ok("HEAD".to_string()),
            }),
        );

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                repo_id,
                scope,
                cursor: None,
                result: Ok(LogPage {
                    commits,
                    next_cursor: None,
                }),
            }),
        );

        assert!(
            state.repos[0].detached_head_commit.is_none(),
            "{scope:?} should not infer detached HEAD from first visible commit {expected_first_visible}"
        );
    }
}

#[test]
fn set_history_scope_bumps_log_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = (state.repos[0].log_rev, state.repos[0].history_state.log_rev);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id,
            scope: LogScope::AllBranches,
        },
    );

    assert!(
        state.repos[0].log_rev > log_before.0,
        "repo log_rev should bump after SetHistoryScope"
    );
    assert!(
        state.repos[0].history_state.log_rev > log_before.1,
        "log_rev should bump after SetHistoryScope"
    );
}

#[test]
fn status_loaded_bumps_status_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let status_before = state.repos[0].status_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus::default()),
        }),
    );

    assert!(
        state.repos[0].status_rev > status_before,
        "status_rev should bump after StatusLoaded"
    );
}
