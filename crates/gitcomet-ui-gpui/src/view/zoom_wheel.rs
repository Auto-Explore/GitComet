//! Ctrl/Cmd+scroll zoom, shared by the window and the viewports that zoom
//! their own content.
use gpui::{App, DispatchPhase, Hitbox, Modifiers, ScrollWheelEvent, Window};

/// Whether a wheel with these modifiers zooms rather than scrolls.
pub(in crate::view) fn is_zoom_wheel(modifiers: &Modifiers) -> bool {
    (modifiers.secondary() || modifiers.control) && !modifiers.alt && !modifiers.function
}

/// Zooms a viewport on Ctrl/Cmd+scroll over `hitbox`. Capture phase runs
/// ahead of the window zoom, which is painted last; anything drawn over the
/// hitbox takes the hit test, and the window zoom applies there instead.
pub(in crate::view) fn on_zoom_wheel(
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
}
