//! Bridges the ordinary control's interaction state to custom painting. The
//! tracker spans the whole control, so a graph node or badge sees the same
//! hover/press background even when the pointer is over a sibling label.
use super::interaction::{InteractionFeedback, InteractionState, InteractionStyle};
use gpui::{prelude::*, *};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub(crate) struct InteractionPaint {
    style: InteractionStyle,
    state: InteractionState,
    pointer: Rc<RefCell<Option<PointerState>>>,
}

struct PointerState {
    hitbox: Hitbox,
    pressed: Rc<Cell<bool>>,
}

impl InteractionPaint {
    pub(crate) fn new(style: InteractionStyle, state: InteractionState) -> Self {
        Self {
            style,
            state,
            pointer: Rc::default(),
        }
    }

    pub(crate) fn background(&self, surface: Rgba, window: &Window) -> Rgba {
        let pointer = self.pointer.borrow();
        let feedback = match pointer.as_ref() {
            Some(pointer) if pointer.pressed.get() => InteractionFeedback::Pressed,
            Some(pointer) if pointer.hitbox.is_hovered(window) => InteractionFeedback::Hovered,
            _ => InteractionFeedback::Resting,
        };
        self.style
            .resolved_background(surface, self.state, feedback)
    }

    pub(crate) fn apply(&self, control: Stateful<Div>) -> Stateful<Div> {
        self.style
            .clone()
            .apply(control, self.state)
            .relative()
            .child(div().absolute().inset_0().child(Tracker {
                paint: self.clone(),
            }))
    }
}

struct Tracker {
    paint: InteractionPaint,
}

impl IntoElement for Tracker {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Tracker {
    type RequestLayoutState = ();
    type PrepaintState = (Hitbox, Rc<Cell<bool>>);

    fn id(&self) -> Option<ElementId> {
        Some("interaction_paint_tracker".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(&StyleRefinement::default().size_full());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let pressed = window.with_element_state(
            id.expect("paint tracker has an id"),
            |pressed: Option<Rc<Cell<bool>>>, _| {
                let pressed = pressed.unwrap_or_default();
                (pressed.clone(), pressed)
            },
        );
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        *self.paint.pointer.borrow_mut() = Some(PointerState {
            hitbox: hitbox.clone(),
            pressed: pressed.clone(),
        });
        (hitbox, pressed)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        pointer: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let (hitbox, pressed) = pointer.clone();
        let pressed_on_down = pressed.clone();
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble
                && !window.default_prevented()
                && hitbox.is_hovered(window)
            {
                pressed_on_down.set(true);
                window.refresh();
            }
        });
        let pressed_on_up = pressed.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
            if phase == DispatchPhase::Capture && pressed_on_up.replace(false) {
                window.refresh();
            }
        });
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase == DispatchPhase::Capture && !event.dragging() && pressed.replace(false) {
                window.refresh();
            }
        });
    }
}
