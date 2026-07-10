//! Eligibility rules and message construction for squashing a contiguous
//! range of history commits into one.

use crate::domain::{Commit, CommitId};
use crate::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use std::collections::{HashMap, HashSet};

/// A validated squash of `commit_count` commits in a linear first-parent
/// chain reachable from HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SquashPlan {
    /// The repo's actual HEAD when the plan was computed. Equals `head` when
    /// the selection ends at HEAD (commit-tree path); differs from `head`
    /// for intermediate ranges (rebase path).
    pub actual_head: CommitId,
    /// Youngest selected commit.
    pub head: CommitId,
    /// Oldest selected commit; its message becomes the squash subject.
    pub oldest: CommitId,
    /// Parent of the oldest selected commit; becomes the squash commit's
    /// parent, and the rebase base when the range does not end at HEAD.
    pub oldest_parent: CommitId,
    pub commit_count: usize,
    /// Selected commits in log order (youngest first).
    pub ordered_ids: Vec<CommitId>,
}

/// Validates a selection against the loaded log page (`commits`) and the
/// current HEAD id. Returns a plan only when all criteria hold:
///
/// 1. more than one distinct commit is selected,
/// 2. every selected commit is present in the loaded page (selections spanning
///    unloaded pages are rejected),
/// 3. the selected commits lie on a single first-parent chain reachable from
///    HEAD — each commit in the chain, selected or not, must have exactly one
///    parent — and the oldest selected commit has a parent (is not the root).
///
/// The range may end at HEAD (commit-tree path) or sit anywhere in the
/// middle of the chain (rebase path). Validation follows the first-parent
/// chain via id lookup rather than page position, so it is unaffected by
/// rows the visible history interleaves into the page (e.g. stash-helper
/// commits) which the selection excludes.
pub fn squash_eligibility(
    commits: &[Commit],
    selected: &[CommitId],
    actual_head: &CommitId,
) -> Option<SquashPlan> {
    if selected.len() < 2 {
        return None;
    }

    let selected_set: HashSet<&CommitId> = selected.iter().collect();
    if selected_set.len() != selected.len() {
        return None;
    }

    // Build a full id → commit lookup for the whole page so we can walk
    // through both selected and non-selected commits.
    let all_by_id: HashMap<&CommitId, &Commit> = commits.iter().map(|c| (&c.id, c)).collect();

    // Every selected commit must be present in the loaded page.
    if selected_set.iter().any(|id| !all_by_id.contains_key(id)) {
        return None;
    }

    // Walk first parents from actual HEAD, collecting selected commits in
    // encounter order. Non-selected commits preceding the range are
    // pass-through; gaps within the selected range are rejected so the
    // squashed set is always contiguous on the chain.
    let mut ordered_ids = Vec::with_capacity(selected.len());
    let mut current: &CommitId = actual_head;
    let mut collected = 0;
    let mut inside_selection = false;
    let mut gap_within_selection = false;

    loop {
        let commit = all_by_id.get(current)?;

        if commit.parent_ids.len() != 1 {
            return None;
        }

        if selected_set.contains(current) {
            if gap_within_selection {
                return None;
            }
            inside_selection = true;
            ordered_ids.push(current.clone());
            collected += 1;

            if collected == selected.len() {
                return Some(SquashPlan {
                    actual_head: actual_head.clone(),
                    head: ordered_ids[0].clone(),
                    oldest: current.clone(),
                    oldest_parent: commit.parent_ids[0].clone(),
                    commit_count: selected.len(),
                    ordered_ids,
                });
            }
        } else if inside_selection {
            gap_within_selection = true;
        }

        current = &commit.parent_ids[0];
    }
}

/// Number of commits an operation on the range `target..head` covers — the
/// commits from `head` (inclusive) down to `target` (exclusive) — when that
/// range is a strictly linear first-parent chain within the loaded page.
///
/// On a strictly linear chain the first-parent count equals `|target..head|`
/// exactly. Returns `None` whenever exactness cannot be guaranteed: a merge on
/// the chain (the range then also contains side-branch commits), the chain
/// leaving the loaded page, or page ordering not listing a child before its
/// parent. Scans `commits` forward with a resuming cursor — log order lists
/// children before parents — so the walk allocates nothing and visits each
/// page entry at most once.
pub fn linear_first_parent_distance(
    commits: &[Commit],
    head: &CommitId,
    target: &CommitId,
) -> Option<usize> {
    let mut count = 0;
    let mut current = head;
    let mut cursor = commits.iter();
    while current != target {
        let commit = cursor.find(|c| &c.id == current)?;
        if commit.parent_ids.len() != 1 {
            return None;
        }
        count += 1;
        current = &commit.parent_ids[0];
    }
    Some(count)
}

/// Splits a commit message into a single-line subject and the remaining body,
/// following git's convention: the subject is the first line and the body is
/// everything after the first line break, with a single leading blank-line
/// separator consumed. Keeping this in core means the UI never has to re-parse
/// the message format.
pub fn split_subject_body(message: &str) -> (String, String) {
    match message.split_once('\n') {
        Some((subject, rest)) => (
            subject.to_string(),
            rest.strip_prefix('\n').unwrap_or(rest).to_string(),
        ),
        None => (message.to_string(), String::new()),
    }
}

/// Index of the entry whose editor invocation determines the final message of
/// the squash run folding into the reword target at `ix`, when that run
/// contains at least one `squash`.
///
/// Git accumulates squashed messages and opens the message editor at the last
/// squash/fixup step of the run — not at the target's own reword step — so a
/// replacement message must be applied there or git re-appends the squashed
/// messages over it. Returns `None` when no editor opens after the target
/// (plain reword, or only fixups/drops follow), in which case the target's own
/// reword step is where a replacement message applies. `drop` entries neither
/// extend nor end the run.
pub fn squash_run_final_entry(entries: &[InteractiveRebaseEntry], ix: usize) -> Option<usize> {
    let mut last_fold = None;
    let mut has_squash = false;
    for (k, entry) in entries.iter().enumerate().skip(ix + 1) {
        match entry.action {
            InteractiveRebaseAction::Squash => {
                has_squash = true;
                last_fold = Some(k);
            }
            InteractiveRebaseAction::Fixup => last_fold = Some(k),
            InteractiveRebaseAction::Drop => {}
            InteractiveRebaseAction::Pick | InteractiveRebaseAction::Reword => break,
        }
    }
    if has_squash { last_fold } else { None }
}

/// Builds the combined squash message: the oldest commit's full message is the
/// subject/body, and each younger message is appended as its own paragraph,
/// oldest to youngest.
pub fn build_squash_message(messages_oldest_first: &[String]) -> String {
    let mut out = String::new();
    for message in messages_oldest_first {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
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
        let plan =
            squash_eligibility(&log, &[id("c"), id("d"), id("b")], &id("d")).expect("eligible");
        assert_eq!(plan.actual_head, id("d"));
        assert_eq!(plan.head, id("d"));
        assert_eq!(plan.oldest, id("b"));
        assert_eq!(plan.oldest_parent, id("a"));
        assert_eq!(plan.commit_count, 3);
        assert_eq!(plan.ordered_ids, vec![id("d"), id("c"), id("b")]);
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
        assert_eq!(plan.ordered_ids, vec![id("c"), id("b")]);
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
        let plan =
            squash_eligibility(&log, &[id("d"), id("c"), id("b")], &id("d")).expect("eligible");
        assert_eq!(plan.actual_head, id("d"));
        assert_eq!(plan.head, id("d"));
        assert_eq!(plan.oldest, id("b"));
        assert_eq!(plan.oldest_parent, id("a"));
        assert_eq!(plan.commit_count, 3);
        assert_eq!(plan.ordered_ids, vec![id("d"), id("c"), id("b")]);
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
}
