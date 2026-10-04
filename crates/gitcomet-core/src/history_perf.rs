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
}

impl Work {
    pub const ALL: [Self; 17] = [
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
    static COUNTS: std::cell::Cell<Option<[u64; Work::ALL.len()]>> = const { std::cell::Cell::new(None) };
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
        if let Some(mut value) = counts.get() {
            value[work as usize] += amount;
            counts.set(Some(value));
        }
    });
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct Capture(Option<[u64; Work::ALL.len()]>);

#[cfg(any(test, feature = "benchmarks"))]
pub fn capture() -> Capture {
    Capture(COUNTS.with(|counts| counts.replace(Some([0; Work::ALL.len()]))))
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn count(work: Work) -> u64 {
    COUNTS.with(|counts| counts.get().unwrap_or_default()[work as usize])
}

#[cfg(any(test, feature = "benchmarks"))]
impl Drop for Capture {
    fn drop(&mut self) {
        COUNTS.with(|counts| counts.set(self.0));
    }
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
