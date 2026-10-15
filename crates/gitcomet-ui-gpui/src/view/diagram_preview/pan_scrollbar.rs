//! Scrollbars over a diagram viewport, driven by its pan.
use super::*;
use crate::kit::ScrollbarDriver;
use crate::view::components::ScrollbarAxis;

/// Drives a diagram viewport's scrollbars from the viewport it shares with
/// the diagram, as the current frame laid it out.
#[derive(Clone)]
pub(super) struct PanScrollDriver {
    pub(super) viewport: Rc<RefCell<Viewport>>,
    pub(super) state: gpui::WeakEntity<DiagramState>,
}

impl ScrollbarDriver for PanScrollDriver {
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        px(self.viewport.borrow().overflow(axis))
    }

    // Negative, like a scroll handle's offset.
    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        -px(self.viewport.borrow().scrolled(axis))
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        self.viewport
            .borrow_mut()
            .scroll_to(axis, f32::from(offset).abs());
    }

    fn interaction_changed(&self, cx: &mut App) {
        let _ = self.state.update(cx, |this, cx| {
            this.invalidate_tiles();
            cx.notify();
        });
    }
}
