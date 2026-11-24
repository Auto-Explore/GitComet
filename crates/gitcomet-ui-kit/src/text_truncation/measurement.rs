use super::TRUNCATED_LAYOUT_CACHE;
use std::cell::Cell;

thread_local! {
    static MEASURE_CANDIDATE_CALLS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_measure_candidate_call() {
    MEASURE_CANDIDATE_CALLS.with(|calls| calls.set(calls.get() + 1));
}

pub fn clear_layout_cache() {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow_mut().clear());
}

#[cfg(any(test, feature = "test-support"))]
pub fn truncated_layout_cache_len_for_test() -> usize {
    TRUNCATED_LAYOUT_CACHE.with(|cache| cache.borrow().len())
}

pub fn reset_measure_candidate_calls() {
    MEASURE_CANDIDATE_CALLS.with(|calls| calls.set(0));
}

pub fn measure_candidate_calls() -> usize {
    MEASURE_CANDIDATE_CALLS.with(Cell::get)
}
