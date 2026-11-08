//! The quick search bar every searchable pane shares: a search icon, the
//! input, the match case / whole word / regex toggles, the match label,
//! previous/next buttons and a close button, all on one row.
//!
//! The bar holds no query state; the owning view keeps its input and
//! matching. What it does keep, in a [`QuickSearchState`] the owner stores,
//! is its own layout: the width it was last given and the overflow menu. As
//! the width shrinks the bar sheds controls in a fixed order (see
//! [`QuickSearchTier`]); the toggles and stepping move into a "⋮" menu
//! rather than disappearing.

use super::{Button, ButtonStyle, ContextMenuEntry, ContextMenuIconSlot, control_height};
use crate::icons::svg_icon;
use crate::theme::{AppTheme, with_alpha};
use crate::tooltip::GitCometTooltipExt as _;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, Context, Div, Pixels, SharedString, Window, anchored, canvas, deferred,
    div, point, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// The bar's match case, whole word and regex toggles.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SearchToggles {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

pub const QUICK_SEARCH_PREV_MATCH_TOOLTIP: &str = "Previous match";
pub const QUICK_SEARCH_NEXT_MATCH_TOOLTIP: &str = "Next match";
pub const QUICK_SEARCH_CLOSE_TOOLTIP: &str = "Close search";

type BarCallback<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>;
type OptionsCallback<V> = Rc<dyn Fn(&mut V, SearchToggles, &mut Window, &mut Context<V>)>;

/// What the bar's match label reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuickSearchStatus {
    /// The query is empty; the label shows nothing.
    Empty,
    /// The query is not a valid regular expression.
    InvalidRegex,
    /// Results for the current query have not arrived yet.
    Searching,
    NoMatches,
    /// `current` is the zero-based index of the focused match, if any;
    /// `complete` is false while more matches may still be found.
    Position {
        current: Option<usize>,
        total: usize,
        complete: bool,
    },
    /// The search could not run.
    Failed,
}

impl QuickSearchStatus {
    pub fn label(self) -> SharedString {
        match self {
            Self::Empty => SharedString::default(),
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

/// How much of the bar fits, widest first. Each step down hides one more
/// group: first the match label and previous/next buttons, then the toggles
/// (into the "⋮" menu, with stepping), then the search icon.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum QuickSearchTier {
    #[default]
    Full,
    NoNavigation,
    Overflow,
    Minimal,
}

impl QuickSearchTier {
    fn shows_navigation(self) -> bool {
        self == Self::Full
    }

    fn shows_toggles(self) -> bool {
        self <= Self::NoNavigation
    }

    fn shows_icon(self) -> bool {
        self <= Self::Overflow
    }
}

/// Which optional controls a bar carries; its tier widths depend on them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QuickSearchFeatures {
    pub toggles: bool,
    pub newline: bool,
    pub navigation: bool,
}

/// Fixed widths, in design pixels at 100%, before the ui scale.
const PAD_X_PX: f32 = 4.0;
const GAP_PX: f32 = 2.0;
const ICON_SLOT_PX: f32 = 22.0;
const LABEL_PX: f32 = 72.0;
/// The narrowest input any tier but [`QuickSearchTier::Minimal`] accepts.
const MIN_INPUT_PX: f32 = 120.0;
/// The input's width when a floating bar has room to spare.
const FLOATING_INPUT_PX: f32 = 220.0;
/// Width a tier must clear by before the bar grows back into it, so a width
/// right on a boundary does not flip tiers each frame.
const HYSTERESIS_PX: f32 = 8.0;

struct Widths {
    icon_button: Pixels,
    option_button: Pixels,
    row: Pixels,
}

fn widths(ui_scale: UiScale) -> Widths {
    Widths {
        icon_button: control_height(ui_scale),
        option_button: ui_scale.row_height(24.0, 32.0),
        row: ui_scale.row_height(26.0, 32.0),
    }
}

/// The height of the bar's control row. A multi-line input pads itself by
/// `(row - line height) / 2` so its first line sits on that row's centre.
pub fn quick_search_row_height(ui_scale: UiScale) -> Pixels {
    widths(ui_scale).row
}

/// The width a bar in `tier` needs with an input `input_px` wide: padding,
/// border, gaps and every control the tier shows. `None` for a tier the
/// features make identical to a wider one.
fn tier_width(
    tier: QuickSearchTier,
    features: QuickSearchFeatures,
    input_px: f32,
    ui_scale: UiScale,
) -> Option<Pixels> {
    let w = widths(ui_scale);
    let mut controls: Vec<Pixels> = Vec::new();
    match tier {
        QuickSearchTier::Full | QuickSearchTier::NoNavigation => {
            if tier == QuickSearchTier::Full && !features.navigation {
                return None;
            }
            if features.newline {
                controls.push(w.icon_button);
            }
            if features.toggles {
                controls.extend([w.option_button; 3]);
            }
            if tier == QuickSearchTier::Full {
                controls.push(ui_scale.px(LABEL_PX));
                controls.extend([w.icon_button; 2]);
            }
        }
        QuickSearchTier::Overflow | QuickSearchTier::Minimal => {
            if features.toggles || features.newline || features.navigation {
                controls.push(w.icon_button);
            }
        }
    }
    // The close button.
    controls.push(w.icon_button);
    let mut width = ui_scale.px(PAD_X_PX * 2.0) + px(2.0) + ui_scale.px(input_px);
    if tier.shows_icon() {
        width += ui_scale.px(ICON_SLOT_PX);
    }
    for control in &controls {
        width += *control + ui_scale.px(GAP_PX);
    }
    Some(width)
}

/// The widest tier that fits `width`. `current` is the tier now shown: growing
/// into a wider tier needs [`HYSTERESIS_PX`] of slack.
pub fn quick_search_tier_for_width(
    width: Pixels,
    features: QuickSearchFeatures,
    current: QuickSearchTier,
    ui_scale: UiScale,
) -> QuickSearchTier {
    for tier in [
        QuickSearchTier::Full,
        QuickSearchTier::NoNavigation,
        QuickSearchTier::Overflow,
    ] {
        let Some(needed) = tier_width(tier, features, MIN_INPUT_PX, ui_scale) else {
            continue;
        };
        let slack = if tier < current {
            ui_scale.px(HYSTERESIS_PX)
        } else {
            px(0.0)
        };
        if width >= needed + slack {
            return tier;
        }
    }
    QuickSearchTier::Minimal
}

/// The bar's own layout state. The owner keeps one per bar (an
/// `Rc<QuickSearchState>`) for as long as the bar can be shown.
#[derive(Debug, Default)]
pub struct QuickSearchState {
    tier: Cell<QuickSearchTier>,
    width: Cell<Option<Pixels>>,
    menu: Cell<Option<Bounds<Pixels>>>,
}

impl QuickSearchState {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    pub fn tier(&self) -> QuickSearchTier {
        self.tier.get()
    }

    pub fn menu_open(&self) -> bool {
        self.menu.get().is_some()
    }

    /// Closes the "⋮" menu. Returns whether it was open, so an Escape handler
    /// can close the menu before the bar.
    pub fn dismiss_menu(&self) -> bool {
        self.menu.take().is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_tier_for_test(&self, tier: QuickSearchTier) {
        self.tier.set(tier);
    }
}

struct Navigation<V> {
    can_step: bool,
    on_prev: BarCallback<V>,
    on_next: BarCallback<V>,
}

/// Builder for the quick search bar. Element ids and debug selectors are
/// derived from `id_prefix`: `{prefix}_input_slot`, `{prefix}_icon`,
/// `{prefix}_newline`, `{prefix}_match_case`, `{prefix}_whole_word`,
/// `{prefix}_regex`, `{prefix}_match_label`, `{prefix}_prev`,
/// `{prefix}_next`, `{prefix}_more`, `{prefix}_menu` and `{prefix}_close`.
pub struct QuickSearchBar<V: 'static> {
    id_prefix: SharedString,
    state: Rc<QuickSearchState>,
    input: Vec<AnyElement>,
    status: QuickSearchStatus,
    options: Option<(SearchToggles, OptionsCallback<V>)>,
    on_newline: Option<BarCallback<V>>,
    navigation: Option<Navigation<V>>,
    on_close: Option<BarCallback<V>>,
    floating: bool,
}

impl<V: 'static> QuickSearchBar<V> {
    pub fn new(
        id_prefix: impl Into<SharedString>,
        state: &Rc<QuickSearchState>,
        status: QuickSearchStatus,
    ) -> Self {
        Self {
            id_prefix: id_prefix.into(),
            state: Rc::clone(state),
            input: Vec::new(),
            status,
            options: None,
            on_newline: None,
            navigation: None,
            on_close: None,
            floating: false,
        }
    }

    /// Adds `element` to the input slot as is, so a caller can wrap its input
    /// in a scroll container. The slot is `relative`, so a later element can
    /// overlay the input (a scrollbar). The input should carry no leading
    /// icon: the bar draws the search icon itself while it has room.
    pub fn input(mut self, element: impl IntoElement) -> Self {
        self.input.push(element.into_any_element());
        self
    }

    /// A bar that floats over content: it keeps a natural width and sits at
    /// the right of the space it is given. Without this the bar fills it.
    pub fn floating(mut self) -> Self {
        self.floating = true;
        self
    }

    /// Shows the match case, whole word and regex toggles. A click hands the
    /// options with that toggle flipped to `on_toggle`.
    pub fn options(
        mut self,
        options: SearchToggles,
        on_toggle: impl Fn(&mut V, SearchToggles, &mut Window, &mut Context<V>) + 'static,
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

    fn features(&self) -> QuickSearchFeatures {
        QuickSearchFeatures {
            toggles: self.options.is_some(),
            newline: self.on_newline.is_some(),
            navigation: self.navigation.is_some(),
        }
    }

    /// The bar inside a full-width slot that measures the width it is given.
    /// Place the returned element where the bar's space is: for a floating
    /// bar that is the whole width it may grow to.
    pub fn render(self, theme: AppTheme, ui_scale: UiScale, cx: &mut Context<V>) -> Div {
        let features = self.features();
        let tier = self.state.tier();
        let state = Rc::clone(&self.state);
        let owner = cx.entity_id();
        let floating = self.floating;
        let natural_width = floating
            .then(|| {
                [QuickSearchTier::Full, QuickSearchTier::NoNavigation]
                    .into_iter()
                    .find_map(|tier| tier_width(tier, features, FLOATING_INPUT_PX, ui_scale))
            })
            .flatten();
        let prefix = self.id_prefix.clone();
        let bar = self.render_bar(tier, theme, ui_scale, cx);
        let bar = match natural_width {
            Some(width) => bar.w(width).max_w_full(),
            None => bar.w_full(),
        };

        div()
            .relative()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .when(floating, |slot| slot.justify_end())
            .debug_selector(move || format!("{prefix}_slot"))
            // Measure the slot, not the bar: a floating bar is only as wide
            // as its tier lets it be.
            .child(
                canvas(
                    move |bounds, _window, cx| {
                        let width = bounds.size.width;
                        if state.width.replace(Some(width)) == Some(width) {
                            return;
                        }
                        let next =
                            quick_search_tier_for_width(width, features, state.tier(), ui_scale);
                        if next != state.tier() {
                            state.tier.set(next);
                            if next.shows_toggles() {
                                state.menu.set(None);
                            }
                            // Notifying while drawing is deferred to the next
                            // frame, which only a later frame would draw.
                            cx.defer(move |cx| cx.notify(owner));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(bar)
    }

    fn render_bar(
        self,
        tier: QuickSearchTier,
        theme: AppTheme,
        ui_scale: UiScale,
        cx: &mut Context<V>,
    ) -> Div {
        let Self {
            id_prefix: prefix,
            state,
            input,
            status,
            options,
            on_newline,
            navigation,
            on_close,
            floating: _,
        } = self;
        let id = |suffix: &str| -> SharedString { format!("{prefix}_{suffix}").into() };
        let selector = |suffix: &str| {
            let selector = format!("{prefix}_{suffix}");
            move || selector
        };

        let w = widths(ui_scale);
        let option_selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.34 } else { 0.24 },
        );
        let label_color = if status.is_error() {
            theme.colors.status.danger.foreground
        } else {
            theme.colors.foreground.secondary
        };
        let icon_button = |suffix: &str, icon: &'static str, color| {
            Button::new(id(suffix), "")
                .start_slot(svg_icon(icon, color, ui_scale.px(14.0)))
                .borderless()
                .style(ButtonStyle::Subtle)
        };

        // Multi-line input grows downwards; the controls stay on the first row.
        // The bar occludes so a floating one does not click through.
        let mut bar = div()
            .occlude()
            .flex()
            .items_start()
            .min_w(px(0.0))
            .gap(ui_scale.px(GAP_PX))
            .px(ui_scale.px(PAD_X_PX))
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(if status.is_error() {
                theme.colors.status.danger.foreground
            } else {
                theme.colors.stroke.default
            })
            .bg(theme.colors.surface.raised);

        if tier.shows_icon() {
            bar = bar.child(
                div()
                    .id(id("icon"))
                    .debug_selector(selector("icon"))
                    .flex_none()
                    .w(ui_scale.px(ICON_SLOT_PX))
                    .h(w.row)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(svg_icon(
                        "icons/zoom.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(13.0),
                    ))
                    .gitcomet_tooltip(theme, "Search: type to find or filter".into()),
            );
        }

        bar = bar.child(
            div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                // One control row tall at least, the input centred on it.
                .min_h(w.row)
                .flex()
                .flex_col()
                .justify_center()
                .debug_selector(selector("input_slot"))
                .children(input),
        );

        if tier.shows_toggles() {
            if let Some(on_newline) = on_newline.clone() {
                bar = bar.child(
                    icon_button(
                        "newline",
                        "icons/line_break.svg",
                        theme.colors.foreground.primary,
                    )
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        on_newline(this, window, cx);
                    })
                    .flex_none()
                    .w(w.icon_button)
                    .h(w.row)
                    .gitcomet_tooltip_keyed(
                        theme,
                        "Insert a newline: search across several lines".into(),
                        vec!["Shift+Enter".into()],
                    )
                    .debug_selector(selector("newline")),
                );
            }
            if let Some((options, on_toggle)) = options.clone() {
                for toggle in TOGGLES {
                    let on_toggle = on_toggle.clone();
                    bar = bar.child(
                        Button::new(id(toggle.suffix), toggle.label)
                            .borderless()
                            .style(ButtonStyle::Subtle)
                            .selected((toggle.get)(options))
                            .selected_bg(option_selected_bg)
                            .on_click(theme, cx, move |this, _e, window, cx| {
                                let mut next = options;
                                (toggle.flip)(&mut next);
                                on_toggle(this, next, window, cx);
                            })
                            .flex_none()
                            .w(w.option_button)
                            .h(w.row)
                            .gitcomet_tooltip(theme, toggle.tooltip.into())
                            .debug_selector(selector(toggle.suffix)),
                    );
                }
            }
        }

        if tier.shows_navigation()
            && let Some(nav) = navigation.as_ref()
        {
            bar = bar.child(
                div()
                    .flex_none()
                    .w(ui_scale.px(LABEL_PX))
                    .h(w.row)
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
            for (suffix, icon, tooltip, key, on_step) in [
                (
                    "prev",
                    "icons/arrow_up.svg",
                    QUICK_SEARCH_PREV_MATCH_TOOLTIP,
                    "F2",
                    nav.on_prev.clone(),
                ),
                (
                    "next",
                    "icons/arrow_down.svg",
                    QUICK_SEARCH_NEXT_MATCH_TOOLTIP,
                    "F3",
                    nav.on_next.clone(),
                ),
            ] {
                bar = bar.child(
                    icon_button(suffix, icon, theme.colors.foreground.primary)
                        .disabled(!nav.can_step)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            on_step(this, window, cx);
                        })
                        .flex_none()
                        .w(w.icon_button)
                        .h(w.row)
                        .gitcomet_tooltip_keyed(theme, tooltip.into(), vec![key.into()])
                        .debug_selector(selector(suffix)),
                );
            }
        }

        let has_menu = !tier.shows_toggles()
            && (options.is_some() || on_newline.is_some() || navigation.is_some());
        let mut menu = None;
        if has_menu {
            let open = state.menu_open();
            let toggle_state = Rc::clone(&state);
            bar = bar.child(
                icon_button(
                    "more",
                    "icons/more_vertical.svg",
                    theme.colors.foreground.secondary,
                )
                .open(open)
                .on_click_with_bounds(theme, cx, move |_this, _e, bounds, _window, cx| {
                    if toggle_state.menu.take().is_none() {
                        toggle_state.menu.set(Some(bounds));
                    }
                    cx.notify();
                })
                .flex_none()
                .w(w.icon_button)
                .h(w.row)
                .gitcomet_tooltip(theme, "More search options".into())
                .debug_selector(selector("more")),
            );
            if let Some(anchor) = state.menu.get() {
                menu = Some(render_menu(
                    MenuParts {
                        prefix: prefix.clone(),
                        state: Rc::clone(&state),
                        status,
                        options,
                        on_newline,
                        navigation,
                    },
                    anchor,
                    theme,
                    ui_scale,
                    cx,
                ));
            }
        }

        if let Some(on_close) = on_close {
            bar = bar.child(
                icon_button(
                    "close",
                    "icons/generic_close.svg",
                    theme.colors.foreground.secondary,
                )
                .style(ButtonStyle::Transparent)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    on_close(this, window, cx);
                })
                .flex_none()
                .w(w.icon_button)
                .h(w.row)
                .gitcomet_tooltip_keyed(
                    theme,
                    QUICK_SEARCH_CLOSE_TOOLTIP.into(),
                    vec!["Esc".into()],
                )
                .debug_selector(selector("close")),
            );
        }

        bar.children(menu)
    }
}

/// One of the three option toggles, shared by the bar and its menu.
#[derive(Clone, Copy)]
struct Toggle {
    suffix: &'static str,
    label: &'static str,
    menu_label: &'static str,
    tooltip: &'static str,
    get: fn(SearchToggles) -> bool,
    flip: fn(&mut SearchToggles),
}

const TOGGLES: [Toggle; 3] = [
    Toggle {
        suffix: "match_case",
        label: "Aa",
        menu_label: "Match case",
        tooltip: "Match case: only find text with the same upper and lower case",
        get: |o| o.match_case,
        flip: |o| o.match_case = !o.match_case,
    },
    Toggle {
        suffix: "whole_word",
        label: "W",
        menu_label: "Whole word",
        tooltip: "Whole word: only find complete words, not parts of words",
        get: |o| o.whole_word,
        flip: |o| o.whole_word = !o.whole_word,
    },
    Toggle {
        suffix: "regex",
        label: ".*",
        menu_label: "Regular expression",
        tooltip: "Regular expression: read the query as a regex pattern",
        get: |o| o.regex,
        flip: |o| o.regex = !o.regex,
    },
];

struct MenuParts<V: 'static> {
    prefix: SharedString,
    state: Rc<QuickSearchState>,
    status: QuickSearchStatus,
    options: Option<(SearchToggles, OptionsCallback<V>)>,
    on_newline: Option<BarCallback<V>>,
    navigation: Option<Navigation<V>>,
}

/// The "⋮" menu: what the narrow bar hid, under the button that opened it.
fn render_menu<V: 'static>(
    parts: MenuParts<V>,
    anchor: Bounds<Pixels>,
    theme: AppTheme,
    ui_scale: UiScale,
    cx: &mut Context<V>,
) -> AnyElement {
    let MenuParts {
        prefix,
        state,
        status,
        options,
        on_newline,
        navigation,
    } = parts;
    let id = |suffix: &str| -> SharedString { format!("{prefix}_menu_{suffix}").into() };
    let mut items: Vec<AnyElement> = Vec::new();

    let label = status.label();
    if !label.is_empty() {
        items.push(
            div()
                .px(ui_scale.px(8.0))
                .py(ui_scale.px(4.0))
                .text_size(theme.ui_text(12.0))
                .text_color(if status.is_error() {
                    theme.colors.status.danger.foreground
                } else {
                    theme.colors.foreground.secondary
                })
                .debug_selector({
                    let prefix = prefix.clone();
                    move || format!("{prefix}_menu_label")
                })
                .child(label)
                .into_any_element(),
        );
    }

    if let Some((options, on_toggle)) = options {
        for toggle in TOGGLES {
            let on_toggle = on_toggle.clone();
            let checked = (toggle.get)(options);
            items.push(
                ContextMenuEntry::new(id(toggle.suffix), toggle.menu_label)
                    .icon(if checked {
                        ContextMenuIconSlot::Icon("icons/check.svg".into())
                    } else {
                        ContextMenuIconSlot::Reserved
                    })
                    .on_select(theme, ui_scale, cx, move |this, _, window, cx| {
                        let mut next = options;
                        (toggle.flip)(&mut next);
                        on_toggle(this, next, window, cx);
                    })
                    .gitcomet_tooltip(theme, toggle.tooltip.into())
                    .into_any_element(),
            );
        }
    }
    if let Some(on_newline) = on_newline {
        let close = Rc::clone(&state);
        items.push(
            ContextMenuEntry::new(id("newline"), "Insert newline")
                .icon(ContextMenuIconSlot::Icon("icons/line_break.svg".into()))
                .shortcut(Some("Shift+Enter".into()))
                .on_select(theme, ui_scale, cx, move |this, _, window, cx| {
                    close.menu.set(None);
                    on_newline(this, window, cx);
                })
                .gitcomet_tooltip(
                    theme,
                    "Insert a newline: search across several lines".into(),
                )
                .into_any_element(),
        );
    }
    if let Some(nav) = navigation {
        for (suffix, icon, label, key, on_step) in [
            (
                "prev",
                "icons/arrow_up.svg",
                QUICK_SEARCH_PREV_MATCH_TOOLTIP,
                "F2",
                nav.on_prev,
            ),
            (
                "next",
                "icons/arrow_down.svg",
                QUICK_SEARCH_NEXT_MATCH_TOOLTIP,
                "F3",
                nav.on_next,
            ),
        ] {
            items.push(
                ContextMenuEntry::new(id(suffix), label)
                    .icon(ContextMenuIconSlot::Icon(icon.into()))
                    .shortcut(Some(key.into()))
                    .disabled(!nav.can_step)
                    .on_select(theme, ui_scale, cx, move |this, _, window, cx| {
                        on_step(this, window, cx);
                    })
                    .into_any_element(),
            );
        }
    }

    let dismiss = Rc::clone(&state);
    deferred(
        anchored()
            .position(point(anchor.right(), anchor.bottom() + ui_scale.px(4.0)))
            .anchor(gpui::Anchor::TopRight)
            .snap_to_window_with_margin(ui_scale.px(8.0))
            .child(
                super::popover_surface(theme)
                    .id(SharedString::from(format!("{prefix}_menu")))
                    .debug_selector({
                        let prefix = prefix.clone();
                        move || format!("{prefix}_menu")
                    })
                    .min_w(ui_scale.px(200.0))
                    .p(ui_scale.px(4.0))
                    .flex()
                    .flex_col()
                    .occlude()
                    .on_mouse_down_out(cx.listener(move |_this, _e, _window, cx| {
                        if dismiss.dismiss_menu() {
                            cx.notify();
                        }
                    }))
                    .children(items),
            ),
    )
    .priority(1)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_search_status_labels() {
        let label = |status: QuickSearchStatus| status.label().to_string();
        assert_eq!(label(QuickSearchStatus::Empty), "");
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

    const ALL: QuickSearchFeatures = QuickSearchFeatures {
        toggles: true,
        newline: false,
        navigation: true,
    };

    fn needed(tier: QuickSearchTier, features: QuickSearchFeatures, scale: UiScale) -> Pixels {
        tier_width(tier, features, MIN_INPUT_PX, scale).expect("tier exists")
    }

    #[test]
    fn tiers_collapse_navigation_then_toggles_then_icon() {
        for percent in [100, 150] {
            let scale = UiScale::from_percent(percent);
            let full = needed(QuickSearchTier::Full, ALL, scale);
            let no_nav = needed(QuickSearchTier::NoNavigation, ALL, scale);
            let overflow = needed(QuickSearchTier::Overflow, ALL, scale);
            assert!(full > no_nav && no_nav > overflow, "{percent}%");
            let tier =
                |width| quick_search_tier_for_width(width, ALL, QuickSearchTier::Minimal, scale);
            // Growing needs the hysteresis slack on top of the tier width.
            let slack = scale.px(HYSTERESIS_PX);
            assert_eq!(tier(full + slack), QuickSearchTier::Full);
            assert_eq!(tier(full), QuickSearchTier::NoNavigation);
            assert_eq!(tier(no_nav + slack), QuickSearchTier::NoNavigation);
            assert_eq!(tier(overflow + slack), QuickSearchTier::Overflow);
            assert_eq!(tier(overflow), QuickSearchTier::Minimal);
            assert!(!QuickSearchTier::Minimal.shows_icon());
            assert!(QuickSearchTier::Overflow.shows_icon());
            assert!(!QuickSearchTier::Overflow.shows_toggles());
            assert!(!QuickSearchTier::NoNavigation.shows_navigation());
        }
    }

    #[test]
    fn shrinking_needs_no_slack_but_growing_does() {
        let scale = UiScale::from_percent(100);
        let full = needed(QuickSearchTier::Full, ALL, scale);
        // Already Full: staying needs only the tier width.
        assert_eq!(
            quick_search_tier_for_width(full, ALL, QuickSearchTier::Full, scale),
            QuickSearchTier::Full
        );
        assert_eq!(
            quick_search_tier_for_width(full - px(1.0), ALL, QuickSearchTier::Full, scale),
            QuickSearchTier::NoNavigation
        );
        // Coming from NoNavigation: the exact width is not enough.
        assert_eq!(
            quick_search_tier_for_width(full, ALL, QuickSearchTier::NoNavigation, scale),
            QuickSearchTier::NoNavigation
        );
    }

    #[test]
    fn a_bar_without_navigation_skips_the_full_tier() {
        let scale = UiScale::from_percent(100);
        let filter = QuickSearchFeatures {
            navigation: false,
            ..ALL
        };
        assert_eq!(
            quick_search_tier_for_width(px(10_000.0), filter, QuickSearchTier::Full, scale),
            QuickSearchTier::NoNavigation
        );
    }
}
