//! Optional GPU execution diagnostics. Never substitute CPU submission durations.
#![allow(missing_docs)]
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

const CAPACITY: usize = 4096;
static NEXT_FRAME: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDERER: AtomicU64 = AtomicU64::new(1);

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("GPUI_GPU_TIMINGS")
            .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
    })
}
pub fn next_frame() -> u64 {
    NEXT_FRAME.fetch_add(1, Ordering::Relaxed)
}
pub fn next_renderer() -> u64 {
    NEXT_RENDERER.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct GpuMemory {
    pub path_bytes: u64,
    pub filter_bytes: Option<u64>,
    pub atlas_bytes: Option<u64>,
    pub upload_bytes: Option<u64>,
    pub cached_bytes: Option<u64>,
    pub retained_frame_bytes: Option<u64>,
    pub device_retention_bytes: Option<u64>,
    pub pooled_bytes: Option<u64>,
    pub pending_release_bytes: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct GpuPassTiming {
    pub name: String,
    pub duration_ns: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct GpuFrameMetrics {
    pub window_id: u64,
    pub frame_id: u64,
    pub renderer_id: u64,
    pub submission_id: u64,
    pub backend: &'static str,
    pub implemented_experiments: &'static [&'static str],
    #[serde(skip)]
    pub submitted_at: Instant,
    pub status: &'static str,
    pub gpu_duration_ns: Option<u64>,
    pub passes: Vec<GpuPassTiming>,
    pub pass_timing_complete: bool,
    pub query_samples_dropped: u64,
    pub primitive_batches: usize,
    pub path_vertices: usize,
    pub path_target: (u32, u32),
    pub memory: GpuMemory,
    pub cache_hits: u64,
    pub cache_misses: u64,
}
impl GpuFrameMetrics {
    pub fn new(
        scene: &crate::Scene,
        renderer_id: u64,
        submission_id: u64,
        backend: &'static str,
    ) -> Self {
        Self {
            window_id: scene.gpu_window_id,
            frame_id: scene.gpu_frame_id,
            renderer_id,
            submission_id,
            backend,
            implemented_experiments: match backend {
                "wgpu" | "metal" | "d3d11" => &[
                    "cropped-paths",
                    "shared-resources",
                    "cached-layers",
                    "batched-paths",
                    "partial-redraw",
                    "pooled-targets",
                ],
                _ => &[],
            },
            submitted_at: Instant::now(),
            status: "unsupported",
            gpu_duration_ns: None,
            passes: Vec::new(),
            pass_timing_complete: false,
            query_samples_dropped: 0,
            primitive_batches: scene.render_commands().len(),
            path_vertices: scene.paths.iter().map(|p| p.vertices.len()).sum(),
            path_target: (0, 0),
            memory: GpuMemory::default(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }
}
#[derive(Default)]
struct Records {
    next: u64,
    records: VecDeque<(u64, GpuFrameMetrics)>,
}
fn records() -> &'static Mutex<Records> {
    static RECORDS: OnceLock<Mutex<Records>> = OnceLock::new();
    RECORDS.get_or_init(Default::default)
}
thread_local! {
    static POLLERS: std::cell::RefCell<Vec<Box<dyn FnMut() -> bool>>> = std::cell::RefCell::new(Vec::new());
}
/// Pollers must be nonblocking and return false when their renderer is gone.
pub fn register_poller(poller: impl FnMut() -> bool + 'static) {
    POLLERS.with_borrow_mut(|pollers| pollers.push(Box::new(poller)));
}
pub fn record(frame: GpuFrameMetrics) {
    let mut log = records().lock();
    if log.records.len() == CAPACITY {
        log.records.pop_front();
    }
    let seq = log.next;
    log.next += 1;
    log.records.push_back((seq, frame));
}
#[derive(Default)]
pub struct GpuFrameCollector {
    cursor: u64,
    pub dropped: u64,
}
impl GpuFrameCollector {
    pub fn collect_unseen(&mut self) -> Vec<GpuFrameMetrics> {
        if !enabled() {
            return Vec::new();
        }
        POLLERS.with_borrow_mut(|pollers| pollers.retain_mut(|poll| poll()));
        let log = records().lock();
        if let Some((first, _)) = log.records.front() {
            self.dropped += first.saturating_sub(self.cursor);
        }
        let result = log
            .records
            .iter()
            .filter(|(seq, _)| *seq >= self.cursor)
            .map(|(_, frame)| frame.clone())
            .collect();
        self.cursor = log.next;
        result
    }
}

/// Reject wraparound, unavailable samples, and non-finite hardware periods.
pub fn elapsed_ns(start: u64, end: u64, period: f64) -> Option<u64> {
    if start == 0 || end < start || !period.is_finite() || period <= 0.0 {
        return None;
    }
    let duration = (end - start) as f64 * period;
    (duration.is_finite() && duration <= u64::MAX as f64).then_some(duration as u64)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_gpu_timestamps_are_missing_not_zero() {
        assert_eq!(elapsed_ns(50, 75, 2.0), Some(50));
        assert_eq!(elapsed_ns(50, 50, 2.0), Some(0));
        for (start, end, period) in [(0, 20, 1.), (20, 10, 1.), (20, 30, f64::NAN), (20, 30, 0.)] {
            assert_eq!(elapsed_ns(start, end, period), None);
        }
    }
}
