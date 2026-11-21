//! Shared focus policy for caret-bearing windows and controls.

use gpui::{App, Context, FocusHandle, Subscription, Window};
use std::cell::Cell;

thread_local! {
    /// One keyboard and one pointer serve every window.
    static LAST_PRESS_WAS_KEY: Cell<bool> = const { Cell::new(false) };
}

/// True from a key press until the next pointer press: the span in which a
/// focused control shows its focus ring. GPUI's own keyboard modality also
/// ends when the pointer merely moves.
pub fn keyboard_focus_shown() -> bool {
    LAST_PRESS_WAS_KEY.with(Cell::get)
}

/// Starts [`keyboard_focus_shown`] at every key press.
/// [`crate::focus_order::install`] installs this.
pub(crate) fn observe_key_presses(cx: &mut App) -> Subscription {
    // An interceptor, as observers miss the keys a control consumes. Nothing
    // to redraw: GPUI shows the ring itself until the pointer moves, and
    // redraws the window when it does.
    cx.intercept_keystrokes(|_, _, _| LAST_PRESS_WAS_KEY.with(|key| key.set(true)))
}

/// Ends [`keyboard_focus_shown`]. Every control reports the presses in its
/// window, so no window needs a hook of its own.
pub(crate) fn pointer_pressed(window: &mut Window) {
    if LAST_PRESS_WAS_KEY.with(|key| key.replace(false)) {
        window.refresh();
    }
}

pub fn is_active(focus: &FocusHandle, window: &Window) -> bool {
    window.is_window_active() && focus.is_focused(window)
}

/// GPUI retains the focused control on window deactivation. Clear it so merely
/// bringing the window forward cannot resume typing in the previous control.
pub fn reset_on_deactivation(window: &mut Window, cx: &mut App) {
    if !window.is_window_active() {
        window.blur(cx);
    }
}

/// Window activation callbacks run even when no frame is drawn (minimizing,
/// for example). Control blur listeners also cover focus moving within a window.
pub fn observe_blur<T: 'static>(
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<T>,
    reset: fn(&mut T, &mut Context<T>),
) -> [Subscription; 2] {
    [
        cx.observe_window_activation(window, move |this, window, cx| {
            if !window.is_window_active() {
                reset(this, cx);
            }
        }),
        cx.on_blur(focus, window, move |this, _window, cx| reset(this, cx)),
    ]
}

/// Answers Tab and Shift-Tab where the focus-order bindings cannot match:
/// with nothing focused, or on a control with no key context on its path. An
/// element that keeps Tab ([`crate::focus_order::CAPTURES_TAB`]) keeps it.
///
/// [`crate::focus_order::install`] installs this, once: a second observer
/// would move focus twice.
pub(crate) fn observe_tab_navigation(cx: &mut App) -> Subscription {
    // With no focus GPUI dispatches only through its root node, so a listener
    // inside the window frame would never receive this first Tab, and with no
    // key context on that path the context-bound Tab binding does not match.
    cx.observe_keystrokes(|event, window, cx| {
        if event.action.is_some()
            || !window.is_window_active()
            || event
                .context_stack
                .iter()
                .any(|context| context.contains(crate::focus_order::CAPTURES_TAB))
            || event.keystroke.key != "tab"
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function
        {
            return;
        }
        let direction = if event.keystroke.modifiers.shift {
            crate::focus_order::Direction::Previous
        } else {
            crate::focus_order::Direction::Next
        };
        crate::focus_order::move_focus_from_outside_forms(direction, window, cx);
        if window.focused(cx).is_some() {
            cx.stop_propagation();
        }
    })
}
