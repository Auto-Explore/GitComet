//! Browser-style keyboard focus order: Tab moves focus to the next field and
//! Shift-Tab to the previous one, in every window.
//!
//! One pair of actions, [`FocusNext`] and [`FocusPrevious`], is bound to Tab
//! and Shift-Tab by [`key_bindings`] and answered here ([`install`]), so a
//! form needs no Tab handling of its own:
//!
//! - **Fields** join the order through their focus handle. [`field`] makes a
//!   new handle a Tab stop; [`TextInput`](crate::TextInput)s and the kit's
//!   buttons already are. Fields are visited by tab index, lowest first, and
//!   fields sharing an index in the order they are painted, so most forms need
//!   no index at all. [`field_at`] or a text input's `tab_index` option moves
//!   one field; numbering every field of a form fixes its order outright. A
//!   handle with `tab_stop(false)` is still focusable by pointer but skipped
//!   by Tab, like `tabindex="-1"` on the web.
//! - **Forms** that Tab must not leave — a dialog, a prompt — are a
//!   [`TabScope`] (`element.tab_scope(&scope)`). Past its last field Tab wraps
//!   to its first, and Shift-Tab the other way. Tab indices restart at zero
//!   inside each scope, so a form numbers its own fields without knowing the
//!   window around it. A [`TabScope::modal`] also takes Tab from the rest of
//!   its window while it is shown.
//! - **Elements that use Tab themselves** — a terminal, a picker that moves
//!   its selection, a menu, a code editor — add [`CAPTURES_TAB`] to their key
//!   context ([`key_context`]), and the binding stands aside for them. A text
//!   input does this from its [`TextInputTab`](crate::TextInputTab) setting.
//!   The bindings match at any depth, so a Tab binding of a view's own never
//!   outranks them: the view captures Tab instead.
//!
//! Outside any scope, focus moves through the window's whole order and wraps
//! at its ends.

use gpui::prelude::*;
use gpui::{App, Div, FocusHandle, Global, KeyBinding, KeyContext, Window, div};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};

gpui::actions!(
    focus_order,
    [
        /// Tab: focus the next field.
        FocusNext,
        /// Shift-Tab: focus the previous field.
        FocusPrevious,
    ]
);

/// The key context identifier of an element that keeps Tab and Shift-Tab for
/// itself. [`key_bindings`] do not apply inside it.
pub const CAPTURES_TAB: &str = "CapturesTab";

/// Bindings apply where no enclosing element captures Tab.
const OUTSIDE_TAB_CAPTURE: &str = "!CapturesTab";

/// The key context of a [`TabScope`]. A context on the path keeps the bindings
/// in reach of a focused control that has none of its own.
const SCOPE_CONTEXT: &str = "TabScope";

/// The tab-group index of the next [`TabScope`]. GPUI orders the fields of
/// groups sharing an index as one run, so each scope takes its own.
static NEXT_GROUP: AtomicIsize = AtomicIsize::new(1);

/// Counts scope paints, to tell which of two shown scopes is on top.
static NEXT_PAINT: AtomicU64 = AtomicU64::new(1);

/// Which way focus moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Next,
    Previous,
}

/// Tab and Shift-Tab, for the host's keymap. They apply wherever no
/// enclosing key context carries [`CAPTURES_TAB`].
pub fn key_bindings() -> [KeyBinding; 2] {
    [
        KeyBinding::new("tab", FocusNext, Some(OUTSIDE_TAB_CAPTURE)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(OUTSIDE_TAB_CAPTURE)),
    ]
}

struct Installed;

impl Global for Installed {}

/// Installs the window-wide focus movement every window falls back on, and
/// Tab from a window with nothing focused. Call once at launch, beside
/// binding [`key_bindings`]; later calls do nothing.
pub fn install(cx: &mut App) {
    if cx.has_global::<Installed>() {
        return;
    }
    cx.set_global(Installed);
    cx.on_action(|_: &FocusNext, cx| move_focus_in_current_window(Direction::Next, cx));
    cx.on_action(|_: &FocusPrevious, cx| move_focus_in_current_window(Direction::Previous, cx));
    crate::window_focus::observe_tab_navigation(cx).detach();
}

/// Moves focus one field through the whole window, wrapping at its ends.
fn move_focus(direction: Direction, window: &mut Window, cx: &mut App) {
    match direction {
        Direction::Next => window.focus_next(cx),
        Direction::Previous => window.focus_prev(cx),
    }
}

/// Tab from outside every form: into the dialog covering the window, if one
/// is shown, and through the whole window otherwise. Focus is outside a shown
/// dialog after the window was deactivated, which clears it.
pub(crate) fn move_focus_from_outside_forms(
    direction: Direction,
    window: &mut Window,
    cx: &mut App,
) {
    if !enter_shown_modal(direction, window, cx) {
        move_focus(direction, window, cx);
    }
}

/// The live [`TabScope::modal`]s of every window. Dead ones are dropped as
/// new ones register, so it holds no more than are alive plus one.
#[derive(Default)]
struct ModalScopes(Vec<WeakTabScope>);

impl Global for ModalScopes {}

struct WeakTabScope {
    container: gpui::WeakFocusHandle,
    end: gpui::WeakFocusHandle,
    group: isize,
    painted: Rc<Cell<u64>>,
}

impl WeakTabScope {
    fn upgrade(&self) -> Option<TabScope> {
        Some(TabScope {
            container: self.container.upgrade()?,
            end: self.end.upgrade()?,
            group: self.group,
            painted: Rc::clone(&self.painted),
        })
    }
}

/// Gives focus to the modal form on top in `window`, answering whether one is
/// shown: to its first or last field, or to the form itself when it has none,
/// so that keys stay off what it covers.
fn enter_shown_modal(direction: Direction, window: &mut Window, cx: &mut App) -> bool {
    let top = cx.try_global::<ModalScopes>().and_then(|scopes| {
        scopes
            .0
            .iter()
            .filter_map(WeakTabScope::upgrade)
            .filter(|scope| scope.is_shown(window))
            // Painted last is painted on top.
            .max_by_key(|scope| scope.painted.get())
    });
    let Some(top) = top else {
        return false;
    };
    if !top.try_enter(direction, window, cx) {
        window.focus(&top.container, cx);
    }
    true
}

/// App-level actions see no window, and the one dispatching is borrowed
/// until they return, so the move waits for it.
fn move_focus_in_current_window(direction: Direction, cx: &mut App) {
    let Some(window_id) = cx.current_window_id() else {
        cx.propagate();
        return;
    };
    cx.defer(move |cx| {
        let window = cx
            .windows()
            .into_iter()
            .find(|window| window.window_id() == window_id);
        if let Some(window) = window {
            let _ = window.update(cx, |_, window, cx| {
                move_focus_from_outside_forms(direction, window, cx)
            });
        }
    });
}

/// A new focus handle that Tab stops at, in paint order among its form's
/// fields.
pub fn field(cx: &App) -> FocusHandle {
    field_at(0, cx)
}

/// A new focus handle that Tab stops at, at `index` in its form: lower
/// indices come first, equal ones in paint order.
pub fn field_at(index: isize, cx: &App) -> FocusHandle {
    cx.focus_handle().tab_index(index).tab_stop(true)
}

/// A key context named `name`, keeping Tab for the element when
/// `captures_tab` is set.
pub fn key_context(name: &'static str, captures_tab: bool) -> KeyContext {
    let mut context = KeyContext::default();
    context.add(name);
    if captures_tab {
        context.add(CAPTURES_TAB);
    }
    context
}

/// A form that keeps Tab inside it: past its last field Tab wraps to its
/// first, and Shift-Tab the other way. Its fields' tab indices count from zero
/// within it.
///
/// Create one with the form's other state, and on every render make the
/// form's element the scope ([`TabScopeExt::tab_scope`]) or wrap its content
/// in one ([`wrap`](Self::wrap)):
///
/// ```ignore
/// struct Dialog {
///     tab_scope: TabScope,
///     name: Entity<TextInput>,
/// }
///
/// impl Render for Dialog {
///     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
///         div()
///             .tab_scope(&self.tab_scope)
///             .child(self.name.clone())
///             .child(ok_button())
///     }
/// }
/// ```
#[derive(Clone, Debug)]
pub struct TabScope {
    container: FocusHandle,
    end: FocusHandle,
    /// Its own tab-group index, so its fields and end marker sort apart from
    /// those of every other scope painted in the window.
    group: isize,
    /// When it was last painted, among all scopes: later is on top.
    painted: Rc<Cell<u64>>,
}

impl TabScope {
    /// A form inside its window. In the window's order the form sorts as one
    /// field with a tab index above zero: after the fields at zero or below,
    /// wherever it is painted.
    pub fn new(cx: &App) -> Self {
        Self {
            // Not stops themselves: the container is focused only to enter
            // from its start, and the end marker to enter from its end. They
            // sort before and after every field, whatever its index.
            container: cx.focus_handle().tab_index(isize::MIN).tab_stop(false),
            end: cx.focus_handle().tab_index(isize::MAX).tab_stop(false),
            group: NEXT_GROUP.fetch_add(1, Ordering::Relaxed),
            painted: Rc::default(),
        }
    }

    /// A form that covers its window while shown: a dialog, a popover. Tab
    /// from anywhere outside it — nothing focused, or a control behind it —
    /// enters it rather than reaching what it covers; with no field to enter,
    /// focus goes to [`container`](Self::container). Of several shown in one
    /// window, the one painted last takes it.
    pub fn modal(cx: &mut App) -> Self {
        let scope = Self::new(cx);
        let scopes = &mut cx.default_global::<ModalScopes>().0;
        // Dialogs make a scope each time they open; keep only the live ones.
        scopes.retain(|scope| scope.upgrade().is_some());
        scopes.push(WeakTabScope {
            container: scope.container.downgrade(),
            end: scope.end.downgrade(),
            group: scope.group,
            painted: Rc::clone(&scope.painted),
        });
        scope
    }

    /// `content`, wrapped in this form. The wrapper adds no styling; style it
    /// like any other `div` when the content needs it to.
    pub fn wrap(&self, content: impl IntoElement) -> Div {
        div().tab_scope(self).child(content)
    }

    /// Whether focus is on this form or inside it.
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.container.contains_focused(window, cx)
    }

    /// The handle a host focuses to hold focus on the form without focusing a
    /// field: Tab from it enters the form's first field, and Shift-Tab its
    /// last. It is not a Tab stop.
    pub fn container(&self) -> &FocusHandle {
        &self.container
    }

    /// Whether the form is in the frame `window` last painted.
    fn is_shown(&self, window: &Window) -> bool {
        // GPUI answers for two handles only when both were painted.
        self.container.contains(&self.end, window)
    }

    /// Moves focus one field within the form, wrapping at its ends. A form
    /// without fields leaves focus where it was.
    pub fn step(&self, direction: Direction, window: &mut Window, cx: &mut App) {
        let before = window.focused(cx);
        move_focus(direction, window, cx);
        // Past an end: wrap.
        if !self.holds_field_focus(window, cx) && !self.try_enter(direction, window, cx) {
            match before {
                Some(before) => window.focus(&before, cx),
                None => window.blur(cx),
            }
        }
    }

    /// Focuses the form's first or last field, answering whether it has one.
    /// When it has none, where focus goes is the caller's to decide.
    fn try_enter(&self, direction: Direction, window: &mut Window, cx: &mut App) -> bool {
        let from = match direction {
            Direction::Next => &self.container,
            Direction::Previous => &self.end,
        };
        window.focus(from, cx);
        move_focus(direction, window, cx);
        self.holds_field_focus(window, cx)
    }

    fn holds_field_focus(&self, window: &Window, cx: &App) -> bool {
        self.contains_focused(window, cx)
            && !self.container.is_focused(window)
            && !self.end.is_focused(window)
    }
}

/// [`TabScope`] as an element builder.
pub trait TabScopeExt: InteractiveElement + ParentElement + Sized {
    /// Makes this element the form `scope` keeps Tab inside. The element
    /// tracks the scope's focus and takes the `TabScope` key context; give it
    /// no focus handle or key context of its own, and wrap it in an element
    /// that has them instead.
    fn tab_scope(self, scope: &TabScope) -> Self {
        let next = scope.clone();
        let previous = scope.clone();
        let pressed = scope.clone();
        let painted = Rc::clone(&scope.painted);
        self
            // GPUI runs an element's own press listeners before the one that
            // focuses it, and after its children's: a press no control of the
            // form took keeps the field that has focus, as pressing a form's
            // label or margin does, and a text input does not take it for a
            // press elsewhere. A press a control took is that control's. From
            // outside the form a press still lands on it, and Tab then enters.
            .on_any_mouse_down(move |_, window, cx| {
                if !window.default_prevented() && pressed.holds_field_focus(window, cx) {
                    window.prevent_default();
                    crate::text_selection_owner::preserve(cx);
                }
            })
            .track_focus(&scope.container)
            .tab_index(scope.group)
            .tab_group()
            .key_context(SCOPE_CONTEXT)
            .on_action(move |_: &FocusNext, window, cx| next.step(Direction::Next, window, cx))
            .on_action(move |_: &FocusPrevious, window, cx| {
                previous.step(Direction::Previous, window, cx)
            })
            // The end marker sorts after every field by its index, wherever it
            // is painted. Out of flow, so the form's layout is its content's.
            .child(
                div()
                    .track_focus(&scope.end)
                    .absolute()
                    .size_0()
                    .child(gpui::canvas(
                        |_, _, _| (),
                        move |_, _, _, _| {
                            painted.set(NEXT_PAINT.fetch_add(1, Ordering::Relaxed));
                        },
                    )),
            )
    }
}

impl<E: InteractiveElement + ParentElement> TabScopeExt for E {}

#[cfg(test)]
mod tests;
