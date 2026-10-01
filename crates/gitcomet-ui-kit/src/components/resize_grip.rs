use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    App, Bounds, DispatchPhase, Div, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    SharedString, Window, canvas, div, fill, point, px, size,
};

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

/// Hovered grip tint: a text-alpha overlay because a surface-level hover tint
/// all but disappears on the elevated chrome band the dividers sit on. Alphas
/// are in scrollbar-thumb territory so a 4px pill still reads against both the
/// canvas and surrounding chrome.
/// The grip's hover and drag colour.
pub fn resize_grip_hover_tint(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.foreground.primary,
        if theme.is_dark { 0.34 } else { 0.30 },
    )
}

/// Dragged grip tint: the accent color, so an in-flight resize is unmistakable
/// and clearly distinct from mere hover.
fn drag_tint(theme: AppTheme) -> gpui::Rgba {
    theme.colors.accent.foreground
}

fn group_is_hovered(group: &str, window: &Window) -> bool {
    let hit_test = window.mouse_hit_test();
    hit_test.iter_hovered().any(|id| {
        id.is_hovered(window)
            && hit_test
                .entry(id)
                .is_some_and(|entry| entry.tags().iter().any(|tag| tag.as_ref() == group))
    })
}

fn show_grip(group: &str, dragging: bool, window: &Window, cx: &App) -> bool {
    dragging
        || (!crate::press_gesture::pointer_is_down(window, cx)
            && !cx.has_active_drag()
            && group_is_hovered(group, window))
}

/// Hover/drag visual for a resize divider: the whole strip stays interactive
/// (cursor, drag, mouse handlers live on the strip), but only this centered
/// segment tints on hover. The strip itself must carry `.group(group)` and no
/// hover/active background of its own. Feedback is resolved during paint so a
/// scrollbar or text-selection drag suppresses even cached grips. Insert as a
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

    let grip = canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let shown = show_grip(group.as_ref(), dragging, window, cx);
            if shown {
                let extent = match axis {
                    ResizeGripAxis::Vertical => size(
                        scale.px(GRIP_THICKNESS_PX),
                        scale.px(GRIP_LEN_PX).min(bounds.size.height * 0.8),
                    ),
                    ResizeGripAxis::Horizontal => size(
                        scale.px(GRIP_LEN_PX).min(bounds.size.width * 0.8),
                        scale.px(GRIP_THICKNESS_PX),
                    ),
                };
                let segment = Bounds::new(
                    point(
                        bounds.center().x - extent.width / 2.0,
                        bounds.center().y - extent.height / 2.0,
                    ),
                    extent,
                );
                // Canvas quads bypass the radius clamping of styled divs. The
                // theme's pill radius can otherwise make this narrow grip invisible.
                let radii =
                    gpui::Corners::all(px(theme.radii.pill)).clamp_radii_for_quad_size(extent);
                window.paint_quad(
                    fill(
                        segment,
                        if dragging {
                            drag_tint(theme)
                        } else {
                            resize_grip_hover_tint(theme)
                        },
                    )
                    .corner_radii(radii),
                );
            }
            // Dirty only this grip's owning view. A window refresh also throws
            // away unrelated cached history and diff views on every click.
            let view = window.current_view();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == MouseButton::Left
                    && shown
                    && !dragging
                {
                    cx.notify(view);
                }
            });
            let release_group = group.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    let released = dragging
                        || (!cx.has_active_drag() && group_is_hovered(&release_group, window));
                    if released != shown {
                        cx.notify(view);
                    }
                }
            });
            let group = group.clone();
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && show_grip(group.as_ref(), dragging, window, cx) != shown
                {
                    cx.notify(view);
                }
            });
        },
    )
    .absolute()
    .inset_0();

    div().relative().size_full().children(hairline).child(grip)
}
