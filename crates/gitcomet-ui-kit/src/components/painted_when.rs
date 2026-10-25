use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

/// Lays `child` out every frame but prepaints and paints it only while
/// `visible` holds. `visible` is asked during prepaint, after any earlier
/// sibling has prepainted, so it can act on a measurement taken this frame
/// (a text's overflow, a scroll container's extent) without a frame of lag.
pub fn painted_when(visible: impl Fn() -> bool + 'static, child: impl IntoElement) -> PaintedWhen {
    PaintedWhen {
        child: Some(child.into_any_element()),
        visible: Box::new(visible),
    }
}

pub struct PaintedWhen {
    child: Option<AnyElement>,
    visible: Box<dyn Fn() -> bool>,
}

impl Element for PaintedWhen {
    type RequestLayoutState = AnyElement;
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("conditional child");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let visible = (self.visible)();
        if visible {
            child.prepaint(window, cx);
        }
        visible
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        visible: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *visible {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for PaintedWhen {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
