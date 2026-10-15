use super::cache::TileKey;
use crate::view::zoom_wheel::{ZoomStep, ZoomWheel};
use gitcomet_diagrams::scene::TILE_SIZE;
use std::ops::Range;

pub(super) const ZOOM_STEPS: [f32; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
/// The 100% step a viewport opens and resets at.
const DEFAULT_ZOOM: u8 = 2;
/// Room a reset leaves around a diagram.
const PADDING: f32 = 16.0;

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
    pub(super) fn clamp(&mut self) {
        let scale = ZOOM_STEPS[self.zoom as usize];
        let axis = |pan: f32, view: f32, graph: f32| {
            pan.clamp(32.0 - (graph * scale).max(32.0), (view - 32.0).max(0.0))
        };
        self.pan = (
            axis(self.pan.0, self.size.0, self.natural.0),
            axis(self.pan.1, self.size.1, self.natural.1),
        );
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
