//! The header above a side panel: an adaptive segmented control that switches
//! the panel's tabs, then the panel's tools. Every side panel switches its
//! tabs with it, so both sides of a window read alike.

use super::{Segment, SegmentedControl, content_header_height, control_height};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, App, Div, Pixels, Rgba, SharedString, Window, div, px};

const PAD_X_PX: f32 = 4.0;
const GAP_PX: f32 = 2.0;
/// The icon of a contributed tab that brings none: distinct from every
/// built-in tab's, so a folded tab is never mistaken for one.
const FALLBACK_TAB_ICON: &str = "icons/box.svg";

/// Built per render, like [`SegmentedControl`].
pub struct PanelHeader {
    control: SegmentedControl,
    tools: usize,
}

impl PanelHeader {
    /// `id` is the control track's element id and debug selector.
    pub fn new(id: impl Into<SharedString>, cx: &mut App) -> Self {
        Self {
            control: SegmentedControl::new(id)
                .adaptive()
                .font(crate::font_preferences::current_ui_font(cx)),
            tools: 0,
        }
    }

    /// A tab; `id` is its element id and debug selector, and `{id}_icon` and
    /// `{id}_label` name its parts. Unselected tabs fold to their icon when
    /// the panel is narrow, so a tab without one takes a generic icon.
    pub fn tab(
        mut self,
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        icon: Option<SharedString>,
        selected: bool,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        self.control = self.control.segment(
            Segment::new(id, title)
                .icon(icon.unwrap_or_else(|| FALLBACK_TAB_ICON.into()))
                .selected(selected)
                .tooltip(tooltip, Vec::new()),
        );
        self
    }

    /// Room for `count` square tools after the tabs, in [`Self::minimum_width`].
    pub fn tools(mut self, count: usize) -> Self {
        self.tools = count;
        self
    }

    /// The narrowest the header lays out: unselected tabs folded to their
    /// icons, the tools in place. A panel is never narrower.
    pub fn minimum_width(&self, theme: AppTheme, scale: UiScale, window: &mut Window) -> Pixels {
        self.control.minimum_width(theme, scale, window)
            + scale.px(PAD_X_PX) * 2.0
            + (control_height(scale) + scale.px(GAP_PX)) * self.tools as f32
    }

    /// The header on `background`, calling `on_select` with a tab's index when
    /// it is clicked, the selected one included; `tools` follow the tabs.
    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        scale: UiScale,
        background: Rgba,
        tools: Vec<AnyElement>,
        cx: &mut gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        let has_tools = !tools.is_empty();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(scale.px(GAP_PX))
            .w_full()
            .min_w(px(0.0))
            .flex_none()
            .h(content_header_height(scale))
            .px(scale.px(PAD_X_PX))
            .bg(background)
            .border_b_1()
            .border_color(theme.colors.stroke.default)
            .child(self.control.render(theme, scale, cx, on_select))
            .when(has_tools, |header| {
                header.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(scale.px(GAP_PX))
                        .children(tools),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Header {
        theme: AppTheme,
        tools: usize,
    }

    impl gpui::Render for Header {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let scale = UiScale::from_percent(100);
            let tools = (0..self.tools)
                .map(|ix| {
                    div()
                        .id(("tool", ix))
                        .debug_selector(move || format!("tool_{ix}"))
                        .size(control_height(scale))
                        .into_any_element()
                })
                .collect();
            PanelHeader::new("header", cx)
                .tab(
                    "tab_on",
                    "History",
                    Some("icons/history.svg".into()),
                    true,
                    "History",
                )
                .tab("tab_off", "Notes", None, false, "Notes")
                .tools(self.tools)
                .render(
                    self.theme,
                    scale,
                    self.theme.colors.surface.canvas,
                    tools,
                    cx,
                    |_, _, _, _| {},
                )
                .debug_selector(|| "panel_header".to_string())
        }
    }

    /// Tabs and tools share the header's row, and a tab without an icon of
    /// its own still shows one, so it can fold like the others.
    #[gpui::test]
    fn tabs_and_tools_share_the_row_and_every_tab_has_an_icon(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| Header { theme, tools: 2 });
        crate::test_support::redraw(cx);
        let header = cx.debug_bounds("panel_header").unwrap();
        for selector in ["tab_on_icon", "tab_off_icon", "tool_0", "tool_1"] {
            let part = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is drawn"));
            assert!(header.contains(&part.center()), "{selector} in the header");
        }
        assert!(
            cx.debug_bounds("tab_off").unwrap().right()
                <= cx.debug_bounds("tool_0").unwrap().left()
        );
    }

    /// At its minimum width the header still shows the selected tab's label
    /// and every tool whole.
    #[gpui::test]
    fn nothing_is_cut_at_the_minimum_width(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::gitcomet_dark();
        let scale = UiScale::from_percent(100);
        let (_view, cx) = cx.add_window_view(|_, _| Header { theme, tools: 2 });
        let minimum = cx.update(|window, app| {
            PanelHeader::new("header", app)
                .tab(
                    "tab_on",
                    "History",
                    Some("icons/history.svg".into()),
                    true,
                    "History",
                )
                .tab("tab_off", "Notes", None, false, "Notes")
                .tools(2)
                .minimum_width(theme, scale, window)
        });
        cx.simulate_resize(gpui::size(minimum, px(120.0)));
        crate::test_support::redraw(cx);
        let header = cx.debug_bounds("panel_header").unwrap();
        assert!(
            cx.debug_bounds("tab_on_label").is_some(),
            "the selected label stays"
        );
        let last = cx.debug_bounds("tool_1").unwrap();
        assert!(last.right() <= header.right(), "{last:?} within {header:?}");
    }
}
