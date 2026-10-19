use super::cache::TileKey;
use gitcomet_diagrams::scene::TILE_SIZE;
use gitcomet_ui_kit::ScrollbarAxis;
use gitcomet_ui_kit::zoom_wheel::{ZoomStep, ZoomWheel};
use std::ops::Range;

pub(super) const ZOOM_STEPS: [f32; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
/// The 100% step a viewport opens and resets at.
const DEFAULT_ZOOM: u8 = 2;
/// Room a reset leaves around a diagram.
const PADDING: f32 = 16.0;
/// Less overflow than this is layout rounding a box to device pixels.
const LEAST_OVERFLOW: f32 = 1.0;

/// The viewport height that shows the whole diagram at 100%.
pub(super) fn full_height(natural_height: f32) -> f32 {
    natural_height * ZOOM_STEPS[DEFAULT_ZOOM as usize] + 2.0 * PADDING
}
#[derive(Clone, Debug)]
pub(super) struct Viewport {
    pub(super) zoom: u8,
    pub(super) pan: (f32, f32),
    pub(super) size: (f32, f32),
    pub(super) natural: (f32, f32),
    pub(super) initialized: bool,
    pub(super) dragging: Option<(f32, f32)>,
    zoom_wheel: ZoomWheel,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: DEFAULT_ZOOM,
            pan: (0.0, 0.0),
            size: (0.0, 0.0),
            natural: (0.0, 0.0),
            initialized: false,
            dragging: None,
            zoom_wheel: ZoomWheel::default(),
        }
    }
}
impl Viewport {
    pub(super) fn reset(&mut self) {
        self.zoom = DEFAULT_ZOOM;
        self.dragging = None;
        self.zoom_wheel = ZoomWheel::default();
        self.initialized = true;
        let position = |view: f32, graph: f32| {
            if graph + 2.0 * PADDING <= view {
                (view - graph) / 2.0
            } else {
                PADDING
            }
        };
        self.pan = (
            position(self.size.0, self.natural.0),
            position(self.size.1, self.natural.1),
        );
    }
    pub(super) fn resize(&mut self, width: f32, height: f32, natural: (f32, f32)) -> bool {
        if self.size == (width, height) && self.natural == natural {
            return false;
        }
        self.size = (width, height);
        self.natural = natural;
        if !self.initialized {
            self.reset();
        } else {
            self.clamp();
        }
        true
    }
    /// Keeps the pan in `pan_range` on both axes.
    pub(super) fn clamp(&mut self) {
        let (left, right) = self.pan_range(ScrollbarAxis::Horizontal);
        let (top, bottom) = self.pan_range(ScrollbarAxis::Vertical);
        self.pan = (self.pan.0.clamp(left, right), self.pan.1.clamp(top, bottom));
    }
    /// The pans dragging, zooming and the scrollbars share along `axis`: where
    /// the diagram overflows, it covers the viewport (the scrollbar's range);
    /// where it fits, it stays inside.
    fn pan_range(&self, axis: ScrollbarAxis) -> (f32, f32) {
        let (view, content) = self.extent(axis);
        // The pan that puts the diagram's far edge, padded, at the viewport's.
        let far = view - content + PADDING;
        (far.min(PADDING), far.max(PADDING))
    }
    /// The viewport's length, and the diagram's at its zoom with the padding
    /// a reset leaves around it, along `axis`.
    fn extent(&self, axis: ScrollbarAxis) -> (f32, f32) {
        let scale = ZOOM_STEPS[self.zoom as usize];
        let (view, natural) = match axis {
            ScrollbarAxis::Horizontal => (self.size.0, self.natural.0),
            ScrollbarAxis::Vertical => (self.size.1, self.natural.1),
        };
        (view, natural * scale + 2.0 * PADDING)
    }
    /// How far the diagram runs past the viewport along `axis`.
    pub(super) fn overflow(&self, axis: ScrollbarAxis) -> f32 {
        let (view, content) = self.extent(axis);
        let overflow = content - view;
        if overflow < LEAST_OVERFLOW {
            0.0
        } else {
            overflow
        }
    }
    /// How far into that overflow the viewport has panned.
    pub(super) fn scrolled(&self, axis: ScrollbarAxis) -> f32 {
        let pan = match axis {
            ScrollbarAxis::Horizontal => self.pan.0,
            ScrollbarAxis::Vertical => self.pan.1,
        };
        (PADDING - pan).clamp(0.0, self.overflow(axis))
    }
    /// Pans so the viewport is `scrolled` into the overflow along `axis`. An
    /// axis that fits has nothing to scroll.
    pub(super) fn scroll_to(&mut self, axis: ScrollbarAxis, scrolled: f32) {
        let overflow = self.overflow(axis);
        if overflow == 0.0 {
            return;
        }
        let pan = PADDING - scrolled.clamp(0.0, overflow);
        match axis {
            ScrollbarAxis::Horizontal => self.pan.0 = pan,
            ScrollbarAxis::Vertical => self.pan.1 = pan,
        }
    }
    /// A plain wheel pans each axis the diagram overflows on: sideways for a
    /// swipe or Shift+wheel, and down too when `vertical`. A mostly vertical
    /// wheel otherwise goes on to the document. Whether the wheel was taken,
    /// which it is at an edge too, so it does not scroll something else.
    pub(super) fn wheel_pan(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        vertical: bool,
        line_height: gpui::Pixels,
    ) -> bool {
        if gitcomet_ui_kit::zoom_wheel::is_zoom_wheel(&event.modifiers) {
            return false;
        }
        let delta = event.delta.pixel_delta(line_height);
        let (dx, dy) = if event.modifiers.shift && delta.x == gpui::px(0.0) {
            (f32::from(delta.y), 0.0)
        } else {
            (f32::from(delta.x), f32::from(delta.y))
        };
        let mut taken = false;
        let mut pan = |axis, delta: f32| {
            if delta != 0.0 && self.overflow(axis) > 0.0 {
                let scrolled = self.scrolled(axis) - delta;
                self.scroll_to(axis, scrolled);
                taken = true;
            }
        };
        if vertical || dx.abs() > dy.abs() {
            pan(ScrollbarAxis::Horizontal, dx);
        }
        if vertical {
            pan(ScrollbarAxis::Vertical, dy);
        }
        taken
    }
    /// Zooms `delta` steps, keeping the scene point under `anchor` (in
    /// viewport coordinates) where it is.
    pub(super) fn zoom_by(&mut self, delta: i8, anchor: (f32, f32)) {
        let old = ZOOM_STEPS[self.zoom as usize];
        self.zoom = (self.zoom as i8 + delta).clamp(0, 7) as u8;
        let ratio = ZOOM_STEPS[self.zoom as usize] / old;
        self.pan = (
            anchor.0 - (anchor.0 - self.pan.0) * ratio,
            anchor.1 - (anchor.1 - self.pan.1) * ratio,
        );
        self.clamp();
    }
    /// Ctrl/Cmd+scroll at `pointer`; whether it took a zoom step.
    pub(super) fn wheel_zoom(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        pointer: (f32, f32),
    ) -> bool {
        let Some(step) = self.zoom_wheel.step(event) else {
            return false;
        };
        let delta = match step {
            ZoomStep::In => 1,
            ZoomStep::Out => -1,
        };
        self.zoom_by(delta, pointer);
        true
    }
    pub(super) fn drag_to(&mut self, position: (f32, f32)) {
        if let Some(previous) = self.dragging {
            self.pan.0 += position.0 - previous.0;
            self.pan.1 += position.1 - previous.1;
            self.dragging = Some(position);
            self.clamp();
        }
    }
    /// Tiles under `visible`: the on-screen part of the viewport, horizontal
    /// then vertical, relative to its top-left corner.
    pub(super) fn keys(
        &self,
        scene: u64,
        dpr: f32,
        visible: (Range<f32>, Range<f32>),
    ) -> Vec<TileKey> {
        if visible.0.is_empty() || visible.1.is_empty() {
            return Vec::new();
        }
        let scale = ZOOM_STEPS[self.zoom as usize] * dpr;
        let axis = |pan: f32, visible: Range<f32>, graph: f32| {
            let start = ((visible.start - pan) * dpr / TILE_SIZE as f32)
                .floor()
                .max(0.0) as u32;
            let end = (((visible.end - pan) * dpr / TILE_SIZE as f32)
                .ceil()
                .max(0.0) as u32)
                .min((graph * scale / TILE_SIZE as f32).ceil() as u32);
            start..end
        };
        let mut keys = Vec::new();
        for row in axis(self.pan.1, visible.1, self.natural.1) {
            for column in axis(self.pan.0, visible.0.clone(), self.natural.0) {
                keys.push(TileKey {
                    scene,
                    zoom: self.zoom,
                    dpr: dpr.to_bits(),
                    column,
                    row,
                });
            }
        }
        keys
    }
}
