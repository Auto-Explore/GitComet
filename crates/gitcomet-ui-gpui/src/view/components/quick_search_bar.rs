//! The floating quick search bar: an input slot followed by the newline,
//! match case, whole word and regex toggles, a match status label, optional
//! previous/next match buttons and a close button.
//!
//! The bar is generic over the view that owns it and holds no state of its
//! own. It sets no `key_context`: each caller keeps its own key handling, and
//! positions (and animates) the returned panel itself.

use super::{Button, ButtonStyle, control_height};
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use crate::view::icons::svg_icon;
use crate::view::tooltip::GitCometTooltipExt as _;
use gitcomet_core::text_search::TextSearchOptions;
use gpui::prelude::*;
use gpui::{AnyElement, Context, Div, SharedString, Window, div, px};
use std::rc::Rc;

pub const QUICK_SEARCH_PREV_MATCH_TOOLTIP: &str = "Previous match (F2)";
pub const QUICK_SEARCH_NEXT_MATCH_TOOLTIP: &str = "Next match (F3)";

type BarCallback<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>;
type OptionsCallback<V> = Rc<dyn Fn(&mut V, TextSearchOptions, &mut Window, &mut Context<V>)>;
/// Id suffix, label, tooltip, whether it is on, and how a click flips it.
type OptionToggle = (
    &'static str,
    &'static str,
    &'static str,
    bool,
    fn(&mut TextSearchOptions),
);

/// What the bar's match label reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuickSearchStatus {
    /// The query is empty.
    Empty,
    /// The query is not a valid regular expression.
    InvalidRegex,
    /// Results for the current query have not arrived yet (history find).
    Searching,
    NoMatches,
    /// `current` is the zero-based index of the focused match, if any;
    /// `complete` is false while more matches may still be found.
    Position {
        current: Option<usize>,
        total: usize,
        complete: bool,
    },
    /// The search could not run (history find).
    Failed,
}

impl QuickSearchStatus {
    pub fn label(self) -> SharedString {
        match self {
            Self::Empty => "Type to search".into(),
            Self::InvalidRegex => "Invalid regex".into(),
            Self::Searching => "Searching…".into(),
            Self::NoMatches => "No matches".into(),
            Self::Failed => "Search failed".into(),
            Self::Position {
                current,
                total,
                complete,
            } => {
                let more = if complete { "" } else { "+" };
                match current {
                    Some(ix) => format!("{}/{total}{more}", ix + 1).into(),
                    None if total == 1 && complete => "1 match".into(),
                    None => format!("{total}{more} matches").into(),
                }
            }
        }
    }

    /// Whether the label reads as an error.
    pub fn is_error(self) -> bool {
        matches!(self, Self::InvalidRegex | Self::Failed)
    }
}

struct Navigation<V> {
    can_step: bool,
    on_prev: BarCallback<V>,
    on_next: BarCallback<V>,
}

/// Builder for the quick search bar. Element ids and debug selectors are
/// derived from `id_prefix`: `{prefix}_input_slot`, `{prefix}_newline`,
/// `{prefix}_match_case`, `{prefix}_whole_word`, `{prefix}_regex`,
/// `{prefix}_match_label`, `{prefix}_prev`, `{prefix}_next` and
/// `{prefix}_close`.
pub struct QuickSearchBar<V: 'static> {
    id_prefix: SharedString,
    input: Vec<AnyElement>,
    status: QuickSearchStatus,
    options: Option<(TextSearchOptions, OptionsCallback<V>)>,
    on_newline: Option<BarCallback<V>>,
    navigation: Option<Navigation<V>>,
    on_close: Option<BarCallback<V>>,
    sidebar: bool,
    on_clear: Option<BarCallback<V>>,
}

impl<V: 'static> QuickSearchBar<V> {
    pub fn new(id_prefix: impl Into<SharedString>, status: QuickSearchStatus) -> Self {
        Self {
            id_prefix: id_prefix.into(),
            input: Vec::new(),
            status,
            options: None,
            on_newline: None,
            navigation: None,
            on_close: None,
            sidebar: false,
            on_clear: None,
        }
    }

    /// Adds `element` to the input slot as is, so a caller can wrap its input
    /// in a scroll container or pass a plain single-line input. The slot is
    /// `relative`, so a later element can overlay the input (a scrollbar).
    pub fn input(mut self, element: impl IntoElement) -> Self {
        self.input.push(element.into_any_element());
        self
    }

    /// A full-width, two-row layout for narrow panels. The input owns its
    /// leading icon; options and status share the second row.
    pub fn sidebar(mut self) -> Self {
        self.sidebar = true;
        self
    }

    pub fn on_clear(
        mut self,
        on_clear: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.on_clear = Some(Rc::new(on_clear));
        self
    }

    /// Shows the match case, whole word and regex toggles. A click hands the
    /// options with that toggle flipped to `on_toggle`.
    pub fn options(
        mut self,
        options: TextSearchOptions,
        on_toggle: impl Fn(&mut V, TextSearchOptions, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.options = Some((options, Rc::new(on_toggle)));
        self
    }

    /// Shows the "Insert newline" button.
    pub fn newline(
        mut self,
        on_newline: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.on_newline = Some(Rc::new(on_newline));
        self
    }

    /// Shows previous/next match buttons, disabled unless `can_step`.
    pub fn navigation(
        mut self,
        can_step: bool,
        on_prev: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
        on_next: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.navigation = Some(Navigation {
            can_step,
            on_prev: Rc::new(on_prev),
            on_next: Rc::new(on_next),
        });
        self
    }

    /// Shows the close button.
    pub fn on_close(
        mut self,
        on_close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(on_close));
        self
    }

    pub fn render(self, theme: AppTheme, ui_scale: UiScale, cx: &mut Context<V>) -> Div {
        let Self {
            id_prefix: prefix,
            input,
            status,
            options,
            on_newline,
            navigation,
            on_close,
            sidebar,
            on_clear,
        } = self;
        let id = |suffix: &str| -> SharedString { format!("{prefix}_{suffix}").into() };
        let selector = |suffix: &str| {
            let selector = format!("{prefix}_{suffix}");
            move || selector
        };

        // A floating toolbar: its controls ride the same ramp as the toolbar
        // buttons they mirror.
        let compact_control_height = ui_scale.row_height(26.0, 32.0);
        let compact_icon_button_width = control_height(ui_scale);
        let compact_option_button_width = ui_scale.row_height(24.0, 32.0);
        let option_selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.34 } else { 0.24 },
        );
        let label_color = if status.is_error() {
            theme.colors.status.danger.foreground
        } else {
            theme.colors.foreground.secondary
        };

        let mut input_row = div()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .when(sidebar, |row| row.w_full())
            .child(
                div()
                    .relative()
                    .when(sidebar, |slot| {
                        slot.flex_1().min_w(px(0.0)).py(ui_scale.px(4.0))
                    })
                    .when(!sidebar, |slot| {
                        slot.w(ui_scale.px(220.0)).min_w(ui_scale.px(140.0))
                    })
                    .debug_selector(selector("input_slot"))
                    .children(input),
            );
        if let Some(on_clear) = on_clear {
            input_row = input_row.child(
                Button::new(id("clear"), "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .borderless()
                    .style(ButtonStyle::Subtle)
                    .on_click(theme, cx, move |this, _, window, cx| {
                        on_clear(this, window, cx)
                    })
                    .w(compact_icon_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Clear search".into())
                    .debug_selector(selector("clear")),
            );
        }
        let mut input_row = Some(input_row);
        let sidebar_input = if sidebar { input_row.take() } else { None };
        let mut bar = div()
            .flex()
            .items_start()
            .gap(ui_scale.px(2.0))
            .when(sidebar, |bar| bar.w_full().min_w(px(0.0)))
            .when(!sidebar, |bar| {
                bar.px(ui_scale.px(4.0))
                    .py(ui_scale.px(2.0))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.raised)
                    .shadow(crate::theme::shadow_surface(theme))
            });
        if let Some(input) = input_row {
            bar = bar.child(input);
        }

        if let Some(on_newline) = on_newline {
            bar = bar.child(
                Button::new(id("newline"), "")
                    .start_slot(svg_icon(
                        "icons/line_break.svg",
                        theme.colors.foreground.primary,
                        ui_scale.px(14.0),
                    ))
                    .borderless()
                    .style(ButtonStyle::Subtle)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        on_newline(this, window, cx);
                    })
                    .w(compact_icon_button_width)
                    .h(compact_control_height)
                    .gitcomet_tooltip(theme, "Insert newline (Shift+Enter)".into())
                    .debug_selector(selector("newline")),
            );
        }

        if let Some((options, on_toggle)) = options {
            let toggles: [OptionToggle; 3] = [
                ("match_case", "Aa", "Match case", options.match_case, |o| {
                    o.match_case = !o.match_case
                }),
                (
                    "whole_word",
                    "W",
                    "Match whole word",
                    options.whole_word,
                    |o| o.whole_word = !o.whole_word,
                ),
                (
                    "regex",
                    ".*",
                    "Use regular expression",
                    options.regex,
                    |o| o.regex = !o.regex,
                ),
            ];
            for (suffix, label, tooltip, selected, flip) in toggles {
                let on_toggle = on_toggle.clone();
                bar = bar.child(
                    Button::new(id(suffix), label)
                        .borderless()
                        .style(ButtonStyle::Subtle)
                        .selected(selected)
                        .selected_bg(option_selected_bg)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            let mut next = options;
                            flip(&mut next);
                            on_toggle(this, next, window, cx);
                        })
                        .w(compact_option_button_width)
                        .h(compact_control_height)
                        .gitcomet_tooltip(theme, tooltip.into())
                        .debug_selector(selector(suffix)),
                );
            }
        }

        bar = bar.child(
            div()
                .when(sidebar, |label| label.flex_1().min_w(px(0.0)))
                .when(!sidebar, |label| {
                    label
                        .w(ui_scale.px(104.0))
                        .min_w(ui_scale.px(104.0))
                        .max_w(ui_scale.px(104.0))
                })
                .h(compact_control_height)
                .flex()
                .items_center()
                .justify_end()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(theme.ui_text(12.0))
                .text_color(label_color)
                .debug_selector(selector("match_label"))
                .child(status.label()),
        );

        if let Some(Navigation {
            can_step,
            on_prev,
            on_next,
        }) = navigation
        {
            for (suffix, icon, tooltip, on_step) in [
                (
                    "prev",
                    "icons/arrow_up.svg",
                    QUICK_SEARCH_PREV_MATCH_TOOLTIP,
                    on_prev,
                ),
                (
                    "next",
                    "icons/arrow_down.svg",
                    QUICK_SEARCH_NEXT_MATCH_TOOLTIP,
                    on_next,
                ),
            ] {
                bar = bar.child(
                    Button::new(id(suffix), "")
                        .start_slot(svg_icon(
                            icon,
                            theme.colors.foreground.primary,
                            ui_scale.px(14.0),
                        ))
                        .borderless()
                        .style(ButtonStyle::Subtle)
                        .disabled(!can_step)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            on_step(this, window, cx);
                        })
                        .w(compact_icon_button_width)
                        .h(compact_control_height)
                        .gitcomet_tooltip(theme, tooltip.into())
                        .debug_selector(selector(suffix)),
                );
            }
        }

        if let Some(on_close) = on_close {
            bar = bar.child(
                Button::new(id("close"), "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(ButtonStyle::Transparent)
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        on_close(this, window, cx);
                    })
                    .w(compact_icon_button_width)
                    .h(compact_control_height)
                    .debug_selector(selector("close")),
            );
        }

        if let Some(input) = sidebar_input {
            div()
                .flex()
                .flex_col()
                .w_full()
                .min_w(px(0.0))
                .px(ui_scale.px(4.0))
                .py(ui_scale.px(2.0))
                .rounded(px(theme.radii.control))
                .border_1()
                .border_color(if status.is_error() {
                    theme.colors.status.danger.foreground
                } else {
                    theme.colors.stroke.default
                })
                .bg(theme.colors.surface.raised)
                .child(input)
                .child(bar)
        } else {
            bar
        }
    }
}

#[cfg(test)]
mod tests {
    use super::QuickSearchStatus;

    #[test]
    fn quick_search_status_labels() {
        let label = |status: QuickSearchStatus| status.label().to_string();
        assert_eq!(label(QuickSearchStatus::Empty), "Type to search");
        assert_eq!(label(QuickSearchStatus::InvalidRegex), "Invalid regex");
        assert_eq!(label(QuickSearchStatus::Searching), "Searching…");
        assert_eq!(label(QuickSearchStatus::NoMatches), "No matches");
        assert_eq!(label(QuickSearchStatus::Failed), "Search failed");
        let position = |current, total, complete| {
            label(QuickSearchStatus::Position {
                current,
                total,
                complete,
            })
        };
        assert_eq!(position(Some(2), 10, true), "3/10");
        assert_eq!(position(Some(0), 10, false), "1/10+");
        assert_eq!(position(None, 10, true), "10 matches");
        assert_eq!(position(None, 10, false), "10+ matches");
        assert_eq!(position(None, 1, true), "1 match");
    }

    #[test]
    fn quick_search_status_errors() {
        assert!(QuickSearchStatus::InvalidRegex.is_error());
        assert!(QuickSearchStatus::Failed.is_error());
        assert!(!QuickSearchStatus::NoMatches.is_error());
        assert!(!QuickSearchStatus::Searching.is_error());
    }
}
