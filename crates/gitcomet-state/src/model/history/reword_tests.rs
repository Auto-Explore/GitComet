use super::*;
use crate::model::RepoId;
use gitcomet_core::history_index::{HistoryIndexBuilder, HistoryIndexHandle};
use gitcomet_core::services::{CancellationToken, HistorySnapshot};
use gitcomet_core::squash::FirstParentPosition;

fn id(value: u8) -> CommitId {
    CommitId(format!("{value:02x}").repeat(20).into())
}

fn commit(value: u8, parents: &[u8]) -> Commit {
    Commit {
        id: id(value),
        parent_ids: parents.iter().map(|&parent| id(parent)).collect(),
        author: "Author".into(),
        summary: "Subject".into(),
        time: std::time::UNIX_EPOCH,
    }
}

fn page(topology: &[(u8, &[u8])]) -> Shared<LogPage> {
    Arc::new(LogPage {
        commits: topology
            .iter()
            .map(|&(value, parents)| commit(value, parents))
            .collect(),
        next_cursor: None,
    })
}

fn index(snapshot: &str, topology: &[(u8, &[u8])]) -> HistoryIndexHandle {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot(snapshot.into()),
        HistoryMode::FullReachable,
        20,
    )
    .unwrap();
    for &(value, parents) in topology {
        let parents: Vec<_> = parents.iter().map(|&parent| [parent; 20]).collect();
        builder
            .push(
                &[value; 20],
                parents.iter().map(|parent| parent.as_slice()),
                false,
            )
            .unwrap();
    }
    builder.finish(&CancellationToken::new()).unwrap()
}

fn repo() -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/reword-position"),
        },
    );
    repo.head_branch = Loadable::Ready("HEAD".into());
    repo.detached_head_commit = Some(id(4));
    repo.history_state.selected_commit = Some(id(2));
    repo.set_log(Loadable::Ready(page(&[
        (4, &[3]),
        (3, &[2]),
        (2, &[1]),
        (1, &[]),
    ])));
    repo.history_state.log_snapshot = Some(HistorySnapshot("initial".into()));
    repo
}

#[test]
fn reword_position_reads_only_the_prepared_selection() {
    let mut repo = repo();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    assert_eq!(
        repo.first_parent_position_from_head(&id(4)),
        FirstParentPosition::Linear { distance: 0 }
    );

    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 2 }
    );
    assert_eq!(
        repo.first_parent_position_from_head(&id(3)),
        FirstParentPosition::NotFound
    );

    repo.history_state.selected_commit = Some(id(3));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    assert_eq!(
        repo.first_parent_position_from_head(&id(3)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(3)),
        FirstParentPosition::Linear { distance: 1 }
    );

    repo.history_state.selected_commit = None;
    repo.prepare_commit_reword_position();
    assert!(repo.history_state.reword_position_cache.is_none());
    assert_eq!(
        repo.first_parent_position_from_head(&id(3)),
        FirstParentPosition::NotFound
    );
}

#[test]
fn reword_position_reuses_the_walk_across_unrelated_updates_and_snapshots() {
    let mut repo = repo();
    repo.prepare_commit_reword_position();
    let cached = repo.history_state.reword_position_cache.clone().unwrap();
    let snapshot = repo.clone();
    for _ in 0..20 {
        repo.ops_rev += 1;
        repo.worktree_dirty_rev += 1;
        repo.prepare_commit_reword_position();
        assert!(Arc::ptr_eq(
            &cached,
            repo.history_state.reword_position_cache.as_ref().unwrap()
        ));
        assert_eq!(
            repo.first_parent_position_from_head(&id(2)),
            FirstParentPosition::Linear { distance: 2 }
        );
    }

    repo.log_rev += 1;
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert!(!Arc::ptr_eq(
        &cached,
        repo.history_state.reword_position_cache.as_ref().unwrap()
    ));
    assert_eq!(
        snapshot.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 2 }
    );
}

#[test]
fn reword_position_invalidates_on_detached_and_attached_head_changes() {
    let mut repo = repo();
    repo.prepare_commit_reword_position();
    repo.detached_head_commit = Some(id(3));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 1 }
    );

    repo.head_branch = Loadable::Ready("main".into());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".into(),
        target: id(4),
        upstream: None,
        divergence: None,
    }]));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 2 }
    );

    // A missing HEAD must never retain the previous eligibility.
    repo.head_branch = Loadable::NotLoaded;
    repo.detached_head_commit = None;
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
}

#[test]
fn reword_position_invalidates_when_a_page_is_replaced_or_unavailable() {
    let mut repo = repo();
    repo.prepare_commit_reword_position();
    // Pointer identity also catches replacement without an accompanying revision.
    repo.log = Loadable::Ready(page(&[(4, &[3, 1]), (3, &[2]), (2, &[1]), (1, &[])]));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::BelowMerge
    );

    repo.log = Loadable::Loading;
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
}

#[test]
fn reword_position_uses_only_an_index_matching_the_log_snapshot() {
    let mut repo = repo();
    repo.set_log(Loadable::Ready(page(&[(4, &[2]), (2, &[1]), (1, &[])])));
    repo.history_state.log_snapshot = Some(HistorySnapshot("initial".into()));
    repo.history_state.indexed.index = Some(index(
        "initial",
        &[(4, &[3]), (3, &[2]), (2, &[1]), (1, &[])],
    ));
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 2 }
    );

    repo.history_state.log_snapshot = Some(HistorySnapshot("replacement".into()));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 1 }
    );

    repo.history_state.indexed.index = Some(index(
        "replacement",
        &[(4, &[3, 1]), (3, &[2]), (2, &[1]), (1, &[])],
    ));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::BelowMerge
    );

    // Replacing an index for the same snapshot must invalidate as well.
    repo.history_state.indexed.index =
        Some(index("replacement", &[(4, &[2]), (2, &[1]), (1, &[])]));
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::NotFound
    );
    repo.prepare_commit_reword_position();
    assert_eq!(
        repo.first_parent_position_from_head(&id(2)),
        FirstParentPosition::Linear { distance: 1 }
    );
}

#[test]
fn reword_position_caches_missing_targets_in_pages_and_indexes() {
    for indexed in [false, true] {
        let mut repo = repo();
        repo.history_state.selected_commit = Some(id(9));
        if indexed {
            repo.history_state.indexed.index = Some(index(
                "initial",
                &[(4, &[3]), (3, &[2]), (2, &[1]), (1, &[])],
            ));
        }
        repo.prepare_commit_reword_position();
        assert_eq!(
            repo.first_parent_position_from_head(&id(9)),
            FirstParentPosition::NotFound
        );
        let cached = repo.history_state.reword_position_cache.clone().unwrap();
        repo.prepare_commit_reword_position();
        assert!(Arc::ptr_eq(
            &cached,
            repo.history_state.reword_position_cache.as_ref().unwrap()
        ));
    }
}
