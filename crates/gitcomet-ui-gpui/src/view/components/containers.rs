use crate::theme::AppTheme;
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, FontWeight, Pixels, ScrollHandle, SharedString, div, px};

#[cfg(test)]
use super::CONTROL_HEIGHT_MD_PX;
use super::CONTROL_HEIGHT_PX;
#[cfg(test)]
use gpui::IntoElement;

#[cfg(test)]
pub fn panel(
    theme: AppTheme,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    content: impl IntoElement,
) -> Div {
    let title: SharedString = title.into();
    let show_header = !title.as_ref().is_empty() || subtitle.is_some();
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(CONTROL_HEIGHT_MD_PX))
        .px_2()
        .border_b_1()
        .border_color(theme.colors.stroke.default)
        .bg(theme.colors.surface.raised)
        .child(div().text_sm().font_weight(FontWeight::BOLD).child(title));

    if let Some(subtitle) = subtitle {
        header = header.child(
            div()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(subtitle),
        );
    }

    div()
        .flex()
        .flex_col()
        .bg(theme.colors.surface.panel)
        .border_1()
        .border_color(theme.colors.stroke.default)
        .rounded(px(theme.radii.panel))
        .overflow_hidden()
        .when(show_header, |this| this.child(header))
        .child(
            div().flex().flex_col().flex_1().min_h(px(0.0)).p_2().child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(content),
            ),
        )
}

#[cfg(test)]
pub fn pill(theme: AppTheme, label: impl Into<SharedString>, bg: gpui::Rgba) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded(px(theme.radii.pill))
        .bg(bg)
        .text_xs()
        .text_color(theme.colors.foreground.primary)
        .child(label.into())
}

/// Empty placeholder for a section whose header already names it — just the
/// centered muted message, no duplicated title.
pub fn empty_state_message(theme: AppTheme, message: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px_2()
        .py_4()
        .child(
            div()
                .text_sm()
                .text_color(theme.colors.foreground.secondary)
                .child(message.into()),
        )
}

pub fn empty_state(
    theme: AppTheme,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .px_2()
        .py_4()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.primary)
                .child(title.into()),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.colors.foreground.secondary)
                .child(message.into()),
        )
}

/// A vertically scrolling area paired with its overlay scrollbar: the wheel is
/// restricted to the vertical axis, the content gets a visible scrollbar
/// gutter, and the scrollbar is anchored to the returned container.
pub struct ScrollContainer {
    surface_id: ElementId,
    scrollbar_id: ElementId,
    container_id: Option<ElementId>,
    debug_selector: Option<&'static str>,
    scroll: ScrollHandle,
    max_height: Pixels,
}

impl ScrollContainer {
    pub fn vertical(
        surface_id: impl Into<ElementId>,
        scrollbar_id: impl Into<ElementId>,
        scroll: ScrollHandle,
        max_height: Pixels,
    ) -> Self {
        Self {
            surface_id: surface_id.into(),
            scrollbar_id: scrollbar_id.into(),
            container_id: None,
            debug_selector: None,
            scroll,
            max_height,
        }
    }

    pub fn container_id(mut self, id: impl Into<ElementId>) -> Self {
        self.container_id = Some(id.into());
        self
    }

    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }

    pub fn render(self, theme: AppTheme, child: impl IntoElement) -> AnyElement {
        let mut surface = div()
            .id(self.surface_id)
            .relative()
            .w_full()
            .min_w(px(0.0))
            .max_h(self.max_height)
            .pr(super::Scrollbar::visible_gutter(
                self.scroll.clone(),
                super::ScrollbarAxis::Vertical,
            ))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if let Some(selector) = self.debug_selector {
            surface = surface.debug_selector(move || selector.to_string());
        }

        let container = div()
            .relative()
            .w_full()
            .min_w(px(0.0))
            .child(crate::view::restrict_scroll_to_vertical_axis(surface).child(child))
            .child(super::Scrollbar::new(self.scrollbar_id, self.scroll).render(theme));

        match self.container_id {
            Some(id) => container.id(id).into_any_element(),
            None => container.into_any_element(),
        }
    }
}

pub fn split_columns_header(
    theme: AppTheme,
    ui_scale_percent: u32,
    left: impl Into<SharedString>,
    right: impl Into<SharedString>,
) -> Div {
    div()
        .h(ui_scale::design_px_from_percent(
            CONTROL_HEIGHT_PX,
            ui_scale_percent,
        ))
        .flex()
        .items_center()
        .text_xs()
        .text_color(theme.colors.foreground.secondary)
        .child(div().flex_1().min_w(px(0.0)).px_2().child(left.into()))
        .child(div().flex_1().min_w(px(0.0)).px_2().child(right.into()))
}
