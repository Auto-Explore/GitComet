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
    pub ordered_ids: std::sync::Arc<Vec<CommitId>>,
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
                    ordered_ids: ordered_ids.into(),
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

/// Where a commit sits on HEAD's first-parent history, for operations that
/// replay `target..head` with a flattening rebase (rewording).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirstParentPosition {
    /// `distance` first-parent steps below HEAD, with no merge above it.
    Linear { distance: usize },
    /// On the first-parent chain, but below a merge.
    BelowMerge,
    /// Not on the first-parent chain within the loaded history.
    NotFound,
}

/// [`FirstParentPosition`] of `target` in the loaded log page. Scans with a
/// resuming cursor like [`linear_first_parent_distance`].
pub fn first_parent_position(
    commits: &[Commit],
    head: &CommitId,
    target: &CommitId,
) -> FirstParentPosition {
    let mut distance = 0;
    let mut below_merge = false;
    let mut current = head;
    let mut cursor = commits.iter();
    while current != target {
        let Some(commit) = cursor.find(|c| &c.id == current) else {
            return FirstParentPosition::NotFound;
        };
        let Some(parent) = commit.parent_ids.first() else {
            return FirstParentPosition::NotFound;
        };
        below_merge |= commit.parent_ids.len() > 1;
        distance += 1;
        current = parent;
    }
    if below_merge {
        FirstParentPosition::BelowMerge
    } else {
        FirstParentPosition::Linear { distance }
    }
}

/// [`first_parent_position`] over the full history index.
pub fn first_parent_position_indexed(
    index: &crate::history_index::HistoryIndex,
    head: &CommitId,
    target: &CommitId,
) -> FirstParentPosition {
    let (Some(mut row), Some(target_row)) = (
        index.position(head.as_ref()),
        index.position(target.as_ref()),
    ) else {
        return FirstParentPosition::NotFound;
    };
    let mut below_merge = false;
    for distance in 0..index.len() {
        if row == target_row {
            return if below_merge {
                FirstParentPosition::BelowMerge
            } else {
                FirstParentPosition::Linear { distance }
            };
        }
        let parents = index.parents(row);
        let Some(&parent) = parents.first() else {
            return FirstParentPosition::NotFound;
        };
        below_merge |= parents.len() > 1;
        row = parent as usize;
    }
    FirstParentPosition::NotFound
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

/// The same eligibility rules over a complete compact snapshot. Commit text
/// need not be loaded to validate a distant selection.
pub fn squash_eligibility_indexed(
    index: &crate::history_index::HistoryIndex,
    selected: &[CommitId],
    actual_head: &CommitId,
) -> Option<SquashPlan> {
    if selected.len() < 2 {
        return None;
    }
    let selected_rows: Option<FxHashSet<usize>> = selected
        .iter()
        .map(|id| index.position(id.as_ref()))
        .collect();
    let selected_rows = selected_rows?;
    if selected_rows.len() != selected.len() {
        return None;
    }
    let mut row = index.position(actual_head.as_ref())?;
    let mut ordered_ids = Vec::with_capacity(selected.len());
    for _ in 0..index.len() {
        let parents = index.parents(row);
        if parents.len() != 1 {
            return None;
        }
        if selected_rows.contains(&row) {
            ordered_ids.push(index.commit_id(row)?);
            if ordered_ids.len() == selected.len() {
                return Some(SquashPlan {
                    actual_head: actual_head.clone(),
                    head: ordered_ids[0].clone(),
                    oldest: index.commit_id(row)?,
                    oldest_parent: index.parent_commit_id(row, 0)?,
                    commit_count: selected.len(),
                    ordered_ids: ordered_ids.into(),
                });
            }
        } else if !ordered_ids.is_empty() {
            return None;
        }
        row = parents[0] as usize;
    }
    None
}

#[cfg(test)]
mod tests;
