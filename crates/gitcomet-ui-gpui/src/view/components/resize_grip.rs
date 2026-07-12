use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, SharedString, div, px, relative};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeGripAxis {
    /// A vertical divider strip (dragged left/right).
    Vertical,
    /// A horizontal divider strip (dragged up/down).
    Horizontal,
}

/// Length of the tinted middle segment along the divider.
const GRIP_LEN_PX: f32 = 44.0;
/// Thickness of the tinted middle segment across the divider.
const GRIP_THICKNESS_PX: f32 = 4.0;

/// Hover/drag visual for a resize divider: the whole strip stays interactive
/// (cursor, drag, mouse handlers live on the strip), but only this centered
/// segment tints on hover — via `group_hover`, so the strip itself must carry
/// `.group(group)` and no hover/active background of its own. Insert as a
/// full-size child of the strip; `idle_line` draws the divider's always-on
/// hairline when the divider separates two visible regions.
pub fn resize_grip(
    theme: AppTheme,
    scale: impl Into<UiScale>,
    group: impl Into<SharedString>,
    axis: ResizeGripAxis,
    dragging: bool,
    idle_line: Option<gpui::Rgba>,
) -> Div {
    let scale = scale.into();
    let group: SharedString = group.into();
    let layer = || {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
    };

    let hairline = idle_line.map(|color| {
        let line = match axis {
            ResizeGripAxis::Vertical => div().w(px(1.0)).h_full(),
            ResizeGripAxis::Horizontal => div().h(px(1.0)).w_full(),
        };
        layer().child(line.bg(color))
    });

    let segment = match axis {
        ResizeGripAxis::Vertical => div()
            .w(scale.px(GRIP_THICKNESS_PX))
            .h(scale.px(GRIP_LEN_PX))
            // Short strips (e.g. table headers) keep the segment inside.
            .max_h(relative(0.8)),
        ResizeGripAxis::Horizontal => div()
            .h(scale.px(GRIP_THICKNESS_PX))
            .w(scale.px(GRIP_LEN_PX))
            .max_w(relative(0.8)),
    };
    let grip = layer().child(
        segment
            .flex_none()
            .rounded(px(theme.radii.pill))
            .when(dragging, |segment| segment.bg(theme.colors.active))
            .when(!dragging, |segment| {
                segment.group_hover(group.clone(), |s| s.bg(theme.colors.hover))
            }),
    );

    div().relative().size_full().children(hairline).child(grip)
}
