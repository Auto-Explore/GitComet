use gpui::profiler::{
    hang::{HangDetector, HangIncident, HangTrigger},
    journal::ForegroundEvent,
};
use gpui::{App, Global, Priority, Task, Window};
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SLOW: Duration = Duration::from_millis(200);
const KEEP: usize = 16;

struct RecentInteractions {
    history: Arc<Mutex<History>>,
    _sampler: Option<Task<()>>,
}

impl Global for RecentInteractions {}

struct History {
    detector: HangDetector,
    incidents: VecDeque<(&'static str, Duration)>,
}

impl History {
    fn collect(&mut self) {
        for incident in self.detector.poll() {
            if let Some((label, elapsed)) = reported_stall(&incident) {
                retain_incident(&mut self.incidents, label, elapsed);
            }
        }
    }
}

fn reported_stall(incident: &HangIncident) -> Option<(&'static str, Duration)> {
    match incident.trigger {
        HangTrigger::Budget => Some(("combined foreground work", incident.snapshot.occupancy())),
        HangTrigger::Threshold => {
            let cause = incident.contributors.first()?;
            let label = match cause {
                ForegroundEvent::Action(_) => "action handler",
                ForegroundEvent::Input(_) => "input dispatch",
                ForegroundEvent::Draw(_) => "window draw",
                ForegroundEvent::Present(_) => "frame submission",
                ForegroundEvent::TaskPoll(_) | ForegroundEvent::SmallPolls(_) => "foreground task",
            };
            Some((label, cause.duration()))
        }
    }
}

fn retain_incident(
    incidents: &mut VecDeque<(&'static str, Duration)>,
    label: &'static str,
    elapsed: Duration,
) {
    if elapsed >= SLOW {
        if incidents.len() == KEEP {
            incidents.pop_front();
        }
        incidents.push_back((label, elapsed));
    }
}

pub(super) fn initialize(cx: &mut App) {
    let history = Arc::new(Mutex::new(History {
        detector: HangDetector::new(cx.foreground_journal(), SLOW, SLOW),
        incidents: VecDeque::with_capacity(KEEP),
    }));
    let sampler = crate::ui_runtime::current()
        .uses_background_compute()
        .then(|| {
            let history = Arc::clone(&history);
            let executor = cx.background_executor().clone();
            cx.background_executor()
                .spawn_with_priority(Priority::Low, async move {
                    loop {
                        executor.timer(Duration::from_secs(2)).await;
                        history.lock().unwrap().collect();
                    }
                })
        });
    cx.set_global(RecentInteractions {
        history,
        _sampler: sampler,
    });
}

pub(super) fn append_summary(summary: &mut String, window: &Window, cx: &mut App) {
    let mut draws = window.frame_duration_snapshot().draw_duration_histogram;
    let mut inputs = window.input_latency_snapshot().latency_histogram;
    for handle in cx.windows() {
        if handle.window_id() != window.window_handle().window_id() {
            let _ = handle.update(cx, |_, other, _| {
                let _ = draws.add(other.frame_duration_snapshot().draw_duration_histogram);
                let _ = inputs.add(other.input_latency_snapshot().latency_histogram);
            });
        }
    }
    summary.push_str("\n\nResponsiveness across open windows");
    for (label, histogram) in [("Frame draw", draws), ("Input to frame", inputs)] {
        if histogram.is_empty() {
            let _ = write!(summary, "\n{label} has no samples");
        } else {
            let ms = |q| histogram.value_at_quantile(q) as f64 / 1_000_000.0;
            let _ = write!(
                summary,
                "\n{label} p50 {:.2} ms, p95 {:.2} ms ({} samples)",
                ms(0.5),
                ms(0.95),
                histogram.len()
            );
        }
    }
    let records = {
        let mut history = cx.global::<RecentInteractions>().history.lock().unwrap();
        history.collect();
        history.incidents.clone()
    };
    let _ = write!(
        summary,
        "\nRecent foreground stalls of 200 ms or more ({})",
        records.len()
    );
    for (label, duration) in records {
        let _ = write!(summary, "\n  {label}, {} ms", duration.as_millis());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_single_events_and_combined_foreground_occupancy() {
        use gpui::profiler::journal::{FrameSnapshot, InputTiming, IntervalBoundary};
        let start = std::time::Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let reported = |spans: &[(u64, u64)]| {
            let snapshot = FrameSnapshot {
                interval_start: start,
                boundary: IntervalBoundary::Idle { ended_at: at(500) },
                events: spans
                    .iter()
                    .map(|&(from, to)| {
                        ForegroundEvent::Input(InputTiming {
                            kind: "key",
                            start: at(from),
                            end: at(to),
                            caused_invalidation: false,
                        })
                    })
                    .collect(),
                small_polls: Vec::new(),
                dropped_events: 0,
                journal_discontinuous: false,
            };
            reported_stall(&HangIncident::detect(snapshot, SLOW, SLOW).unwrap())
        };
        assert_eq!(
            reported(&[(20, 245)]),
            Some(("input dispatch", Duration::from_millis(225)))
        );
        assert_eq!(
            reported(&[(10, 120), (250, 360)]),
            Some(("combined foreground work", Duration::from_millis(220)))
        );
    }

    #[test]
    fn retains_the_latest_slow_interactions_in_order() {
        let mut records = VecDeque::new();
        retain_incident(&mut records, "fast", SLOW - Duration::from_micros(1));
        assert!(records.is_empty());
        for i in 0..=KEEP {
            retain_incident(
                &mut records,
                "input dispatch",
                SLOW + Duration::from_millis(i as u64),
            );
        }
        assert_eq!(records.len(), KEEP);
        assert_eq!(records.front().unwrap().1, SLOW + Duration::from_millis(1));
        assert_eq!(
            records.back().unwrap().1,
            SLOW + Duration::from_millis(KEEP as u64)
        );
    }
}
