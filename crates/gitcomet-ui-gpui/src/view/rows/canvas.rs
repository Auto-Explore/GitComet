use super::*;
use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, Pixels,
    Style, StyleRefinement, Styled, Window,
};

type PrepaintCallback<T> = Box<dyn FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T>;
type PaintCallback<T> = Box<dyn FnOnce(Bounds<Pixels>, T, &mut Window, &mut App)>;

pub(super) fn take_once_or_debug<T>(slot: &mut Option<T>, context: &'static str) -> Option<T> {
    let value = slot.take();
    debug_assert!(value.is_some(), "{context}");
    value
}

pub(super) fn keyed_canvas<T>(
    id: impl Into<ElementId>,
    prepaint: impl 'static + FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T,
    paint: impl 'static + FnOnce(Bounds<Pixels>, T, &mut Window, &mut App),
) -> KeyedCanvas<T> {
    KeyedCanvas {
        id: id.into(),
        prepaint: Some(Box::new(prepaint)),
        paint: Some(Box::new(paint)),
        style: StyleRefinement::default(),
    }
}

pub(super) struct KeyedCanvas<T> {
    id: ElementId,
    prepaint: Option<PrepaintCallback<T>>,
    paint: Option<PaintCallback<T>>,
    style: StyleRefinement,
}

impl<T: 'static> IntoElement for KeyedCanvas<T> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: 'static> Element for KeyedCanvas<T> {
    type RequestLayoutState = Style;
    type PrepaintState = Option<T>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<T> {
        let prepaint =
            take_once_or_debug(&mut self.prepaint, "KeyedCanvas::prepaint callback missing")?;
        Some(prepaint(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepaint_state) =
            take_once_or_debug(prepaint, "KeyedCanvas::paint called without prepaint state")
        else {
            return;
        };
        let Some(paint) =
            take_once_or_debug(&mut self.paint, "KeyedCanvas::paint callback missing")
        else {
            return;
        };

        style.paint(bounds, window, cx, move |window, cx| {
            paint(bounds, prepaint_state, window, cx)
        });
    }
}

impl<T> Styled for KeyedCanvas<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
