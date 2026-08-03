use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Stateful, div, px};

pub struct Tab {
    div: Stateful<Div>,
    selected: bool,
    end_slot: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl Tab {
    const END_TAB_SLOT_SIZE_PX: f32 = 14.0;
    /// Tab height inside the title bar; the difference to the bar height is
    /// the top inset that lets the active tab rise like a browser tab.
    const TAB_HEIGHT_PX: f32 = 34.0;
    /// Bottom padding matching the title bar's top inset. The tab is fused to
    /// the bar's bottom edge, so without this its label would sit below the
    /// bar midline that the title bar icons center on.
    const TAB_BOTTOM_FUSE_PAD_PX: f32 = 4.0;
    /// Tabs shrink no further than this before the strip scrolls.
    const TAB_MIN_WIDTH_PX: f32 = 96.0;
    /// Long repository names truncate rather than widening the tab past this.
    const TAB_MAX_WIDTH_PX: f32 = 180.0;

    /// Overlay an idle tab picks up on hover. Exposed so anything painted on
    /// top of a tab (the label fade) can flatten it into a matching color.
    pub fn hover_overlay(theme: AppTheme) -> gpui::Rgba {
        theme.hover_overlay()
    }

    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            div: div().id(id.clone()),
            selected: false,
            end_slot: None,
            children: Vec::new(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
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

    pub fn render(self, theme: AppTheme, ui_scale: impl Into<UiScale>) -> Stateful<Div> {
        let ui_scale = ui_scale.into();
        let scaled_px = |value| ui_scale.px(value);
        let text_color = if self.selected {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };
        let hover_bg = Self::hover_overlay(theme);
        let active_bg = theme.colors.active;

        let end_slot = div()
            .flex_none()
            .size(scaled_px(Self::END_TAB_SLOT_SIZE_PX))
            .flex()
            .items_center()
            .justify_center()
            .children(self.end_slot);

        // Browser-style tab: both states share the shape — inset from the bar
        // top, sitting on the bar's bottom edge, rounded top corners only.
        // The active tab fills with the content-strip color and carries a
        // top/side border so it reads as the front sheet; the bottom stays
        // open to fuse with the workspace below. Width is clamped: long repo
        // names truncate at the max, and tabs shrink no further than the min
        // before the strip starts scrolling.
        let mut base = self
            .div
            .group("tab")
            .h(scaled_px(Self::TAB_HEIGHT_PX))
            .min_w(scaled_px(Self::TAB_MIN_WIDTH_PX))
            .max_w(scaled_px(Self::TAB_MAX_WIDTH_PX))
            .mx(scaled_px(3.0))
            .px(scaled_px(10.0))
            .pb(scaled_px(Self::TAB_BOTTOM_FUSE_PAD_PX))
            .flex()
            .items_center()
            .gap_1()
            .rounded_tl(px(theme.radii.control))
            .rounded_tr(px(theme.radii.control))
            .text_color(text_color)
            .cursor_pointer()
            .children(self.children)
            .child(end_slot);

        if self.selected {
            base = base
                .bg(theme.colors.sidebar_bg)
                .border_t_1()
                .border_l_1()
                .border_r_1()
                .border_color(theme.colors.border);
        } else {
            base = base
                .bg(gpui::rgba(0x00000000))
                .hover(move |s| s.bg(hover_bg))
                .active(move |s| s.bg(active_bg));
        }

        base
    }
}
