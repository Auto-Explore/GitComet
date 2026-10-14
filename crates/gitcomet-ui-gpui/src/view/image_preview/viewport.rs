pub(super) const STEPS: &[f64] = &[
    0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0, 64.0,
];

#[derive(Clone, Copy, Debug)]
pub(super) struct Camera {
    pub fit: bool,
    pub scale: f64,
    pub center: (f64, f64),
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            fit: true,
            scale: 1.0,
            center: (0.0, 0.0),
        }
    }
}
impl Camera {
    pub fn layout(&mut self, view: (f64, f64), natural: (f64, f64), cap: bool) {
        if view.0 <= 0.0 || view.1 <= 0.0 || natural.0 <= 0.0 || natural.1 <= 0.0 {
            return;
        }
        if self.fit {
            self.scale =
                (view.0 / natural.0)
                    .min(view.1 / natural.1)
                    .min(if cap { 1.0 } else { 64.0 });
            self.center = (natural.0 / 2.0, natural.1 / 2.0);
        }
        self.clamp(view, natural);
    }
    pub fn clamp(&mut self, view: (f64, f64), natural: (f64, f64)) {
        let axis = |p: f64, v: f64, n: f64| {
            let half = v / self.scale / 2.0;
            if half >= n / 2.0 {
                n / 2.0
            } else {
                p.clamp(half, n - half)
            }
        };
        self.center = (
            axis(self.center.0, view.0, natural.0),
            axis(self.center.1, view.1, natural.1),
        );
    }
    pub fn origin(self, view: (f64, f64)) -> (f64, f64) {
        (
            (view.0 / 2.0 - self.center.0 * self.scale).round(),
            (view.1 / 2.0 - self.center.1 * self.scale).round(),
        )
    }
    pub fn zoom(&mut self, next: f64, pointer: (f64, f64), view: (f64, f64)) {
        let offset = (pointer.0 - view.0 / 2.0, pointer.1 - view.1 / 2.0);
        let next = next.clamp(self.scale.min(STEPS[0]), 64.0);
        self.center.0 += offset.0 / self.scale - offset.0 / next;
        self.center.1 += offset.1 / self.scale - offset.1 / next;
        self.scale = next;
        self.fit = false;
    }
    pub fn step(self, up: bool) -> f64 {
        if up {
            STEPS
                .iter()
                .copied()
                .find(|s| *s > self.scale + 1e-8)
                .unwrap_or(64.0)
        } else {
            STEPS
                .iter()
                .rev()
                .copied()
                .find(|s| *s < self.scale - 1e-8)
                .unwrap_or(self.scale)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_preview_lifecycle_zoom_out_below_one_percent_never_zooms_in() {
        for scale in [0.005, 0.001] {
            let mut camera = Camera {
                fit: true,
                scale,
                center: (50000.0, 50000.0),
            };
            camera.zoom(camera.step(false), (250.0, 250.0), (500.0, 500.0));
            assert!(
                camera.scale <= scale,
                "zooming out from {scale} increased the scale to {}",
                camera.scale
            );
        }
    }
    #[test]
    fn pointer_zoom_preserves_source_point_and_clamps_pan() {
        let mut camera = Camera {
            fit: false,
            scale: 1.0,
            center: (500.0, 500.0),
        };
        camera.zoom(3.0, (300.0, 100.0), (400.0, 400.0));
        assert!((camera.center.0 + 100.0 / 3.0 - 600.0).abs() < 1e-8);
        assert!((camera.center.1 - 100.0 / 3.0 - 400.0).abs() < 1e-8);
        camera.center = (-1e6, 1e6);
        camera.clamp((400.0, 400.0), (1000.0, 1000.0));
        assert_eq!(camera.center.0, 200.0 / 3.0);
        assert_eq!(camera.center.1, 1000.0 - 200.0 / 3.0);
    }
    #[test]
    fn fit_uses_device_pixels_and_icon_cap() {
        let mut camera = Camera::default();
        camera.layout((600.0, 300.0), (100.0, 100.0), false);
        assert_eq!(camera.scale, 3.0);
        camera.layout((600.0, 300.0), (100.0, 100.0), true);
        assert_eq!(camera.scale, 1.0);
        assert_eq!(camera.origin((601.0, 301.0)), (251.0, 101.0));
    }
}
