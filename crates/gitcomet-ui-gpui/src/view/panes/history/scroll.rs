use super::*;
use crate::kit::{ScrollbarAxis, ScrollbarDriver};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(super) struct LogicalViewport {
    pub top: usize,
    pub within: f64,
    pub height: f64,
    pub viewport: f64,
    pub total: usize,
    pub down: bool,
}

impl LogicalViewport {
    pub fn new(total: usize, height: f64, viewport: f64) -> Self {
        Self {
            top: 0,
            within: 0.0,
            height,
            viewport,
            total,
            down: true,
        }
    }
    pub fn position(&self) -> f64 {
        self.top as f64 * self.height + self.within
    }
    pub fn max(&self) -> f64 {
        (self.total as f64 * self.height - self.viewport).max(0.0)
    }
    pub fn set_position(&mut self, position: f64) {
        let old = self.position();
        let position = if position.is_finite() {
            position.clamp(0.0, self.max())
        } else {
            old
        };
        self.down = position >= old;
        self.top = (position / self.height).floor() as usize;
        self.within = position - self.top as f64 * self.height;
    }
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        self.top
            ..(self.top + ((self.viewport + self.within) / self.height).ceil() as usize + 1)
                .min(self.total)
    }
}

#[derive(Default)]
pub(super) struct ScrollInteraction {
    pub dragging: bool,
    pub manual_pending: bool,
    pub generation: u64,
    pub logical: Option<LogicalViewport>,
    frozen_extent: Option<Pixels>,
}

pub(super) type SharedScrollInteraction = Rc<RefCell<ScrollInteraction>>;

#[derive(Clone)]
pub(super) struct HistoryScrollDriver {
    pub view: gpui::WeakEntity<HistoryView>,
    pub handle: UniformListScrollHandle,
    pub interaction: SharedScrollInteraction,
}

impl ScrollbarDriver for HistoryScrollDriver {
    fn interaction_changed(&self, cx: &mut gpui::App) {
        let _ = self.view.update(cx, |_, cx| cx.notify());
    }
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        let state = self.interaction.borrow();
        if let Some(logical) = &state.logical {
            return if logical.max() > 0.0 {
                px(1.0)
            } else {
                px(0.0)
            };
        }
        state
            .frozen_extent
            .unwrap_or_else(|| ScrollbarDriver::max_offset(&self.handle, axis))
    }
    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        if let Some(logical) = &self.interaction.borrow().logical {
            return -px((logical.position() / logical.max().max(f64::MIN_POSITIVE)) as f32);
        }
        // Uniform lists always scroll with negative offsets. A transient
        // undersized measurement must not flip the drag direction.
        ScrollbarDriver::raw_offset(&self.handle, axis).min(px(0.0))
    }
    fn logical_metrics(&self, _axis: ScrollbarAxis) -> Option<(f64, f64)> {
        self.interaction.borrow().logical.as_ref().map(|logical| {
            (
                logical.position() / logical.max().max(f64::MIN_POSITIVE),
                logical.viewport / (logical.total as f64 * logical.height).max(1.0),
            )
        })
    }
    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        let mut state = self.interaction.borrow_mut();
        state.manual_pending = true;
        state.generation = state.generation.wrapping_add(1);
        if let Some(logical) = &mut state.logical {
            logical.set_position(f64::from(f32::from(-offset)) * logical.max());
        } else {
            self.handle.0.borrow_mut().deferred_scroll_to_item = None;
            ScrollbarDriver::set_axis_offset(&self.handle, axis, offset.min(px(0.0)));
        }
        trace(&state, "thumb");
    }
    fn drag_started(&self, axis: ScrollbarAxis) {
        let extent = ScrollbarDriver::max_offset(&self.handle, axis);
        let mut state = self.interaction.borrow_mut();
        state.dragging = true;
        state.manual_pending = true;
        state.generation = state.generation.wrapping_add(1);
        state.frozen_extent = Some(extent);
        self.handle.0.borrow_mut().deferred_scroll_to_item = None;
    }
    fn drag_ended(&self, _axis: ScrollbarAxis) {
        let mut state = self.interaction.borrow_mut();
        state.dragging = false;
        state.frozen_extent = None;
    }
}

impl HistoryView {
    pub(super) fn cancel_history_scroll_reveal(&mut self) {
        self.history_scroll.0.borrow_mut().deferred_scroll_to_item = None;
        if let Some(pending) = self.pending_history_reveal.take() {
            self.store.dispatch(Msg::FinishCommitReveal {
                repo_id: pending.repo_id,
            });
        }
    }
    pub(super) fn history_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.cancel_history_scroll_reveal();
        let mut scroll = self.scroll_interaction.borrow_mut();
        scroll.generation = scroll.generation.wrapping_add(1);
        if let Some(logical) = &mut scroll.logical {
            let delta = event.delta.pixel_delta(window.line_height());
            logical.set_position(logical.position() - f64::from(f32::from(delta.y)));
            cx.stop_propagation();
            cx.notify();
        }
        trace(&scroll, "wheel");
        drop(scroll);
        let _ = self.root_view.update(cx, |root, cx| {
            root.close_history_refs_hover(cx);
            root.dismiss_commit_message_hover(cx);
        });
    }
}

/// Opt-in input diagnostics without commit contents or object IDs.
pub(super) fn trace(state: &ScrollInteraction, reason: &str) {
    #[cfg(debug_assertions)]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *ENABLED.get_or_init(|| std::env::var_os("GITCOMET_HISTORY_SCROLL_TRACE").is_some()) {
            if let Some(viewport) = &state.logical {
                eprintln!(
                    "history_scroll reason={reason} generation={} dragging={} top={} within={:.4} total={} viewport={:.2}",
                    state.generation,
                    state.dragging,
                    viewport.top,
                    viewport.within,
                    viewport.total,
                    viewport.viewport
                );
            } else {
                eprintln!(
                    "history_scroll reason={reason} generation={} dragging={} bootstrap=true",
                    state.generation, state.dragging
                );
            }
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (state, reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_deltas_remain_precise_at_two_million_rows() {
        let mut viewport = LogicalViewport::new(2_000_000, 56.0, 800.0);
        viewport.set_position(1_900_000.0 * 56.0 + 0.25);
        for _ in 0..100 {
            viewport.set_position(viewport.position() + 0.25);
        }
        assert_eq!(viewport.top, 1_900_000);
        assert_eq!(viewport.within, 25.25);
        assert!(viewport.visible_range().len() < 20);
        viewport.set_position(f64::MAX);
        assert_eq!(viewport.position(), viewport.max());
    }
}
