use crate::theme::{AppTheme, composite_over};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, ClickEvent, CursorStyle, Div, FocusHandle, IntoElement, Pixels,
    SharedString, Stateful, Window, div, px,
};
use palette::IntoColor;
use std::cell::RefCell;
use std::rc::Rc;

use super::{control_height, control_pad_x, control_pad_y, icon_pad_x};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonStyle {
    Filled,
    Outlined,
    Solid,
    Subtle,
    Transparent,
    Danger,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ButtonRounding {
    #[default]
    All,
    Left,
    Right,
}

pub struct Button {
    id: SharedString,
    label: SharedString,
    style: ButtonStyle,
    disabled: bool,
    selected: bool,
    selected_bg: Option<gpui::Rgba>,
    bg: Option<gpui::Rgba>,
    hover_bg: Option<gpui::Rgba>,
    text_color: Option<gpui::Rgba>,
    rounding: ButtonRounding,
    borderless: bool,
    suppress_hover_border: bool,
    no_focus: bool,
    focus_handle: Option<FocusHandle>,
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
    separate_end_slot: bool,
}

impl Button {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            style: ButtonStyle::Subtle,
            disabled: false,
            selected: false,
            selected_bg: None,
            bg: None,
            hover_bg: None,
            text_color: None,
            rounding: ButtonRounding::All,
            borderless: false,
            suppress_hover_border: false,
            no_focus: false,
            focus_handle: None,
            start_slot: None,
            end_slot: None,
            separate_end_slot: false,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn selected_bg(mut self, bg: gpui::Rgba) -> Self {
        self.selected_bg = Some(bg);
        self
    }

    /// Resting background, for a button that carries its own tint instead of
    /// the style's neutral fill.
    pub fn bg(mut self, bg: gpui::Rgba) -> Self {
        self.bg = Some(bg);
        self
    }

    /// Hover *and* pressed background. A tinted resting background needs this
    /// too: the style's neutral overlay would otherwise read as a step *down*
    /// from the tint under the cursor.
    pub fn hover_bg(mut self, bg: gpui::Rgba) -> Self {
        self.hover_bg = Some(bg);
        self
    }

    pub fn text_color(mut self, color: gpui::Rgba) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Round only the outside edge of a button in a segmented control.
    pub fn rounded_left(mut self) -> Self {
        self.rounding = ButtonRounding::Left;
        self
    }

    /// Round only the outside edge of a button in a segmented control.
    pub fn rounded_right(mut self) -> Self {
        self.rounding = ButtonRounding::Right;
        self
    }

    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }

    pub fn no_hover_border(mut self) -> Self {
        self.suppress_hover_border = true;
        self
    }

    pub fn no_focus(mut self) -> Self {
        self.no_focus = true;
        self
    }

    pub fn focus_handle(mut self, focus_handle: FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle);
        self
    }

    pub fn start_slot(mut self, slot: impl IntoElement) -> Self {
        self.start_slot = Some(slot.into_any_element());
        self
    }

    pub fn end_slot(mut self, slot: impl IntoElement) -> Self {
        self.end_slot = Some(slot.into_any_element());
        self.separate_end_slot = false;
        self
    }

    pub fn separated_end_slot(mut self, slot: impl IntoElement) -> Self {
        self.end_slot = Some(slot.into_any_element());
        self.separate_end_slot = true;
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click<V: 'static>(
        self,
        theme: AppTheme,
        cx: &mut gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;
        let ui_scale = UiScale::current(cx);

        self.render(theme, ui_scale)
            .when(!disabled, |this| this.on_click(cx.listener(f)))
    }

    pub fn on_click_with_bounds<V: 'static>(
        self,
        theme: AppTheme,
        cx: &mut gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, Bounds<Pixels>, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;
        let ui_scale = UiScale::current(cx);

        let last_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let last_bounds_for_prepaint = Rc::clone(&last_bounds);
        let last_bounds_for_click = Rc::clone(&last_bounds);
        let wrapper_id: SharedString = format!("{}_bounds_wrapper", self.id).into();

        let button = self.render(theme, ui_scale).when(!disabled, |this| {
            this.on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                let bounds = (*last_bounds_for_click.borrow())
                    .unwrap_or_else(|| Bounds::new(e.position(), gpui::size(px(0.0), px(0.0))));
                f(this, e, bounds, window, cx);
            }))
        });

        div()
            .on_children_prepainted(move |children_bounds, _window, _cx| {
                if let Some(bounds) = children_bounds.first() {
                    *last_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                }
            })
            .child(button)
            .id(wrapper_id)
    }

    pub fn render(self, theme: AppTheme, ui_scale: impl Into<UiScale>) -> Stateful<Div> {
        let Self {
            id,
            label,
            style,
            disabled,
            selected,
            selected_bg,
            bg: bg_override,
            hover_bg: hover_bg_override,
            text_color: text_color_override,
            rounding,
            borderless,
            suppress_hover_border,
            no_focus,
            focus_handle,
            start_slot,
            end_slot,
            separate_end_slot,
        } = self;
        let ui_scale = ui_scale.into();

        let transparent = gpui::rgba(0x00000000);
        let outlined_border = if theme.is_dark {
            with_alpha(theme.colors.foreground.secondary, 0.38)
        } else {
            theme.colors.stroke.control
        };
        let hover_overlay = theme.hover_overlay();
        let active_overlay = theme.active_overlay();
        let (bg, hover_bg, active_bg, border, hover_border, active_border, text) = match style {
            ButtonStyle::Filled => (
                transparent,
                hover_overlay,
                active_overlay,
                with_alpha(theme.colors.accent.foreground, 0.90),
                with_alpha(theme.colors.accent.foreground, 1.00),
                with_alpha(theme.colors.accent.foreground, 1.00),
                theme.colors.accent.foreground,
            ),
            ButtonStyle::Outlined => (
                transparent,
                hover_overlay,
                active_overlay,
                outlined_border,
                if theme.is_dark {
                    with_alpha(theme.colors.foreground.secondary, 0.55)
                } else {
                    theme.colors.interaction.selected_indicator
                },
                if theme.is_dark {
                    with_alpha(theme.colors.foreground.secondary, 0.62)
                } else {
                    theme.colors.interaction.selected_indicator
                },
                theme.colors.foreground.primary,
            ),
            ButtonStyle::Solid => {
                let bg = theme.colors.surface.raised;
                let hover_bg = if theme.is_dark {
                    mix(bg, theme.colors.foreground.primary, 0.06)
                } else {
                    composite_over(bg, hover_overlay)
                };
                let active_bg = if theme.is_dark {
                    mix(bg, theme.colors.foreground.primary, 0.10)
                } else {
                    composite_over(bg, active_overlay)
                };
                (
                    bg,
                    hover_bg,
                    active_bg,
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.34)
                    } else {
                        theme.colors.stroke.control
                    },
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.55)
                    } else {
                        theme.colors.interaction.selected_indicator
                    },
                    if theme.is_dark {
                        with_alpha(theme.colors.foreground.secondary, 0.62)
                    } else {
                        theme.colors.interaction.selected_indicator
                    },
                    theme.colors.foreground.primary,
                )
            }
            ButtonStyle::Subtle => (
                transparent,
                hover_overlay,
                active_overlay,
                transparent,
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.45 } else { 0.32 },
                ),
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.52 } else { 0.38 },
                ),
                theme.colors.foreground.primary,
            ),
            ButtonStyle::Transparent => (
                transparent,
                hover_overlay,
                active_overlay,
                transparent,
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.40 } else { 0.30 },
                ),
                with_alpha(
                    theme.colors.foreground.secondary,
                    if theme.is_dark { 0.46 } else { 0.34 },
                ),
                theme.colors.foreground.secondary,
            ),
            ButtonStyle::Danger => {
                let danger = theme.colors.status.danger;
                if theme.is_dark {
                    (
                        with_alpha(danger.foreground, 0.18),
                        with_alpha(danger.foreground, 0.26),
                        with_alpha(danger.foreground, 0.32),
                        with_alpha(danger.foreground, 0.42),
                        with_alpha(danger.foreground, 0.46),
                        with_alpha(danger.foreground, 0.52),
                        theme.colors.foreground.primary,
                    )
                } else {
                    (
                        danger.background,
                        composite_over(danger.background, hover_overlay),
                        composite_over(danger.background, active_overlay),
                        danger.border,
                        danger.foreground,
                        danger.foreground,
                        danger.foreground,
                    )
                }
            }
        };

        let bg = bg_override.unwrap_or(bg);
        let hover_bg = hover_bg_override.unwrap_or(hover_bg);
        let active_bg = hover_bg_override.unwrap_or(active_bg);
        let text = text_color_override.unwrap_or(text);

        let separator_color = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.34 } else { 0.26 },
        );
        let label = label.to_string();
        let separator_debug_selector = format!("{}_end_slot_separator", id.as_ref());
        let icon_only = looks_like_icon_button(&label);
        let selected_bg_override = selected_bg;
        let suppress_hover_border = suppress_hover_border || borderless;
        let control_height = control_height(ui_scale);
        let control_pad_x = control_pad_x(ui_scale);
        let control_pad_y = control_pad_y(ui_scale);
        let icon_pad_x = icon_pad_x(ui_scale);
        let content_gap = ui_scale.px(4.0);
        let separated_slot_pad = ui_scale.px(6.0);

        let mut leading = div().flex().items_center().gap(content_gap);
        if let Some(start_slot) = start_slot {
            leading = leading.child(start_slot);
        }
        if !label.is_empty() {
            leading = leading.child(label);
        }
        let inner = match (separate_end_slot, end_slot) {
            (true, Some(end_slot)) => div()
                .flex()
                .items_center()
                .h_full()
                .child(leading.pr(separated_slot_pad))
                .child(
                    div()
                        .debug_selector({
                            let separator_debug_selector = separator_debug_selector.clone();
                            move || separator_debug_selector.clone()
                        })
                        .flex()
                        .items_center()
                        .h_full()
                        .pl(separated_slot_pad)
                        .border_l_1()
                        .border_color(separator_color)
                        .child(end_slot),
                ),
            (_, Some(end_slot)) => leading.child(end_slot),
            (_, None) => leading,
        };

        let control_radius = px(theme.radii.control);
        let mut base = div()
            .id(id.clone())
            .h(control_height)
            .px(if icon_only { icon_pad_x } else { control_pad_x })
            .py(control_pad_y)
            .flex()
            .items_center()
            .justify_center()
            .when(rounding == ButtonRounding::All, |d| {
                d.rounded(control_radius)
            })
            .when(rounding == ButtonRounding::Left, |d| {
                d.rounded_tl(control_radius).rounded_bl(control_radius)
            })
            .when(rounding == ButtonRounding::Right, |d| {
                d.rounded_tr(control_radius).rounded_br(control_radius)
            })
            .bg(bg)
            .text_sm()
            .text_color(text)
            .cursor(CursorStyle::PointingHand)
            .child(inner);

        if let Some(focus_handle) = focus_handle {
            let focus_handle = focus_handle.tab_stop(!disabled);
            base = base.track_focus(&focus_handle);
        } else if !no_focus {
            base = base.tab_index(0);
        }

        if !borderless {
            base = base.border_1().border_color(border);
        }
        base = base.focus(move |s| {
            if borderless {
                s.bg(theme.colors.interaction.focus_background)
            } else {
                s.border_color(theme.colors.interaction.focus_ring)
                    .bg(theme.colors.interaction.focus_background)
            }
        });

        if disabled {
            base = base.opacity(0.5).cursor(CursorStyle::Arrow);
        } else if selected {
            let selected_bg =
                selected_bg_override.unwrap_or(theme.colors.interaction.pressed_background);
            base = base
                .bg(selected_bg)
                .hover(move |s| s.bg(selected_bg))
                .active(move |s| s.bg(selected_bg));
            if !theme.is_dark {
                base = base.shadow(vec![gpui::BoxShadow {
                    color: theme.colors.interaction.selected_indicator.into_color(),
                    offset: gpui::point(px(0.0), px(0.0)),
                    blur_radius: px(0.0),
                    spread_radius: px(1.0),
                    inset: true,
                }]);
            }
        } else if suppress_hover_border {
            base = base
                .hover(move |s| s.bg(hover_bg))
                .active(move |s| s.bg(active_bg));
        } else {
            base = base
                .hover(move |s| s.bg(hover_bg).border_color(hover_border))
                .active(move |s| s.bg(active_bg).border_color(active_border));
        }

        base
    }
}

fn looks_like_icon_button(label: &str) -> bool {
    let trimmed = label.trim();
    trimmed.is_empty()
        || (trimmed.chars().count() <= 2 && !trimmed.chars().any(|c| c.is_alphanumeric()))
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.alpha = alpha;
    color
}

fn mix(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}
