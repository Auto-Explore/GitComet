use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StoreReducerDiagnostics {
    pub dispatch_count: u64,
    pub reducer_total_nanos: u64,
    pub reducer_max_nanos: u64,
    pub clone_on_write_count: u64,
    pub clone_on_write_total_nanos: u64,
    pub clone_on_write_max_nanos: u64,
    pub max_shared_state_handles: u64,
}

impl StoreReducerDiagnostics {
    pub fn average_reducer_nanos(self) -> u64 {
        if self.dispatch_count == 0 {
            0
        } else {
            self.reducer_total_nanos / self.dispatch_count
        }
    }

    pub fn average_clone_on_write_nanos(self) -> u64 {
        if self.clone_on_write_count == 0 {
            0
        } else {
            self.clone_on_write_total_nanos / self.clone_on_write_count
        }
    }
}

static REDUCER_DISPATCH_COUNT: AtomicU64 = AtomicU64::new(0);
static REDUCER_TOTAL_NANOS: AtomicU64 = AtomicU64::new(0);
static REDUCER_MAX_NANOS: AtomicU64 = AtomicU64::new(0);
static CLONE_ON_WRITE_COUNT: AtomicU64 = AtomicU64::new(0);
static CLONE_ON_WRITE_TOTAL_NANOS: AtomicU64 = AtomicU64::new(0);
static CLONE_ON_WRITE_MAX_NANOS: AtomicU64 = AtomicU64::new(0);
static MAX_SHARED_STATE_HANDLES: AtomicU64 = AtomicU64::new(0);

fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn update_max(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while value > current {
        match counter.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

pub(super) fn record_reducer_pass(duration: Duration) {
    REDUCER_DISPATCH_COUNT.fetch_add(1, Ordering::Relaxed);
    let nanos = duration_nanos(duration);
    REDUCER_TOTAL_NANOS.fetch_add(nanos, Ordering::Relaxed);
    update_max(&REDUCER_MAX_NANOS, nanos);
}

pub(super) fn record_clone_on_write(shared_state_handles: usize, duration: Duration) {
    CLONE_ON_WRITE_COUNT.fetch_add(1, Ordering::Relaxed);
    let nanos = duration_nanos(duration);
    CLONE_ON_WRITE_TOTAL_NANOS.fetch_add(nanos, Ordering::Relaxed);
    update_max(&CLONE_ON_WRITE_MAX_NANOS, nanos);
    update_max(
        &MAX_SHARED_STATE_HANDLES,
        shared_state_handles.min(u64::MAX as usize) as u64,
    );
}

pub(super) fn snapshot() -> StoreReducerDiagnostics {
    StoreReducerDiagnostics {
        dispatch_count: REDUCER_DISPATCH_COUNT.load(Ordering::Relaxed),
        reducer_total_nanos: REDUCER_TOTAL_NANOS.load(Ordering::Relaxed),
        reducer_max_nanos: REDUCER_MAX_NANOS.load(Ordering::Relaxed),
        clone_on_write_count: CLONE_ON_WRITE_COUNT.load(Ordering::Relaxed),
        clone_on_write_total_nanos: CLONE_ON_WRITE_TOTAL_NANOS.load(Ordering::Relaxed),
        clone_on_write_max_nanos: CLONE_ON_WRITE_MAX_NANOS.load(Ordering::Relaxed),
        max_shared_state_handles: MAX_SHARED_STATE_HANDLES.load(Ordering::Relaxed),
    }
}
