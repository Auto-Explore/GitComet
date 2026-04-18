use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Stateful, div, px};

use super::Tab;

pub struct TabBar {
    id: ElementId,
    tabs: Vec<AnyElement>,
    end: Vec<AnyElement>,
}

impl TabBar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tabs: Vec::new(),
            end: Vec::new(),
        }
    }

    pub fn tab(mut self, tab: impl IntoElement) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }

    pub fn end_child(mut self, child: impl IntoElement) -> Self {
        self.end.push(child.into_any_element());
        self
    }

    pub fn render(self, theme: AppTheme, ui_scale_percent: u32) -> Stateful<Div> {
        let tabs = div()
            .id((self.id.clone(), "tabs"))
            .flex()
            .items_center()
            .h_full()
            .overflow_x_scroll()
            .scrollbar_width(px(0.0))
            .children(self.tabs);

        div()
            .id(self.id)
            .group("tab_bar")
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h(Tab::container_height(ui_scale_percent))
            .bg(theme.colors.surface_bg)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .overflow_x_hidden()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .border_b_1()
                            .border_color(theme.colors.border),
                    )
                    .child(tabs),
            )
            .when(!self.end.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(0.0))
                        .h_full()
                        .border_b_1()
                        .border_l_1()
                        .border_color(theme.colors.border)
                        .children(self.end),
                )
            })
    }
}
