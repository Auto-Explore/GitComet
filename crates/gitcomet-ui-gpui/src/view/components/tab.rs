use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, ElementId, IntoElement, Pixels, Stateful, Window, div, point, px};

/// Tab height inside the title bar; the difference to the bar height is the
/// uncovered title chrome above a browser-style repository tab.
pub(super) const TAB_HEIGHT_PX: f32 = 34.0;

/// Paints the selected tab as one continuous browser-style silhouette. The
/// lower curves are deliberately concave: each starts on a vertical tab edge
/// one radius above the rail and ends one radius outside the tab on the rail.
fn selected_tab_shape(
    fill: gpui::Rgba,
    border: gpui::Rgba,
    top_radius: Pixels,
    bottom_curve_radius: Pixels,
) -> AnyElement {
    let paint = move |bounds: gpui::Bounds<Pixels>, window: &mut Window| {
        use gpui::PathBuilder;

        // Cubic approximation of a quarter circle.
        const K: f32 = 0.552_284_7;

        let body_left = bounds.left() + bottom_curve_radius;
        let body_right = bounds.right() - bottom_curve_radius;
        let top = bounds.top();
        let bottom = bounds.bottom();
        let top_radius = top_radius
            .min((body_right - body_left) * 0.5)
            .min(bounds.size.height);
        let bottom_curve_radius = bottom_curve_radius.min(bounds.size.height);
        let top_k = top_radius * K;
        let bottom_k = bottom_curve_radius * K;

        let mut silhouette = PathBuilder::fill();
        silhouette.move_to(point(body_left + top_radius, top));
        silhouette.line_to(point(body_right - top_radius, top));
        silhouette.cubic_bezier_to(
            point(body_right, top + top_radius),
            point(body_right - top_radius + top_k, top),
            point(body_right, top + top_radius - top_k),
        );
        silhouette.line_to(point(body_right, bottom - bottom_curve_radius));
        silhouette.cubic_bezier_to(
            point(bounds.right(), bottom),
            point(body_right, bottom - bottom_curve_radius + bottom_k),
            point(bounds.right() - bottom_k, bottom),
        );
        silhouette.line_to(point(bounds.left(), bottom));
        silhouette.cubic_bezier_to(
            point(body_left, bottom - bottom_curve_radius),
            point(bounds.left() + bottom_k, bottom),
            point(body_left, bottom - bottom_curve_radius + bottom_k),
        );
        silhouette.line_to(point(body_left, top + top_radius));
        silhouette.cubic_bezier_to(
            point(body_left + top_radius, top),
            point(body_left, top + top_radius - top_k),
            point(body_left + top_radius - top_k, top),
        );
        silhouette.close();
        if let Ok(path) = silhouette.build() {
            window.paint_path(path, fill);
        }

        // Leave the baseline open so the selected tab remains fused to the
        // workspace surface below it. Unlike filled layout borders, a stroked
        // path is centered on its coordinates. Snap those coordinates to
        // physical pixel centers and use exactly one physical pixel of width;
        // otherwise the straight portions straddle two pixels and look soft.
        let stroke_width = px(1.0 / window.scale_factor().max(1.0));
        let half_stroke = stroke_width * 0.5;
        let outline_left = window.pixel_snap(bounds.left()) + half_stroke;
        let outline_right = window.pixel_snap(bounds.right()) - half_stroke;
        let outline_body_left = window.pixel_snap(body_left) + half_stroke;
        let outline_body_right = window.pixel_snap(body_right) - half_stroke;
        let outline_top = window.pixel_snap(top) + half_stroke;
        let outline_bottom = window.pixel_snap(bottom) - half_stroke;

        let mut outline = PathBuilder::stroke(stroke_width);
        outline.move_to(point(outline_left, outline_bottom));
        outline.cubic_bezier_to(
            point(outline_body_left, outline_bottom - bottom_curve_radius),
            point(outline_left + bottom_k, outline_bottom),
            point(
                outline_body_left,
                outline_bottom - bottom_curve_radius + bottom_k,
            ),
        );
        outline.line_to(point(outline_body_left, outline_top + top_radius));
        outline.cubic_bezier_to(
            point(outline_body_left + top_radius, outline_top),
            point(outline_body_left, outline_top + top_radius - top_k),
            point(outline_body_left + top_radius - top_k, outline_top),
        );
        outline.line_to(point(outline_body_right - top_radius, outline_top));
        outline.cubic_bezier_to(
            point(outline_body_right, outline_top + top_radius),
            point(outline_body_right - top_radius + top_k, outline_top),
            point(outline_body_right, outline_top + top_radius - top_k),
        );
        outline.line_to(point(
            outline_body_right,
            outline_bottom - bottom_curve_radius,
        ));
        outline.cubic_bezier_to(
            point(outline_right, outline_bottom),
            point(
                outline_body_right,
                outline_bottom - bottom_curve_radius + bottom_k,
            ),
            point(outline_right - bottom_k, outline_bottom),
        );
        if let Ok(path) = outline.build() {
            window.paint_path(path, border);
        }
    };

    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(-bottom_curve_radius)
        .right(-bottom_curve_radius)
        .child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, window, _| paint(bounds, window),
            )
            .size_full(),
        )
        .into_any_element()
}

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
    const TAB_TOP_PADDING_PX: f32 = 2.0;
    const TAB_HORIZONTAL_PADDING_PX: f32 = 10.0;
    /// Half the gutter between two neighbouring tabs. Public so anything
    /// painted into that shared gap (the idle-tab separator) tracks it.
    pub const HORIZONTAL_MARGIN_PX: f32 = 3.0;
    /// The selected tab's lower curves flare this far past its box on each
    /// side. Kept at the full gutter width so the curve lands exactly on the
    /// neighbouring tab's edge instead of stopping short or running under it.
    const TAB_BOTTOM_CURVE_RADIUS_PX: f32 = Self::HORIZONTAL_MARGIN_PX * 2.0;
    /// Tabs shrink no further than this before the strip scrolls.
    const TAB_MIN_WIDTH_PX: f32 = 102.0;

    /// Overlay an idle tab picks up on hover. Exposed so anything painted on
    /// top of a tab (the label fade) can flatten it into a matching color.
    pub fn hover_overlay(theme: AppTheme) -> gpui::Rgba {
        theme.hover_overlay()
    }

    /// Stronger shared rule for the active tab outline and the workspace edge
    /// it joins. Pulling the theme border toward its text color increases
    /// contrast in the correct direction for both dark and light themes.
    pub fn outline_color(theme: AppTheme) -> gpui::Rgba {
        let amount = if theme.is_dark { 0.18 } else { 0.14 };
        let border = theme.colors.border;
        let text = theme.colors.text;
        gpui::Rgba {
            r: border.r + (text.r - border.r) * amount,
            g: border.g + (text.g - border.g) * amount,
            b: border.b + (text.b - border.b) * amount,
            a: border.a + (text.a - border.a) * amount,
        }
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
        let natural_width = self.natural_width;
        let selected_shape = self.selected.then(|| {
            selected_tab_shape(
                theme.colors.sidebar_bg,
                Self::outline_color(theme),
                px(theme.radii.control),
                scaled_px(Self::TAB_BOTTOM_CURVE_RADIUS_PX),
            )
        });

        let end_slot = self.end_slot.map(|slot| {
            div()
                .absolute()
                .top(scaled_px(Self::TAB_TOP_PADDING_PX))
                .bottom(scaled_px(Self::TAB_BOTTOM_FUSE_PAD_PX))
                .right(horizontal_padding)
                .flex()
                .items_center()
                .justify_center()
                .child(slot)
        });

        // Browser-style tab: both states are inset from the bar top and sit on
        // its bottom edge. The selected state adds concave lower curves which
        // flare across the gutter and meet the rail at the next tab edge.
        // Every tab reserves transparent top/side borders so changing
        // selection never shifts its contents. The selected-state canvas
        // supplies the visible outline and content-strip fill while leaving
        // the bottom open to fuse with the workspace below. Tabs take their
        // label's natural width while the strip is roomy. Responsive tabs
        // share free space from the minimum floor upward, so shorter labels
        // reach their natural width before longer labels and the strip scrolls
        // only at the floor.
        let mut base = self
            .div
            .group("tab")
            .h(scaled_px(TAB_HEIGHT_PX))
            .min_w(scaled_px(Self::TAB_MIN_WIDTH_PX))
            .mx(scaled_px(Self::HORIZONTAL_MARGIN_PX))
            .px(horizontal_padding)
            .pt(scaled_px(Self::TAB_TOP_PADDING_PX))
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
            .children(selected_shape)
            .children(self.children)
            .children(end_slot);

        if let Some(width) = natural_width {
            // A zero flex basis makes every tab start at the same minimum.
            // Flex max-width then freezes short tabs as soon as they are full
            // and redistributes the remaining room among longer tabs.
            base = base.flex_1().max_w(width);
        }

        if !self.selected {
            let hover_text = theme.colors.text;
            base = base.hover(move |s| s.text_color(hover_text));
        }

        base
    }
}
