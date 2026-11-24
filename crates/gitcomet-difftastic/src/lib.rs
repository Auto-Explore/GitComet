//! Structural comparisons for the diff pane, run in-process by the vendored
//! difftastic on a worker thread of its own.

mod conversion;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use difftastic::api;
use gitcomet_core::diff_engine::{DiffEngineFallback, StructuralDiffOutcome};
use gitcomet_core::panic::ContainedPanic;
use gitcomet_core::services::CancellationToken;

pub use difftastic::api::{BYTE_LIMIT, Input};

const DEADLINE: Duration = Duration::from_secs(5);
// Deeper trees fall back to the line diff. At this limit the worst measured
// shape (JS `a[` nesting) needs ~0.95 MiB of stack in release and ~1.2 MiB at
// difftastic's dev opt-level; STACK_SIZE leaves over 3x of margin.
const NESTING_LIMIT: usize = 1000;
const STACK_SIZE: usize = 4 << 20;
// One comparison can peak at ~0.9 GB (the graph-limit case), so they run one
// at a time; a cancelled one frees its slot within ~0.5 s.
const QUEUE_POLL: Duration = Duration::from_millis(5);
static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);
static WORKER: LazyLock<WorkerSlot> = LazyLock::new(WorkerSlot::default);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cancelled;

#[derive(Clone, Copy)]
pub enum Request<'a> {
    Visible(&'a CancellationToken),
    Prefetch(&'a CancellationToken),
}

impl<'a> Request<'a> {
    pub fn token(self) -> &'a CancellationToken {
        match self {
            Self::Visible(token) | Self::Prefetch(token) => token,
        }
    }
}

pub fn compare(
    input: Input<'_>,
    cancellation: &CancellationToken,
) -> Result<StructuralDiffOutcome, Cancelled> {
    compare_requested(input, Request::Visible(cancellation))
}

pub fn compare_requested(
    input: Input<'_>,
    request: Request<'_>,
) -> Result<StructuralDiffOutcome, Cancelled> {
    let operation = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    log::debug!(
        "difftastic operation={operation} start old_bytes={} new_bytes={}",
        input.old.len(),
        input.new.len()
    );
    let result = compare_queued(input, request, operation);
    let outcome = match &result {
        Ok(StructuralDiffOutcome::Compared(_)) => "compared",
        Ok(StructuralDiffOutcome::Unchanged) => "unchanged",
        Ok(StructuralDiffOutcome::Fallback(_)) => "fallback",
        Err(Cancelled) => "cancelled",
    };
    log::debug!(
        "difftastic operation={operation} end outcome={outcome} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    result
}

fn compare_queued(
    input: Input<'_>,
    request: Request<'_>,
    operation: u64,
) -> Result<StructuralDiffOutcome, Cancelled> {
    if input.old == input.new {
        return Ok(StructuralDiffOutcome::Unchanged);
    }
    // Checked before queueing: a fallback needs no slot.
    if !api::supports(input) {
        return Ok(StructuralDiffOutcome::Fallback(
            DiffEngineFallback::UnsupportedLanguage,
        ));
    }
    let _worker = WORKER.acquire(request)?;
    let deadline = Instant::now() + DEADLINE;
    compare_until(
        input,
        request.token(),
        deadline,
        operation,
        |input, should_stop| api::compare(input, NESTING_LIMIT, should_stop),
    )
}

// Runs `diff` on a thread with STACK_SIZE, stopping it once `cancellation`
// fires or `deadline` passes.
fn compare_until<'a>(
    input: Input<'a>,
    cancellation: &CancellationToken,
    deadline: Instant,
    operation: u64,
    diff: impl FnOnce(Input<'a>, &dyn Fn() -> bool) -> Result<api::Comparison, api::Interrupted> + Send,
) -> Result<StructuralDiffOutcome, Cancelled> {
    let compared = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("difftastic".into())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let should_stop = || cancellation.is_cancelled() || Instant::now() >= deadline;
                gitcomet_core::panic::contain(|| diff(input, &should_stop))
            })
            .map(|thread| thread.join().expect("contain catches the thread's panics"))
    });
    let comparison = match compared {
        Ok(Ok(Ok(comparison))) => comparison,
        Ok(Ok(Err(api::Interrupted))) if cancellation.is_cancelled() => return Err(Cancelled),
        Ok(Ok(Err(api::Interrupted))) => {
            return Ok(StructuralDiffOutcome::Fallback(DiffEngineFallback::Timeout));
        }
        Ok(Err(ContainedPanic { message, location })) => {
            let location =
                location.map_or_else(|| "an unknown location".to_owned(), |l| l.to_string());
            log::warn!("difftastic operation={operation} panicked at {location}: {message}");
            return Ok(StructuralDiffOutcome::Fallback(DiffEngineFallback::Failed));
        }
        Err(error) => {
            log::warn!("difftastic operation={operation} could not start its thread: {error}");
            return Ok(StructuralDiffOutcome::Fallback(DiffEngineFallback::Failed));
        }
    };
    Ok(
        conversion::outcome(comparison, input.old, input.new).unwrap_or_else(|error| {
            log::warn!("difftastic operation={operation} invalid comparison: {error}");
            StructuralDiffOutcome::Fallback(DiffEngineFallback::InvalidOutput)
        }),
    )
}

#[derive(Default)]
struct WorkerSlot {
    state: Mutex<SlotState>,
    available: Condvar,
}

#[derive(Default)]
struct SlotState {
    active: Option<ActiveComparison>,
    waiting: usize,
}

enum ActiveComparison {
    Visible,
    Prefetch(CancellationToken),
}

struct Worker<'a>(&'a WorkerSlot);

impl WorkerSlot {
    fn acquire(&self, request: Request<'_>) -> Result<Worker<'_>, Cancelled> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if request.token().is_cancelled() {
            return Err(Cancelled);
        }
        if let Request::Prefetch(token) = request {
            if state.active.is_some() || state.waiting != 0 {
                return Err(Cancelled);
            }
            state.active = Some(ActiveComparison::Prefetch(token.clone()));
            return Ok(Worker(self));
        }
        if let Some(ActiveComparison::Prefetch(token)) = &state.active {
            token.cancel();
        }
        state.waiting += 1;
        while state.active.is_some() && !request.token().is_cancelled() {
            state = self
                .available
                .wait_timeout(state, QUEUE_POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
        state.waiting -= 1;
        if request.token().is_cancelled() {
            return Err(Cancelled);
        }
        state.active = Some(ActiveComparison::Visible);
        Ok(Worker(self))
    }
}

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active = None;
        self.0.available.notify_one();
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
