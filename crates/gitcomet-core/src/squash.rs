//! Eligibility rules and message construction for squashing a contiguous
//! range of history commits into one.

use crate::domain::{Commit, CommitId};

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

/// Validates a selection against the loaded log page (`commits`, youngest
/// first) and the current HEAD id. Returns a plan only when all criteria hold:
///
/// 1. more than one commit is selected,
/// 2. every selected commit is present in the loaded page (selections spanning
///    unloaded pages are rejected),
/// 3. the selection is contiguous in log order,
/// 4. the youngest selected commit is HEAD,
/// 5. the range is a linear first-parent chain — each selected commit has
///    exactly one parent, which is the next older selected commit — and the
///    oldest selected commit has a parent (is not the root).
pub fn squash_eligibility(
    commits: &[Commit],
    selected: &[CommitId],
    head: &CommitId,
) -> Option<SquashPlan> {
    if selected.len() < 2 {
        return None;
    }

    let mut indices: Vec<usize> = selected
        .iter()
        .map(|id| commits.iter().position(|c| c.id == *id))
        .collect::<Option<Vec<_>>>()?;
    indices.sort_unstable();
    let before_dedup = indices.len();
    indices.dedup();
    if indices.len() != before_dedup {
        return None;
    }

    let youngest_ix = *indices.first()?;
    let oldest_ix = *indices.last()?;
    // Contiguous run in log order; log order is the chronological order the
    // user sees, so this doubles as the "chronologically consecutive" check.
    if oldest_ix - youngest_ix + 1 != indices.len() {
        return None;
    }

    if commits[youngest_ix].id != *head {
        return None;
    }

    for ix in youngest_ix..=oldest_ix {
        let commit = &commits[ix];
        if commit.parent_ids.len() != 1 {
            return None;
        }
        if ix < oldest_ix && commit.parent_ids[0] != commits[ix + 1].id {
            return None;
        }
    }

    Some(SquashPlan {
        head: commits[youngest_ix].id.clone(),
        oldest: commits[oldest_ix].id.clone(),
        oldest_parent: commits[oldest_ix].parent_ids[0].clone(),
        commit_count: indices.len(),
        ordered_ids: commits[youngest_ix..=oldest_ix]
            .iter()
            .map(|c| c.id.clone())
            .collect(),
    })
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
}
