use crate::kit::{Scrollbar, ScrollbarAxis};
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent,
    ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity, Window, div, px,
    uniform_list,
};

use super::shortcut_labels::Shortcut;
use super::{GitCometView, components, restrict_scroll_to_vertical_axis};

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
        id: "rename-branch",
        label: "Rename Branch",
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
    has_active_repo: bool,
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
            has_active_repo,
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

    pub(crate) fn set_has_active_repo(
        &mut self,
        has_active_repo: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.has_active_repo == has_active_repo {
            return;
        }
        self.has_active_repo = has_active_repo;
        if self.open {
            self.rebuild_cached_results();
            self.clamp_selection();
            cx.notify();
        }
    }

    pub(crate) fn open(
        &mut self,
        restore_focus: Option<FocusHandle>,
        fallback_focus: FocusHandle,
        has_active_repo: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open = true;
        self.restore_focus = restore_focus;
        self.fallback_focus = Some(fallback_focus);
        self.has_active_repo = has_active_repo;
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
        self.matches = filtered_commands(self.has_active_repo, self.query.as_ref());
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
            let command = self
                .selected_index
                .and_then(|index| self.matches.get(index))
                .or_else(|| self.matches.first())
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
        cx: &gpui::Context<Self>,
    ) -> AnyElement {
        let highlight = gpui::HighlightStyle {
            color: Some(self.theme.colors.accent.into()),
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
        let mut text = components::TruncatedText::new(label.to_owned())
            .profile(components::TextTruncationProfile::End)
            .text_color(self.theme.colors.text)
            .text_sm();
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
        let scaled_px = |value: f32| ui_scale.px(value);
        let row_height = scaled_px(36.0);
        let hover_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.07 } else { 0.05 });
        let selected_overlay =
            with_alpha(theme.colors.text, if theme.is_dark { 0.11 } else { 0.08 });

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
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.text_muted)
                            .child(title)
                            .into_any_element(),
                        false,
                    ),
                    PaletteRow::Command(command_index) => {
                        let command = self.matches.get(command_index)?;
                        let command_id: SharedString = command.id.into();
                        let command_id_for_click = command_id.clone();
                        let command_row = div()
                            .h(row_height)
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(scaled_px(10.0))
                            .rounded(px(theme.radii.row))
                            .hover(move |style| style.bg(hover_overlay))
                            .cursor(CursorStyle::PointingHand);

                        let label = div()
                            .flex()
                            .items_center()
                            .gap(scaled_px(4.0))
                            .overflow_hidden()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(self.render_label(command.label, &command.positions, cx));

                        let mut content = command_row.child(label);
                        if let Some(shortcut_text) = command.shortcut.label() {
                            content = content.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(scaled_px(4.0))
                                    .flex_shrink_0()
                                    .children(shortcut_text.split('+').map(|key| {
                                        div()
                                            .min_w(scaled_px(22.0))
                                            .h(scaled_px(22.0))
                                            .px(scaled_px(6.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(scaled_px(4.0))
                                            .bg(with_alpha(
                                                theme.colors.text,
                                                if theme.is_dark { 0.06 } else { 0.035 },
                                            ))
                                            .font_family(
                                                crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                            )
                                            .text_xs()
                                            .line_height(scaled_px(14.0))
                                            .text_color(theme.colors.text_muted)
                                            .child(key.to_owned())
                                    })),
                            );
                        }

                        (
                            content
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, _: &MouseDownEvent, window, cx| {
                                            this.close_and_notify_root(
                                                Some(command_id_for_click.clone()),
                                                window,
                                                cx,
                                            );
                                        },
                                    ),
                                )
                                .into_any_element(),
                            self.selected_index == Some(command_index),
                        )
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
                                        .bg(theme.colors.accent),
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
        let scaled_px = |value: f32| ui_scale.px(value);
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
                .text_sm()
                .text_color(theme.colors.text_muted)
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
                    .border_color(theme.colors.border_variant)
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
}
