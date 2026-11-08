use super::*;
use crate::model::LoadKind;

fn listing(branch: &str) -> Arc<Vec<Worktree>> {
    Arc::new(vec![Worktree {
        bare: false,
        path: "/tmp/linked".into(),
        head: Some(CommitId(branch.into())),
        branch: Some(branch.into()),
        detached: false,
    }])
}

#[test]
fn requested_worktree_list_does_not_wait_for_targeted_scan_replays() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    state.repos[0].set_worktree_dirty(Loadable::Ready(Vec::new()));
    // A full scan failed to read the list, but checkout edits keep the dirty
    // lane occupied. Neither the active target nor its repeat publishes lists.
    load_worktree_dirty(&mut state, repo_id);
    worktree_dirty_loaded(
        &mut state,
        repo_id,
        WorktreeDirtyScope::All,
        Err(backend_error("temporary listing failure")),
    );
    let path = PathBuf::from("/tmp/linked");
    assert!(request_worktree_dirty_path_effect(&mut state.repos[0], path.clone()).is_some());
    assert!(request_worktree_dirty_path_effect(&mut state.repos[0], path).is_none());
    let effects = ensure_sidebar_data(
        &mut state,
        repo_id,
        SidebarDataRequest {
            worktrees: true,
            ..Default::default()
        },
    );
    assert!(
        matches!(effects.as_slice(), [Effect::LoadWorktrees { .. }]),
        "a requested list must not wait behind targeted scans: {effects:?}"
    );
}

#[test]
fn worktree_observations_preserve_explicit_load_bookkeeping_and_newer_data() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    assert_eq!(load_worktrees(&mut state, repo_id).len(), 1);
    assert!(load_worktrees(&mut state, repo_id).is_empty());
    let explicit = WorktreeListRead::explicit();
    let observed = WorktreeListRead::from_dirty_scan();
    let newer = listing("newer");
    assert!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            observed,
            Ok(newer.clone())
        )
        .is_empty()
    );
    assert!(
        state.repos[0]
            .loads_in_flight
            .is_in_flight(LoadKind::Worktrees)
    );
    let revision = state.repos[0].worktrees_rev;
    let sidebar_revision = state.repos[0].branch_sidebar_rev;

    // The late explicit reply must finish its own request and promote exactly
    // one repeat, without replacing the more recent scan observation.
    assert!(matches!(
        worktrees_loaded(&crate::store::reducer::test_inputs::clock(), &mut state, repo_id, explicit, Ok(listing("old"))).as_slice(),
        [Effect::LoadWorktrees { repo_id: id }] if *id == repo_id
    ));
    assert_eq!(state.repos[0].worktrees_rev, revision);
    let older_scan = WorktreeListRead::from_dirty_scan();
    assert!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            WorktreeListRead::explicit(),
            Ok(listing("newer"))
        )
        .is_empty()
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(LoadKind::Worktrees)
    );
    assert!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            older_scan,
            Ok(listing("old"))
        )
        .is_empty()
    );
    assert!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            explicit,
            Err(backend_error("obsolete failure"))
        )
        .is_empty()
    );
    assert_eq!(state.repos[0].worktrees_rev, revision);
    assert_eq!(state.repos[0].branch_sidebar_rev, sidebar_revision);
    assert!(Arc::ptr_eq(
        state.repos[0].worktrees.ready().unwrap(),
        &newer
    ));
    assert!(state.repos[0].feedback.diagnostics.is_empty());
}

#[test]
fn worktree_refresh_errors_retain_rows_and_recover_without_stuck_loads() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    let before = listing("before");
    worktrees_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        repo_id,
        WorktreeListRead::from_dirty_scan(),
        Ok(before.clone()),
    );
    let revision = state.repos[0].worktrees_rev;
    load_worktrees(&mut state, repo_id);
    load_worktrees(&mut state, repo_id);
    assert_eq!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            WorktreeListRead::explicit(),
            Err(backend_error("list unavailable"))
        )
        .len(),
        1
    );
    assert_eq!(state.repos[0].worktrees_rev, revision);
    assert!(Arc::ptr_eq(
        state.repos[0].worktrees.ready().unwrap(),
        &before
    ));
    assert_eq!(state.repos[0].feedback.diagnostics.len(), 1);
    assert!(
        worktrees_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            repo_id,
            WorktreeListRead::explicit(),
            Ok(listing("after"))
        )
        .is_empty()
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(LoadKind::Worktrees)
    );
    assert_eq!(
        state.repos[0].worktrees.ready().unwrap()[0]
            .branch
            .as_deref(),
        Some("after")
    );
}

#[test]
fn failed_read_does_not_supersede_an_earlier_successful_read() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    let earlier = WorktreeListRead::from_dirty_scan();
    let later = WorktreeListRead::explicit();
    load_worktrees(&mut state, repo_id);
    worktrees_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        repo_id,
        later,
        Err(backend_error("unavailable")),
    );
    assert!(matches!(state.repos[0].worktrees, Loadable::Error(_)));
    assert_eq!(state.repos[0].worktrees_read_seq, 0);
    worktrees_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        repo_id,
        earlier,
        Ok(listing("success")),
    );
    assert_eq!(state.repos[0].worktrees_read_seq, earlier.seq);
    assert_eq!(
        state.repos[0].worktrees.ready().unwrap()[0]
            .branch
            .as_deref(),
        Some("success")
    );
}

#[test]
fn failed_background_scans_leave_collapsed_worktrees_retryable() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    for _ in 0..3 {
        assert_eq!(load_worktree_dirty(&mut state, repo_id).len(), 1);
        assert!(
            worktree_dirty_loaded(
                &mut state,
                repo_id,
                WorktreeDirtyScope::All,
                Err(backend_error("unavailable"))
            )
            .is_empty()
        );
        assert_eq!(state.repos[0].worktrees, Loadable::NotLoaded);
        assert_eq!(state.repos[0].worktrees_read_seq, 0);
        assert!(state.repos[0].feedback.diagnostics.is_empty());
    }
    assert!(matches!(
        ensure_sidebar_data(
            &mut state,
            repo_id,
            SidebarDataRequest {
                worktrees: true,
                ..Default::default()
            }
        )
        .as_slice(),
        [Effect::LoadWorktrees { .. }]
    ));
}

#[test]
fn expanded_worktrees_load_independently_of_full_scans() {
    let repo_id = RepoId(1);
    for succeeds in [true, false] {
        let mut state = new_state_with_repo(repo_id);
        mark_repo_open_ready(&mut state, repo_id);
        load_worktree_dirty(&mut state, repo_id);
        let effects = ensure_sidebar_data(
            &mut state,
            repo_id,
            SidebarDataRequest {
                worktrees: true,
                ..Default::default()
            },
        );
        assert!(matches!(effects.as_slice(), [Effect::LoadWorktrees { .. }]));
        let result = if succeeds {
            worktrees_loaded(
                &crate::store::reducer::test_inputs::clock(),
                &mut state,
                repo_id,
                WorktreeListRead::from_dirty_scan(),
                Ok(listing("ok")),
            );
            Ok(Vec::new())
        } else {
            Err(backend_error("unavailable"))
        };
        assert!(
            worktree_dirty_loaded(&mut state, repo_id, WorktreeDirtyScope::All, result).is_empty()
        );
        assert!(
            state.repos[0]
                .loads_in_flight
                .is_in_flight(LoadKind::Worktrees)
        );
        assert!(
            worktrees_loaded(
                &crate::store::reducer::test_inputs::clock(),
                &mut state,
                repo_id,
                WorktreeListRead::explicit(),
                Ok(listing("ok"))
            )
            .is_empty()
        );
        assert!(matches!(state.repos[0].worktrees, Loadable::Ready(_)));
        assert!(
            !state.repos[0]
                .loads_in_flight
                .is_in_flight(LoadKind::Worktrees)
        );
        assert!(state.repos[0].feedback.diagnostics.is_empty());
    }
}

#[test]
fn fixture_observations_do_not_finish_an_explicit_load() {
    let repo_id = RepoId(1);
    let mut state = new_state_with_repo(repo_id);
    mark_repo_open_ready(&mut state, repo_id);
    load_worktrees(&mut state, repo_id);
    worktrees_loaded(
        &crate::store::reducer::test_inputs::clock(),
        &mut state,
        repo_id,
        WorktreeListRead::observation_for_test(),
        Ok(listing("fixture")),
    );
    assert!(
        state.repos[0]
            .loads_in_flight
            .is_in_flight(LoadKind::Worktrees)
    );
}
