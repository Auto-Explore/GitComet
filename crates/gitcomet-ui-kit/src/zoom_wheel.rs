//! Ctrl/Cmd+scroll zoom, shared by the window and the viewports that zoom
//! their own content.
use gpui::{
    App, DispatchPhase, Hitbox, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, Window,
};

/// Smooth-scroll distance that makes one zoom step, like one wheel notch.
const SMOOTH_STEP_PX: f32 = 40.0;

/// Whether a wheel with these modifiers zooms rather than scrolls.
pub fn is_zoom_wheel(modifiers: &Modifiers) -> bool {
    (modifiers.secondary() || modifiers.control) && !modifiers.alt && !modifiers.function
}

/// Zooms a viewport on Ctrl/Cmd+scroll over `hitbox`. Capture phase runs
/// ahead of the window zoom, which is painted last; anything drawn over the
/// hitbox takes the hit test, and the window zoom applies there instead.
pub fn on_zoom_wheel(
    window: &mut Window,
    hitbox: Hitbox,
    zoom: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
) {
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Capture
            || !is_zoom_wheel(&event.modifiers)
            || !hitbox.should_handle_scroll(window)
        {
            return;
        }
        zoom(event, window, cx);
        cx.stop_propagation();
    });
}

/// Which way one zoom step goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZoomStep {
    In,
    Out,
}

/// Ctrl/Cmd+scroll distance not yet spent on a zoom step, in the unit the
/// wheel reported it.
#[derive(Clone, Copy, Debug)]
enum Unspent {
    Lines(f32),
    Pixels(f32),
}

/// Gathers Ctrl/Cmd+scroll into zoom steps: one per line of a notched wheel or
/// per 40 px of smooth scrolling, so a trackpad gesture does not run through
/// every level.
#[derive(Clone, Copy, Debug)]
pub struct ZoomWheel(Unspent);

impl Default for ZoomWheel {
    fn default() -> Self {
        Self(Unspent::Pixels(0.0))
    }
}

impl ZoomWheel {
    pub fn step(&mut self, event: &ScrollWheelEvent) -> Option<ZoomStep> {
        // A distance in the other unit never adds up with this one.
        let (delta, unspent, step) = match (event.delta, self.0) {
            (ScrollDelta::Lines(lines), Unspent::Lines(unspent)) => (lines.y, unspent, 1.0),
            (ScrollDelta::Lines(lines), Unspent::Pixels(_)) => (lines.y, 0.0, 1.0),
            (ScrollDelta::Pixels(pixels), Unspent::Pixels(unspent)) => {
                (f32::from(pixels.y), unspent, SMOOTH_STEP_PX)
            }
            (ScrollDelta::Pixels(pixels), Unspent::Lines(_)) => {
                (f32::from(pixels.y), 0.0, SMOOTH_STEP_PX)
            }
        };
        let mut total = delta;
        if event.touch_phase != TouchPhase::Started && delta * unspent >= 0.0 {
            total += unspent;
        }
        let zoom = (total.abs() >= step).then_some(if total > 0.0 {
            ZoomStep::In
        } else {
            ZoomStep::Out
        });
        if zoom.is_some() {
            total = 0.0;
        }
        self.0 = match event.delta {
            ScrollDelta::Lines(_) => Unspent::Lines(total),
            ScrollDelta::Pixels(_) => Unspent::Pixels(total),
        };
        zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window and every viewport agree on what zooms: Alt or Fn with
    /// Ctrl/Cmd is not a zoom anywhere.
    #[test]
    fn ctrl_or_cmd_zooms_unless_alt_or_fn() {
        let modifiers = |control, platform, alt, function, shift| Modifiers {
            control,
            platform,
            alt,
            function,
            shift,
        };
        assert!(is_zoom_wheel(&modifiers(true, false, false, false, false)));
        assert!(is_zoom_wheel(&modifiers(true, false, false, false, true)));
        assert!(is_zoom_wheel(&Modifiers::secondary_key()));
        assert!(!is_zoom_wheel(&modifiers(true, false, true, false, false)));
        assert!(!is_zoom_wheel(&modifiers(true, false, false, true, false)));
        assert!(!is_zoom_wheel(&modifiers(false, false, false, false, true)));
        assert!(!is_zoom_wheel(&Modifiers::default()));
    }

    fn wheel(delta: ScrollDelta) -> ScrollWheelEvent {
        ScrollWheelEvent {
            delta,
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        }
    }

    #[test]
    fn a_step_takes_one_notch_or_40_px_and_units_never_add_up() {
        let pixels = |y: f32| wheel(ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(y))));
        let lines = |y: f32| wheel(ScrollDelta::Lines(gpui::point(0.0, y)));
        let mut zoom = ZoomWheel::default();
        assert_eq!(zoom.step(&pixels(30.0)), None);
        assert_eq!(zoom.step(&pixels(10.0)), Some(ZoomStep::In));
        assert_eq!(zoom.step(&lines(-1.0)), Some(ZoomStep::Out));
        // 30 px left over from a trackpad must not count toward a notch.
        assert_eq!(zoom.step(&pixels(30.0)), None);
        assert_eq!(zoom.step(&lines(0.25)), None);
    }
}
