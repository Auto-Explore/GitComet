//! Shared focus policy for caret-bearing windows and controls.

use gpui::{App, Context, FocusHandle, Subscription, Window};

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

/// Start keyboard navigation after blur without taking Tab from an element
/// that keeps it ([`crate::focus_order::CAPTURES_TAB`]).
///
/// [`crate::focus_order::install`] installs this, once: a second observer
/// would move focus twice. The Tab bindings answer everywhere else.
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
