use gpui::{Entity, FocusHandle, ScrollHandle, SharedString};

pub(crate) struct CommandEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) shortcut: &'static str,
    pub(crate) category: &'static str,
    pub(crate) requires_repo: bool,
}

pub(crate) const COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        id: "commit",
        label: "Commit Changes",
        shortcut: "",
        category: "Commit",
        requires_repo: true,
    },
    CommandEntry {
        id: "stage-all",
        label: "Stage All Changes",
        shortcut: "",
        category: "Working Copy",
        requires_repo: true,
    },
    CommandEntry {
        id: "unstage-all",
        label: "Unstage All Changes",
        shortcut: "",
        category: "Working Copy",
        requires_repo: true,
    },
    CommandEntry {
        id: "create-branch",
        label: "Create Branch",
        shortcut: "",
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "checkout-branch",
        label: "Checkout Branch",
        shortcut: "",
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "delete-branch",
        label: "Delete Branch",
        shortcut: "",
        category: "Branch",
        requires_repo: true,
    },
    CommandEntry {
        id: "pull",
        label: "Pull",
        shortcut: "",
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "push",
        label: "Push",
        shortcut: "",
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "force-push",
        label: "Force Push",
        shortcut: "",
        category: "Sync",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash",
        label: "Stash Changes",
        shortcut: "",
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-pop",
        label: "Pop Stash",
        shortcut: "",
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-apply",
        label: "Apply Stash",
        shortcut: "",
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "stash-drop",
        label: "Drop Stash",
        shortcut: "",
        category: "Stash",
        requires_repo: true,
    },
    CommandEntry {
        id: "open-repository",
        label: "Open Repository",
        shortcut: "Ctrl+O",
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "open-recent",
        label: "Open Recent Repository",
        shortcut: "Ctrl+Shift+O",
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "clone-repository",
        label: "Clone Repository",
        shortcut: "",
        category: "Repository",
        requires_repo: false,
    },
    CommandEntry {
        id: "close-repo-tab",
        label: "Close Repository Tab",
        shortcut: "Ctrl+W",
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "reload-repository",
        label: "Reload Repository",
        shortcut: "",
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "fetch-all",
        label: "Fetch All",
        shortcut: "",
        category: "Repository",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-sidebar",
        label: "Toggle Sidebar",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-details",
        label: "Toggle Details Pane",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-diff-view",
        label: "Toggle Diff View (Split/Inline)",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-diff-word-wrap",
        label: "Toggle Diff Word Wrap",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-line-numbers",
        label: "Toggle Diff Line Numbers",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "toggle-whitespace-chars",
        label: "Toggle Whitespace Characters",
        shortcut: "",
        category: "View",
        requires_repo: true,
    },
    CommandEntry {
        id: "previous-repo-tab",
        label: "Previous Repository Tab",
        shortcut: "Ctrl+Shift+Tab",
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "next-repo-tab",
        label: "Next Repository Tab",
        shortcut: "Ctrl+Tab",
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "open-active-view-search",
        label: "Search in Current View",
        shortcut: "Ctrl+F",
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "create-tag",
        label: "Create Tag",
        shortcut: "",
        category: "Tags",
        requires_repo: true,
    },
    CommandEntry {
        id: "new-window",
        label: "New Window",
        shortcut: "Ctrl+N",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "open-settings",
        label: "Open Settings",
        shortcut: "Ctrl+,",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "quit",
        label: "Quit GitComet",
        shortcut: "Ctrl+Q",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "minimize-window",
        label: "Minimize Window",
        shortcut: "",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "zoom-window",
        label: "Zoom Window",
        shortcut: "",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "toggle-fullscreen",
        label: "Toggle Full Screen",
        shortcut: "F11",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "increase-ui-scale",
        label: "Increase UI Scale",
        shortcut: "Ctrl+=",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "decrease-ui-scale",
        label: "Decrease UI Scale",
        shortcut: "Ctrl+-",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "reset-ui-scale",
        label: "Reset UI Scale",
        shortcut: "Ctrl+0",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "close-window",
        label: "Close Window",
        shortcut: "Ctrl+Shift+W",
        category: "Window",
        requires_repo: false,
    },
    CommandEntry {
        id: "add-remote",
        label: "Add Remote",
        shortcut: "",
        category: "Remotes",
        requires_repo: true,
    },
    CommandEntry {
        id: "add-submodule",
        label: "Add Submodule",
        shortcut: "",
        category: "Submodules",
        requires_repo: true,
    },
    CommandEntry {
        id: "update-submodules",
        label: "Update Submodules",
        shortcut: "",
        category: "Submodules",
        requires_repo: true,
    },
    CommandEntry {
        id: "add-worktree",
        label: "Add Worktree",
        shortcut: "",
        category: "Worktrees",
        requires_repo: true,
    },
    CommandEntry {
        id: "blame",
        label: "Blame / Annotate",
        shortcut: "Alt+B",
        category: "History",
        requires_repo: true,
    },
    CommandEntry {
        id: "back",
        label: "Navigate Back",
        shortcut: "Alt+Left",
        category: "Navigation",
        requires_repo: true,
    },
    CommandEntry {
        id: "forward",
        label: "Navigate Forward",
        shortcut: "Alt+Right",
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

impl CommandPaletteState {
    pub(crate) fn filtered_commands(
        &self,
        has_active_repo: bool,
        query: &str,
    ) -> Vec<&'static CommandEntry> {
        let available: Vec<(usize, &CommandEntry)> = COMMANDS
            .iter()
            .enumerate()
            .filter(|(_, cmd)| !cmd.requires_repo || has_active_repo)
            .collect();

        if query.is_empty() {
            return available.into_iter().map(|(_, cmd)| cmd).collect();
        }

        let query_lower = query.to_ascii_lowercase();
        let mut out: Vec<(usize, usize, &CommandEntry)> = available
            .into_iter()
            .filter_map(|(ix, cmd)| {
                let label_lower = cmd.label.to_ascii_lowercase();
                label_lower.find(&query_lower).map(|pos| (pos, ix, cmd))
            })
            .collect();

        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(_, _, cmd)| cmd).collect()
    }
}
