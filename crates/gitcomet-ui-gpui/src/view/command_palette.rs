use gpui::{Entity, FocusHandle, ScrollHandle, SharedString};

use super::shortcut_labels::Shortcut;

pub(crate) struct CommandEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) shortcut: Shortcut,
    pub(crate) category: &'static str,
    pub(crate) requires_repo: bool,
}

pub(crate) const COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        id: "commit",
        label: "Commit Changes",
        shortcut: Shortcut::None,
        category: "Commit",
        requires_repo: true,
    },
    CommandEntry {
        id: "stage-all",
        label: "Stage All Changes",
        shortcut: Shortcut::None,
        category: "Working Copy",
        requires_repo: true,
    },
    CommandEntry {
        id: "unstage-all",
        label: "Unstage All Changes",
        shortcut: Shortcut::None,
        category: "Working Copy",
        requires_repo: true,
    },
    CommandEntry {
        id: "create-branch",
        label: "Create Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "checkout-branch",
        label: "Checkout Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "delete-branch",
        label: "Delete Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "pull",
        label: "Pull",
        shortcut: Shortcut::None,
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "push",
        label: "Push",
        shortcut: Shortcut::None,
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "force-push",
        label: "Force Push",
        shortcut: Shortcut::None,
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash",
        label: "Stash Changes",
        shortcut: Shortcut::None,
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-pop",
        label: "Pop Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-apply",
        label: "Apply Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-drop",
        label: "Drop Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "open-repository",
        label: "Open Repository",
        shortcut: Shortcut::Secondary("O"),
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "open-recent",
        label: "Open Recent Repository",
        shortcut: Shortcut::Platform {
            macos: "Option+Cmd+O",
            other: "Ctrl+Shift+O",
        },
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "clone-repository",
        label: "Clone Repository",
        shortcut: Shortcut::None,
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "close-repo-tab",
        label: "Close Repository Tab",
        shortcut: Shortcut::Secondary("W"),
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "reload-repository",
        label: "Reload Repository",
        shortcut: Shortcut::None,
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "fetch-all",
        label: "Fetch All",
        shortcut: Shortcut::None,
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-sidebar",
        label: "Toggle Sidebar",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-details",
        label: "Toggle Details Pane",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-diff-view",
        label: "Toggle Diff View (Split/Inline)",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-diff-word-wrap",
        label: "Toggle Diff Word Wrap",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-line-numbers",
        label: "Toggle Diff Line Numbers",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-whitespace-chars",
        label: "Toggle Whitespace Characters",
        shortcut: Shortcut::None,
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "previous-repo-tab",
        label: "Previous Repository Tab",
        shortcut: Shortcut::Platform {
            macos: "Cmd+Shift+[",
            other: "Ctrl+Shift+Tab",
        },
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "next-repo-tab",
        label: "Next Repository Tab",
        shortcut: Shortcut::Platform {
            macos: "Cmd+Shift+]",
            other: "Ctrl+Tab",
        },
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "open-active-view-search",
        label: "Search in Current View",
        shortcut: Shortcut::Secondary("F"),
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "create-tag",
        label: "Create Tag",
        shortcut: Shortcut::None,
        category: "Tags",
        requires_repo: true,
    },
    CommandEntry {
        id: "new-window",
        label: "New Window",
        shortcut: Shortcut::Secondary("N"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "open-settings",
        label: "Open Settings",
        shortcut: Shortcut::Secondary(","),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "quit",
        label: "Quit GitComet",
        shortcut: Shortcut::Secondary("Q"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "minimize-window",
        label: "Minimize Window",
        shortcut: Shortcut::MacOs("Cmd+M"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "zoom-window",
        label: "Zoom Window",
        shortcut: Shortcut::None,
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "toggle-fullscreen",
        label: "Toggle Full Screen",
        shortcut: Shortcut::Platform {
            macos: "Ctrl+Cmd+F",
            other: "F11",
        },
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "increase-ui-scale",
        label: "Increase UI Scale",
        shortcut: Shortcut::Secondary("="),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "decrease-ui-scale",
        label: "Decrease UI Scale",
        shortcut: Shortcut::Secondary("-"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "reset-ui-scale",
        label: "Reset UI Scale",
        shortcut: Shortcut::Secondary("0"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "close-window",
        label: "Close Window",
        shortcut: Shortcut::Secondary("Shift+W"),
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "add-remote",
        label: "Add Remote",
        shortcut: Shortcut::None,
        category: "Remotes",
        requires_repo: true,
    },
    CommandEntry {
        id: "add-submodule",
        label: "Add Submodule",
        shortcut: Shortcut::None,
        category: "Submodules",
        requires_repo: true,
    },
    CommandEntry {
        id: "update-submodules",
        label: "Update Submodules",
        shortcut: Shortcut::None,
        category: "Submodules",
        requires_repo: true,
    },
    CommandEntry {
        id: "add-worktree",
        label: "Add Worktree",
        shortcut: Shortcut::None,
        category: "Worktrees",
        requires_repo: true,
    },
    CommandEntry {
        id: "blame",
        label: "Blame / Annotate",
        shortcut: Shortcut::Alt("B"),
        category: "History",
        requires_repo: true,
    },
    CommandEntry {
        id: "back",
        label: "Navigate Back",
        shortcut: Shortcut::Alt("Left"),
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "forward",
        label: "Navigate Forward",
        shortcut: Shortcut::Alt("Right"),
        category: "Navigation",
        requires_repo: true,
    },
    // TODO: "undo"              - Undo (Edit)
    // TODO: "redo"              - Redo (Edit)
    // TODO: "keyboard-shortcuts" - Keyboard Shortcuts (Help)
    // TODO: "file-history"     - File History (History)
    // TODO: "search-commits"   - Search Commits (Navigation)
    // TODO: "rename-branch"    - Rename Branch (Branch)
    // TODO: "checkout-remote-branch" - Checkout Remote Branch
    // TODO: "delete-remote-branch"   - Delete Remote Branch
    // TODO: "merge"                  - Merge Branch/Ref
    // TODO: "rebase"                 - Rebase Onto
    // TODO: "delete-tag"             - Delete Tag
    // TODO: "remove-remote"          - Remove Remote
    // TODO: "edit-remote-url"        - Edit Remote URL
    // TODO: "remove-submodule"       - Remove Submodule
    // TODO: "remove-worktree"        - Remove Worktree
    // TODO: "discard-all"        - Discard All Changes (Working Copy)
];

pub(crate) struct CommandPaletteState {
    pub(crate) query_input: Option<Entity<crate::view::components::TextInput>>,
    pub(crate) restore_focus: Option<FocusHandle>,
    pub(crate) scroll_handle: ScrollHandle,
    pub(crate) selected_index: Option<usize>,
    pub(crate) previous_query: SharedString,
}

/// A palette entry that survived filtering, plus the label byte positions the
/// query matched (for highlighting). Derefs to the entry so callers keep using
/// `cmd.label` / `cmd.id` / `cmd.category` directly.
pub(crate) struct CommandMatch {
    pub(crate) entry: &'static CommandEntry,
    pub(crate) positions: Vec<usize>,
}

impl std::ops::Deref for CommandMatch {
    type Target = CommandEntry;

    fn deref(&self) -> &CommandEntry {
        self.entry
    }
}

impl CommandPaletteState {
    pub(crate) fn filtered_commands(
        &self,
        has_active_repo: bool,
        query: &str,
    ) -> Vec<CommandMatch> {
        let available = COMMANDS
            .iter()
            .filter(|cmd| !cmd.requires_repo || has_active_repo);

        if query.is_empty() {
            return available
                .map(|entry| CommandMatch {
                    entry,
                    positions: Vec::new(),
                })
                .collect();
        }

        let mut out: Vec<(i32, usize, CommandMatch)> = available
            .enumerate()
            .filter_map(|(order, entry)| {
                fuzzy_subsequence_match(entry.label, query)
                    .map(|(score, positions)| (score, order, CommandMatch { entry, positions }))
            })
            .collect();

        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.2.label.len().cmp(&b.2.label.len()))
                .then_with(|| a.1.cmp(&b.1))
        });
        out.into_iter().map(|(_, _, m)| m).collect()
    }
}

/// Case-insensitive subsequence match of `query` inside `label` (both ASCII).
/// Returns `(score, matched byte positions)`; lower scores are better.
/// Contiguous matches beat gapped ones, word-boundary hits beat mid-word hits,
/// and earlier matches beat later ones — so "push" ranks "Push" over
/// "Force Push", and "cb" still finds "Create Branch".
fn fuzzy_subsequence_match(label: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    let label_bytes = label.as_bytes();
    let mut positions = Vec::with_capacity(query.len());
    let mut gaps: i32 = 0;
    let mut boundary_hits: i32 = 0;
    let mut search_from = 0usize;

    for query_byte in query.bytes() {
        let target = query_byte.to_ascii_lowercase();
        let found = label_bytes
            .iter()
            .enumerate()
            .skip(search_from)
            .find(|(_, label_byte)| label_byte.to_ascii_lowercase() == target)
            .map(|(ix, _)| ix)?;

        if positions.last().is_some_and(|&prev| found > prev + 1) {
            gaps += 1;
        }
        if found == 0 || !label_bytes[found - 1].is_ascii_alphanumeric() {
            boundary_hits += 1;
        }
        positions.push(found);
        search_from = found + 1;
    }

    let first = positions.first().copied().unwrap_or(0) as i32;
    Some((gaps * 100 - boundary_hits * 20 + first, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> CommandPaletteState {
        CommandPaletteState {
            query_input: None,
            restore_focus: None,
            scroll_handle: ScrollHandle::new(),
            selected_index: None,
            previous_query: SharedString::default(),
        }
    }

    #[test]
    fn fuzzy_match_finds_subsequences_across_words() {
        let (_, positions) = fuzzy_subsequence_match("Create Branch", "cb").expect("match");
        assert_eq!(positions, vec![0, 7]);
        assert!(fuzzy_subsequence_match("Create Branch", "xq").is_none());
    }

    #[test]
    fn contiguous_matches_rank_above_gapped_ones() {
        let (push_score, _) = fuzzy_subsequence_match("Push", "push").expect("match");
        let (pull_stash_score, _) = fuzzy_subsequence_match("Pull Stash", "push").expect("match");
        assert!(
            push_score < pull_stash_score,
            "exact word must outrank scattered letters ({push_score} vs {pull_stash_score})"
        );
    }

    #[test]
    fn filtered_commands_ranks_prefix_hits_first_and_keeps_positions() {
        let matches = state().filtered_commands(true, "push");
        let first = matches.first().expect("at least one match");
        assert_eq!(first.label, "Push");
        assert_eq!(first.positions, vec![0, 1, 2, 3]);
        assert!(
            matches.iter().any(|m| m.label == "Force Push"),
            "substring hits elsewhere in the label must still be included"
        );
    }

    #[test]
    fn filtered_commands_without_query_lists_everything_available() {
        let with_repo = state().filtered_commands(true, "");
        let without_repo = state().filtered_commands(false, "");
        assert_eq!(with_repo.len(), COMMANDS.len());
        assert!(without_repo.len() < with_repo.len());
        assert!(with_repo.iter().all(|m| m.positions.is_empty()));
    }
}
