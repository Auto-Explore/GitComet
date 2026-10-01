use super::*;
use crate::model::RepoState;
use gitcomet_core::domain::{CommitId, LogScope, RepoSpec};

#[test]
fn history_selection_acknowledges_noops_and_rejected_requests_by_id() {
    let repo_id = RepoId(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/tmp/history-selection-ack".into(),
        },
    ));
    let mut repos = FxHashMap::default();
    let ids = AtomicU64::new(2);
    for request_id in [11, 12] {
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::SelectCommit {
                repo_id,
                request_id: Some(request_id),
                commit_id: CommitId("1111111".into()),
            },
        );
        assert_eq!(state.repos[0].history_state.selection_ack, Some(request_id));
    }
    let revision = state.repos[0].history_state.selected_commit_rev;
    let index = gitcomet_core::history_index::HistoryIndexBuilder::new(
        gitcomet_core::services::HistorySnapshot("stale".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap()
    .finish(&gitcomet_core::services::CancellationToken::new())
    .unwrap();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::IndexedHistory(crate::indexed_history::IndexedHistoryMsg::Select {
            request_id: Some(13),
            repo_id,
            commit_id: CommitId("2222222".into()),
            mode: crate::msg::CommitSelectMode::Single,
            projection: gitcomet_core::history_index::HistoryProjection::new(index, Vec::new()),
        }),
    );
    assert_eq!(state.repos[0].history_state.selection_ack, Some(13));
    assert_eq!(state.repos[0].history_state.selected_commit_rev, revision);
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SelectCommit {
            repo_id,
            request_id: None,
            commit_id: CommitId("3333333".into()),
        },
    );
    assert_eq!(state.repos[0].history_state.selection_ack, Some(13));
}
