//! Window placement: saved frames, displays, and cascading restored windows.

use super::*;

pub(super) fn frame_from_bounds(bounds: Bounds<Pixels>) -> session::SavedWindowFrame {
    let x: f32 = bounds.origin.x.into();
    let y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    session::SavedWindowFrame {
        x: x.round() as i32,
        y: y.round() as i32,
        width: width.round().max(1.0) as u32,
        height: height.round().max(1.0) as u32,
    }
}

pub(super) fn bounds_from_frame(frame: session::SavedWindowFrame) -> Bounds<Pixels> {
    Bounds::new(
        point(px(frame.x as f32), px(frame.y as f32)),
        size(px(frame.width as f32), px(frame.height as f32)),
    )
}

pub(super) fn captured_normal_frame(
    state: session::SavedWindowState,
    reported_frame: session::SavedWindowFrame,
    previous: Option<&session::PortableWindowPlacement>,
) -> session::SavedWindowFrame {
    match state {
        session::SavedWindowState::Windowed => reported_frame,
        session::SavedWindowState::Maximized | session::SavedWindowState::Fullscreen => previous
            .and_then(|placement| placement.normal_frame)
            .unwrap_or(reported_frame),
    }
}

pub(super) fn display_uuid(display: &dyn gpui::PlatformDisplay) -> Option<String> {
    display.uuid().ok().map(|uuid| uuid.to_string())
}

pub(super) fn restored_workspace_window_bounds(
    placement: &session::PortableWindowPlacement,
    fallback_size: Size<Pixels>,
    min_size: Size<Pixels>,
    cx: &mut App,
) -> (WindowBounds, Option<DisplayId>) {
    let displays = cx.displays();
    let display = placement
        .display_id
        .as_deref()
        .and_then(|wanted| {
            displays
                .iter()
                .find(|display| display_uuid(display.as_ref()).as_deref() == Some(wanted))
                .cloned()
        })
        .or_else(|| cx.primary_display())
        .or_else(|| displays.first().cloned());
    let display_id = display.as_ref().map(|display| display.id());

    let Some(saved_frame) = placement.normal_frame else {
        return (
            WindowBounds::Windowed(Bounds::centered(display_id, fallback_size, cx)),
            display_id,
        );
    };
    let current_visible = display
        .as_ref()
        .map(|display| frame_from_bounds(display.visible_bounds()))
        .unwrap_or(saved_frame);
    let minimum_width: f32 = min_size.width.into();
    let minimum_height: f32 = min_size.height.into();
    let frame = crate::workspaces::rebase_window_frame(
        saved_frame,
        placement.captured_visible_frame,
        current_visible,
        minimum_width.round().max(1.0) as u32,
        minimum_height.round().max(1.0) as u32,
    );
    let occupied_frames = cx
        .windows()
        .into_iter()
        .filter_map(|handle| {
            handle
                .update(cx, |_root, window, cx| {
                    let window_display = window.display(cx).map(|display| display.id());
                    (
                        window_display,
                        frame_from_bounds(window.window_bounds().get_bounds()),
                    )
                })
                .ok()
        })
        .filter_map(|(window_display, frame)| (window_display == display_id).then_some(frame))
        .collect::<Vec<_>>();
    let frame = cascade_colliding_window_frame(frame, current_visible, &occupied_frames);
    let bounds = bounds_from_frame(frame);
    let bounds = match placement.state {
        session::SavedWindowState::Windowed => WindowBounds::Windowed(bounds),
        session::SavedWindowState::Maximized => WindowBounds::Maximized(bounds),
        session::SavedWindowState::Fullscreen => WindowBounds::Fullscreen(bounds),
    };
    (bounds, display_id)
}

pub(super) fn cascade_colliding_window_frame(
    frame: session::SavedWindowFrame,
    visible: session::SavedWindowFrame,
    occupied: &[session::SavedWindowFrame],
) -> session::SavedWindowFrame {
    let collides = |candidate: session::SavedWindowFrame| {
        occupied
            .iter()
            .any(|other| other.x == candidate.x && other.y == candidate.y)
    };
    if !collides(frame) {
        return frame;
    }

    let min_x = visible.x;
    let min_y = visible.y;
    let max_x = visible
        .x
        .saturating_add(visible.width.saturating_sub(frame.width) as i32);
    let max_y = visible
        .y
        .saturating_add(visible.height.saturating_sub(frame.height) as i32);
    const CASCADE_OFFSET: i32 = 28;

    for step in 1_i32..=64 {
        let delta = CASCADE_OFFSET.saturating_mul(step);
        let offsets = [
            (delta, delta),
            (-delta, delta),
            (delta, -delta),
            (-delta, -delta),
            (delta, 0),
            (0, delta),
            (-delta, 0),
            (0, -delta),
        ];
        for (dx, dy) in offsets {
            let candidate = session::SavedWindowFrame {
                x: frame.x.saturating_add(dx).clamp(min_x, max_x),
                y: frame.y.saturating_add(dy).clamp(min_y, max_y),
                ..frame
            };
            if candidate != frame && !collides(candidate) {
                return candidate;
            }
        }
    }

    // A display with no travel (for example a window as large as its usable
    // area) cannot be cascaded without violating the on-screen clamp.
    frame
}

pub(crate) fn capture_window_placement<C>(
    window: &Window,
    cx: &C,
    previous: Option<&session::PortableWindowPlacement>,
) -> session::PortableWindowPlacement
where
    C: std::borrow::Borrow<App>,
{
    let app = cx.borrow();
    let display = window.display(app);
    let state = if window.is_fullscreen() {
        session::SavedWindowState::Fullscreen
    } else if window.is_maximized() {
        session::SavedWindowState::Maximized
    } else {
        session::SavedWindowState::Windowed
    };
    let reported_frame = frame_from_bounds(window.window_bounds().get_bounds());
    let normal_frame = captured_normal_frame(state, reported_frame, previous);
    let tiled = match window.window_decorations() {
        gpui::Decorations::Client { tiling }
            if tiling.top || tiling.left || tiling.right || tiling.bottom =>
        {
            Some(session::SavedWindowTiling {
                top: tiling.top,
                left: tiling.left,
                right: tiling.right,
                bottom: tiling.bottom,
            })
        }
        _ => None,
    };
    session::PortableWindowPlacement {
        normal_frame: Some(normal_frame),
        captured_visible_frame: display
            .as_ref()
            .map(|display| frame_from_bounds(display.visible_bounds())),
        display_id: display
            .as_ref()
            .and_then(|display| display_uuid(display.as_ref())),
        state,
        tiled,
    }
}
