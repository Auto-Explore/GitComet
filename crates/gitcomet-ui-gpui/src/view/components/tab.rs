use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Stateful, div, px};
use std::cmp::Ordering;

/// The position of a tab within a list.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TabPosition {
    First,
    Middle(Ordering),
    Last,
}

pub struct Tab {
    div: Stateful<Div>,
    selected: bool,
    position: TabPosition,
    end_slot: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl Tab {
    const END_TAB_SLOT_SIZE: gpui::Pixels = px(14.0);

    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            div: div().id(id.clone()),
            selected: false,
            position: TabPosition::First,
            end_slot: None,
            children: Vec::new(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn position(mut self, position: TabPosition) -> Self {
        self.position = position;
        self
    }

    pub fn end_slot(mut self, slot: impl IntoElement) -> Self {
        self.end_slot = Some(slot.into_any_element());
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn container_height() -> gpui::Pixels {
        px(32.0)
    }

    pub fn render(self, theme: AppTheme) -> Stateful<Div> {
        let (text_color, tab_bg) = if self.selected {
            (theme.colors.text, theme.colors.active_section)
        } else {
            (theme.colors.text_muted, theme.colors.surface_bg)
        };
        let inactive_hover_bg = if theme.is_dark {
            with_alpha(theme.colors.hover, 0.65)
        } else {
            theme.colors.hover
        };
        let active_bg = theme.colors.active;
        let focus_ring = theme.colors.focus_ring;

        let end_slot = div()
            .flex_none()
            .size(Self::END_TAB_SLOT_SIZE)
            .flex()
            .items_center()
            .justify_center()
            .children(self.end_slot);

        let mut base = self
            .div
            .group("tab")
            .tab_index(0)
            .relative()
            .h(Self::container_height())
            .bg(tab_bg)
            .border_color(theme.colors.border)
            .cursor_pointer()
            .focus(move |s| s.border_color(focus_ring))
            .on_key_down(|event, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "left" => {
                        window.focus_prev(cx);
                        cx.stop_propagation();
                    }
                    "right" => {
                        window.focus_next(cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            })
            .when(self.selected, |tab| {
                let thickness = px(1.0);
                tab.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(thickness)
                        .bg(focus_ring),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(thickness)
                        .bg(focus_ring),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(31.0))
                    .px_1()
                    .text_color(text_color)
                    .children(self.children)
                    .child(end_slot),
            );

        if !self.selected {
            base = base
                .hover(move |s| s.bg(inactive_hover_bg))
                .active(move |s| s.bg(active_bg));
        }

        base = match self.position {
            TabPosition::First => {
                if self.selected {
                    base.pl(px(1.0)).pb(px(1.0))
                } else {
                    base.pl(px(1.0)).pr(px(1.0)).border_b_1()
                }
            }
            TabPosition::Last => {
                if self.selected {
                    base.pb(px(1.0))
                } else {
                    base.pl(px(1.0)).border_b_1().border_r_1()
                }
            }
            TabPosition::Middle(Ordering::Equal) => {
                if self.selected {
                    base.pb(px(1.0))
                } else {
                    base.border_l_1().border_r_1().pb(px(1.0))
                }
            }
            TabPosition::Middle(Ordering::Less) => base.border_l_1().pr(px(1.0)).border_b_1(),
            TabPosition::Middle(Ordering::Greater) => base.border_r_1().pl(px(1.0)).border_b_1(),
        };

        base
    }
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}
