//! One visible text selection per window, and focus follows the same rule.
//!
//! Not [`crate::text_selection`], which is pure text arithmetic and owns no
//! state.
//!
//! Three surfaces paint a selection -- a [`crate::kit::TextInput`], the main
//! pane's diff/preview text, a terminal viewport -- and none stops painting it
//! on blur. So a press collapses the window's selection unless the surface that
//! owns it adopts the press, or the gesture is selection- and focus-neutral
//! ([`preserve`]: scrollbars, splitters, the titlebar, a menu acting *on* the
//! selection). Row and list selections are a different concept and untouched.
//!
//! Two invariants, neither checkable by the compiler:
//!
//! 1. Every path that installs a *visible* highlight must call
//!    [`SelectionOwnerToken::adopt`], and no path that installs none may. There
//!    is no "not yet an owner" state: an unclaimed highlight is reaped by the
//!    next global write.
//! 2. [`install_reset`] must run before any content listener. `dispatch_event`
//!    runs the whole capture phase, then the whole bubble phase, and effects
//!    flush after both -- so the resolver deferred here sees the finished press,
//!    and a bubble handler that adopts is seen to have done so. A capture-phase
//!    `stop_propagation` ahead of it would skip the bubble phase entirely.
//!
//! `focused_diff` renders outside the frame and hosts no selectable surface, so
//! it installs nothing.

use gpui::{
    App, BorrowAppContext as _, DispatchPhase, MouseButton, MouseDownEvent, Window, WindowId,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Identifies the surface allowed to paint a selection in one window.
///
/// Globally unique and monotonic. `0` is the default and means "this surface
/// has never owned a selection"; it can never match a live entry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SelectionOwnerToken(u64);

#[derive(Default)]
struct TextSelectionOwners {
    next_token: u64,
    /// The one surface allowed to paint a selection in each window.
    owner_by_window: FxHashMap<WindowId, u64>,
    /// Set by a neutral gesture during the press in flight; cleared by the next
    /// press, so it cannot leak across presses.
    press_preserved: bool,
    /// True between a press and its resolver.
    press_pending: bool,
    /// Windows with a popover open: they drive keyboard nav through the focused
    /// input behind them, so a press inside one is not "clicking away". Keyed by
    /// window because each renders its own `PopoverHost`.
    overlay_open: FxHashSet<WindowId>,
}

impl gpui::Global for TextSelectionOwners {}

impl SelectionOwnerToken {
    /// Makes this surface the sole selection owner of `window`.
    ///
    /// Every path that installs a *visible* highlight must call it -- from a
    /// press, a keystroke, or programmatically -- or the next global write reaps
    /// the highlight.
    pub(crate) fn adopt(&mut self, window: &Window, cx: &mut App) {
        self.adopt_window(window.window_handle().window_id(), cx);
    }

    fn adopt_window(&mut self, window_id: WindowId, cx: &mut App) {
        // A pending resolver detects adoption by the token *changing*, so never
        // skip while one is in flight. Outside a press (every Shift+Arrow)
        // re-adopting changes nothing.
        let already_owner = cx
            .try_global::<TextSelectionOwners>()
            .is_some_and(|owners| {
                !owners.press_pending && owners.owner_by_window.get(&window_id) == Some(&self.0)
            });
        if already_owner {
            return;
        }
        let token = cx.update_default_global::<TextSelectionOwners, _>(|owners, _cx| {
            owners.next_token = owners.next_token.wrapping_add(1).max(1);
            owners.owner_by_window.insert(window_id, owners.next_token);
            owners.press_preserved = false;
            owners.next_token
        });
        *self = Self(token);
    }

    /// True once anything else has taken the selection. Owners clear on this,
    /// and must never adopt while doing so. Minted tokens are always >= 1, so
    /// the default never matches.
    pub(crate) fn is_stale(self, cx: &App) -> bool {
        !cx.try_global::<TextSelectionOwners>()
            .is_some_and(|owners| owners.owner_by_window.values().any(|&t| t == self.0))
    }
}

/// Registers `on_change` for every ownership change, keeping the global type
/// private to this module.
pub(crate) fn observe<T: 'static>(
    cx: &mut gpui::Context<T>,
    mut on_change: impl FnMut(&mut T, &mut gpui::Context<T>) + 'static,
) -> gpui::Subscription {
    cx.observe_global::<TextSelectionOwners>(move |this, cx| on_change(this, cx))
}

/// Marks the press in flight as selection-neutral, so the window keeps whatever
/// it had. For gestures that manipulate the viewport or act on the selection
/// rather than replacing it.
pub(crate) fn preserve(cx: &mut App) {
    // Recorded even with no selection anywhere: focus reads the same flag.
    let already = cx
        .try_global::<TextSelectionOwners>()
        .is_some_and(|owners| owners.press_preserved);
    if already {
        return;
    }
    cx.update_default_global::<TextSelectionOwners, _>(|owners, _cx| {
        owners.press_preserved = true;
    });
}

/// Records whether an overlay -- popover, picker, context menu -- is open, so a
/// focused input behind it keeps focus while the user drives it.
pub(crate) fn set_overlay_open(window: &Window, open: bool, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let current = cx
        .try_global::<TextSelectionOwners>()
        .is_some_and(|owners| owners.overlay_open.contains(&window_id));
    if current == open {
        return;
    }
    cx.update_default_global::<TextSelectionOwners, _>(|owners, _cx| {
        if open {
            owners.overlay_open.insert(window_id);
        } else {
            owners.overlay_open.remove(&window_id);
        }
    });
}

/// Whether a press should leave focus alone. `press_preserved` is app-global on
/// purpose (one press at a time); the overlay state is per window.
pub(crate) fn press_keeps_focus(window: &Window, cx: &App) -> bool {
    let window_id = window.window_handle().window_id();
    cx.try_global::<TextSelectionOwners>()
        .is_some_and(|owners| owners.press_preserved || owners.overlay_open.contains(&window_id))
}

/// Capture phase. Schedules the collapse of `window`'s selection unless a
/// bubble handler adopts or preserves this press first. Only [`install_reset`]
/// calls it in production; tests use it to stage a press.
pub(crate) fn release_for_press(window: &Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let (token_at_press, needs_reset) = match cx.try_global::<TextSelectionOwners>() {
        Some(owners) => (
            owners.owner_by_window.get(&window_id).copied(),
            owners.press_preserved,
        ),
        None => (None, false),
    };
    // An app with no selection anywhere pays one hash lookup per press and
    // pushes no effect, so no observer is walked.
    if token_at_press.is_none() && !needs_reset {
        return;
    }
    // Closed windows are unreachable but would accumulate, and `is_stale` scans
    // them per observer callback. Pruned here, not in `adopt`: that is on the
    // keystroke path.
    let stale_windows: Vec<WindowId> = cx
        .try_global::<TextSelectionOwners>()
        .filter(|owners| owners.owner_by_window.len() + owners.overlay_open.len() > 1)
        .map(|owners| {
            let live: FxHashSet<WindowId> = cx
                .windows()
                .iter()
                .map(|handle| handle.window_id())
                .collect();
            owners
                .owner_by_window
                .keys()
                .chain(owners.overlay_open.iter())
                .filter(|id| !live.contains(id))
                .copied()
                .collect()
        })
        .unwrap_or_default();
    cx.update_default_global::<TextSelectionOwners, _>(|owners, _cx| {
        for id in &stale_windows {
            owners.owner_by_window.remove(id);
            owners.overlay_open.remove(id);
        }
        owners.press_preserved = false;
        owners.press_pending = token_at_press.is_some();
    });
    let Some(token) = token_at_press else {
        return;
    };
    cx.defer(move |cx| {
        cx.update_default_global::<TextSelectionOwners, _>(|owners, _cx| {
            owners.press_pending = false;
            // The same predicate the blur check uses, so focus and selection
            // never disagree about whether this press was "clicking away".
            // Read, not taken: that check runs after this resolver and needs
            // the same answer; `release_for_press` clears it next press.
            if owners.press_preserved || owners.overlay_open.contains(&window_id) {
                return;
            }
            // A fresh token means the press was adopted; the owner changed
            // under us and must be left alone.
            if owners.owner_by_window.get(&window_id) == Some(&token) {
                owners.owner_by_window.remove(&window_id);
            }
        });
    });
}

/// Installs the press invalidator. Called from the window frame's root hook.
pub(crate) fn install_reset(window: &mut Window) {
    window.on_mouse_event(|event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        // A click that merely brings the window forward from another app is not
        // the user pointing at anything yet.
        if event.first_mouse {
            return;
        }
        // Only left and right can be adopted -- `TextInput` is wired for those
        // two alone -- so releasing on any other button would collapse a
        // selection that nothing is able to take over.
        if !matches!(event.button, MouseButton::Left | MouseButton::Right) {
            return;
        }
        release_for_press(window, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_for(window_id: WindowId, cx: &App) -> Option<u64> {
        cx.try_global::<TextSelectionOwners>()
            .and_then(|owners| owners.owner_by_window.get(&window_id).copied())
    }

    #[gpui::test]
    fn adopting_mints_a_fresh_token_and_stales_the_previous_owner(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        cx.update(|window, app| {
            let mut first = SelectionOwnerToken::default();
            assert!(first.is_stale(app), "a default token never owns anything");

            first.adopt(window, app);
            assert!(!first.is_stale(app));

            let mut second = SelectionOwnerToken::default();
            second.adopt(window, app);
            assert!(!second.is_stale(app));
            assert!(
                first.is_stale(app),
                "one window holds one selection, so adopting displaces the previous owner"
            );
        });
    }

    #[gpui::test]
    fn each_window_keeps_its_own_selection(cx: &mut gpui::TestAppContext) {
        let window_a = cx.add_window(|_window, _cx| gpui::Empty);
        let window_b = cx.add_window(|_window, _cx| gpui::Empty);

        let token_a = window_a
            .update(cx, |_view, window, cx| {
                let mut owner = SelectionOwnerToken::default();
                owner.adopt(window, cx);
                owner
            })
            .unwrap();
        let token_b = window_b
            .update(cx, |_view, window, cx| {
                let mut owner = SelectionOwnerToken::default();
                owner.adopt(window, cx);
                owner
            })
            .unwrap();

        cx.update(|app| {
            assert!(
                !token_a.is_stale(app),
                "selecting in another window must not clear this one"
            );
            assert!(!token_b.is_stale(app));
        });

        // A press in B resolves against B only.
        window_b
            .update(cx, |_view, window, cx| release_for_press(window, cx))
            .unwrap();
        cx.run_until_parked();

        cx.update(|app| {
            assert!(token_b.is_stale(app), "the pressed window lost its owner");
            assert!(
                !token_a.is_stale(app),
                "a press in one window must leave every other window's selection alone"
            );
        });
    }

    #[gpui::test]
    fn a_press_nobody_adopts_drops_the_selection(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        let token = cx.update(|window, app| {
            let mut owner = SelectionOwnerToken::default();
            owner.adopt(window, app);
            release_for_press(window, app);
            assert!(!owner.is_stale(app), "the resolver has not run yet");
            owner
        });
        cx.run_until_parked();
        cx.update(|_window, app| {
            assert!(
                token.is_stale(app),
                "no bubble handler adopted or preserved, so the owner was dropped"
            );
        });
    }

    #[gpui::test]
    fn preserving_a_press_keeps_the_selection(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        let token = cx.update(|window, app| {
            let mut owner = SelectionOwnerToken::default();
            owner.adopt(window, app);
            release_for_press(window, app);
            preserve(app);
            owner
        });
        cx.run_until_parked();
        cx.update(|_window, app| {
            assert!(
                !token.is_stale(app),
                "a selection-neutral gesture leaves the owner in place"
            );
        });
    }

    #[gpui::test]
    fn adopting_during_a_press_survives_that_press(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        let token = cx.update(|window, app| {
            let mut previous = SelectionOwnerToken::default();
            previous.adopt(window, app);
            release_for_press(window, app);
            // The bubble phase: the surface under the pointer takes over.
            let mut next = SelectionOwnerToken::default();
            next.adopt(window, app);
            next
        });
        cx.run_until_parked();
        cx.update(|_window, app| {
            assert!(
                !token.is_stale(app),
                "the resolver must not collapse a selection the press itself created"
            );
        });
    }

    #[gpui::test]
    fn re_adopting_with_no_press_in_flight_is_a_no_op(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        cx.update(|window, app| {
            let mut owner = SelectionOwnerToken::default();
            owner.adopt(window, app);
            let first = owner;
            // Every Shift+Arrow lands here; nothing has changed, so it must not
            // mint a token or push a notification.
            owner.adopt(window, app);
            assert_eq!(
                owner, first,
                "re-adopting outside a press must not churn the token"
            );
            assert!(!owner.is_stale(app));
        });
    }

    #[gpui::test]
    fn re_adopting_during_a_press_still_survives_it(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        let token = cx.update(|window, app| {
            let mut owner = SelectionOwnerToken::default();
            owner.adopt(window, app);
            release_for_press(window, app);
            // The same surface is pressed again -- it already owns the window.
            owner.adopt(window, app);
            owner
        });
        cx.run_until_parked();
        cx.update(|_window, app| {
            assert!(
                !token.is_stale(app),
                "a press on the surface that already owns must not be reaped"
            );
        });
    }

    #[gpui::test]
    fn a_press_with_nothing_at_stake_writes_no_global(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        cx.update(|window, app| {
            release_for_press(window, app);
            assert!(
                app.try_global::<TextSelectionOwners>().is_none(),
                "an app with no selection anywhere must not allocate or notify per press"
            );
        });
    }

    #[gpui::test]
    fn a_stray_preserve_cannot_resurrect_a_dropped_owner(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = cx.add_window_view(|_window, _cx| gpui::Empty);
        let (token, window_id) = cx.update(|window, app| {
            let mut owner = SelectionOwnerToken::default();
            owner.adopt(window, app);
            release_for_press(window, app);
            (owner, window.window_handle().window_id())
        });
        cx.run_until_parked();
        cx.update(|_window, app| {
            assert!(token.is_stale(app));
            assert_eq!(owner_for(window_id, app), None);
            // A `preserve` outside any press must not put the dead owner back.
            preserve(app);
            assert!(token.is_stale(app));
            assert_eq!(owner_for(window_id, app), None);
        });
    }
}
