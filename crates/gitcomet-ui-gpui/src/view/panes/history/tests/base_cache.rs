use super::*;

/// The commit-id index the base cache carries agrees with the visible order it
/// was built from.
///
/// Its readers -- the worktree row anchors and the selected lane's colour --
/// look commits up during layout, and both used to scan the page instead. A
/// map that disagrees with `visible_indices` would anchor rows on the wrong
/// commits, so this pins the two together.
#[test]
fn the_base_cache_indexes_every_visible_commit_by_id() {
    let commits = vec![
        commit("c0", &["c1"], "newest"),
        commit("c1", &["c2"], "middle"),
        commit("c2", &[], "oldest"),
    ];
    let page = log_page(commits, None);
    let base = build_history_base_cache(
        HistoryBaseCacheRequest {
            repo_id: RepoId(1),
            history_scope: LogScope::AllBranches,
            log_fingerprint: 0,
            head_branch_rev: 0,
            detached_head_commit: None,
            head_branch_target: None,
            branches_rev: 0,
            remote_branches_rev: 0,
            stashes_rev: 0,
        },
        &page,
        AppTheme::gitcomet_dark(),
        None,
        &[],
        &[],
        &[],
    );

    for (visible_ix, commit_ix) in base.visible_indices.iter().enumerate() {
        let id = &page.commits[commit_ix].id;
        assert_eq!(
            base.visible_ix_by_commit.get(id).copied(),
            Some(visible_ix),
            "{id:?} should resolve to the row it renders at"
        );
    }
    assert_eq!(base.visible_ix_by_commit.len(), base.visible_indices.len());
    assert_eq!(
        base.visible_ix_by_commit.get(&CommitId("absent".into())),
        None
    );
}
