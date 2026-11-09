//! The actions a host binds to Cmd/Ctrl+F and F2/F3, and how a view with a
//! search of its own answers them.
//!
//! The actions bubble from the focused element up to the host's window, so
//! a view answers them on the way, with [`on_search_keys`], and the host's
//! own searches handle whatever no view took.

use gpui::{Context, InteractiveElement, Window};
use std::rc::Rc;

gpui::actions!(
    search_keys,
    [
        /// Cmd/Ctrl+F: open the search of the view holding focus.
        OpenActiveViewSearch,
        /// F2 outside a text input: the previous search match, or change.
        DiffPrevSearchMatchOrChange,
        /// F3 outside a text input: the next search match, or change.
        DiffNextSearchMatchOrChange,
        /// F2 in a text input, a search's own among them.
        TextInputDiffPrevSearchMatchOrChange,
        /// F3 in a text input, a search's own among them.
        TextInputDiffNextSearchMatchOrChange,
    ]
);

/// A search key, wherever focus was when it was pressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchKey {
    /// Cmd/Ctrl+F.
    Open,
    /// F2 (`forward: false`) or F3.
    Step { forward: bool },
}

/// Hands the search keys pressed while focus is inside `element` to
/// `handle`, which returns whether it took the key; a key it did not take
/// goes on to the host.
pub fn on_search_keys<V: 'static, E: InteractiveElement>(
    element: E,
    cx: &mut Context<V>,
    handle: impl Fn(&mut V, SearchKey, &mut Window, &mut Context<V>) -> bool + 'static,
) -> E {
    let handle = Rc::new(handle);
    let answer = |key: SearchKey| {
        let handle = Rc::clone(&handle);
        move |view: &mut V, window: &mut Window, cx: &mut Context<V>| {
            if !handle(view, key, window, cx) {
                cx.propagate();
            }
        }
    };
    let open = answer(SearchKey::Open);
    let previous = answer(SearchKey::Step { forward: false });
    let next = answer(SearchKey::Step { forward: true });
    let previous_in_input = answer(SearchKey::Step { forward: false });
    let next_in_input = answer(SearchKey::Step { forward: true });
    element
        .on_action(
            cx.listener(move |view, _: &OpenActiveViewSearch, window, cx| open(view, window, cx)),
        )
        .on_action(
            cx.listener(move |view, _: &DiffPrevSearchMatchOrChange, window, cx| {
                previous(view, window, cx)
            }),
        )
        .on_action(
            cx.listener(move |view, _: &DiffNextSearchMatchOrChange, window, cx| {
                next(view, window, cx)
            }),
        )
        .on_action(cx.listener(
            move |view, _: &TextInputDiffPrevSearchMatchOrChange, window, cx| {
                previous_in_input(view, window, cx)
            },
        ))
        .on_action(cx.listener(
            move |view, _: &TextInputDiffNextSearchMatchOrChange, window, cx| {
                next_in_input(view, window, cx)
            },
        ))
}
