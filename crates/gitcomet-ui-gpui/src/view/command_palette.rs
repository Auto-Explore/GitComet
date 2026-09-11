use crate::kit::{Scrollbar, ScrollbarAxis};
use crate::theme::AppTheme;
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window, div, px,
    uniform_list,
};
use palette::IntoColor;

use super::shortcut_labels::Shortcut;
use super::tooltip::GitCometTooltipExt;
use super::{GitCometView, components, restrict_scroll_to_vertical_axis};
use gitcomet_core::tag_push::TagPushMode;

pub(crate) struct CommandEntry {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) shortcut: Shortcut,
    pub(crate) category: &'static str,
    pub(crate) requires_repo: bool,
    /// Extra search terms, matched after the label so wording the user
    /// remembers still finds a command the label no longer spells out.
    pub(crate) keywords: &'static str,
    /// What the command needs beyond a repository. Unlike `requires_repo`,
    /// which hides the command, an unmet need leaves it listed but disabled.
    pub(crate) needs: Needs,
}

/// A precondition a command can be listed without. The palette shows such a
/// command disabled, with the reason as its tooltip, so it stays discoverable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Needs {
    Nothing,
    ExternalEditor,
    Merge,
    /// A rebase, apply, cherry-pick, or revert in progress.
    Sequencer,
    /// A sequencer operation with every conflict resolved.
    SequencerResolved,
    PushWithTags(TagPushMode),
    /// A remote whose URL points at a web page.
    RemoteWebPage,
    MacOs,
    Linux,
}

/// The app state commands are enabled against, taken from the root view.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PaletteContext {
    pub(crate) has_active_repo: bool,
    pub(crate) external_editor: bool,
    pub(crate) merging: bool,
    pub(crate) sequencer: bool,
    pub(crate) unresolved_conflicts: bool,
    /// Why each tag-push mode is unavailable, indexed by `TagPushMode::index`.
    /// Computed by the push menu's own logic so the two cannot disagree.
    pub(crate) push_with_tags_unavailable: [Option<&'static str>; 2],
    /// Why no remote can be opened in a browser, from `remote_web_request`.
    pub(crate) remote_web_page_unavailable: Option<&'static str>,
}

/// Why a command with `needs` cannot run under `ctx`, or `None` when it can.
pub(crate) fn unavailable_reason(needs: Needs, ctx: &PaletteContext) -> Option<&'static str> {
    const NOT_SEQUENCING: &str =
        "Only available while a rebase, cherry-pick, or revert is in progress";
    match needs {
        Needs::Nothing => None,
        Needs::ExternalEditor => {
            (!ctx.external_editor).then_some("Choose an external code editor in Settings first")
        }
        Needs::Merge => (!ctx.merging).then_some("Only available while a merge is in progress"),
        Needs::Sequencer => (!ctx.sequencer).then_some(NOT_SEQUENCING),
        Needs::SequencerResolved => {
            if !ctx.sequencer {
                Some(NOT_SEQUENCING)
            } else if ctx.unresolved_conflicts {
                // Same wording as the action bar's Continue button.
                Some("Resolve all conflicts before continuing")
            } else {
                None
            }
        }
        Needs::PushWithTags(mode) => ctx.push_with_tags_unavailable[mode.index()],
        Needs::RemoteWebPage => ctx.remote_web_page_unavailable,
        Needs::MacOs => (!cfg!(target_os = "macos")).then_some("Only available on macOS"),
        Needs::Linux => (!cfg!(any(target_os = "linux", target_os = "freebsd")))
            .then_some("Only available on Linux"),
    }
}

pub(crate) const COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        id: "commit",
        label: "Commit Changes",
        shortcut: Shortcut::None,
        category: "Commit",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "stage-all",
        label: "Stage All Changes",
        shortcut: Shortcut::None,
        category: "Working Copy",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "unstage-all",
        label: "Unstage All Changes",
        shortcut: Shortcut::None,
        category: "Working Copy",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "create-branch",
        label: "Create Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "checkout-branch",
        label: "Checkout Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "delete-branch",
        label: "Delete Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "rename-branch",
        label: "Rename Branch",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "rebase",
        label: "Rebase Onto",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "rebase onto history rewrite",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "prune-merged-branches",
        label: "Prune Merged Branches",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "delete cleanup merged local branches",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "abort-merge",
        label: "Abort Merge",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "cancel merge conflict",
        requires_repo: true,
        needs: Needs::Merge,
    },
    CommandEntry {
        id: "continue-rebase",
        label: "Continue Rebase, Cherry-Pick, or Revert",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "continue resume rebase cherry pick revert apply am sequencer",
        requires_repo: true,
        needs: Needs::SequencerResolved,
    },
    CommandEntry {
        id: "abort-rebase",
        label: "Abort Rebase, Cherry-Pick, or Revert",
        shortcut: Shortcut::None,
        category: "Branch",
        keywords: "abort cancel rebase cherry pick revert apply am sequencer",
        requires_repo: true,
        needs: Needs::Sequencer,
    },
    CommandEntry {
        id: "pull",
        label: "Pull",
        shortcut: Shortcut::None,
        category: "Sync",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "push",
        label: "Push",
        shortcut: Shortcut::None,
        category: "Sync",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "force-push",
        label: "Force Push",
        shortcut: Shortcut::None,
        category: "Sync",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "push-with-annotated-tags",
        label: "Push with Annotated Tags",
        shortcut: Shortcut::None,
        category: "Sync",
        keywords: "push tags follow-tags annotated",
        requires_repo: true,
        needs: Needs::PushWithTags(TagPushMode::FollowAnnotated),
    },
    CommandEntry {
        id: "push-with-all-tags",
        label: "Push with All Tags",
        shortcut: Shortcut::None,
        category: "Sync",
        keywords: "push tags all",
        requires_repo: true,
        needs: Needs::PushWithTags(TagPushMode::All),
    },
    CommandEntry {
        id: "stash",
        label: "Stash Changes",
        shortcut: Shortcut::None,
        category: "Stash",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "stash-pop",
        label: "Pop Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "stash-apply",
        label: "Apply Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "stash-drop",
        label: "Drop Stash",
        shortcut: Shortcut::None,
        category: "Stash",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "open-repository",
        label: "Open Repository",
        shortcut: Shortcut::Secondary("O"),
        category: "Repository",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "switch-repository",
        label: "Switch Repository",
        shortcut: Shortcut::Platform {
            macos: "Option+Cmd+O",
            other: "Ctrl+Shift+O",
        },
        category: "Repository",
        keywords: "recent reopen",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "clone-repository",
        label: "Clone Repository",
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "initialize-repository",
        label: crate::menu_labels::INITIALIZE_REPOSITORY,
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "init new create empty",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "close-repo-tab",
        label: "Close Repository Tab",
        shortcut: Shortcut::Secondary("W"),
        category: "Repository",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "reload-repository",
        label: "Reload Repository",
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "fetch-all",
        label: "Fetch All",
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "apply-patch",
        label: crate::menu_labels::APPLY_PATCH,
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "patch diff apply import",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "open-in-code-editor",
        label: crate::menu_labels::OPEN_IN_CODE_EDITOR,
        shortcut: Shortcut::Secondary("Shift+E"),
        category: "Repository",
        keywords: "editor ide vscode external",
        requires_repo: true,
        needs: Needs::ExternalEditor,
    },
    CommandEntry {
        id: "open-external-terminal",
        label: "Open in External Terminal",
        shortcut: Shortcut::None,
        category: "Repository",
        keywords: "terminal shell console external",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-sidebar",
        label: "Toggle Sidebar",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-details",
        label: "Toggle Details Pane",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-diff-view",
        label: "Toggle Diff View (Split/Inline)",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-diff-word-wrap",
        label: "Toggle Diff Word Wrap",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-line-numbers",
        label: "Toggle Diff Line Numbers",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-whitespace-chars",
        label: "Toggle Whitespace Characters",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-terminal",
        label: "Toggle Terminal",
        shortcut: Shortcut::None,
        category: "View",
        keywords: "terminal shell console show hide panel",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "previous-repo-tab",
        label: "Previous Repository Tab",
        shortcut: Shortcut::Platform {
            macos: "Cmd+Shift+[",
            other: "Ctrl+Shift+Tab",
        },
        category: "Navigation",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "next-repo-tab",
        label: "Next Repository Tab",
        shortcut: Shortcut::Platform {
            macos: "Cmd+Shift+]",
            other: "Ctrl+Tab",
        },
        category: "Navigation",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "locate-file-in-explorer",
        label: crate::menu_labels::OPEN_IN_FILE_EXPLORER,
        shortcut: Shortcut::Secondary("Shift+L"),
        category: "Navigation",
        keywords: "show locate reveal find sidebar tree folder current",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "reveal-commit",
        label: "Go to",
        shortcut: Shortcut::Secondary("G"),
        category: "Navigation",
        // "reveal commit" keeps the previous label findable.
        keywords: "reveal commit sha hash jump find locate revision branch tag head",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "open-active-view-search",
        label: "Search in Current View",
        shortcut: Shortcut::Secondary("F"),
        category: "Navigation",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "back",
        label: "Navigate Back",
        shortcut: Shortcut::Alt("Left"),
        category: "Navigation",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "forward",
        label: "Navigate Forward",
        shortcut: Shortcut::Alt("Right"),
        category: "Navigation",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "create-tag",
        label: "Create Tag",
        shortcut: Shortcut::None,
        category: "Tags",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "prune-local-tags",
        label: "Prune Local Tags",
        shortcut: Shortcut::None,
        category: "Tags",
        keywords: "delete cleanup tags",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "new-window",
        label: "New Window",
        shortcut: Shortcut::Secondary("N"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "open-settings",
        label: "Open Settings",
        shortcut: Shortcut::Secondary(","),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "quit",
        label: "Quit GitComet",
        shortcut: Shortcut::Secondary("Q"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "minimize-window",
        label: "Minimize Window",
        shortcut: Shortcut::MacOs("Cmd+M"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "hide",
        label: "Hide GitComet",
        shortcut: Shortcut::MacOs("Cmd+H"),
        category: "Window",
        keywords: "hide application",
        requires_repo: false,
        needs: Needs::MacOs,
    },
    CommandEntry {
        id: "hide-others",
        label: "Hide Others",
        shortcut: Shortcut::MacOs("Option+Cmd+H"),
        category: "Window",
        keywords: "hide other applications",
        requires_repo: false,
        needs: Needs::MacOs,
    },
    CommandEntry {
        id: "zoom-window",
        label: "Zoom Window",
        shortcut: Shortcut::None,
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "toggle-fullscreen",
        label: "Toggle Full Screen",
        shortcut: Shortcut::Platform {
            macos: "Ctrl+Cmd+F",
            other: "F11",
        },
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "increase-ui-scale",
        label: "Increase UI Scale",
        shortcut: Shortcut::Secondary("="),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "decrease-ui-scale",
        label: "Decrease UI Scale",
        shortcut: Shortcut::Secondary("-"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "reset-ui-scale",
        label: "Reset UI Scale",
        shortcut: Shortcut::Secondary("0"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "close-window",
        label: "Close Window",
        shortcut: Shortcut::Secondary("Shift+W"),
        category: "Window",
        keywords: "",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "check-for-updates",
        label: crate::menu_labels::CHECK_FOR_UPDATES,
        shortcut: Shortcut::None,
        category: "Window",
        keywords: "update upgrade version release",
        requires_repo: false,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "install-desktop-integration",
        label: "Install Desktop Integration",
        shortcut: Shortcut::None,
        category: "Window",
        keywords: "desktop launcher menu icon linux",
        requires_repo: false,
        needs: Needs::Linux,
    },
    CommandEntry {
        id: "add-remote",
        label: "Add Remote",
        shortcut: Shortcut::None,
        category: "Remotes",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "open-remote-in-browser",
        label: crate::menu_labels::OPEN_REMOTE_IN_BROWSER,
        shortcut: Shortcut::Secondary("K"),
        category: "Remotes",
        keywords: "github gitlab bitbucket forge website url view page issues",
        requires_repo: true,
        needs: Needs::RemoteWebPage,
    },
    CommandEntry {
        id: "add-submodule",
        label: "Add Submodule",
        shortcut: Shortcut::None,
        category: "Submodules",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "update-submodules",
        label: "Update Submodules",
        shortcut: Shortcut::None,
        category: "Submodules",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "add-worktree",
        label: "Add Worktree",
        shortcut: Shortcut::None,
        category: "Worktrees",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "blame",
        label: "Blame / Annotate",
        shortcut: Shortcut::Alt("B"),
        category: "History",
        keywords: "",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    CommandEntry {
        id: "show-reflog",
        label: "Show Reflog",
        shortcut: Shortcut::None,
        category: "History",
        keywords: "reflog log restore reset recover",
        requires_repo: true,
        needs: Needs::Nothing,
    },
    // TODO: "undo"              - Undo (Edit)
    // TODO: "redo"              - Redo (Edit)
    // TODO: "keyboard-shortcuts" - Keyboard Shortcuts (Help)
    // TODO: "file-history"     - File History (History)
    // TODO: "search-commits"   - Search Commits (Navigation)
    // TODO: "checkout-remote-branch" - Checkout Remote Branch
    // TODO: "delete-remote-branch"   - Delete Remote Branch
    // TODO: "merge"                  - Merge Branch/Ref
    // TODO: "delete-tag"             - Delete Tag
    // TODO: "remove-remote"          - Remove Remote
    // TODO: "edit-remote-url"        - Edit Remote URL
    // TODO: "remove-submodule"       - Remove Submodule
    // TODO: "remove-worktree"        - Remove Worktree
    // TODO: "discard-all"        - Discard All Changes (Working Copy)
];

/// A palette entry that survived filtering, plus the label byte positions the
/// query matched (for highlighting). Derefs to the entry so callers keep using
/// `cmd.label` / `cmd.id` / `cmd.category` directly.
#[derive(Clone)]
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

/// Keeps keyword matches below every label match in the ranking, whatever the
/// two raw scores are.
const KEYWORD_MATCH_PENALTY: i32 = 100_000;

pub(crate) fn filtered_commands(has_active_repo: bool, query: &str) -> Vec<CommandMatch> {
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
                .or_else(|| {
                    // Keyword hits carry no highlight positions and sort behind
                    // every label hit.
                    fuzzy_subsequence_match(entry.keywords, query).map(|(score, _)| {
                        (
                            score + KEYWORD_MATCH_PENALTY,
                            order,
                            CommandMatch {
                                entry,
                                positions: Vec::new(),
                            },
                        )
                    })
                })
        })
        .collect();

    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.2.label.len().cmp(&b.2.label.len()))
            .then_with(|| a.1.cmp(&b.1))
    });
    out.into_iter().map(|(_, _, m)| m).collect()
}

/// Translate a selected command index to its visual list index. The unfiltered
/// palette inserts a header for each category, while search results share one
/// compact "Results" header.
#[cfg(test)]
pub(crate) fn command_list_item_index(
    commands: &[CommandMatch],
    selected_index: usize,
    is_searching: bool,
) -> usize {
    if is_searching {
        return selected_index + 1;
    }

    let mut headers_before = 0usize;
    let mut current_category = None;
    for command in commands.iter().take(selected_index.saturating_add(1)) {
        if current_category != Some(command.category) {
            current_category = Some(command.category);
            headers_before += 1;
        }
    }
    selected_index + headers_before
}

#[derive(Clone, Copy)]
enum PaletteRow {
    Header(&'static str),
    Command(usize),
}

pub(crate) struct CommandPaletteView {
    pub(crate) query_input: Entity<components::TextInput>,
    pub(crate) restore_focus: Option<FocusHandle>,
    fallback_focus: Option<FocusHandle>,
    root_view: WeakEntity<GitCometView>,
    theme: AppTheme,
    context: PaletteContext,
    open: bool,
    query: SharedString,
    matches: Vec<CommandMatch>,
    rows: Vec<PaletteRow>,
    command_row_indices: Vec<usize>,
    selected_index: Option<usize>,
    scroll_handle: UniformListScrollHandle,
    _input_subscription: gpui::Subscription,
}

impl CommandPaletteView {
    pub(crate) fn new(
        theme: AppTheme,
        has_active_repo: bool,
        root_view: WeakEntity<GitCometView>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let query_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Search commands…".into(),
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_theme(theme, cx);
            input
        });
        let input_subscription = cx.observe_in(&query_input, window, |this, input, window, cx| {
            this.handle_input_notification(input, window, cx);
        });

        Self {
            query_input,
            restore_focus: None,
            fallback_focus: None,
            root_view,
            theme,
            context: PaletteContext {
                has_active_repo,
                ..PaletteContext::default()
            },
            open: false,
            query: SharedString::default(),
            matches: Vec::new(),
            rows: Vec::new(),
            command_row_indices: Vec::new(),
            selected_index: None,
            scroll_handle: UniformListScrollHandle::default(),
            _input_subscription: input_subscription,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.query_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    /// Refresh what commands are enabled against. Only a repository change
    /// alters which commands are listed; the rest only greys rows out.
    pub(crate) fn set_context(&mut self, context: PaletteContext, cx: &mut gpui::Context<Self>) {
        if self.context == context {
            return;
        }
        let relist = self.context.has_active_repo != context.has_active_repo;
        self.context = context;
        if self.open {
            if relist {
                self.rebuild_cached_results();
                self.clamp_selection();
            }
            cx.notify();
        }
    }

    /// Why `command` cannot run right now, if it cannot.
    fn unavailable(&self, command: &CommandEntry) -> Option<&'static str> {
        unavailable_reason(command.needs, &self.context)
    }

    pub(crate) fn open(
        &mut self,
        restore_focus: Option<FocusHandle>,
        fallback_focus: FocusHandle,
        context: PaletteContext,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open = true;
        self.restore_focus = restore_focus;
        self.fallback_focus = Some(fallback_focus);
        self.context = context;
        self.query = SharedString::default();
        self.selected_index = None;
        self.rebuild_cached_results();
        if !self.rows.is_empty() {
            self.scroll_handle
                .scroll_to_item_strict(0, ScrollStrategy::Top);
        }
        self.query_input
            .update(cx, |input, cx| input.set_text("", cx));
        let focus = self
            .query_input
            .read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        let palette_focus = self
            .query_input
            .read_with(cx, |input, _| input.focus_handle());
        let mut restore_focus = self.restore_focus.take();
        if restore_focus.as_ref() == Some(&palette_focus) {
            restore_focus = None;
        }
        let focus = restore_focus.or_else(|| self.fallback_focus.take());
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn rebuild_cached_results(&mut self) {
        self.matches = filtered_commands(self.context.has_active_repo, self.query.as_ref());
        self.rows.clear();
        self.command_row_indices.clear();

        if !self.query.is_empty() {
            if !self.matches.is_empty() {
                self.rows.push(PaletteRow::Header("Results"));
            }
            for command_index in 0..self.matches.len() {
                self.command_row_indices.push(self.rows.len());
                self.rows.push(PaletteRow::Command(command_index));
            }
            return;
        }

        let mut current_category = None;
        for (command_index, command) in self.matches.iter().enumerate() {
            if current_category != Some(command.category) {
                current_category = Some(command.category);
                self.rows.push(PaletteRow::Header(command.category));
            }
            self.command_row_indices.push(self.rows.len());
            self.rows.push(PaletteRow::Command(command_index));
        }
    }

    fn clamp_selection(&mut self) {
        self.selected_index = match (self.selected_index, self.matches.len()) {
            (_, 0) => None,
            (Some(index), len) => Some(index.min(len - 1)),
            (None, _) => None,
        };
    }

    fn scroll_to_selected(&self) {
        if let Some(row_index) = self
            .selected_index
            .and_then(|index| self.command_row_indices.get(index))
        {
            self.scroll_handle
                .scroll_to_item(*row_index, ScrollStrategy::Center);
        }
    }

    fn handle_input_notification(
        &mut self,
        input: Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (escape_pressed, arrow_up, shift_tab, arrow_down, tab, enter_pressed) =
            input.update(cx, |input, _| {
                (
                    input.take_escape_pressed(),
                    input.take_arrow_up_pressed(),
                    input.take_shift_tab_pressed(),
                    input.take_arrow_down_pressed(),
                    input.take_tab_pressed(),
                    input.take_enter_pressed(),
                )
            });

        if !self.open {
            return;
        }
        if escape_pressed {
            self.close_and_notify_root(None, window, cx);
            return;
        }

        let query_changed =
            input.read_with(cx, |input, _| input.text().trim() != self.query.as_ref());

        // TextInput also notifies for cursor blinking, selection movement, and
        // focus bookkeeping. Those notifications do not affect palette state.
        if !query_changed && !arrow_up && !shift_tab && !arrow_down && !tab && !enter_pressed {
            return;
        }

        if query_changed {
            self.query = input.read_with(cx, |input, _| {
                SharedString::from(input.text().trim().to_owned())
            });
            self.rebuild_cached_results();
            self.selected_index = (!self.matches.is_empty()).then_some(0);
            if !self.rows.is_empty() {
                self.scroll_handle
                    .scroll_to_item_strict(0, ScrollStrategy::Top);
            }
        }

        if arrow_up || shift_tab {
            self.selected_index = match (self.selected_index, self.matches.len()) {
                (_, 0) => None,
                (Some(index), _) if index > 0 => Some(index - 1),
                (_, len) => Some(len - 1),
            };
            self.scroll_to_selected();
            cx.notify();
            return;
        }

        if arrow_down || tab {
            self.selected_index = match (self.selected_index, self.matches.len()) {
                (_, 0) => None,
                (Some(index), len) if index + 1 < len => Some(index + 1),
                _ => Some(0),
            };
            self.scroll_to_selected();
            cx.notify();
            return;
        }

        if enter_pressed {
            // A selected but disabled command does nothing — falling through to
            // another match would run something the user did not pick.
            let command = match self.selected_index {
                Some(index) => self.matches.get(index),
                None => self
                    .matches
                    .iter()
                    .find(|command| self.unavailable(command).is_none()),
            }
            .filter(|command| self.unavailable(command).is_none())
            .map(|command| SharedString::from(command.id));
            if let Some(command) = command {
                self.close_and_notify_root(Some(command), window, cx);
            }
            return;
        }

        cx.notify();
    }

    fn close_and_notify_root(
        &mut self,
        command: Option<SharedString>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.close(window, cx);
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, root_cx| {
            root.command_palette_did_close(command.as_deref(), window, root_cx);
        });
    }

    fn render_label(
        &self,
        label: &str,
        positions: &[usize],
        color: gpui::Rgba,
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let highlight = gpui::HighlightStyle {
            color: Some(self.theme.colors.accent.foreground.into_color()),
            font_weight: Some(FontWeight::BOLD),
            ..gpui::HighlightStyle::default()
        };
        let mut ranges: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = Vec::new();
        for &position in positions {
            match ranges.last_mut() {
                Some((range, _)) if range.end == position => range.end = position + 1,
                _ => ranges.push((position..position + 1, highlight)),
            }
        }
        let focus_range = ranges.first().map(|(range, _)| range.clone());
        let mut text = components::TruncatedText::new(label.to_owned(), self.theme.ui_text(14.0))
            .profile(components::TextTruncationProfile::End)
            .text_color(color);
        if let Some(focus_range) = focus_range {
            text = text.focus_range(Some(focus_range));
        }
        if !ranges.is_empty() {
            text = text.highlights(ranges);
        }
        text.render(cx).into_any_element()
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);
        let row_height = scaled_px(36.0);
        let hover_overlay = theme.hover_overlay();
        let selected_overlay = theme.active_overlay();

        range
            .filter_map(|row_index| {
                let row = *self.rows.get(row_index)?;
                let (element, selected) = match row {
                    PaletteRow::Header(title) => (
                        div()
                            .h(row_height)
                            .w_full()
                            .flex()
                            .items_center()
                            .px(scaled_px(14.0))
                            .text_size(theme.ui_text(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.foreground.secondary)
                            .child(title)
                            .into_any_element(),
                        false,
                    ),
                    PaletteRow::Command(command_index) => {
                        let command = self.matches.get(command_index)?;
                        let command_id: SharedString = command.id.into();
                        let command_id_for_click = command_id.clone();
                        let selected = self.selected_index == Some(command_index);
                        let unavailable = self.unavailable(command);
                        let command_row = div()
                            // Without an id gpui never repaints on mouse-move,
                            // so the hover fill below would be computed and
                            // dropped every frame. The tooltip needs it too.
                            .id(("command_palette_row", command_index))
                            .h(row_height)
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(scaled_px(12.0))
                            .px(scaled_px(10.0))
                            .rounded(px(theme.radii.row));
                        let command_row = match unavailable {
                            None => command_row
                                .hover(move |style| style.bg(hover_overlay))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                        this.close_and_notify_root(
                                            Some(command_id_for_click.clone()),
                                            window,
                                            cx,
                                        );
                                    }),
                                ),
                            Some(reason) => command_row
                                .debug_selector(move || {
                                    format!("command_palette_disabled_{command_id}")
                                })
                                .gitcomet_tooltip(theme, reason.into()),
                        };

                        let label_color = if unavailable.is_some() {
                            theme.colors.foreground.disabled
                        } else {
                            theme.colors.foreground.primary
                        };
                        let label = div()
                            .flex()
                            .items_center()
                            .gap(scaled_px(4.0))
                            .overflow_hidden()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(self.render_label(
                                command.label,
                                &command.positions,
                                label_color,
                                cx,
                            ));

                        let mut content = command_row.child(label);
                        match unavailable {
                            // The palette is keyboard-first and a hover tooltip
                            // never shows for an arrow-key selection, so the
                            // selected disabled row spells out why in place.
                            Some(reason) if selected => {
                                content = content.child(
                                    div()
                                        .debug_selector(|| {
                                            "command_palette_unavailable_reason".to_string()
                                        })
                                        .flex_none()
                                        .text_size(theme.ui_text(12.0))
                                        .text_color(theme.colors.foreground.secondary)
                                        .child(reason),
                                );
                            }
                            Some(_) => {}
                            None => {
                                if let Some(shortcut_text) = command.shortcut.label() {
                                    content = content.child(components::shortcut_keys(
                                        &shortcut_text,
                                        theme,
                                        ui_scale,
                                    ));
                                }
                            }
                        }

                        (content.into_any_element(), selected)
                    }
                };
                Some(
                    div()
                        .relative()
                        .h(row_height)
                        .w_full()
                        .when(selected, |row| {
                            row.rounded_tr(px(theme.radii.row))
                                .rounded_br(px(theme.radii.row))
                                .bg(selected_overlay)
                                .child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .bottom_0()
                                        .w(scaled_px(3.0))
                                        .rounded_tr(px(theme.radii.row))
                                        .rounded_br(px(theme.radii.row))
                                        .bg(theme.colors.accent.foreground),
                                )
                        })
                        .px(scaled_px(6.0))
                        .child(element)
                        .into_any_element(),
                )
            })
            .collect()
    }
}

impl Render for CommandPaletteView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);
        let palette_width = scaled_px(620.0);
        let top_offset = scaled_px(56.0);
        let input_height = scaled_px(48.0);
        let row_height = scaled_px(36.0);
        let list_height = if self.rows.is_empty() {
            scaled_px(72.0)
        } else {
            row_height * self.rows.len().min(10) as f32
        };

        let list_body = if self.rows.is_empty() {
            div()
                .h(list_height)
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .px(scaled_px(12.0))
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("No matching commands")
                .into_any_element()
        } else {
            let scrollbar_gutter =
                Scrollbar::visible_gutter(self.scroll_handle.clone(), ScrollbarAxis::Vertical);
            let list = uniform_list(
                "command_palette_list",
                self.rows.len(),
                cx.processor(Self::render_rows),
            )
            .h(list_height)
            .pr(scrollbar_gutter)
            .track_scroll(&self.scroll_handle);
            let list = restrict_scroll_to_vertical_axis(list);
            let scrollbar = Scrollbar::new("command_palette_scrollbar", self.scroll_handle.clone())
                .render(theme);
            div()
                .id("command_palette_list_container")
                .relative()
                .w_full()
                .h(list_height)
                .min_w(px(0.0))
                .child(list)
                .child(scrollbar)
                .into_any_element()
        };

        let palette_body = components::modal_surface(theme)
            .child(
                div()
                    .w_full()
                    .h(input_height)
                    .flex()
                    .items_center()
                    .px(scaled_px(14.0))
                    .border_b_1()
                    .border_color(theme.colors.stroke.subtle)
                    .child(self.query_input.clone()),
            )
            .child(list_body);

        let scrim = components::modal_scrim(theme).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.close_and_notify_root(None, window, cx);
            }),
        );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim)
            .child(
                div()
                    .absolute()
                    .top(top_offset)
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w(palette_width)
                            .max_w(palette_width)
                            .child(palette_body),
                    ),
            )
            .into_any_element()
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

    #[test]
    fn keyword_matches_find_commands_their_label_no_longer_spells_out() {
        let matches = filtered_commands(true, "recent");

        let ids = matches.iter().map(|m| m.id).collect::<Vec<_>>();
        assert!(
            ids.contains(&"switch-repository"),
            "expected the repository switcher to stay reachable by its old wording, got {ids:?}"
        );
        let switcher = matches
            .iter()
            .find(|m| m.id == "switch-repository")
            .expect("switcher match");
        assert!(
            switcher.positions.is_empty(),
            "keyword hits must not highlight unrelated label bytes"
        );
    }

    #[test]
    fn label_matches_outrank_keyword_matches() {
        let matches = filtered_commands(true, "re");
        let first = matches.first().expect("at least one match");

        assert!(
            !first.positions.is_empty(),
            "expected a label match to lead, got the keyword hit {}",
            first.label
        );
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
        let matches = filtered_commands(true, "push");
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
        let with_repo = filtered_commands(true, "");
        let without_repo = filtered_commands(false, "");
        assert_eq!(with_repo.len(), COMMANDS.len());
        assert!(without_repo.len() < with_repo.len());
        assert!(with_repo.iter().all(|m| m.positions.is_empty()));
    }

    #[test]
    fn visual_list_index_accounts_for_grouped_and_search_headers() {
        let grouped = filtered_commands(true, "");
        let branch_index = grouped
            .iter()
            .position(|command| command.id == "create-branch")
            .expect("Create Branch command");
        assert_eq!(
            command_list_item_index(&grouped, branch_index, false),
            branch_index + 3,
            "Commit, Working Copy, and Branch headers precede Create Branch"
        );

        let results = filtered_commands(true, "branch");
        assert_eq!(
            command_list_item_index(&results, 0, true),
            1,
            "search results have one shared header"
        );
    }

    #[test]
    fn locate_file_in_explorer_is_a_repository_command() {
        assert_eq!(
            filtered_commands(true, "open in file explorer")
                .first()
                .map(|command| command.id),
            Some("locate-file-in-explorer")
        );
        // Keep the previous label and common synonyms searchable.
        assert!(
            filtered_commands(true, "show")
                .iter()
                .any(|command| command.id == "locate-file-in-explorer")
        );
        assert!(
            filtered_commands(false, "open in file explorer").is_empty(),
            "there is no file to open without a repository"
        );
    }

    /// The unfiltered palette prints a header each time the category changes,
    /// so a category split across the table shows its header twice. That had
    /// happened to Navigation before this test existed.
    #[test]
    fn every_category_is_one_contiguous_run() {
        let mut seen: Vec<&str> = Vec::new();
        for command in COMMANDS {
            if seen.last() != Some(&command.category) {
                assert!(
                    !seen.contains(&command.category),
                    "{:?} is split: its entries must sit together in COMMANDS",
                    command.category
                );
                seen.push(command.category);
            }
        }
    }

    #[test]
    fn unavailable_reasons_follow_the_state_they_depend_on() {
        let ctx = PaletteContext {
            has_active_repo: true,
            ..PaletteContext::default()
        };
        assert_eq!(unavailable_reason(Needs::Nothing, &ctx), None);
        for (needs, met) in [
            (
                Needs::ExternalEditor,
                PaletteContext {
                    external_editor: true,
                    ..ctx.clone()
                },
            ),
            (
                Needs::Merge,
                PaletteContext {
                    merging: true,
                    ..ctx.clone()
                },
            ),
            (
                Needs::Sequencer,
                PaletteContext {
                    sequencer: true,
                    ..ctx.clone()
                },
            ),
        ] {
            assert!(
                unavailable_reason(needs, &ctx).is_some(),
                "{needs:?} should be disabled without its state"
            );
            assert_eq!(
                unavailable_reason(needs, &met),
                None,
                "{needs:?} should be enabled once its state holds"
            );
        }
    }

    /// Continue has a second way to be unavailable, mirroring the action bar.
    #[test]
    fn continue_stays_disabled_while_conflicts_remain() {
        let sequencing = PaletteContext {
            sequencer: true,
            ..PaletteContext::default()
        };
        assert_eq!(
            unavailable_reason(Needs::SequencerResolved, &sequencing),
            None
        );
        assert_eq!(
            unavailable_reason(
                Needs::SequencerResolved,
                &PaletteContext {
                    unresolved_conflicts: true,
                    ..sequencing
                }
            ),
            Some("Resolve all conflicts before continuing")
        );
        assert!(unavailable_reason(Needs::SequencerResolved, &PaletteContext::default()).is_some());
    }

    #[test]
    fn each_tag_push_mode_has_its_own_availability() {
        let mut ctx = PaletteContext::default();
        ctx.push_with_tags_unavailable[TagPushMode::All.index()] = Some("no remote");
        assert_eq!(
            unavailable_reason(Needs::PushWithTags(TagPushMode::All), &ctx),
            Some("no remote")
        );
        assert_eq!(
            unavailable_reason(Needs::PushWithTags(TagPushMode::FollowAnnotated), &ctx),
            None
        );
    }

    /// Asked for explicitly: platform-only commands are listed everywhere and
    /// greyed out where they cannot run, not hidden.
    #[test]
    fn platform_commands_are_listed_everywhere_and_disabled_elsewhere() {
        let ctx = PaletteContext::default();
        assert_eq!(
            unavailable_reason(Needs::MacOs, &ctx).is_none(),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            unavailable_reason(Needs::Linux, &ctx).is_none(),
            cfg!(any(target_os = "linux", target_os = "freebsd"))
        );
        let listed = filtered_commands(true, "")
            .iter()
            .map(|command| command.id)
            .collect::<Vec<_>>();
        for id in ["hide", "hide-others", "install-desktop-integration"] {
            assert!(
                listed.contains(&id),
                "{id} should be listed on every platform"
            );
        }
    }

    #[test]
    fn go_to_is_findable_by_its_label_and_by_its_old_reveal_wording() {
        assert_eq!(
            filtered_commands(true, "go to")
                .first()
                .map(|command| command.id),
            Some("reveal-commit")
        );
        for query in ["reveal", "sha", "revision"] {
            assert!(
                filtered_commands(true, query)
                    .iter()
                    .any(|command| command.id == "reveal-commit"),
                "{query:?} should still find Go to"
            );
        }
        assert!(
            filtered_commands(false, "go to").is_empty(),
            "there is nothing to go to without a repository"
        );
    }

    #[test]
    fn open_remote_in_browser_is_findable_by_forge_words() {
        assert_eq!(
            filtered_commands(true, "open remote")
                .first()
                .map(|command| command.id),
            Some("open-remote-in-browser")
        );
        for query in ["browser", "github", "gitlab", "web"] {
            assert!(
                filtered_commands(true, query)
                    .iter()
                    .any(|command| command.id == "open-remote-in-browser"),
                "{query:?} should find Open remote in web browser"
            );
        }
        assert!(
            !filtered_commands(false, "open remote")
                .iter()
                .any(|command| command.id == "open-remote-in-browser"),
            "there is no remote without a repository"
        );
    }

    #[test]
    fn open_remote_in_browser_shows_the_chord_it_is_bound_to() {
        let entry = COMMANDS
            .iter()
            .find(|command| command.id == "open-remote-in-browser")
            .expect("the palette offers Open remote in web browser");
        // `bind_app_keys` binds `secondary-k`; the label is kept in sync by hand.
        assert_eq!(entry.shortcut, Shortcut::Secondary("K"));
        let expected = if cfg!(target_os = "macos") {
            "Cmd+K"
        } else {
            "Ctrl+K"
        };
        assert_eq!(entry.shortcut.label().as_deref(), Some(expected));
        assert_eq!(entry.label, crate::menu_labels::OPEN_REMOTE_IN_BROWSER);
        assert_eq!(entry.category, "Remotes");
        assert!(entry.requires_repo);
        assert_eq!(entry.needs, Needs::RemoteWebPage);
    }

    #[test]
    fn remote_web_page_need_reports_its_context_reason() {
        let mut ctx = PaletteContext::default();
        assert_eq!(unavailable_reason(Needs::RemoteWebPage, &ctx), None);
        ctx.remote_web_page_unavailable = Some("Add a remote first");
        assert_eq!(
            unavailable_reason(Needs::RemoteWebPage, &ctx),
            Some("Add a remote first")
        );
    }

    #[test]
    fn rename_branch_is_available_for_repository_commands() {
        let matches = filtered_commands(true, "rename branch");
        assert_eq!(
            matches.first().map(|command| command.id),
            Some("rename-branch")
        );
        assert!(
            filtered_commands(false, "rename branch").is_empty(),
            "Rename Branch requires an active repository"
        );
    }

    #[test]
    fn rebase_onto_is_available_for_repository_commands() {
        let matches = filtered_commands(true, "rebase onto");
        assert_eq!(matches.first().map(|command| command.id), Some("rebase"));
        assert!(
            filtered_commands(false, "rebase onto").is_empty(),
            "Rebase Onto requires an active repository"
        );
    }
}
