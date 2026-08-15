use super::*;

/// Production always reaches `LogLoaded` through `request_log`, which records the
/// walk as the active one so replies from a superseded walk can be dropped.
/// Tests that dispatch `LogLoaded` directly have to declare it the same way, and
/// send the sequence number it hands back.
/// The sequence number of the log walk the repository already has in flight —
/// what a reply from that walk has to carry. For tests where earlier reduces
/// dispatched the load; [`expect_log_reply`] is for tests that dispatch none.
fn active_log_seq(repo_state: &RepoState) -> crate::model::LogLoadSeq {
    repo_state
        .loads_in_flight
        .active_log_seq()
        .expect("a log walk is in flight")
}

fn commit_details_for(id: &CommitId, message: &str) -> gitcomet_core::domain::CommitDetails {
    gitcomet_core::domain::CommitDetails {
        id: id.clone(),
        message: message.to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-03-08 12:34:56 +0200".to_string(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: vec![],
    }
}

fn expect_log_reply(
    repo_state: &mut RepoState,
    scope: LogScope,
    author: Option<&str>,
    cursor: Option<LogCursor>,
) -> crate::model::LogLoadSeq {
    repo_state.loads_in_flight.clear();
    repo_state
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope,
            author: author.map(str::to_owned),
            limit: 200,
            cursor,
        })
        .expect("a declared log walk starts immediately")
}

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
        has_combined_status_effect(&effects, repo_id)
            || has_worktree_status_effect(&effects, repo_id),
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
            result: Ok(gitcomet_core::services::SequencerState::None),
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
    let seq = active_log_seq(&state.repos[0]);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
            result: Ok(gitcomet_core::services::SequencerState::None),
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
    let seq = active_log_seq(&state.repos[0]);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
            cursor: None,
            ..
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

fn state_with_blamed_unstaged_diff() -> (AppState, RepoId) {
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
    (state, repo_id)
}

#[test]
fn repo_externally_changed_worktree_keeps_blame_until_content_changes() {
    // A worktree/index event reloads the diff, but the reload may well find the
    // same bytes (a window-focus refresh, a touch, a save that changed nothing).
    // Blame is expensive — it shells out to `git blame` — so it must survive
    // until `diff_loaded`/`diff_file_loaded` sees the content actually differ.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, repo_id) = state_with_blamed_unstaged_diff();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );

    // The working-tree diff still reloads against the (possibly new) content...
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id: id, .. } if *id == repo_id)),
        "external worktree change must reload the diff"
    );
    // ...but the annotations stay painted until that reload proves them stale.
    assert!(
        matches!(state.repos[0].history_state.blame, Loadable::Ready(_)),
        "a worktree event alone must not invalidate blame"
    );
}

#[test]
fn repo_externally_changed_git_state_invalidates_loaded_blame() {
    // A moved HEAD (external commit / checkout / rebase) can leave the patch
    // byte-identical while every line's attribution changes, and no downstream
    // content comparison can detect that — so git-state events drop blame up
    // front, preserving the target so it reloads for the same file.
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, repo_id) = state_with_blamed_unstaged_diff();
    let previous = match &state.repos[0].history_state.blame {
        Loadable::Ready(lines) => Arc::clone(lines),
        other => panic!("expected a loaded blame, got {other:?}"),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::all(),
        },
    );

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id: id, .. } if *id == repo_id)),
        "a git-state change must reload the diff"
    );
    assert!(
        matches!(state.repos[0].history_state.blame, Loadable::NotLoaded),
        "blame must be invalidated when refs may have moved"
    );
    // The outgoing annotations are held over so the column does not blank while
    // the reload runs.
    assert!(
        state.repos[0]
            .history_state
            .retained_blame_while_loading
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &previous)),
        "the previous annotations must be retained while blame reloads"
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
        edit_mode: false,
        selected_commit: Some(c.clone()),
        range_selection: None,
        worktree_selection: None,
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
fn reload_repo_clears_a_stale_comparison_mark() {
    // A reload can follow a reset or a dropped branch that took the marked
    // commit with it. Keeping the mark would leave the context menus offering a
    // "Compare with …" whose only possible outcome is a backend error.
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
    state.repos[0].comparison_mark = Some(crate::model::ComparisonMark {
        commit_id: CommitId("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
        label: "feature".into(),
    });

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReloadRepo { repo_id },
    );

    assert!(
        state.repos[0].comparison_mark.is_none(),
        "a mark that a reload may have invalidated must not survive it"
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
            author: None,
            limit: 200,
            cursor: Some(_),
            ..
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
    let seq = repo_state
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope: LogScope::FullReachable,
            author: None,
            limit: 200,
            cursor: None,
        })
        .expect("the first walk starts immediately");

    // Each switch supersedes the walk in flight and is dispatched at once,
    // rather than queueing behind a walk that may run for tens of seconds.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                scope: LogScope::AllBranches,
                cursor: None,
                ..
            }
        )),
        "expected the scope switch to start its load immediately, got {effects:?}"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::NoMerges,
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                scope: LogScope::NoMerges,
                cursor: None,
                ..
            }
        )),
        "expected the second scope switch to start immediately too, got {effects:?}"
    );
    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::NoMerges
    );
    assert!(state.repos[0].log.is_loading());

    // The first walk finally answers. It is superseded, so it must neither land
    // in the log nor disturb the walk that replaced it.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
        effects.is_empty(),
        "a superseded reply must not schedule anything, got {effects:?}"
    );
    assert!(
        !state.repos[0].loads_in_flight.is_active_log_reply(seq),
        "the superseded walk must no longer be the active one"
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

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
fn log_loaded_reconciles_commit_multi_selection() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![CommitId("kept".into()), CommitId("gone".into())],
        anchor: Some(CommitId("gone".into())),
        anchor_index: Some(1),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("kept"), commit("other")],
                next_cursor: None,
            }),
        }),
    );

    let sel = &state.repos[0].history_state.multi_selection;
    assert_eq!(sel.commits, vec![CommitId("kept".into())]);
    assert_eq!(sel.anchor, None);
    assert_eq!(sel.anchor_index, None);
    assert_eq!(sel.anchor_log_rev, None);
}

/// A reveal asks git to resolve the reference before touching the selection, so
/// an abbreviation lands on the full id and the details pane fills in without
/// the log having paged anywhere near the commit.
#[test]
fn reveal_commit_resolves_an_abbreviation_and_shows_it_immediately() {
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

    let reference = CommitId("deadbee".into());
    let full = CommitId("deadbeef0123456789abcdef0123456789abcdef".into());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevealCommit {
            repo_id: RepoId(1),
            reference: reference.clone(),
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::ResolveCommitForReveal { reference: r, .. } if *r == reference
        )),
        "asking to reveal should resolve the reference, got {effects:?}"
    );
    // Nothing is selected until it is known to exist.
    assert_eq!(state.repos[0].history_state.selected_commit, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
            repo_id: RepoId(1),
            reference,
            result: Ok(commit_details_for(&full, "the reland")),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&full));
    assert_eq!(history.reveal_target.as_ref(), Some(&full));
    assert!(
        matches!(&history.commit_details, Loadable::Ready(details) if details.id == full),
        "the details fetched to resolve the reference should be the ones shown"
    );
}

/// A hex-looking run that is not a commit — a Gerrit change id, say — must fail
/// loudly and cheaply instead of sending the log walking to the root.
#[test]
fn reveal_commit_reports_an_unresolvable_reference_without_selecting() {
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

    let reference = CommitId("7a5d480873e839444e4e188ffa87f9c635e2fb81".into());
    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevealCommit {
            repo_id: RepoId(1),
            reference: reference.clone(),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
            repo_id: RepoId(1),
            reference,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("unknown revision".to_string()),
            )),
        }),
    );

    assert!(effects.is_empty(), "a dead reference starts no work");
    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit, None);
    assert_eq!(history.reveal_target, None);
    assert!(
        state
            .notifications
            .iter()
            .any(|note| note.message.contains("Could not find commit")),
        "the user has to be told the reference went nowhere"
    );
}

/// A "load more" only ever grows the page, so a selection it does not contain
/// has not been paged in yet — it has not vanished. Clearing it there used to
/// make the details pane flip to the working tree once per batch for the whole
/// length of a reveal.
#[test]
fn log_loaded_keeps_a_not_yet_paged_selection_when_loading_more() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let deep = CommitId("deep".into());
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![commit("c1")],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    })));
    repo_state.set_selected_commit(Some(deep.clone()));
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![deep.clone()],
        anchor: Some(deep.clone()),
        anchor_index: Some(0),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(LogPage {
                commits: vec![commit("c2")],
                next_cursor: Some(LogCursor {
                    last_seen: CommitId("c2".into()),
                    resume_from: None,
                    resume_token: None,
                }),
            }),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&deep));
    assert_eq!(history.multi_selection.commits, vec![deep]);
}

/// A first page *replaces* the log, so it can genuinely retire a selection —
/// unless a reveal is still walking toward exactly that commit, which is what a
/// mid-reveal scope switch does.
#[test]
fn log_loaded_first_page_keeps_the_commit_a_reveal_is_walking_toward() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let target = CommitId("target".into());
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_reveal_target(Some(target.clone()));
    repo_state.set_selected_commit(Some(target.clone()));
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![target.clone()],
        anchor: Some(target.clone()),
        anchor_index: Some(0),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("other")],
                next_cursor: None,
            }),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&target));
    assert_eq!(history.multi_selection.commits, vec![target]);
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

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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

    let seq = expect_log_reply(&mut state.repos[0], LogScope::AllBranches, None, None);
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
    let seq = expect_log_reply(&mut state.repos[0], history_scope, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
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
    let seq = expect_log_reply(&mut state.repos[0], history_scope, None, None);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
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

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
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

        let seq = expect_log_reply(&mut state.repos[0], scope, None, None);
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                repo_id,
                seq,
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

#[test]
fn external_tags_change_reloads_tags() {
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
    state.repos[0].set_tags(Loadable::Ready(vec![gitcomet_core::domain::Tag {
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange {
                git_state: true,
                tags: true,
                ..Default::default()
            },
        },
    );

    assert!(
        matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should be reset to NotLoaded on external tags change"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTags { repo_id: id } if *id == repo_id)),
        "expected LoadTags effect on external tags change"
    );
}

#[test]
fn external_git_state_change_without_tags_flag_does_not_reload_tags() {
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
    state.repos[0].set_tags(Loadable::Ready(vec![gitcomet_core::domain::Tag {
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert!(
        !effects.iter().any(|e| matches!(e, Effect::LoadTags { .. })),
        "LoadTags should not fire for a git_state change without tags flag"
    );
    assert!(
        !matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should remain Ready when tags flag is not set"
    );
}

#[test]
fn external_tags_change_without_git_state_flag_reloads_tags() {
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
    state.repos[0].set_tags(Loadable::Ready(vec![gitcomet_core::domain::Tag {
        name: "v1.0.0".to_string(),
        target: CommitId("abc123".into()),
    }]));

    // The `tags` flag must drive a tag reload independently of `git_state`, so a
    // change that only sets `tags` still refreshes them.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange {
                git_state: false,
                tags: true,
                ..Default::default()
            },
        },
    );

    assert!(
        matches!(state.repos[0].tags, Loadable::NotLoaded),
        "tags should be reset to NotLoaded when only the tags flag is set"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadTags { repo_id: id } if *id == repo_id)),
        "expected LoadTags effect when only the tags flag is set"
    );
}

/// Picking an author while a walk is already running must dispatch the new load
/// at once. Queueing it behind the old walk is what made filtering feel frozen
/// on a large repository, where a walk runs for tens of seconds and the
/// repo-load pool has one or two threads.
#[test]
fn author_filter_change_starts_its_load_while_a_walk_is_in_flight() {
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

    let scope = state.repos[0].history_state.history_scope;
    let unfiltered = expect_log_reply(&mut state.repos[0], scope, None, None);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryAuthorFilter {
            repo_id: RepoId(1),
            author: Some("alice".to_string()),
        },
    );

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog { author: Some(author), cursor: None, .. } if author == "alice"
        )),
        "expected the filter change to start its load immediately, got {effects:?}"
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_active_log_reply(unfiltered),
        "the unfiltered walk it replaced is no longer the active one"
    );
}

/// A walk cancelled because a newer filter replaced it is routine, not a
/// failure: it must not raise a diagnostic or blank the history out.
#[test]
fn cancelled_log_reply_is_not_reported_as_an_error() {
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

    let scope = state.repos[0].history_state.history_scope;
    let seq = expect_log_reply(&mut state.repos[0], scope, Some("alice"), None);
    state.repos[0].history_state.history_author_filter = Some("alice".to_string());
    state.repos[0].set_log(Loadable::Loading);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope,
            cursor: None,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Cancelled,
            )),
        }),
    );

    assert!(
        state.repos[0].diagnostics.is_empty(),
        "a cancelled walk must not raise a diagnostic, got {:?}",
        state.repos[0].diagnostics
    );
    assert!(
        !matches!(state.repos[0].log, Loadable::Error(_)),
        "a cancelled walk must not blank the history out"
    );
    assert!(effects.is_empty(), "nothing to schedule, got {effects:?}");
}

/// Partial pages land as they are found, so a filter that has to walk a large
/// history shows what it has instead of the previous filter's rows.
#[test]
fn log_chunks_replace_the_page_progressively() {
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

    let scope = state.repos[0].history_state.history_scope;
    let seq = expect_log_reply(&mut state.repos[0], scope, Some("alice"), None);
    state.repos[0].history_state.history_author_filter = Some("alice".to_string());
    state.repos[0].set_log(Loadable::Loading);

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: id.into(),
        author: "alice".into(),
        time: SystemTime::UNIX_EPOCH,
    };
    let mut chunk = |commits: Vec<Commit>, scanned: u64, state: &mut AppState| {
        reduce(
            &mut repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
                repo_id: RepoId(1),
                seq,
                commits,
                scanned,
            }),
        );
    };

    // A partial page shows through the retained slot while the walk runs. It is
    // deliberately not `Ready`: the walk has not answered "is there more?" yet,
    // and a `Ready` page with no cursor would answer "no" on its behalf.
    let partial = |state: &AppState| {
        state.repos[0]
            .history_state
            .retained_log_while_loading
            .clone()
    };

    // Nothing found yet: only the progress readout moves.
    chunk(Vec::new(), 50_000, &mut state);
    assert!(state.repos[0].log.is_loading());
    assert!(partial(&state).is_none());
    assert_eq!(state.repos[0].history_state.log_scan_progress, Some(50_000));

    chunk(vec![commit("c1")], 90_000, &mut state);
    assert!(state.repos[0].log.is_loading());
    assert_eq!(
        partial(&state)
            .expect("the first chunk shows its commits")
            .commits
            .len(),
        1
    );

    // Chunks are prefixes of one another, so a later one simply replaces.
    chunk(vec![commit("c1"), commit("c2")], 140_000, &mut state);
    assert_eq!(
        partial(&state)
            .expect("the second chunk extends the page")
            .commits
            .len(),
        2
    );
    assert_eq!(
        state.repos[0].history_state.log_scan_progress,
        Some(140_000)
    );

    // The finished page clears the progress.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("c1"), commit("c2"), commit("c3")],
                next_cursor: None,
            }),
        }),
    );
    let Loadable::Ready(page) = &state.repos[0].log else {
        panic!("expected the finished page");
    };
    assert_eq!(page.commits.len(), 3);
    assert_eq!(state.repos[0].history_state.log_scan_progress, None);
}

/// Chunks from a walk that a newer filter superseded must not paint rows for
/// the filter the user has already moved off.
#[test]
fn superseded_log_chunks_are_ignored() {
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

    let scope = state.repos[0].history_state.history_scope;
    let superseded = expect_log_reply(&mut state.repos[0], scope, Some("alice"), None);
    // A newer filter takes over; the walk above is cancelled but still running.
    state.repos[0]
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope,
            author: Some("bob".to_string()),
            limit: 200,
            cursor: None,
        })
        .expect("the newer filter starts at once");
    state.repos[0].set_log(Loadable::Loading);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
            repo_id: RepoId(1),
            seq: superseded,
            commits: vec![Commit {
                id: CommitId("stale".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "stale".into(),
                author: "alice".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            scanned: 10,
        }),
    );

    assert!(
        state.repos[0].log.is_loading(),
        "a superseded chunk must not paint rows"
    );
    assert!(
        state.repos[0]
            .history_state
            .retained_log_while_loading
            .is_none(),
        "a superseded chunk must not paint rows"
    );
    assert_eq!(state.repos[0].history_state.log_scan_progress, None);
}

/// A cancelled walk's reply never reaches the reducer — the repo-load guard
/// drops it — so whoever cancels has to take the progress readout down, or the
/// "Scanning history…" banner sits there with a frozen count.
#[test]
fn cancelling_repo_loads_clears_the_scan_progress() {
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

    let scope = state.repos[0].history_state.history_scope;
    let seq = expect_log_reply(&mut state.repos[0], scope, Some("alice"), None);
    state.repos[0].set_log(Loadable::Loading);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
            repo_id: RepoId(1),
            seq,
            commits: Vec::new(),
            scanned: 400_000,
        }),
    );
    assert_eq!(
        state.repos[0].history_state.log_scan_progress,
        Some(400_000)
    );

    // A completed action invalidates every load in flight, the walk included.
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

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::CancelRepoLoads { .. })),
        "the completed action must cancel the loads in flight, got {effects:?}"
    );
    assert_eq!(
        state.repos[0].history_state.log_scan_progress, None,
        "the banner must not outlive the walk it was counting for"
    );
}

/// An open repository with nothing loaded, for the hover-message reducer tests
/// below.
fn hover_message_state(repo_id: RepoId) -> AppState {
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].set_open(Loadable::Ready(()));
    state.active_repo = Some(repo_id);
    state
}

fn hover_message_slot(state: &AppState) -> Option<(CommitId, Loadable<Arc<str>>)> {
    state.repos[0].hover_commit_message.clone()
}

#[test]
fn hovering_a_commit_reads_its_message_once_however_often_the_row_is_re_entered() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = hover_message_state(repo_id);
    let commit_id = CommitId("abc".into());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadHoverCommitMessage { commit_id: c, .. } if *c == commit_id
        )),
        "the first hover has to issue the read, got {effects:?}"
    );
    assert!(matches!(
        hover_message_slot(&state),
        Some((id, Loadable::Loading)) if id == commit_id
    ));

    // The pointer wandering within the same row re-dispatches; the read is
    // already in flight, so it must not be issued a second time.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    assert!(effects.is_empty(), "a read in flight is not re-issued");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
            repo_id,
            commit_id: commit_id.clone(),
            result: Ok("Fix the thing\n\nWhy it broke.".to_string()),
        }),
    );
    assert!(matches!(
        hover_message_slot(&state),
        Some((id, Loadable::Ready(message)))
            if id == commit_id && message.as_ref() == "Fix the thing\n\nWhy it broke."
    ));

    // And once it has arrived, re-hovering re-reads nothing at all.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    assert!(effects.is_empty(), "a message already held is not re-read");
}

#[test]
fn a_hover_message_that_failed_is_retried_on_the_next_hover() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = hover_message_state(repo_id);
    let commit_id = CommitId("abc".into());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
            repo_id,
            commit_id: commit_id.clone(),
            result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
        }),
    );
    assert!(matches!(
        hover_message_slot(&state),
        Some((id, Loadable::Error(_))) if id == commit_id
    ));
    assert!(
        state.notifications.is_empty(),
        "a hover losing its race is not worth telling the user about, got {:?}",
        state.notifications
    );

    // Unlike the loading and loaded cases, a failure does not suppress the next
    // attempt -- otherwise one transient error kills the card for that commit
    // for the rest of the session.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadHoverCommitMessage { commit_id: c, .. } if *c == commit_id
        )),
        "a failed read is retried, got {effects:?}"
    );
}

#[test]
fn a_hover_message_for_a_commit_the_pointer_has_left_is_dropped() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = hover_message_state(repo_id);
    let first = CommitId("aaa".into());
    let second = CommitId("bbb".into());

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: first.clone(),
        },
    );
    // The pointer moves to another row before the first read comes back.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: second.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
            repo_id,
            commit_id: first,
            result: Ok("the row the pointer already left".to_string()),
        }),
    );

    assert!(
        matches!(
            hover_message_slot(&state),
            Some((id, Loadable::Loading)) if id == second
        ),
        "the late result must not overwrite the row now under the pointer"
    );
}

#[test]
fn hovering_a_repository_that_is_not_open_yet_reads_nothing() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = hover_message_state(repo_id);
    state.repos[0].set_open(Loadable::Loading);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadHoverCommitMessage {
            repo_id,
            commit_id: CommitId("abc".into()),
        },
    );

    assert!(effects.is_empty(), "got {effects:?}");
    assert!(
        hover_message_slot(&state).is_none(),
        "and nothing is recorded as being fetched"
    );
}

/// The multi-worktree scan is the most expensive thing the watcher can queue: a
/// full `status` walk of every *other* linked worktree, which (see the known gix
/// behaviour) does not honour the global gitignore and so traverses an un-ignored
/// `target/` or `node_modules/` in full. What keeps that off the hot path is that
/// the index is classified apart from the rest of the git dir: staging or
/// unstaging writes `.git/index` and nothing else, which cannot change any other
/// worktree's status and must not cost a scan. A linked worktree's own index
/// lives at `.git/worktrees/<name>/index`, which is *not* `is_git_index_path`, so
/// it still arrives as a git-state change and still earns one.
#[test]
fn an_index_only_change_does_not_rescan_the_other_worktrees() {
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

    let scans = |effects: &[Effect]| {
        effects
            .iter()
            .filter(
                |e| matches!(e, Effect::LoadWorktreeDirty { repo_id: id, .. } if *id == repo_id),
            )
            .count()
    };

    // The open-repo refresh already queued one; let it finish so the in-flight
    // guard is not what makes the next assertion pass.
    let settle = |repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
                  state: &mut AppState,
                  id_alloc: &AtomicU64| {
        reduce(
            repos,
            id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id,
                result: Ok(Vec::new()),
            }),
        );
    };
    settle(&mut repos, &mut state, &id_alloc);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Index,
        },
    );
    assert_eq!(
        scans(&effects),
        0,
        "an index write cannot change another worktree's status, so it must not \
         queue a walk of every one of them"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert_eq!(
        scans(&effects),
        1,
        "a git-state change can move another worktree's HEAD or index, so it still \
         earns exactly one scan"
    );
    settle(&mut repos, &mut state, &id_alloc);
}
