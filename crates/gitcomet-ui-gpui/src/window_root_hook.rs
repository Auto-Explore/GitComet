//! A zero-size element that installs window-level mouse listeners from `paint`,
//! so they fire for presses anywhere rather than being gated on a hitbox.
//!
//! Deliberately not `capture_any_mouse_down`: that is gated on the root's
//! hitbox and the hit test stops at the first `occlude()`, so a press inside a
//! centered prompt popover would never reach it.
//!
//! Mount as the *first* child of the window frame. Capture-phase listeners run
//! in registration order, and a deep handler calling `stop_propagation` there
//! would skip the bubble phase entirely.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId, Pixels, Style,
    Window, px,
};

/// Runs `install` from `paint`, for every frame it is painted in, so whatever
/// listeners it registers are window-level rather than hitbox-gated.
pub(crate) struct WindowRootHook {
    install: fn(&mut Window),
}

impl WindowRootHook {
    pub(crate) fn new(install: fn(&mut Window)) -> Self {
        Self { install }
    }
}

impl gpui::IntoElement for WindowRootHook {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WindowRootHook {
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
        (self.install)(window);
    }
}
