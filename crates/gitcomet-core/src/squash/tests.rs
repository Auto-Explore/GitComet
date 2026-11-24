use super::*;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

fn id(s: &str) -> CommitId {
    CommitId(Arc::from(s))
}

fn commit(sha: &str, parents: &[&str], age: u64) -> Commit {
    Commit {
        id: id(sha),
        parent_ids: parents.iter().map(|p| id(p)).collect(),
        summary: Arc::from(sha),
        author: Arc::from("author"),
        time: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age),
    }
}

/// d (HEAD) -> c -> b -> a -> root
fn linear_log() -> Vec<Commit> {
    vec![
        commit("d", &["c"], 0),
        commit("c", &["b"], 10),
        commit("b", &["a"], 20),
        commit("a", &["root"], 30),
        commit("root", &[], 40),
    ]
}

#[test]
fn eligible_range_ending_at_head_returns_plan() {
    let log = linear_log();
    let plan = squash_eligibility(&log, &[id("c"), id("d"), id("b")], &id("d")).expect("eligible");
    assert_eq!(plan.actual_head, id("d"));
    assert_eq!(plan.head, id("d"));
    assert_eq!(plan.oldest, id("b"));
    assert_eq!(plan.oldest_parent, id("a"));
    assert_eq!(plan.commit_count, 3);
    assert_eq!(*plan.ordered_ids, vec![id("d"), id("c"), id("b")]);
}

#[test]
fn intermediate_range_in_linear_chain_is_eligible() {
    let log = linear_log();
    let plan = squash_eligibility(&log, &[id("c"), id("b")], &id("d")).expect("eligible");
    assert_eq!(plan.actual_head, id("d"));
    assert_eq!(plan.head, id("c"));
    assert_eq!(plan.oldest, id("b"));
    assert_eq!(plan.oldest_parent, id("a"));
    assert_eq!(plan.commit_count, 2);
    assert_eq!(*plan.ordered_ids, vec![id("c"), id("b")]);
}

#[test]
fn single_selection_is_rejected() {
    let log = linear_log();
    assert!(squash_eligibility(&log, &[id("d")], &id("d")).is_none());
}

#[test]
fn selection_outside_loaded_page_is_rejected() {
    let log = linear_log();
    assert!(squash_eligibility(&log, &[id("d"), id("missing")], &id("d")).is_none());
}

#[test]
fn non_contiguous_selection_is_rejected() {
    let log = linear_log();
    assert!(squash_eligibility(&log, &[id("d"), id("b")], &id("d")).is_none());
}

#[test]
fn selection_unreachable_from_head_is_rejected() {
    let log = linear_log();
    assert!(squash_eligibility(&log, &[id("d"), id("c")], &id("other")).is_none());
}

#[test]
fn intermediate_range_rejected_when_merge_blocks_chain() {
    // A merge commit sits between HEAD and the selected range, so the
    // first-parent walk can't continue past it.
    let log = vec![
        commit("d", &["m1", "m2"], 0),
        commit("m1", &["c"], 5),
        commit("m2", &["x"], 6),
        commit("c", &["b"], 10),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert!(squash_eligibility(&log, &[id("c"), id("b")], &id("d")).is_none());
}

#[test]
fn merge_commit_in_range_is_rejected() {
    let log = vec![
        commit("d", &["c", "x"], 0),
        commit("c", &["b"], 10),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert!(squash_eligibility(&log, &[id("d"), id("c")], &id("d")).is_none());
}

#[test]
fn parent_chain_gap_is_rejected() {
    // Contiguous in log order but "c" is not "d"'s parent (interleaved branch).
    let log = vec![
        commit("d", &["b"], 0),
        commit("c", &["a"], 10),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert!(squash_eligibility(&log, &[id("d"), id("c")], &id("d")).is_none());
}

#[test]
fn root_as_oldest_is_rejected() {
    let log = linear_log();
    assert!(
        squash_eligibility(
            &log,
            &[id("d"), id("c"), id("b"), id("a"), id("root")],
            &id("d"),
        )
        .is_none()
    );
}

#[test]
fn duplicate_selected_ids_are_rejected() {
    let log = linear_log();
    assert!(squash_eligibility(&log, &[id("d"), id("d")], &id("d")).is_none());
}

#[test]
fn message_combines_oldest_first_with_paragraph_breaks() {
    let messages = vec![
        "Oldest subject\n\nOldest body\n".to_string(),
        "Middle change".to_string(),
        "Newest change\n".to_string(),
    ];
    assert_eq!(
        build_squash_message(&messages),
        "Oldest subject\n\nOldest body\n\nMiddle change\n\nNewest change"
    );
}

#[test]
fn message_skips_empty_entries() {
    let messages = vec![
        "Subject".to_string(),
        "  \n".to_string(),
        "Tail".to_string(),
    ];
    assert_eq!(build_squash_message(&messages), "Subject\n\nTail");
}

#[test]
fn interleaved_stash_helper_does_not_break_eligibility() {
    // A stash-helper commit is sorted into the page between real commits,
    // but is not part of the branch's first-parent chain and is excluded
    // from the selection. The range d,c,b stays eligible.
    let log = vec![
        commit("d", &["c"], 0),
        commit("c", &["b"], 10),
        commit("stash", &["a", "idx"], 15),
        commit("b", &["a"], 20),
        commit("a", &["root"], 30),
        commit("root", &[], 40),
    ];
    let plan = squash_eligibility(&log, &[id("d"), id("c"), id("b")], &id("d")).expect("eligible");
    assert_eq!(plan.actual_head, id("d"));
    assert_eq!(plan.head, id("d"));
    assert_eq!(plan.oldest, id("b"));
    assert_eq!(plan.oldest_parent, id("a"));
    assert_eq!(plan.commit_count, 3);
    assert_eq!(*plan.ordered_ids, vec![id("d"), id("c"), id("b")]);
}

#[test]
fn selection_off_the_head_chain_is_rejected() {
    // "x" is in the page and count matches, but is not on d's first-parent
    // chain, so the walk from d never reaches it.
    let log = vec![
        commit("d", &["c"], 0),
        commit("x", &["c"], 5),
        commit("c", &["b"], 10),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert!(squash_eligibility(&log, &[id("d"), id("c"), id("x")], &id("d")).is_none());
}

#[test]
fn linear_distance_counts_range_size() {
    let log = linear_log();
    assert_eq!(
        linear_first_parent_distance(&log, &id("d"), &id("a")),
        Some(3)
    );
    assert_eq!(
        linear_first_parent_distance(&log, &id("d"), &id("c")),
        Some(1)
    );
    assert_eq!(
        linear_first_parent_distance(&log, &id("d"), &id("d")),
        Some(0)
    );
}

#[test]
fn linear_distance_rejects_target_off_the_page() {
    let log = linear_log();
    assert_eq!(
        linear_first_parent_distance(&log, &id("d"), &id("zzz")),
        None
    );
}

#[test]
fn linear_distance_rejects_merge_on_the_chain() {
    // d..HEAD would also replay the merge's side branch, so a first-parent
    // count would understate the range.
    let log = vec![
        commit("e", &["m"], 0),
        commit("m", &["c", "x"], 5),
        commit("c", &["b"], 10),
        commit("x", &["b"], 11),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert_eq!(linear_first_parent_distance(&log, &id("e"), &id("b")), None);
}

#[test]
fn linear_distance_tolerates_interleaved_rows() {
    // Rows not on the chain (e.g. stash helpers) are skipped by the
    // resuming scan as long as children precede parents.
    let log = vec![
        commit("d", &["c"], 0),
        commit("stash", &["a", "idx"], 5),
        commit("c", &["b"], 10),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert_eq!(
        linear_first_parent_distance(&log, &id("d"), &id("b")),
        Some(2)
    );
}

#[test]
fn first_parent_position_tells_linear_merged_and_missing_apart() {
    let log = vec![
        commit("e", &["m"], 0),
        commit("m", &["c", "x"], 5),
        commit("c", &["b"], 10),
        commit("x", &["b"], 11),
        commit("b", &["a"], 20),
        commit("a", &[], 30),
    ];
    assert_eq!(
        first_parent_position(&log, &id("e"), &id("e")),
        FirstParentPosition::Linear { distance: 0 }
    );
    assert_eq!(
        first_parent_position(&log, &id("e"), &id("m")),
        FirstParentPosition::Linear { distance: 1 }
    );
    assert_eq!(
        first_parent_position(&log, &id("e"), &id("c")),
        FirstParentPosition::BelowMerge
    );
    // Only reachable through the merge's second parent.
    assert_eq!(
        first_parent_position(&log, &id("e"), &id("x")),
        FirstParentPosition::NotFound
    );
    assert_eq!(
        first_parent_position(&log, &id("e"), &id("zzz")),
        FirstParentPosition::NotFound
    );
}

fn rebase_entry(action: InteractiveRebaseAction, sha: &str) -> InteractiveRebaseEntry {
    InteractiveRebaseEntry {
        action,
        commit_id: sha.to_string(),
        summary: format!("summary {sha}"),
        message: format!("summary {sha}"),
        new_message: None,
    }
}

#[test]
fn reword_seed_is_full_message_for_plain_reword() {
    let entries = vec![rebase_entry(InteractiveRebaseAction::Reword, "a")];
    assert_eq!(reword_seed_message(&entries, 0), "summary a");
}

#[test]
fn reword_seed_prefers_prior_edit() {
    let mut entry = rebase_entry(InteractiveRebaseAction::Reword, "a");
    entry.new_message = Some("edited".to_string());
    assert_eq!(reword_seed_message(&[entry], 0), "edited");
}

#[test]
fn reword_seed_combines_squashed_messages() {
    let entries = vec![
        rebase_entry(InteractiveRebaseAction::Reword, "a"),
        rebase_entry(InteractiveRebaseAction::Squash, "b"),
        rebase_entry(InteractiveRebaseAction::Squash, "c"),
    ];
    assert_eq!(
        reword_seed_message(&entries, 0),
        "summary a\n\nsummary b\n\nsummary c"
    );
}

#[test]
fn reword_seed_omits_fixup_messages() {
    let entries = vec![
        rebase_entry(InteractiveRebaseAction::Reword, "a"),
        rebase_entry(InteractiveRebaseAction::Fixup, "b"),
    ];
    assert_eq!(reword_seed_message(&entries, 0), "summary a");
}

#[test]
fn reword_seed_stops_at_next_commit() {
    let entries = vec![
        rebase_entry(InteractiveRebaseAction::Reword, "a"),
        rebase_entry(InteractiveRebaseAction::Squash, "b"),
        rebase_entry(InteractiveRebaseAction::Pick, "c"),
        rebase_entry(InteractiveRebaseAction::Squash, "d"),
    ];
    assert_eq!(reword_seed_message(&entries, 0), "summary a\n\nsummary b");
}

#[test]
fn run_final_entry_is_last_squash() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Reword, "a"),
        rebase_entry(Squash, "b"),
        rebase_entry(Squash, "c"),
        rebase_entry(Pick, "d"),
    ];
    assert_eq!(squash_run_final_entry(&entries, 0), Some(2));
}

#[test]
fn run_final_entry_is_trailing_fixup_after_squash() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Reword, "a"),
        rebase_entry(Squash, "b"),
        rebase_entry(Fixup, "c"),
    ];
    // The run contains a squash, so git opens the editor at the run's last
    // fold step even though that step is a fixup.
    assert_eq!(squash_run_final_entry(&entries, 0), Some(2));
}

#[test]
fn run_final_entry_skips_drops_within_the_run() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Reword, "a"),
        rebase_entry(Squash, "b"),
        rebase_entry(Drop, "c"),
        rebase_entry(Squash, "d"),
    ];
    assert_eq!(squash_run_final_entry(&entries, 0), Some(3));
}

#[test]
fn run_final_entry_none_for_plain_reword() {
    use InteractiveRebaseAction::*;
    let entries = vec![rebase_entry(Reword, "a"), rebase_entry(Pick, "b")];
    assert_eq!(squash_run_final_entry(&entries, 0), None);
}

#[test]
fn run_final_entry_none_for_fixup_only_run() {
    use InteractiveRebaseAction::*;
    // Fixup-only runs open no squash editor; the target's reword step is
    // the only editor invocation.
    let entries = vec![rebase_entry(Reword, "a"), rebase_entry(Fixup, "b")];
    assert_eq!(squash_run_final_entry(&entries, 0), None);
}

#[test]
fn run_final_entry_stops_at_next_standalone_commit() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Reword, "a"),
        rebase_entry(Squash, "b"),
        rebase_entry(Pick, "c"),
        rebase_entry(Squash, "d"),
    ];
    assert_eq!(squash_run_final_entry(&entries, 0), Some(1));
}

#[test]
fn split_subject_body_handles_conventional_message() {
    let (subject, body) = split_subject_body("Fix parser\n\nHandle CRLF endings\n");
    assert_eq!(subject, "Fix parser");
    assert_eq!(body, "Handle CRLF endings\n");
}

#[test]
fn split_subject_body_keeps_single_line_wrap_out_of_subject() {
    // A single newline (no blank line) must not land inside the subject.
    let (subject, body) = split_subject_body("Fix parser\nhandle CRLF");
    assert_eq!(subject, "Fix parser");
    assert_eq!(body, "handle CRLF");
}

#[test]
fn split_subject_body_without_newline() {
    let (subject, body) = split_subject_body("Only a subject");
    assert_eq!(subject, "Only a subject");
    assert_eq!(body, "");
}

#[test]
fn fold_target_skips_run_members_and_drops() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Pick, "a"),
        rebase_entry(Fixup, "b"),
        rebase_entry(Drop, "c"),
        rebase_entry(Squash, "d"),
    ];
    assert_eq!(squash_fold_target(&entries, 3), Some(0));
    assert_eq!(squash_fold_target(&entries, 1), Some(0));
}

#[test]
fn fold_target_is_nearest_pick_or_reword() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Pick, "a"),
        rebase_entry(Reword, "b"),
        rebase_entry(Squash, "c"),
    ];
    assert_eq!(squash_fold_target(&entries, 2), Some(1));
}

#[test]
fn fold_target_none_without_commit_creating_entry_above() {
    use InteractiveRebaseAction::*;
    let entries = vec![
        rebase_entry(Drop, "a"),
        rebase_entry(Fixup, "b"),
        rebase_entry(Squash, "c"),
    ];
    assert_eq!(squash_fold_target(&entries, 0), None);
    // b and c only see drops/fold members above, never a pick/reword.
    assert_eq!(squash_fold_target(&entries, 1), None);
}

#[test]
fn todo_role_classifies_actions_and_abbreviations() {
    use TodoLineRole::*;
    let sha = "deadbeef";
    for (line, role) in [
        (format!("pick {sha} subject"), Other),
        (format!("reword {sha} subject"), EditsOwnMessage),
        (format!("r {sha}"), EditsOwnMessage),
        (format!("squash {sha} subject"), FoldEditsMessage),
        (format!("s {sha}"), FoldEditsMessage),
        (format!("fixup {sha} subject"), FoldSilent),
        (format!("f {sha}"), FoldSilent),
        (format!("drop {sha} subject"), Transparent),
        ("exec make test".to_string(), Other),
        ("label onto".to_string(), Other),
    ] {
        assert_eq!(todo_line_role(&line), role, "line: {line}");
    }
}

#[test]
fn todo_role_fixup_message_variants_leave_editor_pending() {
    use TodoLineRole::*;
    assert_eq!(todo_line_role("fixup -c deadbeef subj"), FoldEditsMessage);
    assert_eq!(todo_line_role("fixup -C deadbeef subj"), FoldEditsMessage);
    assert_eq!(todo_line_role("f -c deadbeef"), FoldEditsMessage);
}

#[test]
fn todo_role_merge_is_conservatively_editing() {
    use TodoLineRole::*;
    assert_eq!(todo_line_role("merge -C 1234abcd topic"), EditsOwnMessage);
    assert_eq!(todo_line_role("m topic"), EditsOwnMessage);
}

#[test]
fn todo_role_comments_and_blanks_are_transparent() {
    use TodoLineRole::*;
    assert_eq!(todo_line_role(""), Transparent);
    assert_eq!(todo_line_role("   "), Transparent);
    assert_eq!(todo_line_role("# Rebase abc..def onto abc"), Transparent);
}

#[test]
fn from_todo_word_inverts_to_todo_str() {
    use InteractiveRebaseAction::*;
    for action in [Pick, Reword, Squash, Fixup, Drop] {
        assert_eq!(
            InteractiveRebaseAction::from_todo_word(action.to_todo_str()),
            Some(action)
        );
    }
    assert_eq!(InteractiveRebaseAction::from_todo_word("merge"), None);
    assert_eq!(InteractiveRebaseAction::from_todo_word("exec"), None);
}
