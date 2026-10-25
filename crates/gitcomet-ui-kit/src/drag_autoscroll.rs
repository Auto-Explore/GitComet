//! Autoscroll while a press-and-drag holds the pointer past a viewport edge.
//! Every text-selection surface uses this, so all of them scroll alike.

use gpui::{Pixels, px};
use std::time::Duration;

/// How often a held drag scrolls.
pub const DRAG_AUTOSCROLL_TICK: Duration = Duration::from_millis(16);

/// The largest offset change one tick makes.
pub const DRAG_AUTOSCROLL_MAX_STEP: Pixels = px(48.0);

const MIN_STEP: Pixels = px(2.0);

/// One tick's scroll-offset change on one axis, for a pointer against the
/// viewport span `min..=max`. Zero inside it; past an edge it grows with the
/// distance. Positive moves the offset toward the start (reveals content
/// above or to the left), matching `ScrollHandle` offsets.
pub fn drag_autoscroll_step(pointer: Pixels, min: Pixels, max: Pixels) -> Pixels {
    fn speed(distance: Pixels) -> Pixels {
        (distance * 0.4).max(MIN_STEP).min(DRAG_AUTOSCROLL_MAX_STEP)
    }

    if pointer < min {
        speed(min - pointer)
    } else if pointer > max {
        -speed(pointer - max)
    } else {
        px(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_the_viewport_nothing_scrolls() {
        for pointer in [px(10.0), px(50.0), px(90.0)] {
            assert_eq!(drag_autoscroll_step(pointer, px(10.0), px(90.0)), px(0.0));
        }
    }

    #[test]
    fn past_an_edge_it_scrolls_toward_that_edge_faster_the_further_out() {
        assert_eq!(drag_autoscroll_step(px(0.0), px(10.0), px(90.0)), px(4.0));
        assert_eq!(
            drag_autoscroll_step(px(100.0), px(10.0), px(90.0)),
            px(-4.0)
        );
        assert_eq!(
            drag_autoscroll_step(px(140.0), px(10.0), px(90.0)),
            px(-20.0)
        );
    }

    #[test]
    fn the_step_is_clamped_at_both_ends() {
        assert_eq!(
            drag_autoscroll_step(px(91.0), px(10.0), px(90.0)),
            -MIN_STEP
        );
        assert_eq!(
            drag_autoscroll_step(px(-900.0), px(10.0), px(90.0)),
            DRAG_AUTOSCROLL_MAX_STEP
        );
    }
}
