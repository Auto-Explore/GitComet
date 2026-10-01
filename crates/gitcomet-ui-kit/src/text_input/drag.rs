//! Mouse-drag selection: the head follows the pointer, and a pointer held past
//! the viewport edge keeps scrolling (see [`crate::drag_autoscroll`]).

use super::state::*;
use super::*;
use crate::drag_autoscroll::{DRAG_AUTOSCROLL_TICK, drag_autoscroll_step};

/// Slack for "text continues past this edge", so sub-pixel layout does not
/// read as hidden text.
const EDGE_EPSILON: Pixels = px(0.5);

/// How close a single-line field keeps its caret to the edges. Shared with
/// prepaint's reveal so a drag step is never topped up by the reveal's pad.
pub(super) fn single_line_reveal_pad(viewport_w: Pixels) -> Pixels {
    px(8.0).min(viewport_w / 4.0)
}

impl TextInput {
    pub(super) fn end_mouse_drag(&mut self) {
        self.interaction.is_selecting = false;
        self.interaction.mouse_selection_anchor = None;
        self.interaction.pending_mouse_selection_anchor = None;
        self.interaction.drag_pointer = None;
        self.interaction.drag_resolve_pending = false;
        self.interaction.drag_step_paint_seq = None;
        self.interaction.drag_autoscroll_task = None;
    }

    /// The text's bounds where they are now: the last painted bounds moved by
    /// whatever the host's scroll handle has scrolled since that layout. Hit
    /// tests read this, so one between a scroll and the next paint lands on
    /// the text the user sees.
    pub(super) fn current_text_bounds(&self) -> Option<Bounds<Pixels>> {
        let mut bounds = self.layout.bounds?;
        if let (Some(handle), Some(painted)) = (
            self.interaction.vertical_scroll_handle.as_ref(),
            self.layout.painted_scroll_offset,
        ) {
            // Clamped as the next layout will clamp it; a wheel writes raw.
            let max = handle.max_offset();
            let current = handle.offset();
            let current = point(
                current.x.clamp(-max.x.max(px(0.0)), px(0.0)),
                current.y.clamp(-max.y.max(px(0.0)), px(0.0)),
            );
            bounds.origin += current - painted;
        }
        Some(bounds)
    }

    /// Starts the ticker that scrolls while the drag's pointer is held past an
    /// edge. Replacing the task cancels any earlier one.
    pub(super) fn start_drag_autoscroll(&mut self, cx: &mut Context<Self>) {
        self.interaction.drag_step_paint_seq = None;
        if !self.multiline && self.display_truncation.is_some() {
            // A truncated label never scrolls.
            return;
        }
        let task = cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| loop {
                cx.background_executor().timer(DRAG_AUTOSCROLL_TICK).await;
                let keep_going = input
                    .update_in(cx, |input, window, cx| {
                        // Losing focus ends the drag from `reset_focus`.
                        if !input.interaction.is_selecting
                            || !crate::window_focus::is_active(&input.focus_handle, window)
                        {
                            return false;
                        }
                        input.tick_drag_autoscroll(cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            },
        );
        self.interaction.drag_autoscroll_task = Some(task);
    }

    /// One autoscroll tick: step toward a pointer held past an edge, then
    /// re-resolve the head. Returns whether anything changed.
    pub(super) fn tick_drag_autoscroll(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.interaction.drag_pointer else {
            return false;
        };
        // An unresolved anchor must resolve against the layout it was pressed in.
        let scrolled = self.interaction.mouse_selection_anchor.is_some()
            && self.interaction.drag_step_paint_seq != Some(self.layout.paint_seq)
            && self.step_drag_autoscroll(pointer);
        if scrolled {
            self.interaction.drag_step_paint_seq = Some(self.layout.paint_seq);
            cx.notify();
        }
        let resolved =
            (scrolled || self.interaction.drag_resolve_pending) && self.resolve_drag_head(cx);
        scrolled || resolved
    }

    pub(super) fn drag_mouse_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.interaction.is_selecting {
            return;
        }
        if !event.dragging() {
            // Released where the window never saw it.
            self.end_mouse_drag();
            return;
        }
        self.update_mouse_selection(event.position, cx);
    }

    pub(super) fn update_mouse_selection(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.interaction.drag_pointer = Some(position);
        self.resolve_drag_head(cx);
    }

    /// Moves the head to `drag_pointer`. Returns whether the selection changed.
    fn resolve_drag_head(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.interaction.drag_pointer else {
            return false;
        };
        if self.interaction.mouse_selection_anchor.is_none() {
            let Some(anchor_position) = self.interaction.pending_mouse_selection_anchor else {
                return false;
            };
            let Some(anchor) = self.try_index_for_mouse_position(anchor_position) else {
                self.interaction.drag_resolve_pending = true;
                return false;
            };
            self.interaction.mouse_selection_anchor = Some(anchor);
            self.interaction.pending_mouse_selection_anchor = None;
        }
        let Some(index) = self.drag_index_for_pointer(pointer) else {
            self.interaction.drag_resolve_pending = true;
            return false;
        };
        self.interaction.drag_resolve_pending = false;
        let before = (self.selection.range.clone(), self.selection.reversed);
        self.select_mouse_to_index(index, cx);
        before != (self.selection.range.clone(), self.selection.reversed)
    }

    fn drag_scrolls_horizontally(&self) -> bool {
        self.multiline && !self.soft_wrap && self.interaction.content_width_layout
    }

    /// The host viewport a multiline drag scrolls, if it has a usable one.
    fn drag_viewport(&self) -> Option<(ScrollHandle, Bounds<Pixels>)> {
        let handle = self.interaction.vertical_scroll_handle.clone()?;
        let viewport = handle.bounds();
        (viewport.size.width > px(0.0) && viewport.size.height > px(0.0))
            .then_some((handle, viewport))
    }

    /// Scrolls the host one step toward `pointer` when it is past an edge.
    /// Returns whether it scrolled.
    fn step_drag_autoscroll(&mut self, pointer: Point<Pixels>) -> bool {
        if !self.multiline {
            return self.step_single_line_drag_autoscroll(pointer);
        }
        let Some((handle, viewport)) = self.drag_viewport() else {
            return false;
        };
        let mut step = point(
            px(0.0),
            drag_autoscroll_step(pointer.y, viewport.top(), viewport.bottom()),
        );
        if self.drag_scrolls_horizontally() {
            step.x = drag_autoscroll_step(pointer.x, viewport.left(), viewport.right());
        }

        let max = handle.max_offset();
        let current = handle.offset();
        let next = point(
            (current.x + step.x).clamp(-max.x.max(px(0.0)), px(0.0)),
            (current.y + step.y).clamp(-max.y.max(px(0.0)), px(0.0)),
        );
        if next == current {
            return false;
        }
        handle.set_offset(next);
        // The drag owns the scroll now; a queued caret reveal would fight it.
        self.interaction.pending_cursor_autoscroll = false;
        true
    }

    fn step_single_line_drag_autoscroll(&mut self, pointer: Point<Pixels>) -> bool {
        if self.content.is_empty() {
            return false;
        }
        let (Some(bounds), Some(TextInputLayout::Plain(lines))) =
            (self.layout.bounds, self.layout.last.as_ref())
        else {
            return false;
        };
        let Some(line) = lines.get(0) else {
            return false;
        };
        let max_scroll_x = (line.width - bounds.size.width.max(px(0.0))).max(px(0.0));
        let step = drag_autoscroll_step(pointer.x, bounds.left(), bounds.right());
        let next = (self.layout.scroll_x - step).clamp(px(0.0), max_scroll_x);
        if next == self.layout.scroll_x {
            return false;
        }
        // Prepaint reads it back; the head is clamped inside its reveal pad.
        self.layout.scroll_x = next;
        true
    }

    /// The offset a drag's pointer selects to. A pointer past an edge that
    /// still hides text is held at that edge, so the head stays on what is
    /// visible and follows it as the ticker scrolls. `None` when that spot
    /// was not laid out last frame; the caller keeps the old head.
    fn drag_index_for_pointer(&self, pointer: Point<Pixels>) -> Option<usize> {
        let bounds = self.current_text_bounds()?;
        let mut at = pointer;
        if self.multiline {
            if let Some((_, viewport)) = self.drag_viewport() {
                if at.y >= viewport.bottom() && bounds.bottom() > viewport.bottom() + EDGE_EPSILON {
                    at.y = viewport.bottom() - px(1.0);
                } else if at.y < viewport.top() && bounds.top() < viewport.top() - EDGE_EPSILON {
                    at.y = viewport.top();
                }
                if self.drag_scrolls_horizontally() {
                    if at.x >= viewport.right() && bounds.right() > viewport.right() + EDGE_EPSILON
                    {
                        at.x = viewport.right() - px(1.0);
                    } else if at.x < viewport.left()
                        && bounds.left() < viewport.left() - EDGE_EPSILON
                    {
                        at.x = viewport.left();
                    }
                }
            }
        } else if let Some(TextInputLayout::Plain(lines)) = self.layout.last.as_ref() {
            // One line: drifting above or below it must not select to an end.
            at.y = bounds.center().y;
            let width = bounds.size.width.max(px(0.0));
            let pad = single_line_reveal_pad(width);
            let line_w = lines.get(0).map(|line| line.width).unwrap_or_default();
            let scroll_x = self.layout.scroll_x;
            if at.x > bounds.right() - pad && line_w - scroll_x > width + EDGE_EPSILON {
                at.x = bounds.right() - pad;
            } else if at.x < bounds.left() + pad && scroll_x > px(0.0) {
                at.x = bounds.left() + pad;
            }
        }
        self.index_for_mouse_position_in(at, true)
    }
}
