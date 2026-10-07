//! Byte-based geometry helpers for single-line GPUI layouts.
//!
//! Geometry comes from the text backend that shaped the line, including its
//! cluster and bidirectional caret model.

use gpui::{CaretPosition, LineLayout, Pixels, point, px};

pub trait TextLayoutExt {
    fn x_for_index(&self, index: usize) -> Pixels;
    fn index_for_x(&self, x: Pixels) -> Option<usize>;
    fn closest_index_for_x(&self, x: Pixels) -> usize;
}

impl TextLayoutExt for LineLayout {
    fn x_for_index(&self, index: usize) -> Pixels {
        self.platform_layout
            .caret_bounds(
                CaretPosition::attached_to_next_cluster(index.min(self.len)),
                px(1.0),
            )
            .map_or(self.width, |bounds| bounds.origin.x)
    }

    fn index_for_x(&self, x: Pixels) -> Option<usize> {
        self.platform_layout
            .byte_index_from_pixel_point(point(x, px(0.5)), px(1.0))
            .ok()
    }

    fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.platform_layout
            .caret_from_pixel_point(point(x, px(0.5)), px(1.0))
            .unwrap_or_else(|caret| caret)
            .index
    }
}
