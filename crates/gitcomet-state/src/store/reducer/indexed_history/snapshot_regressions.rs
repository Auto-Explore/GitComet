use super::*;

#[test]
fn indexed_history_snapshot_clones_share_a_hundred_thousand_selected_ids() {
    let mut repo = crate::model::RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: "/tmp/selection-sharing".into(),
        },
    );
    repo.set_commit_multi_selection(crate::model::CommitMultiSelection {
        commits: Arc::new(
            (0..100_000)
                .map(|row| gitcomet_core::domain::CommitId(format!("{row:040x}").into()))
                .collect(),
        ),
        ..Default::default()
    });
    let next = repo.clone();
    assert!(Arc::ptr_eq(
        &repo.history_state.multi_selection.commits,
        &next.history_state.multi_selection.commits
    ));
    assert_eq!(next.history_state.multi_selection.commits.len(), 100_000);
}
