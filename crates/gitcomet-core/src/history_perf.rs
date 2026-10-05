//! Work counters for structural regression tests and opt-in live profiling.
//! Without an active operation trace, shipping builds only check its enable flag;
//! no per-row environment reads, allocations, or trace records are needed.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Work {
    IndexObjectRead,
    RangeObjectRead,
    GraphTransition,
    PaintRow,
    CheckpointRestore,
    DecorationBuild,
    TextWindowBuild,
    ComparisonCard,
    PaintPath,
    ContainmentWalk,
    PaintSegmentQuad,
    RangeStoreReopen,
    TextMeasurement,
    PickerModelBuild,
    PickerFilterItem,
    LogWalkObjectRead,
    LogTopologyBuild,
    HistoryIndexBuild,
}

impl Work {
    pub const ALL: [Self; 18] = [
        Self::IndexObjectRead,
        Self::RangeObjectRead,
        Self::GraphTransition,
        Self::PaintRow,
        Self::CheckpointRestore,
        Self::DecorationBuild,
        Self::TextWindowBuild,
        Self::ComparisonCard,
        Self::PaintPath,
        Self::ContainmentWalk,
        Self::PaintSegmentQuad,
        Self::RangeStoreReopen,
        Self::TextMeasurement,
        Self::PickerModelBuild,
        Self::PickerFilterItem,
        Self::LogWalkObjectRead,
        Self::LogTopologyBuild,
        Self::HistoryIndexBuild,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::IndexObjectRead => "index_object_read",
            Self::RangeObjectRead => "range_object_read",
            Self::GraphTransition => "graph_transition",
            Self::PaintRow => "paint_row",
            Self::CheckpointRestore => "checkpoint_restore",
            Self::DecorationBuild => "decoration_build",
            Self::TextWindowBuild => "text_window_build",
            Self::ComparisonCard => "comparison_card",
            Self::PaintPath => "paint_path",
            Self::ContainmentWalk => "containment_walk",
            Self::PaintSegmentQuad => "paint_segment_quad",
            Self::RangeStoreReopen => "range_store_reopen",
            Self::TextMeasurement => "text_measurement",
            Self::PickerModelBuild => "picker_model_build",
            Self::PickerFilterItem => "picker_filter_item",
            Self::LogWalkObjectRead => "log_walk_object_read",
            Self::LogTopologyBuild => "log_topology_build",
            Self::HistoryIndexBuild => "history_index_build",
        }
    }
}

static LIVE_COUNTS: [AtomicU64; Work::ALL.len()] = [const { AtomicU64::new(0) }; Work::ALL.len()];

/// Cumulative process-wide work, including background workers. Phase boundaries
/// take differences; concurrent increments may land on either side of a boundary.
pub fn snapshot() -> [u64; Work::ALL.len()] {
    std::array::from_fn(|index| LIVE_COUNTS[index].load(Ordering::Relaxed))
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static COUNTS: std::cell::RefCell<Option<CaptureContext>> = const { std::cell::RefCell::new(None) };
}

#[inline]
pub fn record(work: Work) {
    record_many(work, 1);
}

#[inline]
pub fn record_many(work: Work, amount: u64) {
    if crate::op_trace::enabled() {
        LIVE_COUNTS[work as usize].fetch_add(amount, Ordering::Relaxed);
    }
    #[cfg(any(test, feature = "benchmarks"))]
    COUNTS.with(|counts| {
        if let Some(value) = counts.borrow().as_ref() {
            value[work as usize].fetch_add(amount, Ordering::Relaxed);
        }
    });
}

#[cfg(any(test, feature = "benchmarks"))]
pub type CaptureContext = std::sync::Arc<[AtomicU64; Work::ALL.len()]>;

#[cfg(any(test, feature = "benchmarks"))]
pub struct Capture(Option<CaptureContext>);

/// Pass a diagnostic capture to work that moves to a background queue.
#[cfg(any(test, feature = "benchmarks"))]
pub fn capture_context() -> Option<CaptureContext> {
    COUNTS.with(|counts| counts.borrow().clone())
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn attach_capture(context: Option<CaptureContext>) -> Capture {
    Capture(COUNTS.with(|counts| counts.replace(context)))
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn capture() -> Capture {
    attach_capture(Some(std::sync::Arc::new(std::array::from_fn(|_| {
        AtomicU64::new(0)
    }))))
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn count(work: Work) -> u64 {
    COUNTS.with(|counts| {
        counts
            .borrow()
            .as_ref()
            .map_or(0, |value| value[work as usize].load(Ordering::Relaxed))
    })
}

#[cfg(any(test, feature = "benchmarks"))]
impl Drop for Capture {
    fn drop(&mut self) {
        COUNTS.with(|counts| counts.replace(self.0.take()));
    }
}

/// Preserve optional structural diagnostics when a reader moves to a worker.
/// Shipping builds compile out the capture and attachment entirely.
#[doc(hidden)]
pub fn with_capture_context<T>(work: impl FnOnce() -> T) -> impl FnOnce() -> T {
    #[cfg(any(test, feature = "benchmarks"))]
    let context = capture_context();
    move || {
        #[cfg(any(test, feature = "benchmarks"))]
        let _capture = attach_capture(context);
        work()
    }
}

// Read on demand in opt-in phase diagnostics, never on the row paint path.
type MemoryProvider = fn() -> (usize, usize, usize);
static SHARED_MEMORY: std::sync::OnceLock<MemoryProvider> = std::sync::OnceLock::new();

#[doc(hidden)]
pub fn register_shared_memory_provider(provider: MemoryProvider) {
    let _ = SHARED_MEMORY.set(provider);
}

/// Optional retained bytes, caller-pinned bytes, optional retained rows.
/// Pinned and retained ownership can overlap and must not be added together.
#[doc(hidden)]
pub fn shared_memory() -> Option<(usize, usize, usize)> {
    SHARED_MEMORY.get().map(|provider| provider())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_work_and_nested_captures_preserve_outer_counts() {
        let _outer = capture();
        record_many(Work::PickerFilterItem, 40);
        {
            let _inner = capture();
            record(Work::PickerFilterItem);
            record(Work::TextMeasurement);
            assert_eq!(count(Work::PickerFilterItem), 1);
        }
        assert_eq!(count(Work::PickerFilterItem), 40);
        assert_eq!(count(Work::TextMeasurement), 0);
        for (index, work) in Work::ALL.iter().enumerate() {
            assert_eq!(*work as usize, index);
            assert!(
                !Work::ALL[..index]
                    .iter()
                    .any(|other| other.name() == work.name())
            );
        }
    }
}
