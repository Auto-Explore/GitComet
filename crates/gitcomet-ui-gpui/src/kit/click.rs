//! Completed pointer clicks shared by elements, canvases and text subtargets.
//! GPUI pairs element clicks; this layer also gives the innermost control sole
//! ownership and rejects mismatched buttons. Canvas targets use the same gate.
use gpui::{prelude::*, *};

/// A text subtarget can observe a press without taking selection away from
/// its editor. Both endpoints must identify the same link, and selection
/// movement cancels it. This also works when the editor stops bubbling down.
pub(crate) struct SubtargetClick<T> {
    pending: Option<(T, MouseDownEvent)>,
}

impl<T> Default for SubtargetClick<T> {
    fn default() -> Self {
        Self { pending: None }
    }
}

impl<T: PartialEq> SubtargetClick<T> {
    pub(crate) fn press(&mut self, target: Option<T>, event: &MouseDownEvent) {
        self.pending = target
            .filter(|_| {
                event.button == MouseButton::Left && event.click_count == 1 && !event.first_mouse
            })
            .map(|target| (target, event.clone()));
    }

    pub(crate) fn moved(&mut self, event: &MouseMoveEvent) {
        if !event.dragging()
            || self
                .pending
                .as_ref()
                .is_some_and(|(_, down)| (event.position - down.position).magnitude() > 2.0)
        {
            self.pending = None;
        }
    }

    pub(crate) fn release(
        &mut self,
        target: Option<&T>,
        event: &MouseUpEvent,
    ) -> Option<ClickEvent> {
        let (origin, down) = self.pending.take()?;
        if target != Some(&origin) || down.button != event.button {
            return None;
        }
        Some(ClickEvent::Mouse(MouseClickEvent {
            down,
            up: event.clone(),
        }))
    }
}

#[derive(Default)]
struct ClickOrigin {
    pending: Option<PendingClick>,
}

struct PendingClick {
    window: WindowId,
    target: ClickTarget,
    down: MouseDownEvent,
    consumed: bool,
    released_on: Option<ClickTarget>,
    cleanup_scheduled: bool,
}

#[derive(Clone, PartialEq)]
enum ClickTarget {
    Element(GlobalElementId),
    Canvas(ElementId),
}

impl Global for ClickOrigin {}

pub(crate) fn reset(cx: &mut App) {
    cx.set_global(ClickOrigin::default());
}

pub(crate) fn is_pending(cx: &App) -> bool {
    cx.try_global::<ClickOrigin>()
        .is_some_and(|state| state.pending.is_some())
}

fn begin(target: ClickTarget, down: &MouseDownEvent, window: &Window, cx: &mut App) {
    if is_pending(cx) {
        return;
    }
    cx.set_global(ClickOrigin {
        pending: Some(PendingClick {
            window: window.window_handle().window_id(),
            target,
            down: down.clone(),
            consumed: down.first_mouse,
            released_on: None,
            cleanup_scheduled: false,
        }),
    });
}

/// Capture visits outer controls first. The innermost hovered control replaces
/// the release target, even if it never received the original press.
fn release_over(target: Option<ClickTarget>, cx: &mut App) {
    let Some(state) = cx.try_global::<ClickOrigin>() else {
        return;
    };
    let Some(pending) = state.pending.as_ref() else {
        return;
    };
    let needs_cleanup = !pending.cleanup_scheduled;
    let pending = cx.global_mut::<ClickOrigin>().pending.as_mut().unwrap();
    if target.is_some() {
        pending.released_on = target;
    }
    pending.cleanup_scheduled = true;
    if needs_cleanup {
        // Keep ownership until every bubble listener has observed this release.
        // An outside release must also cancel canvas and other custom targets.
        cx.defer(reset);
    }
}

fn complete(
    target: &ClickTarget,
    up: &MouseUpEvent,
    window: &Window,
    cx: &mut App,
) -> Option<ClickEvent> {
    let pending = cx.try_global::<ClickOrigin>()?.pending.as_ref()?;
    if pending.consumed
        || pending.window != window.window_handle().window_id()
        || &pending.target != target
    {
        return None;
    }
    let down = pending.down.clone();
    let same_target = pending.released_on.as_ref() == Some(target);
    cx.global_mut::<ClickOrigin>().pending.as_mut()?.consumed = true;
    if !same_target || down.button != up.button || cx.has_active_drag() {
        return None;
    }
    Some(ClickEvent::Mouse(MouseClickEvent {
        down,
        up: up.clone(),
    }))
}

/// Install native pairing without changing focus or keyboard policy.
pub(crate) fn on_click<E: Element + InteractiveElement>(
    mut control: Stateful<E>,
    button: MouseButton,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<E> {
    let target = std::rc::Rc::new(std::cell::RefCell::new(None));
    let rendered_target = target.clone();
    control.interactivity().on_prepaint(move |_, window, _| {
        // Local ids are reusable in different subtrees. Capture the full path
        // during layout, so even identically named nested actions stay distinct.
        window.with_global_id("gitcomet-click".into(), |id, _| {
            *rendered_target.borrow_mut() = Some(ClickTarget::Element(id.clone()));
        });
    });
    let press_target = target.clone();
    let release_target = target.clone();
    control = control
        .on_mouse_down_all(|_, phase, _, _, cx| {
            if phase == DispatchPhase::Capture {
                reset(cx);
            }
        })
        .on_mouse_move(|event, _, cx| {
            if !event.dragging() {
                reset(cx);
            }
        })
        .on_mouse_down(button, move |event, window, cx| {
            if button == MouseButton::Right {
                crate::text_selection_owner::preserve(cx);
            }
            if let Some(target) = press_target.borrow().clone() {
                begin(target, event, window, cx);
            }
        })
        .on_mouse_up_all(move |event, phase, hitbox, window, cx| {
            if phase == DispatchPhase::Capture {
                let target = (event.button == button && hitbox.is_hovered(window))
                    .then(|| release_target.borrow().clone())
                    .flatten();
                release_over(target, cx);
            }
        });
    let listener = move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
        if let ClickEvent::Mouse(mouse) = event
            && (mouse.down.button != button
                || target
                    .borrow()
                    .as_ref()
                    .is_none_or(|target| complete(target, &mouse.up, window, cx).is_none()))
        {
            return;
        }
        cx.stop_propagation();
        handler(event, window, cx);
    };
    if button == MouseButton::Left {
        control.interactivity().on_click(listener);
    } else {
        control.interactivity().on_aux_click(listener);
    }
    control
}

/// Pointer-only actions (including secondary context menus) receive the
/// original press only after the matching release. Selection/focus setup can
/// stay in a separate press listener; commands belong in this callback.
pub(crate) trait PointerClickExt: Sized {
    fn on_any_pointer_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let handler = std::rc::Rc::new(handler);
        for button in MouseButton::all() {
            let handler = handler.clone();
            self =
                self.on_pointer_click(button, move |event, window, cx| handler(event, window, cx));
        }
        self
    }

    fn on_pointer_click(
        self,
        button: MouseButton,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self;
}

impl<E: Element + InteractiveElement> PointerClickExt for Stateful<E> {
    fn on_pointer_click(
        self,
        button: MouseButton,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        on_click(self, button, move |event, window, cx| {
            if let ClickEvent::Mouse(event) = event {
                handler(&event.down, window, cx);
            }
        })
    }
}

/// Each canvas action has a stable semantic id, including its view, document
/// revision and subtarget. A redraw retains ownership; replacement content
/// cannot inherit it. Call during paint, just as for ordinary hitbox handlers.
pub(crate) fn on_canvas_click(
    window: &mut Window,
    target: ElementId,
    hitbox: &Hitbox,
    button: MouseButton,
    consume_press: bool,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) {
    let target = ClickTarget::Canvas(target);
    let press_target = target.clone();
    let press_hitbox = hitbox.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            reset(cx);
        } else if event.button == button && press_hitbox.is_hovered(window) {
            if crate::press_gesture::is_press_claimed(cx) {
                return;
            }
            if button == MouseButton::Right {
                crate::text_selection_owner::preserve(cx);
            }
            begin(press_target.clone(), event, window, cx);
            if consume_press {
                cx.stop_propagation();
            }
        }
    });
    let hitbox = hitbox.clone();
    window.on_mouse_event(|event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture && !event.dragging() {
            reset(cx);
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            let target =
                (event.button == button && hitbox.is_hovered(window)).then(|| target.clone());
            release_over(target, cx);
        } else if phase == DispatchPhase::Bubble
            && hitbox.is_hovered(window)
            && let Some(click) = complete(&target, event, window, cx)
        {
            cx.stop_propagation();
            handler(&click, window, cx);
        }
    });
}
