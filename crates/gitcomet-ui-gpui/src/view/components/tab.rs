use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Stateful, div, px};

/// Tab height inside the title bar; the difference to the bar height is the
/// uncovered title chrome above a browser-style repository tab.
pub(super) const TAB_HEIGHT_PX: f32 = 34.0;

pub struct Tab {
    div: Stateful<Div>,
    selected: bool,
    horizontal_padding: Option<gpui::Pixels>,
    natural_width: Option<gpui::Pixels>,
    end_slot: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl Tab {
    /// Bottom padding matching the title bar's top inset. The tab is fused to
    /// the bar's bottom edge, so without this its label would sit below the
    /// bar midline that the title bar icons center on.
    const TAB_BOTTOM_FUSE_PAD_PX: f32 = 4.0;
    const TAB_HORIZONTAL_PADDING_PX: f32 = 10.0;
    /// Tabs shrink no further than this before the strip scrolls.
    const TAB_MIN_WIDTH_PX: f32 = 102.0;

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
            horizontal_padding: None,
            natural_width: None,
            end_slot: None,
            children: Vec::new(),
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Overrides the default side padding.
    pub fn horizontal_padding(mut self, padding: gpui::Pixels) -> Self {
        self.horizontal_padding = Some(padding);
        self
    }

    /// Lets the layout grow this tab evenly from its minimum width up to its
    /// natural width. Tabs with short labels reach their cap first, leaving
    /// the remaining space for longer labels without a measured-width redraw.
    pub fn responsive_width(mut self, natural_width: gpui::Pixels) -> Self {
        self.natural_width = Some(natural_width);
        self
    }

    /// Natural border-box width for `content_width`, including the tab's
    /// padding and side borders. The trailing action is an overlay and does
    /// not participate in this width.
    pub fn natural_width(
        content_width: gpui::Pixels,
        horizontal_padding: gpui::Pixels,
        ui_scale: impl Into<UiScale>,
    ) -> gpui::Pixels {
        let ui_scale = ui_scale.into();
        let chrome = horizontal_padding * 2.0
            // Left and right borders are physical one-pixel rules.
            + px(2.0);
        (content_width + chrome).max(ui_scale.px(Self::TAB_MIN_WIDTH_PX))
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
        let horizontal_padding = self
            .horizontal_padding
            .unwrap_or_else(|| scaled_px(Self::TAB_HORIZONTAL_PADDING_PX));
        let text_color = if self.selected {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };
        let hover_bg = Self::hover_overlay(theme);
        let active_bg = theme.colors.active;
        let natural_width = self.natural_width;

        let end_slot = self.end_slot.map(|slot| {
            div()
                .absolute()
                .top_0()
                .bottom(scaled_px(Self::TAB_BOTTOM_FUSE_PAD_PX))
                .right(horizontal_padding)
                .flex()
                .items_center()
                .justify_center()
                .child(slot)
        });

        // Browser-style tab: both states share the shape — inset from the bar
        // top, sitting on the bar's bottom edge, rounded top corners only.
        // Every tab reserves the active tab's top/side border so changing
        // selection never shifts its contents. The active tab colors that
        // border and fills with the content-strip color; the bottom stays open
        // to fuse with the workspace below. Tabs take their label's natural
        // width while the strip is roomy. Responsive tabs share free space
        // from the minimum floor upward, so shorter labels reach their natural
        // width before longer labels and the strip scrolls only at the floor.
        let mut base = self
            .div
            .group("tab")
            .h(scaled_px(TAB_HEIGHT_PX))
            .min_w(scaled_px(Self::TAB_MIN_WIDTH_PX))
            .mx(scaled_px(3.0))
            .px(horizontal_padding)
            .pb(scaled_px(Self::TAB_BOTTOM_FUSE_PAD_PX))
            .relative()
            .flex()
            .items_center()
            .rounded_tl(px(theme.radii.control))
            .rounded_tr(px(theme.radii.control))
            .border_t_1()
            .border_l_1()
            .border_r_1()
            .border_color(gpui::transparent_black())
            .text_color(text_color)
            .cursor_pointer()
            .block_mouse_except_scroll()
            .children(self.children)
            .children(end_slot);

        if let Some(width) = natural_width {
            // A zero flex basis makes every tab start at the same minimum.
            // Flex max-width then freezes short tabs as soon as they are full
            // and redistributes the remaining room among longer tabs.
            base = base.flex_1().max_w(width);
        }

        if self.selected {
            base = base
                .bg(theme.colors.sidebar_bg)
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
