//! Eligibility rules and message construction for squashing a contiguous
//! range of history commits into one.

use crate::domain::{Commit, CommitId};
use std::collections::{HashMap, HashSet};

/// A validated squash of `commit_count` commits ending at `head`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SquashPlan {
    /// Youngest selected commit; must be HEAD.
    pub head: CommitId,
    /// Oldest selected commit; its message becomes the squash subject.
    pub oldest: CommitId,
    /// Parent of the oldest selected commit; becomes the squash commit's parent.
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
/// 3. HEAD is the youngest selected commit,
/// 4. walking first parents from HEAD visits exactly the selected set in a
///    linear chain — each visited commit has exactly one parent, which is the
///    next selected commit — and the oldest selected commit has a parent
///    (is not the root).
///
/// Validation follows the first-parent chain via id lookup rather than page
/// position, so it is unaffected by rows the visible history interleaves into
/// the page (e.g. stash-helper commits) but which the selection excludes.
pub fn squash_eligibility(
    commits: &[Commit],
    selected: &[CommitId],
    head: &CommitId,
) -> Option<SquashPlan> {
    if selected.len() < 2 {
        return None;
    }

    let selected_set: HashSet<&CommitId> = selected.iter().collect();
    // Reject duplicate ids in the selection.
    if selected_set.len() != selected.len() {
        return None;
    }
    // HEAD must be the youngest selected commit; the chain walk starts there.
    if !selected_set.contains(head) {
        return None;
    }

    // Look up only the selected commits; every one must be in the loaded page.
    let mut by_id: HashMap<&CommitId, &Commit> = HashMap::with_capacity(selected.len());
    for commit in commits {
        if selected_set.contains(&commit.id) {
            by_id.insert(&commit.id, commit);
        }
    }
    if by_id.len() != selected_set.len() {
        return None;
    }

    // Walk first parents from HEAD, requiring each step to stay within the
    // selection and to be a single-parent (non-merge, non-root) commit.
    let mut ordered_ids = Vec::with_capacity(selected.len());
    let mut current = head;
    loop {
        let commit = by_id.get(current)?;
        if commit.parent_ids.len() != 1 {
            return None;
        }
        let parent = &commit.parent_ids[0];
        ordered_ids.push(current.clone());

        if ordered_ids.len() == selected.len() {
            return Some(SquashPlan {
                head: head.clone(),
                oldest: current.clone(),
                oldest_parent: parent.clone(),
                commit_count: selected.len(),
                ordered_ids,
            });
        }

        // The parent must be the next selected commit; if it left the
        // selection before consuming every selected id, the range is not a
        // contiguous linear chain.
        if !selected_set.contains(parent) {
            return None;
        }
        current = parent;
    }
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
    fn eligible_range_returns_plan() {
        let log = linear_log();
        let plan =
            squash_eligibility(&log, &[id("c"), id("d"), id("b")], &id("d")).expect("eligible");
        assert_eq!(plan.head, id("d"));
        assert_eq!(plan.oldest, id("b"));
        assert_eq!(plan.oldest_parent, id("a"));
        assert_eq!(plan.commit_count, 3);
        assert_eq!(plan.ordered_ids, vec![id("d"), id("c"), id("b")]);
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
    fn youngest_not_head_is_rejected() {
        let log = linear_log();
        assert!(squash_eligibility(&log, &[id("c"), id("b")], &id("d")).is_none());
        assert!(squash_eligibility(&log, &[id("d"), id("c")], &id("other")).is_none());
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
        assert!(squash_eligibility(
            &log,
            &[id("d"), id("c"), id("b"), id("a"), id("root")],
            &id("d"),
        )
        .is_none());
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
        let messages = vec!["Subject".to_string(), "  \n".to_string(), "Tail".to_string()];
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
