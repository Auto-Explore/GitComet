use gpui::{ScrollWheelEvent, TouchPhase};
use std::cell::Cell;
#[cfg(any(target_os = "macos", test))]
use std::time::Duration;
use std::time::Instant;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaySource {
    Touch,
    Momentum,
    Wheel,
}
pub(super) use ReplaySource as Source;

/// Metadata for a synchronously dispatched, explicitly synthetic replay packet.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ReplayInput {
    pub source: ReplaySource,
    pub phase: TouchPhase,
    pub at: Instant,
    pub momentum_ended: bool,
}

thread_local! {
    static REPLAY_INPUT: Cell<Option<ReplayInput>> = const { Cell::new(None) };
}

#[doc(hidden)]
pub fn with_replay_input<R>(input: ReplayInput, dispatch: impl FnOnce() -> R) -> R {
    struct Restore(Option<ReplayInput>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REPLAY_INPUT.set(self.0);
        }
    }
    let _restore = Restore(REPLAY_INPUT.replace(Some(input)));
    dispatch()
}

pub(super) struct Input {
    pub phase: TouchPhase,
    pub source: Option<Source>,
    pub at: Option<Instant>,
    pub momentum_ended: bool,
}

#[cfg(any(target_os = "macos", test))]
fn timestamp(now: Instant, native_time: f64, uptime: f64) -> Option<Instant> {
    let age = uptime - native_time;
    (native_time.is_finite() && uptime.is_finite() && (0.0..=0.1).contains(&age))
        .then(|| now.checked_sub(Duration::from_secs_f64(age)))
        .flatten()
}

/// GPUI's adapter drops momentum phase and native sample time. Only borrow
/// AppKit metadata for the matching packet, then map uptime into our clock.
pub(super) fn read(input: &ScrollWheelEvent, now: Instant) -> Input {
    if let Some(replay) = REPLAY_INPUT.get() {
        return Input {
            phase: replay.phase,
            source: Some(replay.source),
            at: Some(replay.at)
                .filter(|at| *at <= now && now.saturating_duration_since(*at).as_millis() <= 100),
            momentum_ended: replay.momentum_ended,
        };
    }
    let phase = input.touch_phase;
    #[cfg(all(target_os = "macos", not(test)))]
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        use gpui::ScrollDelta;
        use objc2_app_kit::{NSApplication, NSEventPhase, NSEventType};
        if let Some(event) = NSApplication::sharedApplication(main_thread).currentEvent()
            && event.r#type() == NSEventType::ScrollWheel
        {
            let native_phase = event.phase();
            let mapped_phase = match native_phase {
                NSEventPhase::MayBegin | NSEventPhase::Began => TouchPhase::Started,
                NSEventPhase::Ended => TouchPhase::Ended,
                _ => TouchPhase::Moved,
            };
            let delta = match input.delta {
                ScrollDelta::Pixels(delta) => delta.map(f32::from),
                ScrollDelta::Lines(delta) => delta,
            };
            if phase == mapped_phase
                && input.delta.precise() == event.hasPreciseScrollingDeltas()
                && delta.x == event.scrollingDeltaX() as f32
                && delta.y == event.scrollingDeltaY() as f32
            {
                let momentum = event.momentumPhase();
                return Input {
                    phase: if native_phase.contains(NSEventPhase::Cancelled)
                        || momentum.contains(NSEventPhase::Cancelled)
                    {
                        TouchPhase::Cancelled
                    } else {
                        phase
                    },
                    source: Some(if momentum != NSEventPhase::None {
                        Source::Momentum
                    } else if native_phase != NSEventPhase::None {
                        Source::Touch
                    } else {
                        Source::Wheel
                    }),
                    at: timestamp(
                        now,
                        event.timestamp(),
                        objc2_foundation::NSProcessInfo::processInfo().systemUptime(),
                    ),
                    momentum_ended: momentum.contains(NSEventPhase::Ended),
                };
            }
        }
    }
    let _ = now;
    Input {
        phase,
        source: None,
        at: None,
        momentum_ended: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> ScrollWheelEvent {
        ScrollWheelEvent {
            position: Default::default(),
            delta: gpui::ScrollDelta::Pixels(Default::default()),
            touch_phase: TouchPhase::Moved,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn native_time_is_mapped_by_age_and_invalid_clock_values_are_rejected() {
        let now = Instant::now();
        assert_eq!(
            timestamp(now, 99.996, 100.0),
            now.checked_sub(Duration::from_secs_f64(100.0 - 99.996))
        );
        for (sample, uptime) in [
            (101.0, 100.0),
            (99.0, 100.0),
            (f64::NAN, 100.0),
            (1.0, f64::INFINITY),
        ] {
            assert!(timestamp(now, sample, uptime).is_none());
        }
    }
    #[test]
    fn replay_scope_restores_nested_metadata_and_unwinds_without_leaking() {
        let now = Instant::now();
        let a = ReplayInput {
            source: Source::Momentum,
            phase: TouchPhase::Moved,
            at: now,
            momentum_ended: true,
        };
        let b = ReplayInput {
            source: Source::Touch,
            momentum_ended: false,
            ..a
        };
        assert!(read(&event(), now).source.is_none());
        with_replay_input(a, || {
            assert_eq!(read(&event(), now).source, Some(Source::Momentum));
            let result = std::panic::catch_unwind(|| {
                with_replay_input(b, || {
                    assert_eq!(read(&event(), now).source, Some(Source::Touch));
                    panic!("scoped replay");
                })
            });
            assert!(result.is_err());
            assert_eq!(read(&event(), now).source, Some(Source::Momentum));
        });
        assert!(read(&event(), now).source.is_none());
    }
}
