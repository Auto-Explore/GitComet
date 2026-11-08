use gpui::TouchPhase;
use std::time::{Duration, Instant};

const MAX_STRETCH: f32 = 72.0;
const RESISTANCE: f32 = 0.4;
const SPRING_RATE: f32 = 20.0;
const MOMENTUM_GAP: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct Motion {
    travel: f32,
    offset: f32,
    spring: Option<Spring>,
    touching: bool,
    trackpad: bool,
    bounced: bool,
    last_event: Option<Instant>,
    velocity_start: Option<Instant>,
    velocity_distance: f32,
    velocity: f32,
}

struct Spring {
    started: Instant,
    position: f32,
    velocity: f32,
}

fn rubber_band(travel: f32) -> f32 {
    let distance = travel.abs() * RESISTANCE;
    travel.signum() * MAX_STRETCH * distance / (MAX_STRETCH + distance)
}

fn travel_for_offset(offset: f32) -> f32 {
    let offset = offset.clamp(-MAX_STRETCH + 0.01, MAX_STRETCH - 0.01);
    offset * MAX_STRETCH / (RESISTANCE * (MAX_STRETCH - offset.abs()))
}

impl Motion {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn animating(&self) -> bool {
        self.spring.is_some()
    }

    pub fn touching(&self) -> bool {
        self.touching
    }

    pub fn allow_native_momentum(&mut self, now: Instant) {
        // Entering a pane during momentum must not leave a missed finger-lift
        // holding an old stretch indefinitely.
        self.release(now);
        if !self.trackpad {
            self.bounced = false;
        }
        self.trackpad = true;
    }

    pub fn advance(&mut self, now: Instant) {
        let Some(spring) = &self.spring else { return };
        let t = now.saturating_duration_since(spring.started).as_secs_f32();
        // Analytic critically damped spring: stable even after a slow frame.
        let coefficient = spring.velocity + SPRING_RATE * spring.position;
        let decay = (-SPRING_RATE * t).exp();
        let position = (spring.position + coefficient * t) * decay;
        let velocity = (spring.velocity - SPRING_RATE * coefficient * t) * decay;
        if t >= 0.65 || (position.abs() < 0.1 && velocity.abs() < 2.0) {
            self.offset = 0.0;
            self.spring = None;
        } else {
            self.offset = position.clamp(-MAX_STRETCH, MAX_STRETCH);
        }
    }

    pub fn begin(&mut self, phase: TouchPhase, now: Instant) {
        self.advance(now);
        if !self.touching
            && self
                .last_event
                .is_some_and(|last| now.saturating_duration_since(last) > MOMENTUM_GAP)
        {
            // Smooth mouse-wheel drivers also emit pixel deltas, but no touch
            // phases. Do not mistake their next gesture for trackpad momentum.
            self.trackpad = false;
        }
        match phase {
            TouchPhase::Started => {
                if !self.touching {
                    self.travel = travel_for_offset(self.offset);
                    self.spring = None;
                    self.velocity_start = Some(now);
                    self.velocity_distance = 0.0;
                    self.velocity = 0.0;
                    self.bounced = false;
                }
                self.touching = true;
                self.trackpad = true;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => self.release(now),
            TouchPhase::Moved => {}
        }
        self.last_event = Some(now);
    }

    pub fn release(&mut self, now: Instant) {
        if !self.touching {
            return;
        }
        self.touching = false;
        self.travel = 0.0;
        if self.offset != 0.0 {
            self.start_spring(now, 0.0);
            self.bounced = true;
        }
    }

    /// Returning the finger toward the document first takes up the stretched
    /// distance. Only the remainder moves the logical scroll position.
    pub fn take_reversal(&mut self, delta: f32, now: Instant) -> f32 {
        if !self.touching {
            if self.offset != 0.0 && delta.signum() != self.offset.signum() {
                let previous = self.offset;
                self.offset = if delta.abs() < previous.abs() {
                    previous + delta
                } else {
                    0.0
                };
                self.spring = None;
                if self.offset != 0.0 {
                    self.start_spring(now, 0.0);
                }
                return delta - (self.offset - previous);
            }
            return delta;
        }
        if self.travel == 0.0 || delta.signum() == self.travel.signum() {
            return delta;
        }
        let previous = self.travel;
        self.travel = if delta.abs() < previous.abs() {
            previous + delta
        } else {
            0.0
        };
        self.offset = rubber_band(self.travel);
        delta - (self.travel - previous)
    }

    pub fn record_delta(&mut self, delta: f32, now: Instant) {
        let start = *self.velocity_start.get_or_insert(now);
        self.velocity_distance += delta;
        let elapsed = now.saturating_duration_since(start).as_secs_f32();
        // Multiple packets may arrive in one frame. Aggregate them instead of
        // dividing each packet by an almost-zero processing interval.
        if elapsed >= 0.008 {
            self.velocity = (self.velocity_distance / elapsed).clamp(-8000.0, 8000.0);
            if elapsed >= 0.05 {
                self.velocity_start = Some(now);
                self.velocity_distance = 0.0;
            }
        }
    }

    pub fn overscroll(&mut self, unused: f32, now: Instant) {
        if !self.trackpad || unused == 0.0 || !unused.is_finite() {
            return;
        }
        if self.touching {
            self.travel = (self.travel + unused).clamp(-100_000.0, 100_000.0);
            self.offset = rubber_band(self.travel);
        } else if !self.bounced {
            // macOS already supplies the decelerating scroll stream. Throw an
            // edge once, then let the spring settle; later outward momentum
            // must not keep stretching or restart the bounce every packet.
            self.bounced = true;
            self.offset = rubber_band(unused);
            let velocity = if self.velocity.signum() == unused.signum() {
                self.velocity * 0.5
            } else {
                0.0
            };
            self.start_spring(
                now,
                velocity.clamp(-MAX_STRETCH * SPRING_RATE, MAX_STRETCH * SPRING_RATE),
            );
        }
    }

    fn start_spring(&mut self, now: Instant, velocity: f32) {
        self.spring = Some(Spring {
            started: now,
            position: self.offset,
            velocity,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finger_stretch_is_resisted_bounded_and_returns_after_lift() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(100.0, now);
        assert!((20.0..30.0).contains(&motion.offset()));
        assert!(
            !motion.animating(),
            "holding a finger still needs no frames"
        );
        motion.overscroll(1_000_000.0, now);
        assert!(motion.offset() <= MAX_STRETCH);
        motion.begin(TouchPhase::Ended, now);
        assert!(motion.animating());
        motion.advance(now + Duration::from_secs(1));
        assert_eq!(motion.offset(), 0.0);
        assert!(!motion.animating());
    }

    #[test]
    fn release_has_a_gentle_return_then_settles_without_an_idle_animation() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(100.0, now);
        let stretched = motion.offset();
        motion.begin(TouchPhase::Ended, now);
        motion.advance(now + Duration::from_millis(40));
        assert!((0.75..0.9).contains(&(motion.offset() / stretched)));
        motion.advance(now + Duration::from_millis(200));
        assert!((0.05..0.15).contains(&(motion.offset() / stretched)));
        motion.advance(now + Duration::from_millis(650));
        assert_eq!(motion.offset(), 0.0);
        assert!(!motion.animating());
    }

    #[test]
    fn native_momentum_recovers_a_missed_finger_lift() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(100.0, now);
        motion.allow_native_momentum(now + Duration::from_millis(20));
        assert!(!motion.touching());
        assert!(motion.animating());
        motion.advance(now + Duration::from_secs(1));
        assert_eq!(motion.offset(), 0.0);
    }

    #[test]
    fn reversal_takes_up_stretch_before_moving_the_document() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(100.0, now);
        assert_eq!(motion.take_reversal(-40.0, now), 0.0);
        assert!(motion.offset() > 0.0);
        assert_eq!(motion.take_reversal(-80.0, now), -20.0);
        assert_eq!(motion.offset(), 0.0);
    }

    #[test]
    fn momentum_hits_an_edge_once_and_never_synthesizes_interior_scrolling() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.record_delta(-40.0, now + Duration::from_millis(20));
        motion.begin(TouchPhase::Ended, now + Duration::from_millis(21));
        motion.begin(TouchPhase::Moved, now + Duration::from_millis(30));
        motion.overscroll(-40.0, now + Duration::from_millis(30));
        assert!(motion.animating());
        let started = motion.spring.as_ref().unwrap().started;
        for i in 4..20 {
            let time = now + Duration::from_millis(i * 10);
            motion.begin(TouchPhase::Moved, time);
            motion.overscroll(-20.0, time);
            assert_eq!(motion.spring.as_ref().unwrap().started, started);
        }
        motion.advance(now + Duration::from_secs(1));
        motion.overscroll(-20.0, now + Duration::from_secs(1));
        assert!(!motion.animating());
        assert_eq!(motion.offset(), 0.0);
    }

    #[test]
    fn phaseless_smooth_wheel_does_not_bounce_even_after_an_old_trackpad_gesture() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Moved, now);
        motion.overscroll(80.0, now);
        assert_eq!(motion.offset(), 0.0);
        motion.begin(TouchPhase::Started, now);
        motion.begin(TouchPhase::Ended, now);
        motion.begin(TouchPhase::Moved, now + Duration::from_secs(1));
        motion.overscroll(80.0, now + Duration::from_secs(1));
        assert_eq!(motion.offset(), 0.0);
    }

    #[test]
    fn a_new_touch_catches_a_returning_edge_without_a_jump() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(-100.0, now);
        motion.begin(TouchPhase::Ended, now);
        let time = now + Duration::from_millis(40);
        motion.advance(time);
        let previous = motion.offset();
        motion.begin(TouchPhase::Started, time);
        motion.overscroll(0.0, time);
        assert!((motion.offset() - previous).abs() < 0.001);
        assert!(!motion.animating());
    }

    #[test]
    fn inward_momentum_takes_up_visual_stretch_before_entering_the_document() {
        let now = Instant::now();
        let mut motion = Motion::default();
        motion.begin(TouchPhase::Started, now);
        motion.overscroll(100.0, now);
        motion.begin(TouchPhase::Ended, now);
        let stretch = motion.offset();
        assert_eq!(motion.take_reversal(-2.0, now), 0.0);
        assert!((motion.offset() - (stretch - 2.0)).abs() < 0.001);
        let remaining = motion.take_reversal(-(stretch + 10.0), now);
        assert!((remaining + 12.0).abs() < 0.001);
        assert_eq!(motion.offset(), 0.0);
        assert!(!motion.animating());
    }
}
