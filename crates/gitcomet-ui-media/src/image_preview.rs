//! Device-pixel image inspection shared by files, diffs and conflict previews.
use gitcomet_ui_kit::theme::AppTheme;
use gpui::App;
use gpui::Entity;
use gpui::IntoElement;
use gpui::Pixels;
use gpui::Point;
use gpui::WeakEntity;
use gpui::Window;
use gpui::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "benchmarks")]
pub mod benchmarks;
mod cache;
mod controls;
mod filter;
mod preferences;
mod resample;
mod source;
mod surface;
pub mod svg;
mod viewport;
use cache::{Cache, TileKey};
pub use controls::{OpenMenu, apply_menu_zoom, toolbar, viewport};
pub use filter::{ImageSamplingMode, ImageSmoothFilter};
pub use preferences::Preferences;
pub use source::Source;
pub use source::{limit_decoder, raster_format};
use viewport::Camera;

#[derive(Clone, Default)]
pub struct Input {
    pub source: Option<Arc<Source>>,
    pub preview: Option<Arc<gpui::RenderImage>>,
    pub frame: usize,
}

/// A toolbar menu the host opens on the chip's behalf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageMenu {
    Sampling,
    Zoom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fit,
    Scale(f64),
    In,
    Out,
}

struct Side {
    input: Input,
    camera: Camera,
    device_size: (f64, f64),
    generation: Arc<AtomicU64>,
    task: Option<gpui::Task<()>>,
    requested: Vec<TileKey>,
    error: Option<String>,
    focus: gpui::FocusHandle,
}
impl Side {
    fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.task = None;
        self.requested.clear();
        self.error = None;
    }
}

pub struct Group {
    pub target: String,
    sides: [Side; 3],
    cache: Arc<Cache>,
    owns_cache: bool,
    pub linked: bool,
    pub paused: bool,
    active: usize,
    cap_fit: bool,
    dragging: Option<(usize, Point<Pixels>)>,
    // Ctrl/Cmd+scroll distance not yet spent on a zoom step.
    zoom_scroll: f32,
}

/// Re-renders an owner when its next animation frame is due.
#[derive(Default)]
pub struct FrameTimer {
    deadline: Option<std::time::Instant>,
    task: Option<gpui::Task<()>>,
}
impl FrameTimer {
    pub fn schedule<V: 'static>(
        &mut self,
        deadline: Option<std::time::Instant>,
        timer: fn(&mut V) -> &mut Self,
        cx: &mut gpui::Context<V>,
    ) {
        if self.deadline == deadline {
            return;
        }
        self.deadline = deadline;
        // Replacing the task cancels the earlier timer.
        self.task = deadline.map(|deadline| {
            cx.spawn(async move |view: WeakEntity<V>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(deadline.saturating_duration_since(std::time::Instant::now()))
                    .await;
                let _ = view.update(cx, |this, cx| {
                    timer(this).deadline = None;
                    cx.notify();
                });
            })
        });
    }
}

/// Whether owners advance animation frames; the viewport shows detail otherwise.
pub fn animates(window: &Window, cx: &App) -> bool {
    window.is_window_active() && !cx.reduce_motion()
}

fn shown(sides: &[Side; 3]) -> Vec<u64> {
    sides
        .iter()
        .filter_map(|side| side.input.source.as_ref().map(|source| source.id))
        .collect()
}

impl Group {
    pub fn new(cx: &mut gpui::Context<Self>) -> Self {
        let originals = cx.default_global::<cache::SharedOriginals>().0.clone();
        Self::with_cache(Arc::new(Cache::new(originals)), true, cx)
    }

    pub fn shared(owner: &Entity<Self>, cx: &mut gpui::Context<Self>) -> Self {
        let cache = owner.read(cx).cache.clone();
        Self::with_cache(cache, false, cx)
    }

    fn with_cache(cache: Arc<Cache>, owns_cache: bool, cx: &mut gpui::Context<Self>) -> Self {
        cx.observe_global::<Preferences>(|this, cx| {
            for side in &mut this.sides {
                side.cancel();
            }
            cx.notify();
        })
        .detach();
        cx.on_release(|this, cx| {
            for side in &mut this.sides {
                side.cancel();
            }
            this.cache.show(&shown(&this.sides), &[]);
            if this.owns_cache {
                this.cache.close(cx);
            } else {
                this.cache.drain(cx);
            }
        })
        .detach();
        Self {
            target: String::new(),
            sides: std::array::from_fn(|_| Side {
                input: Input::default(),
                camera: Camera::default(),
                device_size: (0.0, 0.0),
                generation: Arc::new(AtomicU64::new(0)),
                task: None,
                requested: Vec::new(),
                error: None,
                focus: cx.focus_handle(),
            }),
            cache,
            owns_cache,
            linked: true,
            paused: false,
            active: 0,
            cap_fit: false,
            dragging: None,
            zoom_scroll: 0.0,
        }
    }

    pub fn set_inputs(
        &mut self,
        target: String,
        inputs: [Input; 3],
        cap_fit: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let reset = self.target != target;
        if reset {
            self.target = target;
            self.linked = true;
            self.paused = false;
            self.dragging = None;
            self.zoom_scroll = 0.0;
            self.active = inputs.iter().position(|i| i.source.is_some()).unwrap_or(0);
        }
        let mut changed = reset;
        if inputs[self.active].source.is_none() {
            self.active = inputs
                .iter()
                .position(|input| input.source.is_some())
                .unwrap_or(0);
        }
        self.cap_fit = cap_fit;
        let before = shown(&self.sides);
        for (side, input) in self.sides.iter_mut().zip(inputs) {
            if reset
                || side.input.source.as_ref().map(|s| s.id) != input.source.as_ref().map(|s| s.id)
                || side.input.frame != input.frame
            {
                side.cancel();
                changed = true;
            }
            if reset {
                side.camera = Camera::default();
                side.device_size = (0.0, 0.0);
            }
            side.input = input;
        }
        let after = shown(&self.sides);
        if before != after {
            self.cache.show(&before, &after);
            self.cache.drain(cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn natural(&self, index: usize) -> (f64, f64) {
        let dimensions = |side: &Side| side.input.source.as_ref().map_or((0, 0), |s| s.dimensions);
        let (w, h) = if self.linked {
            self.sides
                .iter()
                .map(dimensions)
                .fold((0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1)))
        } else {
            dimensions(&self.sides[index])
        };
        (f64::from(w), f64::from(h))
    }

    fn view_size(&self, index: usize) -> (f64, f64) {
        if !self.linked {
            return self.sides[index].device_size;
        }
        self.sides
            .iter()
            .filter(|side| {
                side.input.source.is_some() && side.device_size.0 > 0.0 && side.device_size.1 > 0.0
            })
            .map(|side| side.device_size)
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))
            .unwrap_or(self.sides[index].device_size)
    }

    fn layout(&mut self, index: usize) {
        let natural = self.natural(index);
        let view = self.view_size(index);
        let mut camera = self.sides[if self.linked { self.active } else { index }].camera;
        camera.layout(view, natural, self.cap_fit);
        if self.linked {
            for side in &mut self.sides {
                side.camera = camera;
            }
        } else {
            self.sides[index].camera = camera;
        }
    }

    pub fn zoom(&mut self, zoom: Zoom, pointer: Option<(f64, f64)>, cx: &mut gpui::Context<Self>) {
        let index = self.active;
        self.layout(index);
        let view = self.sides[index].device_size;
        let camera = &mut self.sides[index].camera;
        match zoom {
            Zoom::Fit => camera.fit = true,
            _ => {
                let next = match zoom {
                    Zoom::Scale(s) => s,
                    Zoom::In => camera.step(true),
                    Zoom::Out => camera.step(false),
                    Zoom::Fit => unreachable!(),
                };
                camera.zoom(next, pointer.unwrap_or((view.0 / 2.0, view.1 / 2.0)), view);
            }
        }
        self.layout(index);
        cx.notify();
    }

    fn zoom_side(&mut self, index: usize, zoom: Zoom, cx: &mut gpui::Context<Self>) {
        if self.sides[index].input.source.is_some() {
            self.active = index;
            self.zoom(zoom, None, cx);
        }
    }

    fn pan(&mut self, index: usize, delta: (f64, f64), cx: &mut gpui::Context<Self>) {
        self.active = index;
        let camera = &mut self.sides[index].camera;
        // Fit shows the whole image, so there is nothing to pan; keeping Fit
        // lets a resize refit.
        if camera.fit {
            return;
        }
        camera.center.0 -= delta.0 / camera.scale;
        camera.center.1 -= delta.1 / camera.scale;
        self.layout(index);
        cx.notify();
    }

    /// Ctrl/Cmd+scroll: one step per wheel notch or per 40 px of smooth
    /// scrolling, so a trackpad gesture does not run through every level.
    fn wheel_zoom(
        &mut self,
        index: usize,
        event: &gpui::ScrollWheelEvent,
        pointer: Point<Pixels>,
        dpr: f64,
        cx: &mut gpui::Context<Self>,
    ) {
        self.active = index;
        let (delta, step) = match event.delta {
            gpui::ScrollDelta::Lines(lines) => (lines.y, 1.0),
            gpui::ScrollDelta::Pixels(pixels) => (f32::from(pixels.y), 40.0),
        };
        if event.touch_phase == gpui::TouchPhase::Started || delta * self.zoom_scroll < 0.0 {
            self.zoom_scroll = 0.0;
        }
        self.zoom_scroll += delta;
        if self.zoom_scroll.abs() < step {
            return;
        }
        let zoom = if self.zoom_scroll > 0.0 {
            Zoom::In
        } else {
            Zoom::Out
        };
        self.zoom_scroll = 0.0;
        self.zoom(
            zoom,
            Some((
                f64::from(f32::from(pointer.x)) * dpr,
                f64::from(f32::from(pointer.y)) * dpr,
            )),
            cx,
        );
    }

    pub fn toggle_linked(&mut self, cx: &mut gpui::Context<Self>) {
        self.linked = !self.linked;
        if self.linked {
            // Adopt the active pane's actual zoom, including when that pane
            // was fitted to a smaller source. Fit can be requested again for
            // the combined source extents.
            self.sides[self.active].camera.fit = false;
        }
        self.layout(self.active);
        cx.notify();
    }
    pub fn toggle_paused(&mut self, cx: &mut gpui::Context<Self>) {
        self.paused = !self.paused;
        for side in &mut self.sides {
            side.cancel();
        }
        cx.notify();
    }
    pub fn animated(&self) -> bool {
        self.sides.iter().any(|s| {
            s.input
                .preview
                .as_ref()
                .is_some_and(|i| i.frame_count() > 1)
        })
    }
    pub fn comparison(&self) -> bool {
        self.sides
            .iter()
            .filter(|s| s.input.source.is_some())
            .count()
            > 1
    }
    pub fn raster(&self) -> bool {
        self.sides
            .iter()
            .any(|s| s.input.source.as_ref().is_some_and(|s| !s.vector))
    }
    pub fn zoom_label(&self) -> String {
        self.zoom_label_for(self.active)
    }
    fn zoom_label_for(&self, index: usize) -> String {
        let camera = self.sides[index].camera;
        if camera.fit {
            format!("Fit · {:.0}%", camera.scale * 100.0)
        } else {
            format!("{:.0}%", camera.scale * 100.0)
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn requested_tiles(&self) -> usize {
        self.sides.iter().map(|side| side.requested.len()).sum()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Group {
    /// The decoded preview `side` shows while its detailed tiles load.
    pub fn side_preview(&self, side: usize) -> Option<&Arc<gpui::RenderImage>> {
        self.sides[side].input.preview.as_ref()
    }

    /// Why `side`'s last detail request failed, if it did.
    pub fn side_error(&self, side: usize) -> Option<&str> {
        self.sides[side].error.as_deref()
    }
}

#[derive(gpui::IntoElement)]
pub struct ImageViewport {
    pub group: Entity<Group>,
    pub side: usize,
    pub theme: AppTheme,
}
impl gpui::RenderOnce for ImageViewport {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.group.update(cx, |group, cx| {
            group.render_viewport(self.side, self.theme, cx)
        })
    }
}

#[cfg(test)]
mod tests;
