use crate::theme::AppTheme;
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, FontWeight, Pixels, ScrollHandle, SharedString, div, px};

#[cfg(any(test, feature = "test-support"))]
use super::CONTROL_HEIGHT_MD_PX;
use super::CONTROL_HEIGHT_PX;
#[cfg(any(test, feature = "test-support"))]
use gpui::IntoElement;

/// A pane's header bar: standard height for the density, raised surface, and
/// the rule under it.
pub fn content_header_bar(theme: AppTheme, ui_scale: impl Into<crate::ui_scale::UiScale>) -> Div {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    div()
        .flex()
        .items_center()
        .h(super::content_header_height(ui_scale))
        .px_2()
        .bg(theme.colors.surface.raised)
        .border_b_1()
        .border_color(theme.colors.stroke.default)
}

#[cfg(any(test, feature = "test-support"))]
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
        .child(
            div()
                .text_size(theme.ui_text(14.0))
                .font_weight(FontWeight::BOLD)
                .child(title),
        );

    if let Some(subtitle) = subtitle {
        header = header.child(
            div()
                .text_size(theme.ui_text(12.0))
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

#[cfg(any(test, feature = "test-support"))]
pub fn pill(theme: AppTheme, label: impl Into<SharedString>, bg: gpui::Rgba) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded(px(theme.radii.pill))
        .bg(bg)
        .text_size(theme.ui_text(12.0))
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
                .text_size(theme.ui_text(14.0))
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
                .text_size(theme.ui_text(18.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.primary)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child(message.into()),
        )
}

/// A vertically scrolling area paired with its overlay scrollbar: the wheel is
/// restricted to the vertical axis, the content gets a stable scrollbar
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
            // Growing past max_height must not change the width of wrapped
            // text, otherwise showing the thumb causes a second reflow.
            .pr(super::Scrollbar::gutter(super::ScrollbarAxis::Vertical))
            .overflow_hidden()
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if let Some(selector) = self.debug_selector {
            surface = surface.debug_selector(move || selector.to_string());
        }

        let container = div()
            .relative()
            .w_full()
            .min_w(px(0.0))
            .child(crate::restrict_scroll_to_vertical_axis(surface).child(child))
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
        .text_size(theme.ui_text(12.0))
        .text_color(theme.colors.foreground.secondary)
        .child(div().flex_1().min_w(px(0.0)).px_2().child(left.into()))
        .child(div().flex_1().min_w(px(0.0)).px_2().child(right.into()))
}

/// The unfilled track of a [`progress_bar`].
pub fn progress_bar_track_color(theme: AppTheme) -> gpui::Rgba {
    crate::theme::with_alpha(
        theme.colors.stroke.default,
        if theme.is_dark { 0.40 } else { 0.22 },
    )
}

/// The outline of a [`progress_bar`].
pub fn progress_bar_border_color(theme: AppTheme) -> gpui::Rgba {
    crate::theme::with_alpha(
        theme.colors.stroke.default,
        if theme.is_dark { 0.72 } else { 0.42 },
    )
}

/// A horizontal bar with `fraction` (0..=1) of its width filled with `fill`.
/// It takes its container's width.
pub fn progress_bar(
    theme: AppTheme,
    ui_scale: impl Into<crate::ui_scale::UiScale>,
    fraction: f32,
    fill: gpui::Rgba,
) -> Div {
    let ui_scale = ui_scale.into();
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Flex weights rather than a percentage width, so the fill rounds to the
    // track's own pixel grid.
    let mut filled = div()
        .h_full()
        .bg(fill)
        .rounded(px(999.0))
        .when(fraction > 0.0, |this| this.min_w(px(2.0)));
    filled.style().flex_grow = Some(fraction);
    filled.style().flex_shrink = Some(0.0);
    filled.style().flex_basis = Some(gpui::relative(0.0).into());
    let mut rest = div().h_full();
    rest.style().flex_grow = Some(1.0 - fraction);
    rest.style().flex_shrink = Some(0.0);
    rest.style().flex_basis = Some(gpui::relative(0.0).into());
    div()
        .w_full()
        .h(ui_scale.px(8.0))
        .flex()
        .rounded(px(999.0))
        .overflow_hidden()
        .bg(progress_bar_track_color(theme))
        .border_1()
        .border_color(progress_bar_border_color(theme))
        .child(filled)
        .child(rest)
}

#[cfg(test)]
mod progress_bar_tests {
    use super::*;

    struct Bars {
        theme: AppTheme,
    }

    impl gpui::Render for Bars {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let fill = self.theme.colors.accent.solid;
            div().w(px(200.0)).flex().flex_col().children(
                [0.0, 0.5, 2.0, f32::NAN]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, fraction)| {
                        div()
                            .id(("bar", ix))
                            .debug_selector(move || format!("bar_{ix}"))
                            .w_full()
                            .child(progress_bar(
                                self.theme,
                                crate::ui_scale::UiScale::from_percent(100),
                                fraction,
                                fill,
                            ))
                    }),
            )
        }
    }

    /// The fill follows the fraction and never leaves the track, whatever
    /// the caller passes.
    #[gpui::test]
    fn progress_bars_fill_their_fraction_of_the_track(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| Bars { theme });
        crate::test_support::redraw(cx);
        let fill: gpui::Background = theme.colors.accent.solid.into();
        // Painted quads are in device pixels; layout bounds are logical.
        let widths: Vec<f32> = (0..4)
            .map(|ix| {
                let selector: &'static str = Box::leak(format!("bar_{ix}").into_boxed_str());
                let bounds = cx.debug_bounds(selector).unwrap();
                cx.update(|window, _| {
                    let scale = window.scale_factor();
                    window
                        .painted_quads()
                        .into_iter()
                        .filter(|quad| {
                            let top = quad.bounds.origin.y.0 / scale;
                            quad.background == fill
                                && top >= f32::from(bounds.origin.y)
                                && top < f32::from(bounds.origin.y + bounds.size.height)
                        })
                        .map(|quad| quad.bounds.size.width.0 / scale)
                        .sum::<f32>()
                })
            })
            .collect();
        assert_eq!(widths[0], 0.0, "{widths:?}");
        assert!((95.0..=103.0).contains(&widths[1]), "{widths:?}");
        assert!((195.0..=200.0).contains(&widths[2]), "{widths:?}");
        assert_eq!(widths[3], 0.0, "{widths:?}");
    }
}
