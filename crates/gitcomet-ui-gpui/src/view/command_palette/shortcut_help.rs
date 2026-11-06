//! A searchable reference built from the shortcut documentation and live commands.

use super::*;
use gitcomet_markdown::{MarkdownPreviewRowKind, parse_markdown};
use gpui::{ScrollHandle, point};

const SHORTCUTS: &str = include_str!("../../../../../docs/shortcuts.md");
const EXPLORER: &str = include_str!("../../../../../docs/file-management.md");

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

pub(super) struct ShortcutHelpState {
    entries: Vec<Entry>,
    categories: Vec<String>,
    selected: Option<String>,
    scroll: ScrollHandle,
    navigation_scroll: ScrollHandle,
}

impl ShortcutHelpState {
    pub(super) fn new(extensions: &[CommandEntry]) -> Self {
        let entries = catalog(cfg!(target_os = "macos"), extensions);
        let mut categories = Vec::new();
        for entry in &entries {
            if !categories.contains(&entry.category) {
                categories.push(entry.category.clone());
            }
        }
        Self {
            entries,
            categories,
            selected: None,
            scroll: ScrollHandle::new(),
            navigation_scroll: ScrollHandle::new(),
        }
    }

    pub(super) fn reset(&mut self) {
        self.selected = None;
        self.scroll_to_top();
        self.navigation_scroll.set_offset(point(px(0.0), px(0.0)));
    }

    pub(super) fn scroll_to_top(&self) {
        self.scroll.set_offset(point(px(0.0), px(0.0)));
    }

    pub(super) fn scroll_by(&self, delta: f32) {
        let offset = self.scroll.offset();
        self.scroll
            .set_offset(point(offset.x, offset.y + px(delta)));
    }

    fn visible(&self, query: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| {
                self.selected
                    .as_ref()
                    .is_none_or(|selected| *selected == entry.category)
                    && entry.matches(query)
            })
            .collect()
    }
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
    let mut entries = documented(SHORTCUTS, is_macos, false);
    entries.extend(documented(EXPLORER, is_macos, true));
    for command in COMMANDS.iter().chain(extensions) {
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

fn row(entry: &Entry, index: usize, theme: AppTheme, scale: ui_scale::UiScale) -> AnyElement {
    let p = crate::ui_scale::scaler(scale);
    div()
        .id(("shortcut_help_row", index))
        .flex_shrink_0()
        .w_full()
        .flex()
        .items_center()
        .gap(p(18.0))
        .px(p(16.0))
        .py(p(10.0))
        .border_b_1()
        .border_color(theme.colors.stroke.subtle)
        .hover_style(|style| style.bg(theme.hover_overlay()))
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
                .children(entry.keys.iter().enumerate().flat_map(|(index, key)| {
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
                })),
        )
        .when(!entry.note.is_empty(), |row| {
            row.gitcomet_tooltip(theme, entry.note.clone().into())
        })
        .into_any_element()
}

pub(super) fn render(
    palette: &mut CommandPaletteView,
    window: &mut Window,
    cx: &mut gpui::Context<CommandPaletteView>,
) -> AnyElement {
    let theme = palette.theme;
    let scale = ui_scale::UiScale::current(cx);
    let p = crate::ui_scale::scaler(scale);
    let width = p(1040.0).min(window.viewport_size().width - p(40.0));
    let height = p(800.0).min(window.viewport_size().height - p(64.0));
    let help = &palette.shortcut_help;
    let visible = help.visible(palette.query.as_ref());
    let count = visible.len();
    let mut rows = Vec::new();
    for group in &help.categories {
        let entries: Vec<_> = visible
            .iter()
            .filter(|entry| entry.category == *group)
            .collect();
        if entries.is_empty() {
            continue;
        }
        rows.push(
            div()
                .flex_shrink_0()
                .px(p(16.0))
                .pt(p(22.0))
                .pb(p(10.0))
                .text_size(theme.ui_text(12.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.secondary)
                .child(group.clone())
                .into_any_element(),
        );
        for entry in entries {
            rows.push(row(entry, rows.len(), theme, scale));
        }
    }
    if rows.is_empty() {
        rows.push(
            div()
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
                .into_any_element(),
        );
    }
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
                    this.shortcut_help.selected = group.clone();
                    this.shortcut_help.scroll_to_top();
                    cx.notify();
                })
                .debug_selector(move || format!("shortcut_help_category_{index}"))
                .w_full()
                .flex_shrink_0()
                .justify_start()
                .into_any_element()
        })
        .collect();
    let scrollbar = Scrollbar::new("shortcut_help_scrollbar", help.scroll.clone()).render(theme);
    let body = div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .child(
            div()
                .id("shortcut_help_navigation")
                .flex()
                .flex_col()
                .gap(p(2.0))
                .w(p(190.0))
                .flex_shrink_0()
                .min_h(px(0.0))
                .border_r_1()
                .border_color(theme.colors.stroke.subtle)
                .p(p(10.0))
                .overflow_y_scroll()
                .track_scroll(&help.navigation_scroll)
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
                        .overflow_y_scroll()
                        .track_scroll(&help.scroll)
                        .pr(Scrollbar::visible_gutter(
                            help.scroll.clone(),
                            ScrollbarAxis::Vertical,
                        ))
                        .pb(p(18.0))
                        .children(rows),
                )
                .child(scrollbar),
        );
    let close = components::Button::new("shortcut_help_close", "×")
        .style(components::ButtonStyle::Transparent)
        .on_click(theme, cx, |this, _, window, cx| {
            this.close_and_notify_root(None, window, cx)
        });
    let surface = components::modal_surface(theme)
        .debug_selector(|| "keyboard_shortcuts_dialog".to_string())
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                this.close_and_notify_root(None, window, cx);
            }
        }))
        .w(width)
        .h(height)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(p(14.0))
                .px(p(22.0))
                .pt(p(20.0))
                .pb(p(16.0))
                .child(
                    div()
                        .size(p(42.0))
                        .rounded(p(10.0))
                        .bg(theme.colors.surface.raised)
                        .border_1()
                        .border_color(theme.colors.stroke.subtle)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(theme.ui_text(22.0))
                        .child(if cfg!(target_os = "macos") {
                            "⌘"
                        } else {
                            "Ctrl"
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(p(4.0))
                        .child(
                            div()
                                .text_size(theme.ui_text(20.0))
                                .font_weight(FontWeight::BOLD)
                                .child("Keyboard shortcuts"),
                        )
                        .child(
                            div()
                                .text_size(theme.ui_text(13.0))
                                .text_color(theme.colors.foreground.secondary)
                                .child("Find an action, a key, or a view."),
                        ),
                )
                .child(close),
        )
        .child(
            div()
                .mx(p(22.0))
                .mb(p(18.0))
                .px(p(12.0))
                .h(p(42.0))
                .flex()
                .items_center()
                .gap(p(10.0))
                .border_1()
                .border_color(theme.colors.stroke.subtle)
                .rounded(p(8.0))
                .bg(theme.colors.surface.raised)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(palette.query_input.clone()),
                )
                .child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(format!("{count} shortcuts")),
                ),
        )
        .child(div().h(px(1.0)).w_full().bg(theme.colors.stroke.subtle))
        .child(body)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(p(12.0))
                .px(p(22.0))
                .py(p(12.0))
                .border_t_1()
                .border_color(theme.colors.stroke.subtle)
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(if cfg!(target_os = "macos") {
                    "macOS · Shortcuts are grouped by where they work"
                } else {
                    "Windows / Linux · Shortcuts are grouped by where they work"
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(p(8.0))
                        .child(components::shortcut_keys("Esc", theme, scale))
                        .child("to close"),
                ),
        );
    let scrim = components::modal_scrim(theme).on_pointer_click(
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
                .top(p(32.0))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(surface),
        )
        .into_any_element()
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
                    .any(|entry| entry.keys.iter().any(|value| value.ends_with("+?")))
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
