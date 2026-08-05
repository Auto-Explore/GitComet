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
//! the next press, by [`PressGestureReset`]. Clearing it on the release itself
//! would be too early, because the reset runs in the capture phase and the
//! handlers that read it run in the bubble phase of that same event.
//!
//! Every window that renders [`crate::view::window_frame`] mounts the reset.
//! `focused_diff` renders its own root outside the frame; it hosts no text
//! input and no click-like release handler, so it neither claims nor reads.
//!
//! Not covered: context-menu entries (`view/panels/popover/context_menu.rs`)
//! activate on release *by design* — the menu opens on press and the pointer
//! drags onto the entry — so they cannot be guarded this way.

use gpui::{
    App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    MouseDownEvent, MouseMoveEvent, Pixels, Style, Window, px,
};

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

/// Zero-size element that installs the window-level claim reset.
///
/// Deliberately not `capture_any_mouse_down`: that is gated on the root's
/// hitbox, and the hit test stops at the first `occlude()`. A press inside a
/// centered prompt popover — which hosts text inputs and gets no dismiss scrim
/// — would never reach the root, and the claim would stick.
pub(crate) struct PressGestureReset;

impl gpui::IntoElement for PressGestureReset {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PressGestureReset {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // Capture phase, so the reset lands before the element under the
        // pointer claims the new press in the bubble phase.
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
}
