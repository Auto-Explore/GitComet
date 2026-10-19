use crate::theme::AppTheme;
use crate::tooltip_host::TooltipHost;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, Rgba, SharedString, Stateful, WeakEntity, div, px};

use super::control_height_md;
use super::{TextTruncationProfile, TruncatedText, TruncatedTextTooltipMode, shortcut_keys};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextMenuText {
    text: SharedString,
    profile: TextTruncationProfile,
    max_lines: Option<usize>,
    tooltip_mode: TruncatedTextTooltipMode,
}

impl ContextMenuText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            profile: TextTruncationProfile::End,
            max_lines: None,
            tooltip_mode: TruncatedTextTooltipMode::None,
        }
    }

    pub fn path_single_line(text: impl Into<SharedString>) -> Self {
        Self::new(text)
            .profile(TextTruncationProfile::Path)
            .max_lines(1)
            .tooltip_mode(TruncatedTextTooltipMode::FullTextIfTruncated)
    }

    pub fn profile(mut self, profile: TextTruncationProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines.max(1));
        self
    }

    pub fn tooltip_mode(mut self, tooltip_mode: TruncatedTextTooltipMode) -> Self {
        self.tooltip_mode = tooltip_mode;
        self
    }

    fn resolved_max_lines(&self, default: usize) -> usize {
        self.max_lines.unwrap_or(default).max(1)
    }
}

impl AsRef<str> for ContextMenuText {
    fn as_ref(&self) -> &str {
        self.text.as_ref()
    }
}

impl From<SharedString> for ContextMenuText {
    fn from(value: SharedString) -> Self {
        Self::new(value)
    }
}

impl From<String> for ContextMenuText {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ContextMenuText {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

/// What occupies the fixed-width icon slot at the start of a menu entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ContextMenuIconSlot {
    /// No icon and no reserved space; the label starts at the row edge.
    #[default]
    None,
    /// Keep the icon column empty so the label stays aligned with sibling
    /// entries that carry an icon.
    Reserved,
    Icon(SharedString),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContextMenuTone {
    #[default]
    Accent,
    Neutral,
    Success,
    Warning,
    Danger,
}

pub fn context_menu(theme: AppTheme, content: impl IntoElement) -> Div {
    div()
        .w_full()
        .min_w_full()
        .flex()
        .flex_col()
        .items_stretch()
        .text_color(theme.colors.foreground.primary)
        .child(content)
}

/// Type scale of a menu's heading block, mirroring a worktree-picker row: the
/// primary line names the object at the same size as the entries below it, and
/// the secondary line under it is smaller and muted.
///
/// Both sizes have to be handed to [`TruncatedText`] explicitly. A `.text_size(theme.ui_text(12.0))`
/// on the wrapping div never reaches it — it measures and shapes its line from
/// `window.text_style()` inside a deferred measure closure, after the ancestor
/// text style has been unwound — so a single-line heading takes the window
/// default instead. That is what made the two lines render at the same size.
const MENU_PRIMARY_REMS: f32 = 0.875;
const MENU_SECONDARY_REMS: f32 = 0.75;

/// The menu's primary heading: what the menu is about.
pub fn context_menu_header<V: 'static>(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    title: impl Into<ContextMenuText>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let title = title.into();
    let max_lines = title.resolved_max_lines(1);
    div()
        .px(scaled_px(8.0))
        // At least an entry-sized box, not padding around a text run:
        // `TruncatedText` reserves its own line box. A minimum rather than a
        // fixed height so a multi-line heading grows instead of clipping.
        .min_h(control_height_md(ui_scale))
        .flex()
        .items_center()
        .overflow_hidden()
        .child(
            div()
                .debug_selector(|| "context_menu_header_text".to_string())
                .min_w(px(0.0))
                .overflow_hidden()
                .text_size(theme.ui_text(MENU_PRIMARY_REMS * 16.0))
                .line_height(ui_scale.ui_text(18.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.colors.foreground.primary)
                .when(max_lines == 1, |s| s.whitespace_nowrap())
                .when(max_lines > 1, |s| s.line_clamp(max_lines))
                .child(context_menu_text_content(
                    title,
                    tooltip_host,
                    cx,
                    max_lines,
                    theme.colors.foreground.primary,
                    theme.ui_text(MENU_PRIMARY_REMS * 16.0),
                    Some(gpui::FontWeight::MEDIUM),
                )),
        )
}

/// Muted helper text under a menu header — explains what the options do,
/// visually subordinate to both the header and the entries.
pub fn context_menu_description<V: 'static>(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    text: impl Into<ContextMenuText>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let text = text.into();
    let max_lines = text.resolved_max_lines(3);
    div()
        .px(scaled_px(8.0))
        .pb(scaled_px(4.0))
        .text_size(theme.ui_text(MENU_SECONDARY_REMS * 16.0))
        .line_height(ui_scale.ui_text(14.0))
        .text_color(theme.colors.foreground.secondary)
        .when(max_lines == 1, |s| s.whitespace_nowrap().overflow_hidden())
        .when(max_lines > 1, |s| s.line_clamp(max_lines))
        .child(context_menu_text_content(
            text,
            tooltip_host,
            cx,
            max_lines,
            theme.colors.foreground.secondary,
            theme.ui_text(MENU_SECONDARY_REMS * 16.0),
            None,
        ))
}

/// The menu's secondary heading — the line under the header that says *which*
/// object the menu acts on (a file's full path, a worktree's location, a link's
/// URL), plus the empty/loading lines that stand in for a missing list. Smaller
/// and muted against the primary heading, the way a picker row's detail line
/// reads against its title.
pub fn context_menu_label<V: 'static>(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    text: impl Into<ContextMenuText>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let text = text.into();
    let max_lines = text.resolved_max_lines(2);
    div()
        .px(scaled_px(8.0))
        .pb(scaled_px(4.0))
        .text_size(theme.ui_text(MENU_SECONDARY_REMS * 16.0))
        .line_height(ui_scale.ui_text(14.0))
        .text_color(theme.colors.foreground.secondary)
        .when(max_lines == 1, |s| s.whitespace_nowrap().overflow_hidden())
        .when(max_lines > 1, |s| s.line_clamp(max_lines))
        .child(context_menu_text_content(
            text,
            tooltip_host,
            cx,
            max_lines,
            theme.colors.foreground.secondary,
            theme.ui_text(MENU_SECONDARY_REMS * 16.0),
            None,
        ))
}

pub fn context_menu_separator(theme: AppTheme, ui_scale: impl Into<UiScale>) -> Div {
    let ui_scale = ui_scale.into();
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    div()
        .my(scaled_px(2.0))
        .border_t_1()
        .border_color(theme.colors.stroke.subtle)
}

/// Which way a menu's scroll arrow scrolls.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MenuScrollDirection {
    Up,
    Down,
}

/// Height of a menu scroll arrow at 100%.
pub const MENU_SCROLL_ARROW_HEIGHT_PX: f32 = 16.0;

/// The strip on the top or bottom edge of a menu taller than the window: a
/// chevron on the menu's own surface, laid over the rows it hides (macOS
/// style). The host shows it only while rows are hidden that way and scrolls
/// while it is hovered; its parent must be `relative()`.
pub fn context_menu_scroll_arrow(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    direction: MenuScrollDirection,
    hovered: bool,
) -> Div {
    let scaled_px = crate::ui_scale::scaler(ui_scale.into());
    let icon = match direction {
        MenuScrollDirection::Up => "icons/chevron_up.svg",
        MenuScrollDirection::Down => "icons/chevron_down.svg",
    };
    let color = if hovered {
        theme.colors.foreground.primary
    } else {
        theme.colors.foreground.secondary
    };
    div()
        .absolute()
        .left_0()
        .right_0()
        .map(|strip| match direction {
            MenuScrollDirection::Up => strip.top_0(),
            MenuScrollDirection::Down => strip.bottom_0(),
        })
        .h(scaled_px(MENU_SCROLL_ARROW_HEIGHT_PX))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.colors.surface.raised)
        .child(crate::icons::svg_icon(icon, color, scaled_px(12.0)))
}

/// One inline submenu (its header row plus the rows it expands to) on the
/// sidebar-header surface, so rows that act on the same object read as a unit.
/// Rows inside keep a transparent rest, so hover and selection overlays land
/// on the tint instead of replacing it.
pub fn context_menu_group(theme: AppTheme, ui_scale: impl Into<UiScale>, spaced: bool) -> Div {
    let scaled_px = crate::ui_scale::scaler(ui_scale.into());
    div()
        .flex()
        .flex_col()
        .items_stretch()
        .rounded(px(theme.radii.row))
        .bg(theme.colors.surface.panel)
        .when(spaced, |group| group.mt(scaled_px(2.0)))
}

pub struct ContextMenuEntry {
    id: ElementId,
    label: ContextMenuText,
    icon: ContextMenuIconSlot,
    tone: ContextMenuTone,
    shortcut: Option<SharedString>,
    shortcut_keycaps: bool,
    selected: bool,
    disabled: bool,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
}

impl ContextMenuEntry {
    pub fn new(id: impl Into<ElementId>, label: impl Into<ContextMenuText>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: ContextMenuIconSlot::None,
            tone: ContextMenuTone::Accent,
            shortcut: None,
            shortcut_keycaps: false,
            selected: false,
            disabled: false,
            tooltip_host: None,
        }
    }

    pub fn icon(mut self, icon: ContextMenuIconSlot) -> Self {
        self.icon = icon;
        self
    }

    pub fn tone(mut self, tone: ContextMenuTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn shortcut(mut self, shortcut: Option<SharedString>) -> Self {
        self.shortcut = shortcut;
        self
    }

    pub fn shortcut_keycaps(mut self, shortcut_keycaps: bool) -> Self {
        self.shortcut_keycaps = shortcut_keycaps;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip_host(mut self, tooltip_host: WeakEntity<TooltipHost>) -> Self {
        self.tooltip_host = Some(tooltip_host);
        self
    }

    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
    ) -> Stateful<Div> {
        context_menu_entry(self, theme, ui_scale, cx)
    }

    /// All menu entries share completed-click activation, focus preservation and the
    /// same disabled gate, including menus embedded in pickers and prompts.
    pub fn on_select<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
        handler: impl Fn(&mut V, &gpui::ClickEvent, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        use crate::interaction::ControlInteractionExt as _;
        let disabled = self.disabled;
        self.render(theme, ui_scale, cx)
            .on_menu_activate(disabled, cx.listener(handler))
    }
}

fn context_menu_entry<V: 'static>(
    entry: ContextMenuEntry,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    cx: &gpui::Context<V>,
) -> Stateful<Div> {
    let ContextMenuEntry {
        id,
        label,
        icon,
        tone,
        shortcut,
        shortcut_keycaps,
        selected,
        disabled,
        tooltip_host,
    } = entry;
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let scaled_px = crate::ui_scale::scaler(ui_scale);
    let max_lines = label.resolved_max_lines(2);
    let icon_color = context_menu_icon_color(theme, disabled, tone);
    let drawn_icon = match &icon {
        ContextMenuIconSlot::Icon(path) => Some(path.clone()),
        ContextMenuIconSlot::Reserved | ContextMenuIconSlot::None => None,
    };
    let text_color = context_menu_entry_text_color(theme, disabled, tone);
    let mut row = crate::menu::menu_item(id, theme, ui_scale, selected, disabled)
        .text_color(text_color)
        .child(
            div()
                .flex()
                .items_center()
                .gap(scaled_px(8.0))
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .when(!matches!(icon, ContextMenuIconSlot::None), |row| {
                    row.child(
                        div()
                            .w(scaled_px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(drawn_icon, |this, path| {
                                let icon_label = label.text.clone();
                                this.child(
                                    crate::icons::svg_icon(path, icon_color, scaled_px(13.0))
                                        .debug_selector(move || {
                                            format!("context_menu_entry_icon_{icon_label}")
                                        }),
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(theme.ui_text(MENU_PRIMARY_REMS * 16.0))
                        .line_height(ui_scale.ui_text(18.0))
                        .text_color(text_color)
                        .when(max_lines == 1, |s| s.whitespace_nowrap().overflow_hidden())
                        .when(max_lines > 1, |s| s.line_clamp(max_lines))
                        .child(context_menu_text_content(
                            label,
                            tooltip_host,
                            cx,
                            max_lines,
                            text_color,
                            theme.ui_text(MENU_PRIMARY_REMS * 16.0),
                            None,
                        )),
                ),
        );

    let mut end = div()
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_size(theme.ui_text(12.0))
        .line_height(ui_scale.ui_text(14.0))
        .text_color(theme.colors.foreground.secondary);

    if let Some(shortcut) = shortcut {
        end = if shortcut_keycaps {
            end.child(shortcut_keys(shortcut.as_ref(), theme, ui_scale))
        } else {
            end.child(shortcut)
        };
    }
    row = row.child(end);

    if disabled {
        row = row
            .text_color(theme.colors.foreground.secondary)
            .cursor(CursorStyle::Arrow);
    }

    row
}

fn context_menu_entry_text_color(
    theme: AppTheme,
    disabled: bool,
    tone: ContextMenuTone,
) -> gpui::Rgba {
    if disabled {
        theme.colors.foreground.secondary
    } else if tone == ContextMenuTone::Danger {
        theme.colors.status.danger.foreground
    } else {
        theme.colors.foreground.primary
    }
}

fn context_menu_icon_color(theme: AppTheme, disabled: bool, tone: ContextMenuTone) -> Rgba {
    if disabled {
        return theme.colors.foreground.secondary;
    }
    match tone {
        ContextMenuTone::Accent => theme.colors.accent.foreground,
        ContextMenuTone::Neutral => theme.colors.foreground.secondary,
        ContextMenuTone::Success => theme.colors.status.success.foreground,
        ContextMenuTone::Warning => theme.colors.status.warning.foreground,
        ContextMenuTone::Danger => theme.colors.status.danger.foreground,
    }
}

fn context_menu_text_content<V: 'static>(
    text: ContextMenuText,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
    max_lines: usize,
    text_color: Rgba,
    text_size: gpui::Rems,
    font_weight: Option<gpui::FontWeight>,
) -> impl IntoElement {
    if max_lines == 1 {
        // `text_size`/`font_weight` are set on the element itself: it shapes its
        // line from `window.text_style()` in a deferred measure closure, so the
        // caller's text styling on the wrapping div does not reach it.
        let mut truncated = TruncatedText::new(text.text.clone(), text_size)
            .profile(text.profile)
            .text_color(text_color);
        if let Some(font_weight) = font_weight {
            truncated = truncated.font_weight(font_weight);
        }
        if let (Some(tooltip_host), TruncatedTextTooltipMode::FullTextIfTruncated) =
            (tooltip_host, text.tooltip_mode)
        {
            truncated = truncated.full_text_tooltip(tooltip_host);
        }
        return truncated.render(cx).into_any_element();
    }

    // The wrapped branch is an ordinary text child, which does inherit.
    div()
        .text_color(text_color)
        .child(text.text)
        .into_any_element()
}

#[cfg(test)]
mod tests;
