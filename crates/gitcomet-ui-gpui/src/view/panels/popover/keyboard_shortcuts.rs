//! The keyboard shortcut reference: a default dialog listing the shortcut
//! documentation and the live commands, searchable and grouped by where they work.

use super::*;
use crate::kit::components::Shortcut;
use crate::kit::{Scrollbar, ScrollbarAxis};
use crate::view::command_palette::{self, CommandEntry};
use gitcomet_markdown::{MarkdownPreviewRowKind, parse_markdown};
use gpui::{ListAlignment, ListOffset, ListState};
use std::sync::OnceLock;

const SHORTCUTS: &str = include_str!("../../../../../../docs/shortcuts.md");
const EXPLORER: &str = include_str!("../../../../../../docs/file-management.md");
const ESTIMATED_ROW_HEIGHT: f32 = 44.0;
const DIALOG_WIDTH_PX: f32 = 960.0;
const DIALOG_HEIGHT_PX: f32 = 760.0;
const RAIL_WIDTH_PX: f32 = 200.0;
/// Room the centered placement leaves around the dialog: its 80px top
/// offset plus a bottom margin.
const WINDOW_INSET_PX: f32 = 104.0;
/// Preferred is the floor: the panel takes the window-fitted width itself.
pub(super) const WIDTH: PopoverWidthSpec = PopoverWidthSpec::range(360.0, 360.0, DIALOG_WIDTH_PX);

#[derive(Clone, Debug)]
struct Entry {
    category: String,
    action: String,
    keys: Vec<String>,
    note: String,
}

impl Entry {
    fn matches(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {} {} {}",
            self.category,
            self.action,
            self.keys.join(" "),
            self.note
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }
}

pub(super) struct State {
    entries: Vec<Entry>,
    categories: Vec<String>,
    selected: Option<String>,
    query: String,
    rows: Vec<HelpRow>,
    scroll: ListState,
    estimated_row_height: Pixels,
    navigation_scroll: ScrollHandle,
    search_input: Option<Entity<components::TextInput>>,
    _search_subscription: Option<gpui::Subscription>,
}

enum HelpRow {
    Header(usize),
    Entry(usize),
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            categories: Vec::new(),
            selected: None,
            query: String::new(),
            rows: Vec::new(),
            scroll: ListState::new(0, ListAlignment::Top, px(0.0)),
            estimated_row_height: px(ESTIMATED_ROW_HEIGHT),
            navigation_scroll: ScrollHandle::new(),
            search_input: None,
            _search_subscription: None,
        }
    }

    fn reset(&mut self, extensions: &[CommandEntry]) {
        self.entries = catalog(cfg!(target_os = "macos"), extensions);
        self.categories.clear();
        for entry in &self.entries {
            if !self.categories.contains(&entry.category) {
                self.categories.push(entry.category.clone());
            }
        }
        self.selected = None;
        self.query.clear();
        self.navigation_scroll.set_offset(point(px(0.0), px(0.0)));
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        for (category_ix, category) in self.categories.iter().enumerate() {
            if self
                .selected
                .as_ref()
                .is_some_and(|selected| selected != category)
            {
                continue;
            }
            let mut header_added = false;
            for (index, entry) in self.entries.iter().enumerate() {
                if entry.category != *category || !entry.matches(&self.query) {
                    continue;
                }
                if !header_added {
                    self.rows.push(HelpRow::Header(category_ix));
                    header_added = true;
                }
                self.rows.push(HelpRow::Entry(index));
            }
        }
        self.scroll
            .reset_with_uniform_height(self.rows.len(), self.estimated_row_height);
        self.scroll.scroll_to(ListOffset::default());
    }
}

/// Fills the reference and focuses its search field; part of `open_popover`.
pub(super) fn open(
    this: &mut PopoverHost,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.keyboard_shortcuts
        .reset(crate::view::extension_host::palette_entries(cx));
    let input = search_input(this, window, cx);
    input.update(cx, |input, cx| {
        input.clear_transient_key_presses();
        input.set_text("", cx);
    });
    let focus = input.read_with(cx, |input, _| input.focus_handle());
    window.focus(&focus, cx);
}

fn search_input(
    this: &mut PopoverHost,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> Entity<components::TextInput> {
    if let Some(input) = &this.keyboard_shortcuts.search_input {
        return input.clone();
    }
    let input = cx.new(|cx| {
        components::TextInput::new(
            components::TextInputOptions {
                placeholder: "Search actions or keys…".into(),
                ..Default::default()
            },
            window,
            cx,
        )
    });
    let subscription = cx.observe_in(&input, window, |this, input, window, cx| {
        if this.popover != Some(PopoverKind::KeyboardShortcuts) {
            return;
        }
        let keys = input.update(cx, |input, _| picker_nav::PickerNavKeys::take(input));
        // Escape normally arrives as `PopoverPromptDismiss`; this covers a
        // keymap without that binding.
        if keys.escape {
            this.close_popover_and_restore_focus(window, cx);
            return;
        }
        let help = &mut this.keyboard_shortcuts;
        let query = input.read(cx).text().trim();
        if query != help.query {
            help.query = query.to_owned();
            help.rebuild_rows();
            cx.notify();
        }
        if keys.arrow_up || keys.arrow_down {
            help.scroll
                .scroll_by(px(if keys.arrow_up { -72.0 } else { 72.0 }));
            cx.notify();
        }
    });
    this.keyboard_shortcuts.search_input = Some(input.clone());
    this.keyboard_shortcuts._search_subscription = Some(subscription);
    input
}

fn category(heading: &str) -> String {
    match heading {
        "App window shortcuts" => "Window",
        "Repository and layout shortcuts" => "Repository & layout",
        "Image preview shortcuts" => "Images & diagrams",
        "Editing" => "Text editing",
        "Cursor movement and selection" => "Cursor & selection",
        "Word movement and word deletion" => "Word editing",
        "Commit composer" => "Commit composer",
        "Commit message editor" => "Commit message editor",
        "History view shortcuts" => "Commit history",
        "Picker shortcuts" => "Pickers",
        "File history tab" => "File history",
        "Embedded terminal shortcuts" => "Terminal",
        "Diff view shortcuts" => "Diff & conflicts",
        "Context menu shortcuts" => "Context menus",
        "Focused diff window" => "Focused diff window",
        "Explorer" => "File explorer",
        other => other,
    }
    .to_owned()
}

fn doc_keys(value: &str, is_macos: bool) -> Vec<String> {
    if value == "Letter or digit shown beside the item" {
        return vec!["Letter".to_owned(), "Digit".to_owned()];
    }
    if value == "Cut, then Paste" {
        return vec!["Ctrl+X Ctrl+V".to_owned()];
    }
    let value = value
        .replace("Ctrl/Cmd", if is_macos { "Cmd" } else { "Ctrl" })
        .replace(" or ", ", ")
        .replace(" / ", ", ")
        .replace(" or double-click", "");
    value
        .split(", ")
        .map(str::trim)
        .filter(|key| !key.is_empty() && !matches!(*key, "None" | "—" | "double-click"))
        .map(|key| {
            let mut key = key
                .replace(" + ", "+")
                .replace("Arrow keys", "Arrows")
                .replace("arrow keys", "Arrows");
            for name in ["Option", "Cmd", "Ctrl", "Alt", "Shift", "Fn"] {
                key = key.replace(&format!("{name}-"), &format!("{name}+"));
            }
            components::ordered_shortcut_label(&key)
        })
        .collect()
}

fn documented(source: &str, is_macos: bool, explorer_only: bool) -> Vec<Entry> {
    let Some(document) = parse_markdown(source) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut group = String::new();
    let mut platform_table = false;
    let mut shortcut_table = false;
    for row in document.rows {
        if let MarkdownPreviewRowKind::Heading { level: 2 | 3 } = row.kind {
            group = category(row.text.as_ref());
        }
        if explorer_only && group != "File explorer" {
            continue;
        }
        let Some(table) = row.table else {
            continue;
        };
        let cells: Vec<&str> = table
            .cells
            .iter()
            .map(|cell| &row.text[cell.range.clone()])
            .collect();
        if matches!(
            row.kind,
            MarkdownPreviewRowKind::TableRow { is_header: true }
        ) {
            platform_table = cells.get(1).is_some_and(|cell| *cell == "macOS");
            shortcut_table = platform_table || cells.get(1).is_some_and(|cell| *cell == "Shortcut");
            continue;
        }
        if !shortcut_table || cells.len() < 2 {
            continue;
        }
        let column = if platform_table && !is_macos { 2 } else { 1 };
        let Some(keys) = cells.get(column) else {
            continue;
        };
        // The history find-options row describes mouse controls, not hotkeys.
        if keys.contains("buttons") || *keys == "Drag or scroll" {
            continue;
        }
        let keys = doc_keys(keys, is_macos);
        if keys.is_empty() {
            continue;
        }
        let note_column = if platform_table { 3 } else { 2 };
        result.push(Entry {
            category: group.clone(),
            action: cells[0].to_owned(),
            keys,
            note: cells.get(note_column).unwrap_or(&"").to_string(),
        });
    }
    result
}

fn catalog(is_macos: bool, extensions: &[CommandEntry]) -> Vec<Entry> {
    static DOCUMENTED: [OnceLock<Vec<Entry>>; 2] = [OnceLock::new(), OnceLock::new()];
    let mut entries = DOCUMENTED[usize::from(is_macos)]
        .get_or_init(|| {
            let mut entries = documented(SHORTCUTS, is_macos, false);
            entries.extend(documented(EXPLORER, is_macos, true));
            entries
        })
        .clone();
    for command in command_palette::COMMANDS.iter().chain(extensions) {
        let key = match command.shortcut {
            Shortcut::None => continue,
            Shortcut::Secondary(suffix) => {
                format!("{}+{suffix}", if is_macos { "Cmd" } else { "Ctrl" })
            }
            Shortcut::Alt(suffix) => {
                format!("{}+{suffix}", if is_macos { "Option" } else { "Alt" })
            }
            Shortcut::Platform { macos, other } => if is_macos { macos } else { other }.to_owned(),
            Shortcut::MacOs(key) => {
                if is_macos {
                    key.to_owned()
                } else {
                    continue;
                }
            }
            Shortcut::Keystrokes(key) => components::keystrokes_label_for(key, is_macos),
        };
        let key = components::ordered_shortcut_label(&key);
        if entries.iter().any(|entry| entry.keys.contains(&key)) {
            continue;
        }
        entries.push(Entry {
            category: if command.id.starts_with("extension:") {
                "Extensions"
            } else {
                command.category
            }
            .to_owned(),
            action: command.label().into_owned(),
            keys: vec![key],
            note: String::new(),
        });
    }
    entries
}

fn row(
    this: &PopoverHost,
    entry: &Entry,
    index: usize,
    scale: ui_scale::UiScale,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let p = crate::ui_scale::scaler(scale);
    let keys = entry.keys.iter().enumerate().flat_map(|(index, key)| {
        let separator = (index > 0).then(|| {
            div()
                .text_size(theme.ui_text(11.0))
                .text_color(theme.colors.foreground.disabled)
                .child("or")
                .into_any_element()
        });
        separator.into_iter().chain(std::iter::once(
            components::shortcut_keys(key, theme, scale).into_any_element(),
        ))
    });
    let row = div()
        .id(("shortcut_help_row", index))
        .debug_selector(move || format!("shortcut_help_row_{index}"))
        .flex_shrink_0()
        .w_full()
        .flex()
        .items_center()
        .gap(p(18.0))
        .px_2()
        .py(p(10.0))
        .border_b_1()
        .border_color(theme.colors.stroke.subtle)
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(theme.ui_text(13.0))
                .child(entry.action.clone()),
        )
        .child(
            div()
                .w(gpui::relative(0.48))
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(p(6.0))
                .children(keys),
        );
    if entry.note.is_empty() {
        return row.into_any_element();
    }
    // Popovers hide gpui tooltips; their own tooltip host shows the note.
    let note = SharedString::from(entry.note.clone());
    let (note_for_move, host_for_move, host_for_hover) = (
        note.clone(),
        this.tooltip_host.clone(),
        this.tooltip_host.clone(),
    );
    row.on_mouse_move(cx.listener(move |_, event: &MouseMoveEvent, _, cx| {
        let _ = host_for_move.update(cx, |host, cx| {
            host.on_mouse_moved(event.position, cx);
            host.set_tooltip_text_if_changed(Some(note_for_move.clone()), cx);
        });
    }))
    .on_hover(cx.listener(move |_, hovering: &bool, _, cx| {
        if !*hovering {
            let _ = host_for_hover.update(cx, |host, cx| host.clear_tooltip_if_matches(&note, cx));
        }
    }))
    .into_any_element()
}

fn rows(
    this: &PopoverHost,
    theme: AppTheme,
    scale: ui_scale::UiScale,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let p = crate::ui_scale::scaler(scale);
    if this.keyboard_shortcuts.rows.is_empty() {
        return div()
            .debug_selector(|| "shortcut_help_empty".to_string())
            .p(p(32.0))
            .flex()
            .flex_col()
            .gap(p(8.0))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child("No shortcuts found"),
            )
            .child(
                div()
                    .text_color(theme.colors.foreground.secondary)
                    .child("Try an action, a view, or a key name."),
            )
            .into_any_element();
    }
    gpui::list(
        this.keyboard_shortcuts.scroll.clone(),
        cx.processor(move |this, index, _, cx| {
            let help = &this.keyboard_shortcuts;
            match help.rows[index] {
                HelpRow::Header(category) => div()
                    .px_2()
                    .pt(p(22.0))
                    .pb(p(10.0))
                    .text_size(theme.ui_text(12.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.secondary)
                    .child(help.categories[category].clone())
                    .into_any_element(),
                HelpRow::Entry(entry) => row(this, &help.entries[entry], index, scale, cx),
            }
        }),
    )
    .size_full()
    .into_any_element()
}

/// Any chord bound to Keyboard Shortcuts. Prompts unbind them, so the dialog
/// closes itself on one.
fn is_toggle(keystroke: &gpui::Keystroke) -> bool {
    let mods = keystroke.modifiers;
    mods.secondary()
        && !mods.alt
        && !mods.function
        && mods.number_of_modifiers() == if mods.shift { 2 } else { 1 }
        && matches!(keystroke.key.as_str(), "/" | "?")
}

pub(super) fn panel(
    this: &mut PopoverHost,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scale = popover_ui_scale(cx);
    let p = crate::ui_scale::scaler(scale);
    let window_size = crate::kit::window_chrome::window_surface_bounds(window).size;
    let width = p(DIALOG_WIDTH_PX)
        .min(window_size.width - p(32.0))
        .max(WIDTH.preferred_px(scale));
    let height = p(DIALOG_HEIGHT_PX)
        .min(window_size.height - p(WINDOW_INSET_PX))
        .max(p(240.0));

    let help = &mut this.keyboard_shortcuts;
    let estimated_row_height = p(ESTIMATED_ROW_HEIGHT);
    if help.estimated_row_height != estimated_row_height {
        help.estimated_row_height = estimated_row_height;
        let top = help.scroll.logical_scroll_top();
        help.scroll
            .reset_with_uniform_height(help.rows.len(), estimated_row_height);
        help.scroll.scroll_to(top);
    }
    let count = help
        .rows
        .iter()
        .filter(|row| matches!(row, HelpRow::Entry(_)))
        .count();
    let scroll = help.scroll.clone();
    let navigation_scroll = help.navigation_scroll.clone();
    let search_input = help.search_input.clone();
    let categories: Vec<_> = std::iter::once(None)
        .chain(help.categories.iter().cloned().map(Some))
        .enumerate()
        .map(|(index, group)| {
            let active = help.selected == group;
            let label = group.clone().unwrap_or_else(|| "All shortcuts".to_owned());
            components::Button::new(format!("shortcut_help_category_{index}"), label)
                .style(if active {
                    components::ButtonStyle::Subtle
                } else {
                    components::ButtonStyle::Transparent
                })
                .on_click(theme, cx, move |this, _, _, cx| {
                    let help = &mut this.keyboard_shortcuts;
                    help.selected = group.clone();
                    help.rebuild_rows();
                    cx.notify();
                })
                .debug_selector(move || format!("shortcut_help_category_{index}"))
                .w_full()
                .flex_shrink_0()
                .justify_start()
                .into_any_element()
        })
        .collect();

    // Rows are measured as they scroll in; a new width re-estimates the rest.
    let previous_width = scroll.viewport_bounds().size.width;
    let estimate_scroll = scroll.clone();
    let estimate_unmeasured_rows = gpui::canvas(
        move |_, _, _| {
            if estimate_scroll.viewport_bounds().size.width != previous_width {
                estimate_scroll
                    .clone()
                    .with_uniform_item_height(estimated_row_height);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();

    let search = div()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().min_w(px(0.0)).children(search_input))
        .child(
            div()
                .flex_none()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(format!("{count} shortcuts")),
        );
    let body = div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .child(
            div()
                .id("shortcut_help_navigation")
                .flex()
                .flex_col()
                .gap(px(1.0))
                .w(p(RAIL_WIDTH_PX))
                .flex_none()
                .min_h(px(0.0))
                .p_1()
                .border_r_1()
                .border_color(theme.colors.stroke.default)
                .overflow_y_scroll()
                .track_scroll(&navigation_scroll)
                .children(categories),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(
                    div()
                        .id("shortcut_help_scroll")
                        .flex()
                        .flex_col()
                        .size_full()
                        .pr(Scrollbar::visible_gutter(
                            scroll.clone(),
                            ScrollbarAxis::Vertical,
                        ))
                        .child(rows(this, theme, scale, cx))
                        .child(estimate_unmeasured_rows),
                )
                .child(Scrollbar::new("shortcut_help_scrollbar", scroll).render(theme)),
        );
    let close = cancel_button_labeled(
        "shortcut_help_close",
        "shortcut_help_close_hint",
        "Close",
        theme,
    )
    .on_click(theme, cx, |this, _, window, cx| {
        this.close_popover_and_restore_focus(window, cx);
    })
    .debug_selector(|| "shortcut_help_close".to_string());

    ConfirmDialog::new("Keyboard shortcuts", WIDTH)
        .section(search)
        .divider(theme)
        .section(body)
        .render(theme, div(), close, cx)
        .debug_selector(|| "keyboard_shortcuts_dialog".to_string())
        .w(width)
        .h(height)
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if is_toggle(&event.keystroke) {
                cx.stop_propagation();
                this.close_popover_and_restore_focus(window, cx);
            }
        }))
}

#[cfg(test)]
impl PopoverHost {
    pub(in crate::view) fn keyboard_shortcuts_search_input(
        &self,
    ) -> Option<Entity<components::TextInput>> {
        self.keyboard_shortcuts.search_input.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_covers_documented_contexts_and_live_commands_on_both_platforms() {
        for is_macos in [false, true] {
            let entries = catalog(is_macos, &[]);
            assert!(entries.len() > 100, "all documented views must be included");
            for group in [
                "Window",
                "Repository & layout",
                "Text editing",
                "Terminal",
                "Diff & conflicts",
                "File explorer",
                "Pickers",
            ] {
                assert!(
                    entries.iter().any(|entry| entry.category == group),
                    "missing {group}"
                );
            }
            let key = if is_macos {
                "Cmd+Option+N"
            } else {
                "Ctrl+Alt+N"
            };
            assert!(entries.iter().any(|entry| entry.action == "Create branch"
                && entry.keys.iter().any(|value| value == key)));
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.keys.iter().any(|value| value.ends_with("+/")))
            );
            assert!(
                !entries
                    .iter()
                    .any(|entry| entry.keys.iter().any(|value| value == "None"))
            );
        }
    }

    #[test]
    fn reference_search_matches_actions_modifiers_and_contexts() {
        let entries = catalog(true, &[]);
        assert!(entries.iter().any(|entry| entry.matches("branch cmd")));
        assert!(
            entries
                .iter()
                .any(|entry| entry.matches("file explorer rename"))
        );
        assert!(entries.iter().any(|entry| entry.matches("terminal paste")));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.matches("no-such-action-9382"))
        );
        assert_eq!(doc_keys("Option-Cmd-V", true), vec!["Cmd+Option+V"]);
        assert_eq!(doc_keys("Cmd-,", true), vec!["Cmd+,"]);
        assert_eq!(doc_keys("Cmd-+ / Cmd-=", true), vec!["Cmd++", "Cmd+="]);
    }
}
