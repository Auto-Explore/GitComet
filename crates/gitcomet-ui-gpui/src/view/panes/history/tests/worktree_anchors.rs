use super::*;

#[test]
fn a_dirty_worktree_anchors_to_its_head_commit() {
    let commits = vec![
        commit("c0", &["c1"], "newest"),
        commit("c1", &["c2"], "middle"),
        commit("c2", &[], "oldest"),
    ];
    let worktrees = [("/wt/a", "c1"), ("/wt/b", "c2")];
    assert_eq!(
        worktree_anchors_for(&commits, &worktrees, &["/wt/a"]),
        vec![1]
    );
    assert_eq!(
        worktree_anchors_for(&commits, &worktrees, &["/wt/a", "/wt/b"]),
        vec![1, 2]
    );
}

#[test]
fn a_worktree_whose_head_is_not_on_screen_gets_no_row() {
    let commits = vec![commit("c0", &["c1"], "newest"), commit("c1", &[], "older")];
    // `c9` is on a branch outside the current scope, or past the loaded page.
    let worktrees = [("/wt/offscreen", "c9")];
    assert!(
        worktree_anchors_for(&commits, &worktrees, &["/wt/offscreen"]).is_empty(),
        "a worktree with no visible HEAD must not be anchored anywhere"
    );
}

#[test]
fn a_clean_worktree_gets_no_row_even_though_it_is_listed() {
    let commits = vec![commit("c0", &[], "only")];
    let worktrees = [("/wt/clean", "c0")];
    // `dirty_paths` is the scan's output, which only ever lists dirty trees.
    assert!(worktree_anchors_for(&commits, &worktrees, &[]).is_empty());
}

/// The plan must place the rows the anchors describe, in log order.
#[test]
fn anchors_become_rows_above_their_commits() {
    let commits = vec![
        commit("c0", &["c1"], "newest"),
        commit("c1", &["c2"], "middle"),
        commit("c2", &[], "oldest"),
    ];
    let worktrees = [("/wt/a", "c2"), ("/wt/b", "c0")];
    let anchors = worktree_anchors_for(&commits, &worktrees, &["/wt/a", "/wt/b"]);
    let plan = HistoryListPlan::new(
        true,
        anchors
            .iter()
            .enumerate()
            .map(|(worktree_ix, &visible_ix)| HistoryWorktreeRowAnchor {
                visible_ix,
                worktree_ix,
            })
            .collect(),
    );

    // working tree row, wt/b above c0, c0, c1, wt/a above c2, c2
    assert_eq!(plan.list_len(3), 6);
    assert_eq!(plan.list_ix_for_visible(0), 2);
    assert_eq!(plan.list_ix_for_visible(2), 5);
    assert_eq!(plan.list_ix_for_worktree(1), Some(1));
    assert_eq!(plan.list_ix_for_worktree(0), Some(4));
}
