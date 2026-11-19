//! Retimes a bounded, already known native momentum path; never predicts it.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(super) const DELAY: Duration = Duration::from_micros(12_500);
const SAMPLE_INTERVAL: Duration = Duration::from_nanos(8_333_333);
const MAX_GAP: Duration = Duration::from_millis(100);
const CAPACITY: usize = 16;

struct Sample {
    at: Instant,
    position: f64,
}

#[derive(Default)]
pub(super) struct Momentum {
    samples: VecDeque<Sample>,
    total: f64,
    applied: f64,
    ended: bool,
    direction: f64,
    last_arrival: Option<Instant>,
    previous_direct: Option<Instant>,
    epoch: u64,
    scheduled: bool,
}

impl Momentum {
    pub fn cancel(&mut self) {
        self.samples.clear();
        self.total = 0.0;
        self.applied = 0.0;
        self.ended = false;
        self.direction = 0.0;
        self.last_arrival = None;
        self.previous_direct = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.scheduled = false;
    }

    pub fn direct(&mut self, at: Instant) {
        self.cancel();
        self.previous_direct = Some(at);
    }

    pub fn active(&self) -> bool {
        !self.samples.is_empty()
    }

    pub fn reverses(&self, delta: f64) -> bool {
        self.direction * delta < 0.0
    }

    pub fn pending(&self) -> f64 {
        self.total - self.applied
    }

    /// Returns false without modifying the queue when this packet cannot be
    /// interpolated safely. The caller then routes pending + incoming normally.
    pub fn push(&mut self, delta: f64, at: Instant, now: Instant, ended: bool) -> bool {
        if !delta.is_finite()
            || at > now
            || now.saturating_duration_since(at) > MAX_GAP
            || self.ended
            || self.samples.back().is_some_and(|last| at < last.at)
            || self
                .last_arrival
                .is_some_and(|last| now.saturating_duration_since(last) > MAX_GAP)
            || self.samples.len() >= CAPACITY
            || self.reverses(delta)
        {
            return false;
        }
        if self.samples.is_empty() {
            let baseline = self
                .previous_direct
                .filter(|previous| *previous < at && at.duration_since(*previous) <= MAX_GAP)
                .unwrap_or_else(|| at.checked_sub(SAMPLE_INTERVAL).unwrap_or(at));
            self.samples.push_back(Sample {
                at: baseline,
                position: 0.0,
            });
        }
        self.total += delta;
        if delta != 0.0 {
            self.direction = delta.signum();
        }
        if let Some(last) = self.samples.back_mut().filter(|last| last.at == at) {
            last.position = self.total;
        } else {
            self.samples.push_back(Sample {
                at,
                position: self.total,
            });
        }
        self.last_arrival = Some(now);
        self.ended = ended;
        true
    }

    pub fn schedule(&mut self) -> Option<u64> {
        if !self.active() || self.scheduled {
            return None;
        }
        self.scheduled = true;
        Some(self.epoch)
    }

    pub fn begin_frame(&mut self, epoch: u64) -> bool {
        if self.epoch != epoch || !self.scheduled {
            return false;
        }
        self.scheduled = false;
        self.active()
    }

    /// Returns only the movement between visible poses, never beyond the last
    /// known packet. Retain the left interpolation endpoint when pruning.
    pub fn advance(&mut self, now: Instant) -> f64 {
        let Some(target) = now.checked_sub(DELAY) else {
            return 0.0;
        };
        while self.samples.len() > 1 && self.samples[1].at <= target {
            self.samples.pop_front();
        }
        let Some(first) = self.samples.front() else {
            return 0.0;
        };
        let position = if target <= first.at {
            first.position
        } else if let Some(next) = self.samples.get(1) {
            let span = next.at.duration_since(first.at).as_secs_f64();
            let fraction = target.duration_since(first.at).as_secs_f64() / span;
            first.position + (next.position - first.position) * fraction.clamp(0.0, 1.0)
        } else {
            first.position
        };
        let delta = position - self.applied;
        self.applied = position;
        if self.samples.len() == 1
            && target >= first.at
            && (self.ended
                || self
                    .last_arrival
                    .is_some_and(|last| now.saturating_duration_since(last) >= MAX_GAP))
        {
            self.cancel();
        }
        delta
    }
}

#[cfg(test)]
#[path = "momentum/tests.rs"]
mod tests;
