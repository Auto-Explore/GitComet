//! Process-wide caps on the child processes GitComet runs at once.
//!
//! Each window, repository tab and linked worktree refreshes through executors
//! that are bounded on their own, but nothing bounded them together: many large
//! repositories refreshing at once could each start their Git commands, and a
//! burst of watcher flushes multiplied that by every open checkout. The budget
//! here is shared by the whole process, so the number of Git children stays
//! bounded however many repositories are open. Status walks, whose workers
//! start filter processes, keep a budget of their own of the same kind.
//!
//! Commands of a user-started operation (one with a
//! [`GitOperationContext`](gitcomet_core::git_operation::GitOperationContext))
//! may use slots that background refreshes leave free, so a commit or push
//! never queues behind refreshes that saturated the budget.
//!
//! A command waiting for a slot polls its cancellation: work superseded while
//! queued, such as a refresh replaced by a newer one, leaves without starting a
//! process.

use super::{Error, ErrorKind, Result};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Git commands GitComet runs at once, across every window and repository.
pub(crate) const GIT_PROCESS_LIMIT: usize = 16;
/// Slots only commands of a user-started operation take.
pub(crate) const GIT_PROCESS_FOREGROUND_RESERVE: usize = 4;
/// How often a queued command rechecks its cancellation.
const CANCELLATION_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessPriority {
    /// Refreshes and reads nobody started explicitly.
    Background,
    /// Part of an operation the user started and is waiting on.
    Foreground,
}

pub(crate) struct ProcessBudget {
    capacity: usize,
    reserved: usize,
    state: Mutex<BudgetState>,
    released: Condvar,
}

struct BudgetState {
    in_use: usize,
    peak: usize,
}

/// Slots held for a running command or walk, returned when dropped.
#[must_use = "the slots are released when the permit is dropped"]
pub(crate) struct ProcessPermit<'a> {
    budget: &'a ProcessBudget,
    slots: usize,
}

impl ProcessPermit<'_> {
    pub(crate) fn slots(&self) -> usize {
        self.slots
    }
}

impl Drop for ProcessPermit<'_> {
    fn drop(&mut self) {
        if self.slots == 0 {
            return;
        }
        let mut state = self.budget.lock();
        state.in_use -= self.slots;
        drop(state);
        self.budget.released.notify_all();
    }
}

impl ProcessBudget {
    pub(crate) const fn new(capacity: usize, reserved: usize) -> Self {
        let capacity = if capacity == 0 { 1 } else { capacity };
        // Background work always keeps at least one slot.
        let reserved = if reserved >= capacity {
            capacity - 1
        } else {
            reserved
        };
        Self {
            capacity,
            reserved,
            state: Mutex::new(BudgetState { in_use: 0, peak: 0 }),
            released: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BudgetState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn limit(&self, priority: ProcessPriority) -> usize {
        match priority {
            ProcessPriority::Foreground => self.capacity,
            ProcessPriority::Background => self.capacity - self.reserved,
        }
    }

    fn take(&self, state: &mut BudgetState, slots: usize) -> ProcessPermit<'_> {
        state.in_use += slots;
        state.peak = state.peak.max(state.in_use);
        ProcessPermit {
            budget: self,
            slots,
        }
    }

    /// Waits for a slot, or fails with [`ErrorKind::Cancelled`] once
    /// `cancelled` reports true while the command is still queued.
    pub(crate) fn acquire(
        &self,
        priority: ProcessPriority,
        cancelled: impl Fn() -> bool,
    ) -> Result<ProcessPermit<'_>> {
        let limit = self.limit(priority);
        let mut state = self.lock();
        loop {
            if cancelled() {
                return Err(Error::new(ErrorKind::Cancelled));
            }
            if state.in_use < limit {
                return Ok(self.take(&mut state, 1));
            }
            state = self
                .released
                .wait_timeout(state, CANCELLATION_POLL)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// Takes up to `wanted` free slots without waiting, possibly none.
    pub(crate) fn try_acquire_up_to(
        &self,
        priority: ProcessPriority,
        wanted: usize,
    ) -> ProcessPermit<'_> {
        let limit = self.limit(priority);
        let mut state = self.lock();
        let slots = wanted.min(limit.saturating_sub(state.in_use));
        self.take(&mut state, slots)
    }

    #[cfg(test)]
    pub(crate) fn in_use(&self) -> usize {
        self.lock().in_use
    }

    #[cfg(test)]
    pub(crate) fn peak(&self) -> usize {
        self.lock().peak
    }
}

static GIT_PROCESSES: ProcessBudget =
    ProcessBudget::new(GIT_PROCESS_LIMIT, GIT_PROCESS_FOREGROUND_RESERVE);

#[cfg(test)]
thread_local! {
    static TEST_BUDGET: std::cell::Cell<Option<&'static ProcessBudget>> =
        const { std::cell::Cell::new(None) };
}

/// The budget every Git command started through [`super::process`] draws from.
pub(crate) fn git_processes() -> &'static ProcessBudget {
    #[cfg(test)]
    if let Some(budget) = TEST_BUDGET.with(std::cell::Cell::get) {
        return budget;
    }
    &GIT_PROCESSES
}

/// Runs `run` with commands on this thread drawing from `budget`, so a test
/// can saturate a budget of its own without slowing unrelated tests.
#[cfg(test)]
pub(crate) fn with_git_process_budget<T>(
    budget: &'static ProcessBudget,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<&'static ProcessBudget>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_BUDGET.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(TEST_BUDGET.with(|slot| slot.replace(Some(budget))));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn never() -> bool {
        false
    }

    #[test]
    fn background_work_leaves_the_reserve_to_user_operations() {
        let budget = ProcessBudget::new(3, 1);
        let first = budget.acquire(ProcessPriority::Background, never).unwrap();
        let second = budget.acquire(ProcessPriority::Background, never).unwrap();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let queued = scope.spawn(|| {
                budget
                    .acquire(ProcessPriority::Background, || {
                        cancelled.load(Ordering::Relaxed)
                    })
                    .map(drop)
            });
            // The reserved slot is free, but not to background work.
            let foreground = budget.acquire(ProcessPriority::Foreground, never).unwrap();
            assert_eq!(budget.in_use(), 3);
            drop(foreground);
            std::thread::sleep(CANCELLATION_POLL * 3);
            assert!(!queued.is_finished());
            cancelled.store(true, Ordering::Relaxed);
            assert!(matches!(
                queued.join().unwrap().unwrap_err().kind(),
                ErrorKind::Cancelled
            ));
        });
        drop((first, second));
        assert_eq!(budget.in_use(), 0);
        assert_eq!(budget.peak(), 3);
    }

    #[test]
    fn a_released_slot_wakes_a_queued_command() {
        let budget = ProcessBudget::new(1, 0);
        let held = budget.acquire(ProcessPriority::Background, never).unwrap();
        std::thread::scope(|scope| {
            let queued = scope.spawn(|| {
                budget
                    .acquire(ProcessPriority::Background, never)
                    .map(drop)
                    .is_ok()
            });
            std::thread::sleep(CANCELLATION_POLL * 3);
            assert!(!queued.is_finished());
            drop(held);
            assert!(queued.join().unwrap());
        });
        assert_eq!(budget.peak(), 1);
        assert_eq!(budget.in_use(), 0);
    }

    #[test]
    fn a_cancelled_command_never_takes_a_free_slot() {
        let budget = ProcessBudget::new(2, 0);
        assert!(
            budget
                .acquire(ProcessPriority::Foreground, || true)
                .is_err()
        );
        assert_eq!(budget.peak(), 0);
    }

    #[test]
    fn a_walk_takes_only_the_slots_left_without_waiting() {
        let budget = ProcessBudget::new(6, 2);
        let command = budget.acquire(ProcessPriority::Background, never).unwrap();
        let walk = budget.try_acquire_up_to(ProcessPriority::Background, 8);
        assert_eq!(walk.slots(), 3);
        let starved = budget.try_acquire_up_to(ProcessPriority::Background, 8);
        assert_eq!(starved.slots(), 0);
        assert_eq!(
            budget
                .try_acquire_up_to(ProcessPriority::Foreground, 8)
                .slots(),
            2
        );
        drop((command, walk, starved));
        assert_eq!(budget.in_use(), 0);
        assert_eq!(budget.peak(), 6);
    }

    #[test]
    fn degenerate_sizes_keep_one_background_slot() {
        let budget = ProcessBudget::new(0, 4);
        let permit = budget.acquire(ProcessPriority::Background, never).unwrap();
        assert_eq!(budget.in_use(), 1);
        drop(permit);
    }
}
