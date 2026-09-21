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
//!
//! One-off sections such as window creation are timed with [`time_section`]
//! and logged as their own lines.
//!
//! Everything here is inert unless the env var is set: the pinger thread is
//! never started and gpui's profiler tracing stays off.

use gitcomet_core::process::write_stderr_line;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const ENABLED_ENV: &str = "GITCOMET_UI_PROBE";
const LOG_PATH_ENV: &str = "GITCOMET_UI_PROBE_LOG";
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
    file: Option<Mutex<File>>,
}

/// `None` until [`start_if_enabled`] runs with the probe switched on.
static LOG: OnceLock<ProbeLog> = OnceLock::new();

fn log_line(text: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    let stamped = format!("[+{:8.3}s] {text}", log.started.elapsed().as_secs_f64());
    write_stderr_line(format_args!("{stamped}"));
    if let Some(file) = &log.file {
        let mut file = file.lock().unwrap_or_else(|error| error.into_inner());
        let line = format!("{stamped}\n");
        let _ = file.write_all(line.as_bytes());
    }
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

    let file = std::env::var_os(LOG_PATH_ENV).and_then(|path| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    });
    if LOG
        .set(ProbeLog {
            started: Instant::now(),
            file,
        })
        .is_err()
    {
        // Another start won the race. Dropping the receiver makes this call's
        // pinger notice the closed channel and exit.
        drop(ping_rx);
        return;
    }

    // Do not turn on process-wide frame collection until a collector and the
    // pinger that drives it are both guaranteed to exist.
    gpui::profiler::set_trace_enabled(true);

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

    cx.spawn(async move |_cx: &mut gpui::AsyncApp| {
        let mut frame_collector = gpui::profiler::FrameTimingCollector::new();
        let mut wake_latencies: Vec<Duration> = Vec::with_capacity(512);
        let mut interval_started = Instant::now();
        let mut cpu_sample_started = Instant::now();
        let mut cpu_at_sample_start = main_cpu.cpu_time();
        let mut main_cpu_pct = None;

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

            let summary = IntervalSummary::new(
                now.duration_since(interval_started),
                &frame_collector.collect_unseen(),
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
        let mut dirty_to_draws = Vec::with_capacity(frame_events.len());
        let mut invalidations = 0;
        for event in frame_events {
            let gpui::profiler::FrameEvent::Draw(frame) = event else {
                continue;
            };
            draws.push(frame.draw_duration());
            dirty_to_draws.extend(frame.dirty_to_draw_duration());
            invalidations += frame.invalidations;
        }

        draws.sort_unstable();
        dirty_to_draws.sort_unstable();
        wake_latencies.sort_unstable();

        Self {
            wall,
            frames: draws.len(),
            slow_frames: draws.iter().filter(|d| **d > SLOW_FRAME).count(),
            invalidations,
            draw: DurationStats::from_sorted(&draws),
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
