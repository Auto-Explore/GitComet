use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{
    AnyElement, Bounds, ClickEvent, CursorStyle, Div, IntoElement, Pixels, SharedString, Stateful,
    Window, div, px,
};
use std::cell::RefCell;
use std::rc::Rc;

use super::{CONTROL_HEIGHT_PX, CONTROL_PAD_X_PX, CONTROL_PAD_Y_PX, ICON_PAD_X_PX};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonStyle {
    Filled,
    Outlined,
    Solid,
    Subtle,
    Transparent,
    Danger,
    DangerSolid,
}

pub struct Button {
    id: SharedString,
    label: SharedString,
    style: ButtonStyle,
    disabled: bool,
    selected: bool,
    selected_bg: Option<gpui::Rgba>,
    borderless: bool,
    suppress_hover_border: bool,
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
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
            borderless: false,
            suppress_hover_border: false,
            start_slot: None,
            end_slot: None,
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

    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }

    pub fn no_hover_border(mut self) -> Self {
        self.suppress_hover_border = true;
        self
    }

    pub fn start_slot(mut self, slot: impl IntoElement) -> Self {
        self.start_slot = Some(slot.into_any_element());
        self
    }

    pub fn end_slot(mut self, slot: impl IntoElement) -> Self {
        self.end_slot = Some(slot.into_any_element());
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
        cx: &gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;

        self.render(theme)
            .when(!disabled, |this| this.on_click(cx.listener(f)))
    }

    pub fn on_click_with_bounds<V: 'static>(
        self,
        theme: AppTheme,
        cx: &gpui::Context<V>,
        f: impl Fn(&mut V, &ClickEvent, Bounds<Pixels>, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Stateful<Div> {
        let disabled = self.disabled;

        let last_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
        let last_bounds_for_prepaint = Rc::clone(&last_bounds);
        let last_bounds_for_click = Rc::clone(&last_bounds);
        let wrapper_id: SharedString = format!("{}_bounds_wrapper", self.id).into();

        let button = self.render(theme).when(!disabled, |this| {
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

    pub fn render(self, theme: AppTheme) -> Stateful<Div> {
        let transparent = gpui::rgba(0x00000000);
        let outlined_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.38 } else { 0.28 },
        );
        let hover_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.07 } else { 0.05 });
        let active_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.11 } else { 0.08 });
        let hover_overlay_muted =
            with_alpha(theme.colors.text, if theme.is_dark { 0.05 } else { 0.04 });
        let active_overlay_muted =
            with_alpha(theme.colors.text, if theme.is_dark { 0.08 } else { 0.06 });
        let (bg, hover_bg, active_bg, border, hover_border, active_border, text) = match self.style
        {
            ButtonStyle::Filled => (
                transparent,
                hover_overlay,
                active_overlay,
                with_alpha(theme.colors.accent, 0.90),
                with_alpha(theme.colors.accent, 1.00),
                with_alpha(theme.colors.accent, 1.00),
                theme.colors.accent,
            ),
            ButtonStyle::Outlined => (
                transparent,
                hover_overlay,
                active_overlay,
                outlined_border,
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.55 } else { 0.40 },
                ),
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.62 } else { 0.46 },
                ),
                theme.colors.text,
            ),
            ButtonStyle::Solid => {
                let bg = theme.colors.surface_bg_elevated;
                let hover_bg = mix(
                    bg,
                    theme.colors.text,
                    if theme.is_dark { 0.06 } else { 0.03 },
                );
                let active_bg = mix(
                    bg,
                    theme.colors.text,
                    if theme.is_dark { 0.10 } else { 0.05 },
                );
                (
                    bg,
                    hover_bg,
                    active_bg,
                    with_alpha(
                        theme.colors.text_muted,
                        if theme.is_dark { 0.34 } else { 0.26 },
                    ),
                    with_alpha(
                        theme.colors.text_muted,
                        if theme.is_dark { 0.55 } else { 0.40 },
                    ),
                    with_alpha(
                        theme.colors.text_muted,
                        if theme.is_dark { 0.62 } else { 0.46 },
                    ),
                    theme.colors.text,
                )
            }
            ButtonStyle::Subtle => (
                transparent,
                hover_overlay,
                active_overlay,
                transparent,
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.45 } else { 0.32 },
                ),
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.52 } else { 0.38 },
                ),
                theme.colors.text,
            ),
            ButtonStyle::Transparent => (
                transparent,
                hover_overlay_muted,
                active_overlay_muted,
                transparent,
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.40 } else { 0.30 },
                ),
                with_alpha(
                    theme.colors.text_muted,
                    if theme.is_dark { 0.46 } else { 0.34 },
                ),
                theme.colors.text_muted,
            ),
            ButtonStyle::Danger => (
                with_alpha(theme.colors.danger, if theme.is_dark { 0.18 } else { 0.14 }),
                with_alpha(theme.colors.danger, if theme.is_dark { 0.26 } else { 0.20 }),
                with_alpha(theme.colors.danger, if theme.is_dark { 0.32 } else { 0.26 }),
                with_alpha(theme.colors.danger, if theme.is_dark { 0.42 } else { 0.32 }),
                with_alpha(theme.colors.danger, if theme.is_dark { 0.46 } else { 0.36 }),
                with_alpha(theme.colors.danger, if theme.is_dark { 0.52 } else { 0.42 }),
                theme.colors.text,
            ),
            ButtonStyle::DangerSolid => {
                let bg = theme.colors.danger;
                let black = gpui::rgba(0x000000ff);
                let hover_bg = mix(bg, black, if theme.is_dark { 0.16 } else { 0.12 });
                let active_bg = mix(bg, black, if theme.is_dark { 0.26 } else { 0.18 });
                (
                    bg,
                    hover_bg,
                    active_bg,
                    mix(bg, black, if theme.is_dark { 0.34 } else { 0.26 }),
                    mix(bg, black, if theme.is_dark { 0.40 } else { 0.32 }),
                    mix(bg, black, if theme.is_dark { 0.48 } else { 0.40 }),
                    gpui::rgba(0xffffffff),
                )
            }
        };

        let label = self.label.to_string();
        let icon_only = looks_like_icon_button(&label);
        let selected = self.selected;
        let selected_bg_override = self.selected_bg;
        let borderless = self.borderless;
        let suppress_hover_border = self.suppress_hover_border || borderless;

        let mut inner = div().flex().items_center().gap_1();
        if let Some(start_slot) = self.start_slot {
            inner = inner.child(start_slot);
        }
        if !label.is_empty() {
            inner = inner.child(label);
        }
        if let Some(end_slot) = self.end_slot {
            inner = inner.child(end_slot);
        }

        let mut base = div()
            .id(self.id.clone())
            .tab_index(0)
            .h(px(CONTROL_HEIGHT_PX))
            .px(px(if icon_only {
                ICON_PAD_X_PX
            } else {
                CONTROL_PAD_X_PX
            }))
            .py(px(CONTROL_PAD_Y_PX))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme.radii.row))
            .bg(bg)
            .text_sm()
            .text_color(text)
            .cursor(CursorStyle::PointingHand)
            .child(inner);

        if !borderless {
            base = base.border_1().border_color(border);
        }
        base = base.focus(move |s| {
            if borderless {
                s.bg(theme.colors.focus_ring_bg)
            } else {
                s.border_color(theme.colors.focus_ring)
                    .bg(theme.colors.focus_ring_bg)
            }
        });

        if self.disabled {
            base = base.opacity(0.5).cursor(CursorStyle::Arrow);
        } else if selected {
            let selected_bg = selected_bg_override.unwrap_or(theme.colors.active);
            base = base
                .bg(selected_bg)
                .hover(move |s| s.bg(selected_bg))
                .active(move |s| s.bg(selected_bg));
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
    matches!(label.trim(), "✕" | "＋" | "▾" | "≡" | "" | "⋯" | "⟳" | "↻")
        || (label.chars().count() <= 2 && !label.chars().any(|c| c.is_alphanumeric()))
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}

fn mix(a: gpui::Rgba, b: gpui::Rgba, t: f32) -> gpui::Rgba {
    let t = t.clamp(0.0, 1.0);
    gpui::Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}
