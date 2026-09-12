//! Opt-in work counters for production history benchmarks. Shipping builds
//! without the benchmark feature compile calls away; no per-row environment reads.
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
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static COUNTS: std::cell::Cell<Option<[u64; 12]>> = const { std::cell::Cell::new(None) };
}

#[inline]
pub fn record(work: Work) {
    #[cfg(any(test, feature = "benchmarks"))]
    COUNTS.with(|counts| {
        if let Some(mut value) = counts.get() {
            value[work as usize] += 1;
            counts.set(Some(value));
        }
    });
    #[cfg(not(any(test, feature = "benchmarks")))]
    let _ = work;
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct Capture(Option<[u64; 12]>);

#[cfg(any(test, feature = "benchmarks"))]
pub fn capture() -> Capture {
    Capture(COUNTS.with(|counts| counts.replace(Some([0; 12]))))
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
