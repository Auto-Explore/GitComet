use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Stateful, div, px};

pub struct TabBar {
    id: ElementId,
    tabs: Vec<AnyElement>,
    filler: Option<AnyElement>,
}

impl TabBar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tabs: Vec::new(),
            filler: None,
        }
    }

    pub fn tab(mut self, tab: impl IntoElement) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }

    /// Element stretched into the empty space after the tabs (e.g. a window
    /// drag surface). It collapses to zero width once tabs fill the bar.
    pub fn filler(mut self, filler: impl IntoElement) -> Self {
        self.filler = Some(filler.into_any_element());
        self
    }

    pub fn render(self, _theme: AppTheme, _ui_scale_percent: u32) -> Stateful<Div> {
        // The width-constrained inner row is what lets tabs flex-shrink down
        // to their minimum before the strip overflows into scrolling; children
        // measured directly by the scroll container never feel width pressure.
        let tabs = div()
            .id((self.id.clone(), "tabs"))
            .h_full()
            .overflow_x_scroll()
            .scrollbar_width(px(0.0))
            .child(
                div()
                    .flex()
                    .items_end()
                    .w_full()
                    .min_w(px(0.0))
                    .h_full()
                    .children(self.tabs)
                    .when_some(self.filler, |this, filler| {
                        this.child(div().flex_1().min_w(px(0.0)).h_full().child(filler))
                    }),
            );

        div()
            .id(self.id)
            .group("tab_bar")
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h_full()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .overflow_x_hidden()
                    .child(tabs),
            )
    }
}
