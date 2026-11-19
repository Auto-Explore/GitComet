use super::*;
use gpui::prelude::*;

/// Adds an explicit, persistent model to an otherwise implicit GPUI scroller.
pub struct TrackedElasticScroll {
    id: ElementId,
    surface: Option<gpui::Stateful<gpui::Div>>,
    axis: ScrollbarAxis,
}

impl TrackedElasticScroll {
    pub(super) fn new(
        id: ElementId,
        surface: gpui::Stateful<gpui::Div>,
        axis: ScrollbarAxis,
    ) -> Self {
        Self {
            id,
            surface: Some(surface),
            axis,
        }
    }
}

impl IntoElement for TrackedElasticScroll {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TrackedElasticScroll {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let scroll = window.with_element_state(id.expect("tracked scroll id"), |scroll, _| {
            let scroll: ScrollHandle = scroll.unwrap_or_default();
            (scroll.clone(), scroll)
        });
        let mut surface = self.surface.take().expect("tracked scroll surface");
        if ElasticScroll::uses_precise_wheel_routing() {
            surface.style().restrict_scroll_to_axis = Some(self.axis == ScrollbarAxis::Vertical);
        }
        let surface = surface.track_scroll(&scroll);
        let mut child = match self.axis {
            ScrollbarAxis::Vertical => {
                ElasticScroll::new("elastic", scroll, surface).vertical_only()
            }
            ScrollbarAxis::Horizontal => ElasticScroll::horizontal("elastic", scroll, surface),
        }
        .into_any_element();
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}
