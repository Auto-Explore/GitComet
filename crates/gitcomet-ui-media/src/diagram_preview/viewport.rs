use super::cache::TileKey;
use gitcomet_diagrams::scene::TILE_SIZE;

pub(super) const ZOOM_STEPS: [f32; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
#[derive(Clone, Debug)]
pub(super) struct Viewport {
    pub(super) zoom: u8,
    pub(super) pan: (f32, f32),
    pub(super) size: (f32, f32),
    pub(super) natural: (f32, f32),
    pub(super) initialized: bool,
    pub(super) dragging: Option<(f32, f32)>,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 2,
            pan: (0.0, 0.0),
            size: (0.0, 0.0),
            natural: (0.0, 0.0),
            initialized: false,
            dragging: None,
        }
    }
}
impl Viewport {
    pub(super) fn reset(&mut self) {
        self.zoom = 2;
        self.dragging = None;
        self.initialized = true;
        let position = |view: f32, graph: f32| {
            if graph + 32.0 <= view {
                (view - graph) / 2.0
            } else {
                16.0
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
    pub(super) fn zoom_by(&mut self, delta: i8) {
        let old = ZOOM_STEPS[self.zoom as usize];
        self.zoom = (self.zoom as i8 + delta).clamp(0, 7) as u8;
        let ratio = ZOOM_STEPS[self.zoom as usize] / old;
        // Keep the scene point under the viewport center stationary.
        self.pan = (
            self.size.0 / 2.0 - (self.size.0 / 2.0 - self.pan.0) * ratio,
            self.size.1 / 2.0 - (self.size.1 / 2.0 - self.pan.1) * ratio,
        );
        self.clamp();
    }
    pub(super) fn drag_to(&mut self, position: (f32, f32)) {
        if let Some(previous) = self.dragging {
            self.pan.0 += position.0 - previous.0;
            self.pan.1 += position.1 - previous.1;
            self.dragging = Some(position);
            self.clamp();
        }
    }
    pub(super) fn keys(&self, scene: u64, dpr: f32) -> Vec<TileKey> {
        let scale = ZOOM_STEPS[self.zoom as usize] * dpr;
        let axis = |pan: f32, view: f32, graph: f32| {
            let start = (-pan * dpr / TILE_SIZE as f32).floor().max(0.0) as u32;
            let end = (((view - pan) * dpr / TILE_SIZE as f32).ceil().max(0.0) as u32)
                .min((graph * scale / TILE_SIZE as f32).ceil() as u32);
            start..end
        };
        let mut keys = Vec::new();
        for row in axis(self.pan.1, self.size.1, self.natural.1) {
            for column in axis(self.pan.0, self.size.0, self.natural.0) {
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
