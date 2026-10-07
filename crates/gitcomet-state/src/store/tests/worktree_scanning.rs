//! Targeted linked-checkout refreshes must preserve the rest of the snapshot.
use super::*;
use crate::model::{RepoLoadsInFlight, WorktreeDirtyScope};
use crate::msg::InternalMsg;
use gitcomet_core::domain::{FileStatus, FileStatusKind, WorktreeDirtySummary};

fn summary(path: &str, modified: usize) -> WorktreeDirtySummary {
    WorktreeDirtySummary {
        path: PathBuf::from(path),
        head: Some(CommitId("tip".into())),
        branch: Some(path.into()),
        detached: false,
        added: 0,
        modified,
        deleted: 0,
        staged: Vec::new(),
        unstaged: vec![FileStatus {
            path: PathBuf::from("changed.txt"),
            kind: FileStatusKind::Modified,
            conflict: None,
        }],
        line_stats: Default::default(),
    }
}

fn state() -> AppState {
    let mut state = AppState::test_default();
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/repo".into(),
        },
    );
    repo.set_open(Loadable::Ready(()));
    repo.set_worktree_dirty(Loadable::Ready(vec![
        summary("/wt/a", 1),
        summary("/wt/b", 2),
    ]));
    repo.set_worktree_selection(Some("/wt/b".into()));
    state.repos.push(repo);
    state.active_repo = Some(RepoId(1));
    state
}

fn dispatch(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    reduce(&mut FxHashMap::default(), &AtomicU64::new(2), state, msg)
}

fn select(path: &str) -> Msg {
    Msg::SelectWorktreeUncommitted {
        request_id: None,
        repo_id: RepoId(1),
        path: path.into(),
    }
}

fn changed(state: &AppState, path: &str) -> Msg {
    Msg::WorktreeExternallyChanged {
        repo_id: RepoId(1),
        lifetime: state.repos[0].lifetime(),
        path: path.into(),
        change: crate::msg::RepoExternalChange::Worktree,
    }
}

fn loaded(paths: &[&str], result: Result<Vec<WorktreeDirtySummary>>) -> Msg {
    Msg::Internal(InternalMsg::WorktreeDirtyLoaded {
        repo_id: RepoId(1),
        scope: WorktreeDirtyScope::Paths(paths.iter().map(PathBuf::from).collect()),
        result,
    })
}

fn scan(effects: &[Effect]) -> (&WorktreeDirtyScope, &Option<PathBuf>) {
    let mut scans = effects.iter().filter_map(|effect| match effect {
        Effect::LoadWorktreeDirty {
            scope, files_for, ..
        } => Some((scope, files_for)),
        _ => None,
    });
    let scan = scans.next().expect("one linked checkout scan");
    assert!(scans.next().is_none(), "refreshes must coalesce");
    scan
}

#[test]
fn selection_and_checkout_watch_refresh_only_the_named_checkout() {
    for watch in [false, true] {
        let mut state = state();
        let msg = if watch {
            changed(&state, "/wt/a")
        } else {
            select("/wt/a")
        };
        let effects = dispatch(&mut state, msg);
        let (scope, files_for) = scan(&effects);
        assert_eq!(*scope, WorktreeDirtyScope::Paths(vec!["/wt/a".into()]));
        assert_eq!(
            *files_for,
            Some(if watch { "/wt/b" } else { "/wt/a" }.into())
        );
    }
}

#[test]
fn targeted_updates_preserve_other_rows_and_their_selected_files() {
    let mut state = state();
    let selected = state.repos[0].worktree_dirty.ready().unwrap()[1].clone();
    dispatch(
        &mut state,
        loaded(&["/wt/a"], Ok(vec![summary("/wt/a", 3)])),
    );
    let dirty = state.repos[0].worktree_dirty.ready().unwrap();
    assert_eq!(dirty.len(), 2);
    assert_eq!(dirty[0].modified, 3);
    assert!(
        dirty[0].unstaged.is_empty(),
        "unselected lists stay discarded"
    );
    assert_eq!(
        dirty[1], selected,
        "the untouched selected checkout keeps its files"
    );
    assert_eq!(
        state.repos[0].history_state.worktree_selection.as_deref(),
        Some(Path::new("/wt/b"))
    );

    // A clean or removed checkout deletes only its own row.
    dispatch(&mut state, loaded(&["/wt/a"], Ok(Vec::new())));
    assert_eq!(
        state.repos[0].worktree_dirty.ready().unwrap().as_ref(),
        &[selected]
    );
    dispatch(&mut state, loaded(&["/wt/b"], Ok(Vec::new())));
    assert!(state.repos[0].worktree_dirty.ready().unwrap().is_empty());
    assert!(state.repos[0].history_state.worktree_selection.is_none());
}

#[test]
fn another_checkout_update_does_not_reload_the_open_diff() {
    let mut state = state();
    let entries = crate::model::worktree_inline_diff_entries(
        &state.repos[0].worktree_dirty.ready().unwrap()[1],
    );
    dispatch(
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            repo_id: RepoId(1),
            origin: crate::model::ForeignDiffOrigin::Worktree {
                branch: Some("/wt/b".into()),
                detached: false,
            },
            submodule_repo_path: "/wt/b".into(),
            parent_submodule_path: "/wt/b".into(),
            entries: entries.into(),
            selected_ix: 0,
        },
    );
    let previous = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .clone()
        .expect("open linked-worktree diff");
    let effects = dispatch(
        &mut state,
        loaded(&["/wt/a"], Ok(vec![summary("/wt/a", 3)])),
    );
    assert!(
        effects.is_empty(),
        "another checkout must not trigger diff reads: {effects:?}"
    );
    let current = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff remains open");
    assert_eq!(current.target, previous.target);
    assert_eq!(current.diff_rev, previous.diff_rev);
    assert_eq!(current.diff_file_rev, previous.diff_file_rev);
    assert!(Arc::ptr_eq(&current.entries, &previous.entries));
}

#[test]
fn targeted_updates_add_new_dirty_rows_and_keep_previous_rows_on_cancellation() {
    let mut state = state();
    dispatch(
        &mut state,
        loaded(&["/wt/c"], Ok(vec![summary("/wt/c", 4)])),
    );
    let previous = state.repos[0].worktree_dirty.clone();
    assert_eq!(previous.ready().unwrap().len(), 3);
    dispatch(
        &mut state,
        loaded(&["/wt/b"], Err(Error::new(ErrorKind::Cancelled))),
    );
    assert_eq!(state.repos[0].worktree_dirty, previous);
    assert_eq!(
        state.repos[0].history_state.worktree_selection.as_deref(),
        Some(Path::new("/wt/b"))
    );
}

#[test]
fn queued_watch_paths_are_merged_and_latest_selection_gets_its_files() {
    let mut state = state();
    scan(&dispatch(&mut state, select("/wt/a")));
    let msg = changed(&state, "/wt/c");
    assert!(dispatch(&mut state, msg).is_empty());
    assert!(dispatch(&mut state, select("/wt/b")).is_empty());
    let msg = changed(&state, "/wt/c");
    assert!(dispatch(&mut state, msg).is_empty());
    let effects = dispatch(
        &mut state,
        loaded(&["/wt/a"], Ok(vec![summary("/wt/a", 1)])),
    );
    let (scope, files_for) = scan(&effects);
    assert_eq!(
        *scope,
        WorktreeDirtyScope::Paths(vec!["/wt/c".into(), "/wt/b".into()])
    );
    assert_eq!(*files_for, Some("/wt/b".into()));
    let effects = dispatch(
        &mut state,
        loaded(&["/wt/c", "/wt/b"], Ok(vec![summary("/wt/b", 2)])),
    );
    assert!(effects.is_empty());
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::WORKTREE_DIRTY)
    );
}

#[test]
fn queued_full_refresh_takes_precedence_over_targeted_refreshes() {
    for full_first in [false, true] {
        let mut state = state();
        scan(&dispatch(&mut state, select("/wt/a")));
        let full = Msg::LoadWorktreeDirty { repo_id: RepoId(1) };
        let targeted = changed(&state, "/wt/c");
        let messages = if full_first {
            [full, targeted]
        } else {
            [targeted, full]
        };
        for msg in messages {
            assert!(dispatch(&mut state, msg).is_empty());
        }
        let effects = dispatch(
            &mut state,
            loaded(&["/wt/a"], Ok(vec![summary("/wt/a", 1)])),
        );
        assert_eq!(*scan(&effects).0, WorktreeDirtyScope::All);
        dispatch(
            &mut state,
            Msg::Internal(InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                scope: WorktreeDirtyScope::All,
                result: Ok(vec![summary("/wt/a", 1)]),
            }),
        );
        assert_eq!(
            state.repos[0].worktree_dirty.ready().unwrap().len(),
            1,
            "full results replace the snapshot"
        );
    }
}

#[test]
fn first_checkout_refresh_establishes_a_full_snapshot() {
    let mut state = state();
    state.repos[0].set_worktree_dirty(Loadable::NotLoaded);
    let effects = dispatch(&mut state, select("/wt/a"));
    assert_eq!(*scan(&effects).0, WorktreeDirtyScope::All);
}

#[test]
fn failed_initial_scan_replays_a_full_snapshot_before_targeted_updates() {
    let mut state = state();
    state.repos[0].set_worktree_dirty(Loadable::NotLoaded);
    scan(&dispatch(&mut state, select("/wt/a")));
    let msg = changed(&state, "/wt/b");
    assert!(dispatch(&mut state, msg).is_empty());
    let effects = dispatch(
        &mut state,
        Msg::Internal(InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: WorktreeDirtyScope::All,
            result: Err(Error::new(ErrorKind::Cancelled)),
        }),
    );
    assert_eq!(*scan(&effects).0, WorktreeDirtyScope::All);
}
