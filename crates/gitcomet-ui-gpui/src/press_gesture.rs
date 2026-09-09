//! Ownership of the mouse press currently in flight.
//!
//! `div().on_click()` pairs press and release itself — gpui only remembers a
//! press that hit the element's own hitbox — so a click handler can never fire
//! for a release that began somewhere else. Hand-rolled `MouseUp` handlers get
//! no such pairing: they run for *any* release over their bounds, whatever the
//! press was doing. Dragging a text selection out of an input and letting go
//! over a commit row used to select that commit.
//!
//! So a gesture owner — text-input drag-selection, a resize handle, a scrollbar
//! thumb — claims the press in its own mouse-*down* handler with
//! [`claim_press`], and release handlers that cannot use `on_click` consult
//! [`is_press_claimed`] and stand down.
//!
//! The claim deliberately outlives the release: it is cleared at the *start* of
//! the next press, by [`install_reset`]. Clearing it on the release itself
//! would be too early, because the reset runs in the capture phase and the
//! handlers that read it run in the bubble phase of that same event.
//!
//! Every window that renders `view::window_frame` mounts the reset.
//! `focused_diff` renders its own root outside the frame; it hosts no text
//! input and no click-like release handler, so it neither claims nor reads.
//!
//! Not covered: context-menu entries (`view/panels/popover/context_menu.rs`)
//! activate on release *by design* — the menu opens on press and the pointer
//! drags onto the entry — so they cannot be guarded this way.

use gpui::{App, DispatchPhase, MouseDownEvent, MouseMoveEvent, Window};

/// Set while the press in flight belongs to an element that owns the whole
/// press → drag → release gesture.
#[derive(Default)]
struct PressGesture {
    claimed: bool,
}

impl gpui::Global for PressGesture {}

/// True when the release being handled belongs to another element's gesture.
pub(crate) fn is_press_claimed(cx: &App) -> bool {
    cx.try_global::<PressGesture>()
        .is_some_and(|state| state.claimed)
}

/// Claims the press in flight. Call from the gesture owner's own mouse-*down*
/// handler, unconditionally — a double-click that turns into a drag has to be
/// covered too.
pub(crate) fn claim_press(cx: &mut App) {
    set_claimed(true, cx);
}

fn set_claimed(claimed: bool, cx: &mut App) {
    if is_press_claimed(cx) != claimed {
        cx.set_global(PressGesture { claimed });
    }
}

/// Installs the claim resets. Called from the window frame's single root hook.
pub(crate) fn install_reset(window: &mut Window) {
    // Capture phase, so the reset lands before the element under the pointer
    // claims the new press in the bubble phase.
    window.on_mouse_event(|_event: &MouseDownEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture {
            set_claimed(false, cx);
        }
    });

    // A move with no button held means the gesture is definitively over.
    // Bounds any claim left stranded by a release the window never saw.
    window.on_mouse_event(|event: &MouseMoveEvent, phase, _window, cx| {
        if phase == DispatchPhase::Capture && !event.dragging() {
            set_claimed(false, cx);
        }
    });
}
