//! Eligibility rules and message construction for squashing a contiguous
//! range of history commits into one.

use crate::domain::{Commit, CommitId};
use crate::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use rustc_hash::{FxHashMap, FxHashSet};

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

    let selected_set: FxHashSet<&CommitId> = selected.iter().collect();
    if selected_set.len() != selected.len() {
        return None;
    }

    // Build a full id → commit lookup for the whole page so we can walk
    // through both selected and non-selected commits.
    let all_by_id: FxHashMap<&CommitId, &Commit> = commits.iter().map(|c| (&c.id, c)).collect();

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

/// Index of the entry the squash/fixup at `ix` folds into: the nearest
/// preceding entry that will create a commit (`pick`/`reword`). Squash/fixup
/// entries in between are members of the same fold run, and `drop` entries
/// are transparent — the same run rules [`squash_run_final_entry`] encodes
/// looking forward.
pub fn squash_fold_target(entries: &[InteractiveRebaseEntry], ix: usize) -> Option<usize> {
    entries[..ix].iter().rposition(|e| {
        matches!(
            e.action,
            InteractiveRebaseAction::Pick | InteractiveRebaseAction::Reword
        )
    })
}

/// How a raw rebase-todo (or `done`) line participates in message editing.
/// Single source of the todo-line rules the git backend needs when deciding
/// whether continuing a paused rebase may open a commit-message editor; kept
/// next to [`squash_run_final_entry`] so the run rules cannot drift apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TodoLineRole {
    /// Executes as its own commit and opens a message editor: `reword`, and
    /// `merge` conservatively — only its `-c` form rewords, but misparsing an
    /// option must err toward "editor pending", never a silent accept.
    EditsOwnMessage,
    /// Folds into the current run and leaves the run's message editor
    /// pending: `squash`, and `fixup -c`/`-C` (`-c` reopens the editor; `-C`
    /// replaces the run's final message without one — treated the same,
    /// conservatively, so neither is ever silently finalized).
    FoldEditsMessage,
    /// Folds into the current run with no message editing: plain `fixup`.
    FoldSilent,
    /// Invisible to fold runs: `drop`, comments, blank lines.
    Transparent,
    /// Every other command (`pick`, `exec`, `label`, …): starts or ends a
    /// run without editing a message.
    Other,
}

/// The commit-id word of a todo line: the first non-option argument after
/// the command (`squash <id> subject`, `fixup -c <id> subject`). Callers use
/// it on message-editing lines; for commands whose argument is not a commit
/// (`label onto`) it returns that argument verbatim.
pub fn todo_line_commit_word(line: &str) -> Option<&str> {
    let mut words = line.split_whitespace();
    words.next()?;
    words.find(|w| !w.starts_with('-'))
}

pub fn todo_line_role(line: &str) -> TodoLineRole {
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else {
        return TodoLineRole::Transparent;
    };
    if first.starts_with('#') {
        return TodoLineRole::Transparent;
    }
    match InteractiveRebaseAction::from_todo_word(first) {
        Some(InteractiveRebaseAction::Reword) => TodoLineRole::EditsOwnMessage,
        Some(InteractiveRebaseAction::Squash) => TodoLineRole::FoldEditsMessage,
        Some(InteractiveRebaseAction::Fixup) => match words.next() {
            Some("-c" | "-C") => TodoLineRole::FoldEditsMessage,
            _ => TodoLineRole::FoldSilent,
        },
        Some(InteractiveRebaseAction::Drop) => TodoLineRole::Transparent,
        Some(InteractiveRebaseAction::Pick) => TodoLineRole::Other,
        None => match first {
            "merge" | "m" => TodoLineRole::EditsOwnMessage,
            _ => TodoLineRole::Other,
        },
    }
}

/// Message to seed the reword dialog with for the entry at `ix`. When commits
/// squash into `ix`, the seed is the combined message (the target's message
/// followed by each squashed commit's message), matching what the rebase would
/// otherwise produce; `fixup` commits fold in but contribute no message and
/// `drop` commits are transparent. Otherwise it is the entry's full original
/// message (or a prior edit).
///
/// Kept next to [`squash_run_final_entry`] so the run-membership rules the two
/// encode cannot drift apart from each other or from the backend.
pub fn reword_seed_message(entries: &[InteractiveRebaseEntry], ix: usize) -> String {
    let Some(target) = entries.get(ix) else {
        return String::new();
    };
    if let Some(msg) = &target.new_message {
        return msg.clone();
    }
    let mut messages = vec![target.message.clone()];
    messages.extend(squash_run_message_entries(entries, ix).map(|e| e.message.clone()));
    if messages.len() > 1 {
        build_squash_message(&messages)
    } else {
        target.message.clone()
    }
}

/// The squash entries whose messages fold into the entry at `ix`'s combined
/// message, in run order: `fixup` folds without contributing a message,
/// `drop` is transparent, and the next `pick`/`reword` ends the run. The
/// single walk behind [`reword_seed_message`] and edit-staleness checks, so
/// the two cannot disagree about a run's message sources.
pub fn squash_run_message_entries(
    entries: &[InteractiveRebaseEntry],
    ix: usize,
) -> impl Iterator<Item = &InteractiveRebaseEntry> {
    entries
        .get(ix + 1..)
        .unwrap_or_default()
        .iter()
        .take_while(|e| {
            !matches!(
                e.action,
                InteractiveRebaseAction::Pick | InteractiveRebaseAction::Reword
            )
        })
        .filter(|e| e.action == InteractiveRebaseAction::Squash)
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
}
