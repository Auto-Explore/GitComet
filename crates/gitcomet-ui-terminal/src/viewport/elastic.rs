use super::*;
use gitcomet_ui_kit::elastic_scroll::{ElasticScrollDriver, ElasticScrollState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Snapshot {
    history: usize,
    display: usize,
    line_height_bits: u32,
    normal_buffer: bool,
}

impl Snapshot {
    fn from_term(term: &alacritty_terminal::Term<GitCometListener>, line_height: Pixels) -> Self {
        let mode = terminal_live_modes(term);
        Self {
            history: term.grid().history_size(),
            display: term.grid().display_offset(),
            line_height_bits: f32::from(line_height).to_bits(),
            normal_buffer: !mode.mouse_mode() && !mode.contains(TerminalModes::ALT_SCREEN),
        }
    }

    fn line_height(self) -> f64 {
        f64::from(f32::from_bits(self.line_height_bits))
    }

    fn max(self) -> f64 {
        self.history as f64 * self.line_height()
    }

    fn position(self) -> f64 {
        -(self.history.saturating_sub(self.display) as f64) * self.line_height()
    }
}

#[derive(Default)]
struct Position {
    snapshot: Option<Snapshot>,
    pixels: f64,
}

impl Position {
    fn synchronize(&mut self, snapshot: Snapshot) {
        if self.snapshot != Some(snapshot) {
            self.pixels = snapshot.position();
            self.snapshot = Some(snapshot);
        }
    }
}

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(super) enum WheelAttempt {
    #[default]
    Declined,
    Owned,
}

/// Alacritty renders complete rows. Keep sub-row wheel intent in the logical
/// model instead of reporting its rounding remainder as elastic edge travel.
#[derive(Clone, Default)]
pub(super) struct TerminalElasticScroll {
    pub(super) motion: ElasticScrollState,
    position: Rc<RefCell<Position>>,
    wheel_attempt: Rc<Cell<WheelAttempt>>,
}

impl TerminalElasticScroll {
    pub(super) fn reset(&self) {
        self.motion.reset();
        self.position.borrow_mut().snapshot = None;
        self.wheel_attempt.set(WheelAttempt::Declined);
    }

    pub(super) fn take_wheel_attempt(&self) -> WheelAttempt {
        self.wheel_attempt.replace(WheelAttempt::Declined)
    }

    pub(super) fn driver(
        &self,
        term_lock: Option<AlacrittyTermLock>,
        line_height: Pixels,
        content: Option<&TerminalContent>,
    ) -> TerminalScrollDriver {
        let fallback = Snapshot {
            history: content.map_or(0, |content| content.history_size),
            display: content.map_or(0, |content| content.display_offset),
            line_height_bits: f32::from(line_height).to_bits(),
            normal_buffer: content.is_none_or(|content| {
                !content.mode.mouse_mode() && !content.mode.contains(TerminalModes::ALT_SCREEN)
            }),
        };
        TerminalScrollDriver {
            term_lock,
            position: self.position.clone(),
            wheel_attempt: self.wheel_attempt.clone(),
            fallback,
        }
    }

    pub(super) fn accepts_wheel(
        &self,
        term_lock: Option<&AlacrittyTermLock>,
        event: &gpui::ScrollWheelEvent,
    ) -> bool {
        event.delta.precise()
            && term_lock.is_some_and(|lock| {
                let term = lock.lock();
                let mode = terminal_live_modes(&term);
                !mode.mouse_mode() && !mode.contains(TerminalModes::ALT_SCREEN)
            })
    }

    pub(super) fn fixed_bounds(&self, mut painted: Bounds<Pixels>) -> Bounds<Pixels> {
        painted.origin.y -= self.motion.displacement();
        painted
    }
}

pub(super) struct TerminalScrollDriver {
    term_lock: Option<AlacrittyTermLock>,
    position: Rc<RefCell<Position>>,
    wheel_attempt: Rc<Cell<WheelAttempt>>,
    fallback: Snapshot,
}

impl ElasticScrollDriver for TerminalScrollDriver {
    fn metrics(&self) -> (f64, f64) {
        // Painting must not wait behind a PTY output batch. Prefer the latest
        // scroll-side snapshot when the parser owns the lock, then the canvas's.
        let snapshot = self
            .term_lock
            .as_ref()
            .and_then(|lock| {
                lock.try_lock_unfair().map(|term| {
                    Snapshot::from_term(&term, px(f32::from_bits(self.fallback.line_height_bits)))
                })
            })
            .unwrap_or_else(|| self.position.borrow().snapshot.unwrap_or(self.fallback));
        let mut position = self.position.borrow_mut();
        position.synchronize(snapshot);
        if snapshot.normal_buffer {
            (position.pixels, snapshot.max())
        } else {
            (0.0, 0.0)
        }
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        self.wheel_attempt.set(WheelAttempt::Declined);
        let Some(lock) = &self.term_lock else {
            return px(0.0);
        };
        let mut term = lock.lock();
        let line_height = px(f32::from_bits(self.fallback.line_height_bits));
        let snapshot = Snapshot::from_term(&term, line_height);
        let mut position = self.position.borrow_mut();
        position.synchronize(snapshot);
        if !snapshot.normal_buffer || !f32::from(delta).is_finite() {
            return px(0.0);
        }
        // Record ownership under the same lock as the mode check and scroll.
        // Even a clamped or sub-row delta belongs to this normal-buffer route.
        self.wheel_attempt.set(WheelAttempt::Owned);
        let old = position.pixels;
        let next = (old + f64::from(f32::from(delta))).clamp(-snapshot.max(), 0.0);
        let rows_from_top = (-next / snapshot.line_height()).floor() as usize;
        let target = snapshot.history.saturating_sub(rows_from_top);
        let steps = target.abs_diff(snapshot.display).min(i32::MAX as usize) as i32;
        let change = if target > snapshot.display {
            steps
        } else {
            -steps
        };
        if change != 0 {
            term.scroll_display(Scroll::Delta(change));
        }
        let actual = Snapshot::from_term(&term, line_height);
        position.snapshot = Some(actual);
        position.pixels = if actual.display == target {
            next
        } else {
            actual.position()
        };
        px((position.pixels - old) as f32)
    }
}
