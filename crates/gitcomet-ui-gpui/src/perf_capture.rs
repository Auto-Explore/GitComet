//! Ad-hoc workflow profiling.
//!
//! Brackets a named workflow at runtime — "open this repo", "scroll this
//! diff", "stage 400 files" — and records what the CPU, the allocator and the
//! GPU did while it ran. Stopping the capture writes a sidecar JSON report in
//! the same shape the Criterion perf suite emits, so the existing comparison
//! tooling can read it, and marks the region on the Tracy timeline when a
//! profiler is attached.
//!
//! The whole module is behind the `workflow-profiler` feature. A default
//! release build compiles none of it, keeps the plain mimalloc global
//! allocator, and registers no actions.
//!
//! Three sources feed one capture:
//!
//! * **CPU** — Tracy zones from `#[profiling::function]` and
//!   `profiling::scope!`, including the ones already inside gpui-ce's window
//!   draw and text shaping. Frame boundaries come from gpui-ce's existing
//!   `profiling::finish_frame!` at present.
//! * **Allocations** — the process allocator is wrapped so Tracy sees every
//!   alloc and free with a call stack, while the existing [`crate::perf_alloc`]
//!   counters keep working for the report's totals.
//! * **GPU** — per-frame execution spans read back from timestamp queries by
//!   gpui-ce's renderer, forwarded here and republished as Tracy GPU zones.
//!   Only present in a `workflow-profiler-gpu` build.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use gpui::{App, Global, Window, actions};
use serde_json::{Map, Value, json};

use crate::{
    perf_alloc::{self, PerfAllocMetrics},
    perf_sidecar::{PerfSidecarReport, write_sidecar},
};

actions!(
    perf,
    [
        /// Starts a workflow profiling capture, or stops the running one.
        ToggleWorkflowCapture,
    ]
);

/// Directory the reports land in, overridable so a scripted run can collect
/// them somewhere other than the working directory.
const OUTPUT_DIR_ENV: &str = "GITCOMET_WORKFLOW_CAPTURE_DIR";

/// Call stack depth recorded for each allocation. Deeper stacks attribute
/// allocations more precisely but cost more per allocation; 16 frames is
/// enough to get out of the allocator wrappers and into GitComet code.
pub const ALLOC_CALLSTACK_DEPTH: u16 = 16;

/// The global allocator for a workflow-profiler build: Tracy's allocation
/// tracker wrapped around the existing stats-tracking mimalloc, so both the
/// Tracy timeline and [`perf_alloc::current_alloc_metrics`] observe every
/// allocation.
pub type CaptureAllocator =
    tracy_client::ProfiledAllocator<&'static perf_alloc::PerfTrackingAllocator>;

/// Builds the allocator installed as `#[global_allocator]` by the binary.
pub const fn capture_allocator() -> CaptureAllocator {
    tracy_client::ProfiledAllocator::new(&perf_alloc::TRACKING_MIMALLOC, ALLOC_CALLSTACK_DEPTH)
}

/// GPU spans collected since the last capture ended.
///
/// Written from the renderer thread by the callback installed in [`init`] and
/// drained on the foreground thread when a capture stops, so it lives outside
/// the gpui `App` rather than in a [`Global`].
static GPU_FRAMES: OnceLock<Mutex<GpuFrameLog>> = OnceLock::new();

fn gpu_frames() -> &'static Mutex<GpuFrameLog> {
    GPU_FRAMES.get_or_init(|| Mutex::new(GpuFrameLog::default()))
}

/// Bounded so a capture left running overnight cannot grow without limit. At
/// 120 fps this holds a bit over eight minutes of frames.
#[cfg(any(feature = "workflow-profiler-gpu", test))]
const MAX_GPU_FRAMES: usize = 60_000;

#[derive(Default)]
struct GpuFrameLog {
    durations_nanos: Vec<u64>,
    dropped: u64,
}

impl GpuFrameLog {
    #[cfg(any(feature = "workflow-profiler-gpu", test))]
    fn record(&mut self, duration_nanos: u64) {
        if self.durations_nanos.len() >= MAX_GPU_FRAMES {
            self.dropped += 1;
            return;
        }
        self.durations_nanos.push(duration_nanos);
    }

    fn take(&mut self) -> (Vec<u64>, u64) {
        (std::mem::take(&mut self.durations_nanos), std::mem::replace(&mut self.dropped, 0))
    }
}

/// One running capture.
struct ActiveCapture {
    name: String,
    started_at: Instant,
    alloc_at_start: PerfAllocMetrics,
    /// Number of frame-loop ticks observed during the capture.
    frames: u64,
    /// Gaps between successive ticks.
    frame_intervals: Vec<Duration>,
    last_frame_at: Option<Instant>,
    /// Held for the capture's lifetime so the whole workflow shows up as one
    /// zone on the Tracy timeline.
    _tracy_span: Option<tracy_client::Span>,
}

#[derive(Default)]
struct CaptureState {
    active: Option<ActiveCapture>,
}

impl Global for CaptureState {}

/// Installs the Tracy client, starts receiving GPU frame timings, and
/// registers the capture action.
///
/// Safe to call when no profiler is attached: Tracy's on-demand mode collects
/// nothing until one connects.
pub fn init(cx: &mut App) {
    tracy_client::Client::start();
    install_gpu_frame_timing_sink();

    cx.set_global(CaptureState::default());
    cx.on_action(|_: &ToggleWorkflowCapture, cx| {
        toggle(cx);
    });
}

#[cfg(feature = "workflow-profiler-gpu")]
fn install_gpu_frame_timing_sink() {
    use std::sync::Arc;

    gpui_wgpu::set_gpu_frame_timing_callback(Some(Arc::new(|timing: gpui_wgpu::GpuFrameTiming| {
        if let Ok(mut log) = gpu_frames().lock() {
            log.record(timing.duration_nanos());
        }
        emit_tracy_gpu_zone(timing);
    })));
}

#[cfg(not(feature = "workflow-profiler-gpu"))]
fn install_gpu_frame_timing_sink() {}

/// Republishes one GPU frame span onto the Tracy timeline.
///
/// The renderer already converted timestamp ticks to nanoseconds, so the
/// context is created with a period of 1 ns/tick and the first frame's start
/// as its calibration point.
#[cfg(feature = "workflow-profiler-gpu")]
fn emit_tracy_gpu_zone(timing: gpui_wgpu::GpuFrameTiming) {
    static CONTEXT: OnceLock<Option<tracy_client::GpuContext>> = OnceLock::new();

    let context = CONTEXT.get_or_init(|| {
        let client = tracy_client::Client::running()?;
        client
            .new_gpu_context(
                Some("GPU"),
                tracy_client::GpuContextType::Vulkan,
                timing.start_nanos as i64,
                1.0,
            )
            .inspect_err(|error| eprintln!("Tracy GPU context unavailable: {error}"))
            .ok()
    });

    let Some(context) = context else {
        return;
    };
    match context.span_alloc("frame", "present", file!(), line!()) {
        Ok(mut span) => {
            span.end_zone();
            span.upload_timestamp_start(timing.start_nanos as i64);
            span.upload_timestamp_end(timing.end_nanos as i64);
        }
        Err(error) => eprintln!("Tracy GPU span unavailable: {error}"),
    }
}

/// Whether a capture is currently running.
pub fn is_active(cx: &App) -> bool {
    cx.try_global::<CaptureState>()
        .is_some_and(|state| state.active.is_some())
}

/// Starts a capture if none is running, otherwise stops the running one and
/// writes its report.
pub fn toggle(cx: &mut App) {
    if is_active(cx) {
        match stop(cx) {
            Ok(path) => eprintln!("workflow capture written to {}", path.display()),
            Err(error) => eprintln!("workflow capture failed: {error}"),
        }
    } else {
        start(default_capture_name(), cx);
    }
}

fn default_capture_name() -> String {
    format!(
        "workflow_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since_epoch| since_epoch.as_secs())
            .unwrap_or_default()
    )
}

/// Begins a capture under the given name, replacing any capture already
/// running.
pub fn start(name: impl Into<String>, cx: &mut App) {
    let name = name.into();

    // Discard GPU frames from before the workflow so the report only covers
    // the bracketed region.
    if let Ok(mut log) = gpu_frames().lock() {
        let _discarded = log.take();
    }

    let tracy_span = tracy_client::Client::running()
        .map(|client| client.span_alloc(Some(&name), "workflow_capture", file!(), line!(), 0));

    let capture = ActiveCapture {
        name: name.clone(),
        started_at: Instant::now(),
        alloc_at_start: perf_alloc::current_alloc_metrics(),
        frames: 0,
        frame_intervals: Vec::new(),
        last_frame_at: None,
        _tracy_span: tracy_span,
    };

    cx.global_mut::<CaptureState>().active = Some(capture);

    // Frame sampling needs a window, and the first one that accepts the update
    // is enough since presentation cadence is shared across windows. Deferred
    // because a capture usually starts from an action handler, which may
    // already be inside a window update.
    cx.defer(|cx| {
        for handle in cx.windows() {
            if handle
                .update(cx, |_, window, cx| observe_frames(window, cx))
                .is_ok()
            {
                break;
            }
        }
    });

    eprintln!("workflow capture '{name}' started");
}

/// Ends the running capture and writes its report, returning where it landed.
pub fn stop(cx: &mut App) -> Result<PathBuf, String> {
    let capture = cx
        .global_mut::<CaptureState>()
        .active
        .take()
        .ok_or_else(|| "no workflow capture is running".to_string())?;

    let elapsed = capture.started_at.elapsed();
    let alloc_delta = perf_alloc::current_alloc_metrics().delta_since(capture.alloc_at_start);
    let (gpu_durations, gpu_dropped) = gpu_frames()
        .lock()
        .map(|mut log| log.take())
        .unwrap_or_else(|_| (Vec::new(), 0));

    let mut metrics = Map::new();
    metrics.insert("wall_nanos".to_string(), json!(elapsed.as_nanos() as u64));
    alloc_delta.append_to_payload_with_prefix(&mut metrics, "alloc_");
    append_frame_metrics(&mut metrics, capture.frames, &capture.frame_intervals);
    append_gpu_metrics(&mut metrics, &gpu_durations, gpu_dropped);

    let report = PerfSidecarReport::new(capture.name.clone(), metrics);
    let path = report_path(&capture.name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    write_sidecar(&report, &path)?;
    Ok(path)
}

/// Records one frame-loop tick against the running capture.
pub fn record_frame(cx: &mut App) {
    if !is_active(cx) {
        return;
    }
    let Some(capture) = cx.global_mut::<CaptureState>().active.as_mut() else {
        return;
    };

    let now = Instant::now();
    capture.frames += 1;
    if let Some(last_frame_at) = capture.last_frame_at {
        capture.frame_intervals.push(now - last_frame_at);
    }
    capture.last_frame_at = Some(now);
}

/// Keeps [`record_frame`] fed for as long as a capture is running.
///
/// Re-arming from inside the callback samples gpui's frame loop rather than a
/// timer, so the gaps track presentation cadence. They are not exactly a
/// present count: gpui runs next-frame callbacks before deciding whether the
/// window is dirty enough to draw, so an idle tick is counted too. Use
/// `gpu_frame_count` (or Tracy's frame marks) when an exact presented-frame
/// count matters.
///
/// Registering a callback also opts the window into gpui's frame-rate
/// throttling for inactive and thermally-limited states, so a capture can see
/// a slightly different cadence than an unobserved run.
pub fn observe_frames(window: &mut Window, cx: &mut App) {
    if !is_active(cx) {
        return;
    }
    window.on_next_frame(|window, cx| {
        record_frame(cx);
        observe_frames(window, cx);
    });
}

fn append_frame_metrics(metrics: &mut Map<String, Value>, frames: u64, intervals: &[Duration]) {
    metrics.insert("frame_count".to_string(), json!(frames));
    let mut nanos: Vec<u64> = intervals
        .iter()
        .map(|interval| interval.as_nanos() as u64)
        .collect();
    append_distribution(metrics, "frame_interval", &mut nanos);
}

fn append_gpu_metrics(metrics: &mut Map<String, Value>, durations: &[u64], dropped: u64) {
    metrics.insert("gpu_frame_count".to_string(), json!(durations.len() as u64));
    if dropped > 0 {
        metrics.insert("gpu_frames_dropped".to_string(), json!(dropped));
    }
    let mut durations = durations.to_vec();
    append_distribution(metrics, "gpu_frame", &mut durations);
}

/// Adds total/mean/percentile fields for a set of nanosecond samples. Sorts
/// `samples` in place.
fn append_distribution(metrics: &mut Map<String, Value>, prefix: &str, samples: &mut [u64]) {
    if samples.is_empty() {
        return;
    }
    samples.sort_unstable();

    let total: u128 = samples.iter().map(|sample| u128::from(*sample)).sum();
    let mean = (total / samples.len() as u128) as u64;
    metrics.insert(format!("{prefix}_total_nanos"), json!(total as u64));
    metrics.insert(format!("{prefix}_mean_nanos"), json!(mean));
    for (label, percentile) in [("p50", 50), ("p95", 95), ("p99", 99)] {
        let index = (samples.len() - 1) * percentile / 100;
        // `index` is derived from `len`, so it is always in bounds here.
        if let Some(sample) = samples.get(index) {
            metrics.insert(format!("{prefix}_{label}_nanos"), json!(sample));
        }
    }
    if let Some(max) = samples.last() {
        metrics.insert(format!("{prefix}_max_nanos"), json!(max));
    }
}

fn report_path(name: &str) -> PathBuf {
    let directory = std::env::var_os(OUTPUT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new("target").join("workflow-captures"));
    directory.join(format!("{}.json", sanitize(name)))
}

/// Keeps a capture name usable as a single path component.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_reports_percentiles_over_sorted_samples() {
        let mut metrics = Map::new();
        let mut samples = vec![50, 10, 30, 20, 40];
        append_distribution(&mut metrics, "frame_interval", &mut samples);

        assert_eq!(metrics["frame_interval_total_nanos"], json!(150));
        assert_eq!(metrics["frame_interval_mean_nanos"], json!(30));
        assert_eq!(metrics["frame_interval_p50_nanos"], json!(30));
        assert_eq!(metrics["frame_interval_max_nanos"], json!(50));
    }

    #[test]
    fn distribution_of_no_samples_adds_nothing() {
        let mut metrics = Map::new();
        append_distribution(&mut metrics, "gpu_frame", &mut []);
        assert!(metrics.is_empty());
    }

    #[test]
    fn gpu_frame_log_stops_growing_at_the_cap() {
        let mut log = GpuFrameLog::default();
        for _ in 0..MAX_GPU_FRAMES + 5 {
            log.record(1_000);
        }

        let (durations, dropped) = log.take();
        assert_eq!(durations.len(), MAX_GPU_FRAMES);
        assert_eq!(dropped, 5);
        assert_eq!(log.take(), (Vec::new(), 0));
    }

    #[test]
    fn capture_names_stay_single_path_components() {
        assert_eq!(sanitize("open repo/../etc"), "open_repo____etc");
        assert_eq!(sanitize("scroll-diff_1"), "scroll-diff_1");
    }
}
