use super::*;

fn anchor(visible_ix: usize, worktree_ix: usize) -> HistoryWorktreeRowAnchor {
    HistoryWorktreeRowAnchor {
        visible_ix,
        worktree_ix,
    }
}

/// Every list index maps to exactly one row, and every commit round-trips
/// back to the index it was placed at.
fn assert_plan_round_trips(plan: &HistoryListPlan, visible_len: usize) {
    let len = plan.list_len(visible_len);
    let mut seen_commits = Vec::new();
    let mut seen_worktrees = Vec::new();
    for list_ix in 0..len {
        match plan.row_at(list_ix).expect("row within list length") {
            HistoryListRow::WorkingTreeSummary => assert_eq!(list_ix, 0),
            HistoryListRow::WorktreeUncommitted { visible_ix, .. } => {
                seen_worktrees.push((list_ix, visible_ix))
            }
            HistoryListRow::Commit { visible_ix } => {
                assert_eq!(
                    plan.list_ix_for_visible(visible_ix),
                    list_ix,
                    "commit {visible_ix} did not round-trip"
                );
                seen_commits.push(visible_ix);
            }
        }
    }
    assert_eq!(seen_commits, (0..visible_len).collect::<Vec<_>>());
    assert_eq!(seen_worktrees.len(), plan.worktree_row_count());
    // A worktree row always renders directly above the commit it anchors to.
    for (list_ix, visible_ix) in seen_worktrees {
        assert!(plan.list_ix_for_visible(visible_ix) > list_ix);
    }
}

#[test]
fn a_plan_without_synthetic_rows_is_the_identity() {
    let plan = HistoryListPlan::new(false, Vec::new());
    assert_eq!(plan.list_len(5), 5);
    assert_eq!(plan.list_ix_for_visible(3), 3);
    assert_eq!(
        plan.row_at(3),
        Some(HistoryListRow::Commit { visible_ix: 3 })
    );
    assert_plan_round_trips(&plan, 5);
}

#[test]
fn the_working_tree_row_shifts_every_commit_by_one() {
    let plan = HistoryListPlan::new(true, Vec::new());
    assert_eq!(plan.list_len(5), 6);
    assert_eq!(plan.row_at(0), Some(HistoryListRow::WorkingTreeSummary));
    assert_eq!(plan.list_ix_for_visible(0), 1);
    assert_plan_round_trips(&plan, 5);
}

#[test]
fn a_worktree_row_sits_directly_above_its_head_commit() {
    let plan = HistoryListPlan::new(false, vec![anchor(2, 0)]);
    assert_eq!(plan.list_len(5), 6);
    assert_eq!(
        plan.row_at(1),
        Some(HistoryListRow::Commit { visible_ix: 1 })
    );
    assert_eq!(
        plan.row_at(2),
        Some(HistoryListRow::WorktreeUncommitted {
            visible_ix: 2,
            worktree_ix: 0,
        })
    );
    assert_eq!(
        plan.row_at(3),
        Some(HistoryListRow::Commit { visible_ix: 2 })
    );
    assert_eq!(plan.list_ix_for_visible(2), 3);
    assert_plan_round_trips(&plan, 5);
}

#[test]
fn several_worktrees_on_the_same_commit_stack_above_it() {
    let plan = HistoryListPlan::new(true, vec![anchor(1, 0), anchor(1, 1)]);
    assert_eq!(plan.list_len(4), 7);
    assert_eq!(
        plan.row_at(2),
        Some(HistoryListRow::WorktreeUncommitted {
            visible_ix: 1,
            worktree_ix: 0,
        })
    );
    assert_eq!(
        plan.row_at(3),
        Some(HistoryListRow::WorktreeUncommitted {
            visible_ix: 1,
            worktree_ix: 1,
        })
    );
    assert_eq!(plan.list_ix_for_visible(1), 4);
    assert_plan_round_trips(&plan, 4);
}

#[test]
fn anchors_are_sorted_so_construction_order_does_not_matter() {
    let scrambled = HistoryListPlan::new(false, vec![anchor(4, 1), anchor(0, 0), anchor(2, 2)]);
    let ordered = HistoryListPlan::new(false, vec![anchor(0, 0), anchor(2, 2), anchor(4, 1)]);
    for list_ix in 0..ordered.list_len(6) {
        assert_eq!(scrambled.row_at(list_ix), ordered.row_at(list_ix));
    }
    assert_plan_round_trips(&scrambled, 6);
}

/// `row_at` is deliberately unbounded above: the plan never learns how many
/// commits are visible (`list_len` takes that as an argument), so an index
/// past the end still resolves to a commit row that is simply not there.
/// Every caller bounds the index itself before looking the commit up. Pinned
/// here so a caller that forgets is a bug in the caller, not a surprise from
/// a method that looks like it range-checks.
#[test]
fn row_at_leaves_the_upper_bound_to_its_callers() {
    let plan = HistoryListPlan::new(true, vec![anchor(0, 0)]);
    let len = plan.list_len(2);
    assert_eq!(
        plan.row_at(len - 1),
        Some(HistoryListRow::Commit { visible_ix: 1 }),
        "the last in-range row is the last visible commit"
    );
    assert_eq!(
        plan.row_at(len),
        Some(HistoryListRow::Commit { visible_ix: 2 }),
        "one past the end keeps counting commits instead of returning None"
    );
    assert_plan_round_trips(&plan, 2);
}
