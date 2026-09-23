//! Opt-in UI responsiveness probe, for chasing "the app feels laggy" reports.
//!
//! Enabled with `GITCOMET_UI_PROBE=1`. Every interval (default one second,
//! `GITCOMET_UI_PROBE_INTERVAL_MS`) it writes one summary line to stderr, and
//! to the file named by `GITCOMET_UI_PROBE_LOG` when that is set:
//!
//! - `frames`: windows drawn in the interval, with `draw` time statistics
//!   (time spent inside `Window::draw`, i.e. layout + prepaint + paint on the
//!   main thread) and `slow16`, the number of frames over 16 ms.
//! - `dirty_to_draw`: how long the first invalidation of a frame waited until
//!   that frame finished drawing. This is the closest proxy for input latency
//!   the app can measure itself.
//! - `wake`: how long a foreground task wake-up takes to reach the main thread.
//!   A plain thread stamps a timestamp every few milliseconds; the probe's
//!   foreground task measures how late it runs. Large values mean the main
//!   thread was busy (drawing, running tasks, or blocked in a platform call)
//!   and could not service its queue.
//! - `main_cpu`: CPU time the main thread consumed as a share of wall time,
//!   averaged over at least five seconds to smooth Windows scheduler-tick
//!   quantization (Windows only; `n/a` elsewhere).
//! - `submit`: CPU time in platform drawing/presentation, excluding GPU/display
//!   completion. `GITCOMET_UI_PROBE_JSONL` additionally records individual frame
//!   events and interval input-to-submission histograms. Timestamps are relative
//!   to the `start` record's Unix clock anchor, for external scenario alignment.
//!
//! One-off sections such as window creation are timed with [`time_section`]
//! and logged as their own lines.
//!
//! Everything here is inert unless the env var is set: the pinger thread is
//! never started and gpui's profiler tracing stays off.

use gitcomet_core::process::write_stderr_line;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ENABLED_ENV: &str = "GITCOMET_UI_PROBE";
const LOG_PATH_ENV: &str = "GITCOMET_UI_PROBE_LOG";
const JSONL_PATH_ENV: &str = "GITCOMET_UI_PROBE_JSONL";
const INTERVAL_ENV: &str = "GITCOMET_UI_PROBE_INTERVAL_MS";
const DEFAULT_INTERVAL: Duration = Duration::from_millis(1000);
/// `GetThreadTimes` is scheduler-tick quantized on Windows. Average several UI
/// reporting intervals so `main_cpu` is useful rather than tick noise.
const CPU_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
/// Spacing between wake-latency pings. Small enough that a stall of a few
/// frames is sampled several times, large enough to stay negligible.
const PING_INTERVAL: Duration = Duration::from_millis(4);
const SLOW_FRAME: Duration = Duration::from_millis(16);

struct ProbeLog {
    started: Instant,
    writer: mpsc::SyncSender<ProbeRecord>,
    jsonl: bool,
    dropped: AtomicU64,
}

impl ProbeLog {
    fn enqueue(&self, record: ProbeRecord) {
        if self.writer.try_send(record).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

enum ProbeRecord {
    Text(String),
    Json(Value),
    /// Acknowledged once every earlier record is written and flushed.
    Flush(mpsc::SyncSender<()>),
}
static NEXT_ACTION: AtomicU64 = AtomicU64::new(1);

/// `None` until [`start_if_enabled`] runs with the probe switched on.
static LOG: OnceLock<ProbeLog> = OnceLock::new();

fn open_log(variable: &str) -> Option<File> {
    let path = std::env::var_os(variable).filter(|path| !path.is_empty())?;
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(file),
        Err(error) => {
            write_stderr_line(format_args!(
                "ui-probe: cannot open {variable}={path:?}: {error}"
            ));
            None
        }
    }
}

fn write_json(records: &[Value]) {
    let Some(log) = LOG.get().filter(|log| log.jsonl) else {
        return;
    };
    for record in records {
        log.enqueue(ProbeRecord::Json(record.clone()));
    }
}

fn probe_writer(rx: mpsc::Receiver<ProbeRecord>, file: Option<File>, jsonl: Option<File>) {
    let mut file = file.map(BufWriter::new);
    let mut jsonl = jsonl.map(BufWriter::new);
    let result = (|| -> std::io::Result<()> {
        while let Ok(first) = rx.recv() {
            for record in std::iter::once(first).chain(rx.try_iter().take(255)) {
                match record {
                    ProbeRecord::Text(text) => {
                        write_stderr_line(format_args!("{text}"));
                        if let Some(file) = file.as_mut() {
                            writeln!(file, "{text}")?;
                        }
                    }
                    ProbeRecord::Json(value) => {
                        if let Some(file) = jsonl.as_mut() {
                            serde_json::to_writer(&mut *file, &value)?;
                            file.write_all(b"\n")?;
                        }
                    }
                    ProbeRecord::Flush(ack) => {
                        if let Some(file) = file.as_mut() {
                            file.flush()?;
                        }
                        if let Some(file) = jsonl.as_mut() {
                            file.flush()?;
                        }
                        let _ = ack.send(());
                    }
                }
            }
            if let Some(file) = file.as_mut() {
                file.flush()?;
            }
            if let Some(file) = jsonl.as_mut() {
                file.flush()?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        write_stderr_line(format_args!("ui-probe writer failed: {error}"));
    }
}

/// Waits, at most `timeout`, until everything logged so far is written. The
/// writer thread is detached, so records still queued at exit are otherwise lost.
pub(crate) fn flush(timeout: Duration) {
    let Some(log) = LOG.get() else {
        return;
    };
    flush_writer(&log.writer, timeout);
}

fn flush_writer(writer: &mpsc::SyncSender<ProbeRecord>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    let mut record = ProbeRecord::Flush(ack_tx);
    // Never block on a full queue: a writer stuck on a closed stderr pipe
    // must not hang the app's quit.
    loop {
        match writer.try_send(record) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                record = returned;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return,
        }
    }
    let _ = ack_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
}

/// A diagnostic action starts at application handling, not at hardware input.
pub(crate) fn begin_action(kind: &'static str) -> u64 {
    let Some(log) = LOG.get().filter(|log| log.jsonl) else {
        return 0;
    };
    let id = NEXT_ACTION.fetch_add(1, Ordering::Relaxed);
    write_json(&[
        json!({"event":"action", "kind":kind, "phase":"begin", "id":id,
        "at_ms":milliseconds(log.started.elapsed())}),
    ]);
    id
}

pub(crate) fn action_phase(id: u64, phase: &'static str, detail: impl FnOnce() -> Value) {
    if id == 0 {
        return;
    }
    let Some(log) = LOG.get() else {
        return;
    };
    write_json(&[json!({"event":"action", "phase":phase, "id":id,
        "at_ms":milliseconds(log.started.elapsed()), "detail":detail()})]);
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn frame_record(event: &gpui::profiler::FrameEvent, origin: Instant) -> Value {
    let timestamp = |at: Instant| milliseconds(at.saturating_duration_since(origin));
    match event {
        gpui::profiler::FrameEvent::Draw(frame) => json!({
            "event": "draw", "window": format!("{:?}", frame.window_id),
            "at_ms": timestamp(frame.draw_end), "start_ms": timestamp(frame.draw_start),
            "dirty_ms": frame.dirty_at.map(timestamp), "duration_ms": milliseconds(frame.draw_duration()),
            "invalidations": frame.invalidations,
        }),
        gpui::profiler::FrameEvent::Present(frame) => json!({
            "event": "submit", "window": format!("{:?}", frame.window_id),
            "at_ms": timestamp(frame.present_end), "start_ms": timestamp(frame.present_start),
            "duration_ms": milliseconds(frame.present_duration()),
            "animation_interval_ms": frame.animation_interval.map(milliseconds),
        }),
    }
}

fn log_line(text: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let stamped = format!("[+{:8.3}s] {text}", log.started.elapsed().as_secs_f64());
    log.enqueue(ProbeRecord::Text(stamped));
}

/// Run `f`, logging how long it took when the probe is enabled. Free when the
/// probe is off beyond one relaxed load.
pub(crate) fn time_section<R>(label: &str, f: impl FnOnce() -> R) -> R {
    if LOG.get().is_none() {
        return f();
    }
    let started = Instant::now();
    let result = f();
    log_line(&format!(
        "ui-probe section {label} took {}",
        ms(started.elapsed())
    ));
    result
}

pub(crate) fn start_if_enabled(cx: &mut gpui::App) {
    if !crate::startup_probe::env_flag(ENABLED_ENV) || LOG.get().is_some() {
        return;
    }

    let interval = std::env::var(INTERVAL_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_INTERVAL);
    let main_cpu = MainThreadCpu::capture();

    // The pinger only stamps timestamps; the measurement happens on the main
    // thread when the foreground task below resumes. Unbounded so a long stall
    // shows up as many late samples rather than as dropped ones.
    let (ping_tx, ping_rx) = smol::channel::unbounded::<Instant>();
    let spawned = std::thread::Builder::new()
        .name("ui-probe-pinger".to_string())
        .spawn(move || {
            // `try_send` on an unbounded channel only fails once the receiver
            // (and with it the foreground task, and the app) is gone.
            while ping_tx.try_send(Instant::now()).is_ok() {
                std::thread::sleep(PING_INTERVAL);
            }
        });
    if let Err(err) = spawned {
        write_stderr_line(format_args!(
            "ui-probe: failed to start pinger thread: {err}"
        ));
        return;
    }

    let file = open_log(LOG_PATH_ENV);
    let jsonl = open_log(JSONL_PATH_ENV);
    let jsonl_enabled = jsonl.is_some();
    let (writer, records) = mpsc::sync_channel(8192);
    if std::thread::Builder::new()
        .name("ui-probe-writer".into())
        .spawn(move || probe_writer(records, file, jsonl))
        .is_err()
    {
        return;
    }
    let started = Instant::now();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(milliseconds);
    if LOG
        .set(ProbeLog {
            started,
            writer,
            jsonl: jsonl_enabled,
            dropped: AtomicU64::new(0),
        })
        .is_err()
    {
        // Another start won the race. Dropping the receiver makes this call's
        // pinger notice the closed channel and exit.
        drop(ping_rx);
        return;
    }

    cx.on_app_quit(|_| {
        flush(Duration::from_secs(2));
        async {}
    })
    .detach();

    // Do not turn on process-wide frame collection until a collector and the
    // pinger that drives it are both guaranteed to exist.
    gpui::profiler::set_trace_enabled(true);
    write_json(&[json!({"event": "start", "version": 2, "unix_ms": unix_ms,
        "pid": std::process::id(), "os": std::env::consts::OS,
        "debug_assertions": cfg!(debug_assertions), "interval_ms": interval.as_millis()})]);

    log_line(&format!(
        "ui-probe start os={} debug_assertions={} interval={}ms ping={}ms main_cpu={}",
        std::env::consts::OS,
        cfg!(debug_assertions),
        interval.as_millis(),
        PING_INTERVAL.as_millis(),
        if main_cpu.available() {
            "available"
        } else {
            "n/a"
        },
    ));

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let mut frame_collector = gpui::profiler::FrameTimingCollector::new();
        let mut wake_latencies: Vec<Duration> = Vec::with_capacity(512);
        let mut interval_started = Instant::now();
        let mut cpu_sample_started = Instant::now();
        let mut cpu_at_sample_start = main_cpu.cpu_time();
        let mut main_cpu_pct = None;
        let mut previous_input = HashMap::<gpui::WindowId, gpui::profiler::InputLatencySnapshot>::new();

        loop {
            let Ok(sent_at) = ping_rx.recv().await else {
                break;
            };
            wake_latencies.push(sent_at.elapsed());

            let elapsed = interval_started.elapsed();
            if elapsed < interval {
                continue;
            }

            let now = Instant::now();
            let cpu_sample_elapsed = now.duration_since(cpu_sample_started);
            if cpu_sample_elapsed >= CPU_SAMPLE_INTERVAL {
                let cpu_now = main_cpu.cpu_time();
                main_cpu_pct = main_cpu_percent(cpu_at_sample_start, cpu_now, cpu_sample_elapsed);
                cpu_sample_started = now;
                cpu_at_sample_start = cpu_now;
            }

            let frames = frame_collector.collect_unseen();
            if LOG.get().is_some_and(|log| log.jsonl) {
                let mut records: Vec<_> = frames.iter().map(|frame| frame_record(frame, started)).collect();
                records.push(json!({"event": "interval", "at_ms": milliseconds(now.duration_since(started)),
                    "records_dropped": LOG.get().map(|log| log.dropped.load(Ordering::Relaxed)).unwrap_or(0),
                    "wall_ms": milliseconds(now.duration_since(interval_started)), "main_cpu_percent": main_cpu_pct,
                    "wake_ms": wake_latencies.iter().copied().map(milliseconds).collect::<Vec<_>>() }));
                cx.update(|app| {
                    let windows = app.windows();
                    previous_input.retain(|id, _| windows.iter().any(|window| window.window_id() == *id));
                    for handle in windows {
                        let _ = handle.update(app, |_, window, _| {
                            let snapshot = window.input_latency_snapshot();
                            let mut latency = snapshot.latency_histogram.clone();
                            let mut dropped = snapshot.mid_draw_events_dropped;
                            if let Some(previous) = previous_input.get(&handle.window_id()) {
                                // A newly created/reset histogram starts another interval.
                                if latency.subtract(&previous.latency_histogram).is_err() {
                                    latency = snapshot.latency_histogram.clone();
                                }
                                dropped = dropped.saturating_sub(previous.mid_draw_events_dropped);
                            }
                            previous_input.insert(handle.window_id(), snapshot);
                            records.push(json!({"event": "input_interval", "window": format!("{:?}", handle.window_id()),
                                "at_ms": milliseconds(now.duration_since(started)), "count": latency.len(),
                                "wall_ms": milliseconds(now.duration_since(interval_started)),
                                "p95_ms": if latency.is_empty() { None } else { Some(latency.value_at_quantile(0.95) as f64 / 1e6) },
                                "max_ms": if latency.is_empty() { None } else { Some(latency.max() as f64 / 1e6) },
                                "histogram_ns": latency.iter_recorded().map(|value| (value.value_iterated_to(), value.count_since_last_iteration())).collect::<Vec<_>>(),
                                "mid_draw_events_dropped": dropped}));
                        });
                    }
                });
                write_json(&records);
            }
            let summary = IntervalSummary::new(
                now.duration_since(interval_started),
                &frames,
                &mut wake_latencies,
                main_cpu_pct,
            );
            log_line(&summary.render());

            wake_latencies.clear();
            interval_started = now;
        }
    })
    .detach();
}

fn main_cpu_percent(
    before: Option<Duration>,
    after: Option<Duration>,
    wall: Duration,
) -> Option<f64> {
    let (Some(before), Some(after)) = (before, after) else {
        return None;
    };
    if wall.is_zero() {
        return None;
    }
    Some(
        (after.saturating_sub(before).as_secs_f64() / wall.as_secs_f64() * 100.0).clamp(0.0, 100.0),
    )
}

struct DurationStats {
    count: usize,
    avg: Duration,
    p95: Duration,
    p99: Duration,
    max: Duration,
}

impl DurationStats {
    fn from_sorted(sorted: &[Duration]) -> Option<Self> {
        if sorted.is_empty() {
            return None;
        }
        let total: Duration = sorted.iter().sum();
        Some(Self {
            count: sorted.len(),
            avg: total / sorted.len() as u32,
            p95: percentile(sorted, 95),
            p99: percentile(sorted, 99),
            max: sorted[sorted.len() - 1],
        })
    }
}

fn percentile(sorted: &[Duration], pct: usize) -> Duration {
    debug_assert!(!sorted.is_empty());
    debug_assert!((1..=100).contains(&pct));
    let rank = sorted.len().saturating_mul(pct).div_ceil(100);
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn ms(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

struct IntervalSummary {
    wall: Duration,
    frames: usize,
    slow_frames: usize,
    invalidations: u64,
    draw: Option<DurationStats>,
    submit: Option<DurationStats>,
    dirty_to_draw: Option<DurationStats>,
    wake: Option<DurationStats>,
    main_cpu_pct: Option<f64>,
}

impl IntervalSummary {
    fn new(
        wall: Duration,
        frame_events: &[gpui::profiler::FrameEvent],
        wake_latencies: &mut [Duration],
        main_cpu_pct: Option<f64>,
    ) -> Self {
        let mut draws = Vec::with_capacity(frame_events.len());
        let mut submissions = Vec::with_capacity(frame_events.len());
        let mut dirty_to_draws = Vec::with_capacity(frame_events.len());
        let mut invalidations = 0;
        for event in frame_events {
            match event {
                gpui::profiler::FrameEvent::Draw(frame) => {
                    draws.push(frame.draw_duration());
                    dirty_to_draws.extend(frame.dirty_to_draw_duration());
                    invalidations += frame.invalidations;
                }
                gpui::profiler::FrameEvent::Present(frame) => {
                    submissions.push(frame.present_duration())
                }
            }
        }

        draws.sort_unstable();
        submissions.sort_unstable();
        dirty_to_draws.sort_unstable();
        wake_latencies.sort_unstable();

        Self {
            wall,
            frames: draws.len(),
            slow_frames: draws.iter().filter(|d| **d > SLOW_FRAME).count(),
            invalidations,
            draw: DurationStats::from_sorted(&draws),
            submit: DurationStats::from_sorted(&submissions),
            dirty_to_draw: DurationStats::from_sorted(&dirty_to_draws),
            wake: DurationStats::from_sorted(wake_latencies),
            main_cpu_pct,
        }
    }

    fn render(&self) -> String {
        let mut out = format!(
            "ui-probe wall={} frames={} slow16={} invalidations={}",
            ms(self.wall),
            self.frames,
            self.slow_frames,
            self.invalidations
        );
        match &self.draw {
            Some(draw) => out.push_str(&format!(
                " draw[avg={} p95={} max={}]",
                ms(draw.avg),
                ms(draw.p95),
                ms(draw.max)
            )),
            None => out.push_str(" draw[-]"),
        }
        match &self.dirty_to_draw {
            Some(latency) => out.push_str(&format!(
                " dirty_to_draw[avg={} p95={} max={}]",
                ms(latency.avg),
                ms(latency.p95),
                ms(latency.max)
            )),
            None => out.push_str(" dirty_to_draw[-]"),
        }
        match &self.wake {
            Some(wake) => out.push_str(&format!(
                " wake[n={} avg={} p99={} max={}]",
                wake.count,
                ms(wake.avg),
                ms(wake.p99),
                ms(wake.max)
            )),
            None => out.push_str(" wake[-]"),
        }
        match self.main_cpu_pct {
            Some(pct) => out.push_str(&format!(" main_cpu={pct:.1}%")),
            None => out.push_str(" main_cpu=n/a"),
        }
        match &self.submit {
            Some(submit) => out.push_str(&format!(
                " submit[n={} avg={} p95={} max={}]",
                submit.count,
                ms(submit.avg),
                ms(submit.p95),
                ms(submit.max)
            )),
            None => out.push_str(" submit[-]"),
        }
        out
    }
}

/// CPU time of the main thread, sampled from the main thread's own tasks.
struct MainThreadCpu {
    #[cfg(target_os = "windows")]
    clock: Option<gitcomet_win32_window_utils::ThreadCpuClock>,
}

impl MainThreadCpu {
    /// Must be called on the main thread.
    fn capture() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            clock: gitcomet_win32_window_utils::ThreadCpuClock::for_current_thread(),
        }
    }

    fn available(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.clock.is_some()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    fn cpu_time(&self) -> Option<Duration> {
        #[cfg(target_os = "windows")]
        {
            self.clock.as_ref().and_then(|clock| clock.cpu_time())
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_flushes_while_the_application_is_still_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("live.jsonl");
        let file = File::create(&path).unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let writer = std::thread::spawn(move || probe_writer(rx, None, Some(file)));
        tx.send(ProbeRecord::Json(json!({"event": "live"})))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(&path).unwrap() != "{\"event\":\"live\"}\n" {
            assert!(
                Instant::now() < deadline,
                "records must flush without shutdown or a flush request"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!writer.is_finished());
        drop(tx);
        writer.join().unwrap();
    }

    #[test]
    fn full_probe_queue_counts_drops_without_blocking_the_ui() {
        let (writer, rx) = mpsc::sync_channel(1);
        let log = ProbeLog {
            started: Instant::now(),
            writer,
            jsonl: true,
            dropped: AtomicU64::new(0),
        };
        log.enqueue(ProbeRecord::Json(json!({"id": 1})));
        log.enqueue(ProbeRecord::Json(json!({"id": 2})));
        assert_eq!(log.dropped.load(Ordering::Relaxed), 1);
        assert!(matches!(rx.try_recv(), Ok(ProbeRecord::Json(value)) if value["id"] == 1));
    }

    #[test]
    fn flush_returns_after_every_queued_record_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.jsonl");
        let (tx, rx) = mpsc::sync_channel(8192);
        let file = File::create(&path).unwrap();
        let writer = std::thread::spawn(move || probe_writer(rx, None, Some(file)));
        // Large enough that the writer is still busy when this thread reads.
        let padding = "x".repeat(2048);
        for id in 0..4000 {
            tx.try_send(ProbeRecord::Json(json!({ "id": id, "padding": padding })))
                .unwrap();
        }
        flush_writer(&tx, Duration::from_secs(10));
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written.lines().count(), 4000);
        drop(tx);
        writer.join().unwrap();
    }

    #[test]
    fn submission_time_is_separate_from_cpu_drawing_and_raw_timestamps() {
        use gpui::profiler::{FrameEvent, FrameTiming, PresentTiming};
        let origin = Instant::now();
        let frames = [
            FrameEvent::Draw(FrameTiming {
                window_id: 1_u64.into(),
                dirty_at: Some(origin),
                invalidations: 2,
                draw_start: origin + Duration::from_millis(8),
                draw_end: origin + Duration::from_millis(12),
            }),
            FrameEvent::Present(PresentTiming {
                window_id: 1_u64.into(),
                present_start: origin + Duration::from_millis(12),
                present_end: origin + Duration::from_millis(34),
                animation_interval: None,
            }),
        ];
        let summary = IntervalSummary::new(Duration::from_secs(1), &frames, &mut [], None);
        assert_eq!(summary.frames, 1);
        assert_eq!(summary.slow_frames, 0);
        assert_eq!(summary.draw.as_ref().unwrap().avg, Duration::from_millis(4));
        assert_eq!(
            summary.submit.as_ref().unwrap().avg,
            Duration::from_millis(22)
        );
        assert_eq!(
            summary.dirty_to_draw.as_ref().unwrap().avg,
            Duration::from_millis(12)
        );
        let raw = frame_record(&frames[1], origin);
        assert_eq!(raw["event"], "submit");
        assert_eq!(raw["start_ms"], 12.0);
        assert_eq!(raw["at_ms"], 34.0);
        assert_eq!(raw["duration_ms"], 22.0);
    }

    #[test]
    fn percentile_uses_nearest_rank_for_small_samples() {
        let samples = [Duration::from_millis(3), Duration::from_millis(40)];

        assert_eq!(percentile(&samples, 95), Duration::from_millis(40));
        assert_eq!(percentile(&samples, 99), Duration::from_millis(40));
    }

    #[test]
    fn main_cpu_percentage_is_bounded_to_one_thread() {
        assert_eq!(
            main_cpu_percent(
                Some(Duration::ZERO),
                Some(Duration::from_millis(1016)),
                Duration::from_secs(1),
            ),
            Some(100.0)
        );
    }
}
