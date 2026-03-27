use mimalloc::MiMalloc;
use serde_json::{Map, Value, json};
use stats_alloc::{Region, Stats, StatsAlloc};

pub type PerfTrackingAllocator = StatsAlloc<MiMalloc>;

pub static TRACKING_MIMALLOC: PerfTrackingAllocator = StatsAlloc::new(MiMalloc);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PerfAllocMetrics {
    pub alloc_ops: u64,
    pub dealloc_ops: u64,
    pub realloc_ops: u64,
    pub alloc_bytes: u64,
    pub dealloc_bytes: u64,
    pub realloc_bytes_delta: i64,
    pub net_alloc_bytes: i64,
}

impl PerfAllocMetrics {
    pub fn append_to_payload(self, payload: &mut Map<String, Value>) {
        self.append_to_payload_with_prefix(payload, "");
    }

    pub fn append_to_payload_with_prefix(self, payload: &mut Map<String, Value>, prefix: &str) {
        payload.insert(format!("{prefix}alloc_ops"), json!(self.alloc_ops));
        payload.insert(format!("{prefix}dealloc_ops"), json!(self.dealloc_ops));
        payload.insert(format!("{prefix}realloc_ops"), json!(self.realloc_ops));
        payload.insert(format!("{prefix}alloc_bytes"), json!(self.alloc_bytes));
        payload.insert(format!("{prefix}dealloc_bytes"), json!(self.dealloc_bytes));
        payload.insert(
            format!("{prefix}realloc_bytes_delta"),
            json!(self.realloc_bytes_delta),
        );
        payload.insert(
            format!("{prefix}net_alloc_bytes"),
            json!(self.net_alloc_bytes),
        );
    }
}

pub fn current_alloc_metrics() -> PerfAllocMetrics {
    TRACKING_MIMALLOC.stats().into()
}

pub fn measure_allocations<T>(f: impl FnOnce() -> T) -> (T, PerfAllocMetrics) {
    let region = Region::new(&TRACKING_MIMALLOC);
    let value = f();
    std::hint::black_box(&value);
    let metrics = region.change().into();
    (value, metrics)
}

impl From<Stats> for PerfAllocMetrics {
    fn from(value: Stats) -> Self {
        let alloc_bytes = value.bytes_allocated.min(u64::MAX as usize) as u64;
        let dealloc_bytes = value.bytes_deallocated.min(u64::MAX as usize) as u64;
        Self {
            alloc_ops: value.allocations.min(u64::MAX as usize) as u64,
            dealloc_ops: value.deallocations.min(u64::MAX as usize) as u64,
            realloc_ops: value.reallocations.min(u64::MAX as usize) as u64,
            alloc_bytes,
            dealloc_bytes,
            realloc_bytes_delta: value
                .bytes_reallocated
                .clamp(i64::MIN as isize, i64::MAX as isize)
                as i64,
            net_alloc_bytes: (i128::from(alloc_bytes) - i128::from(dealloc_bytes))
                .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                as i64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[global_allocator]
    static GLOBAL: &PerfTrackingAllocator = &TRACKING_MIMALLOC;

    #[test]
    fn measure_allocations_tracks_requested_bytes() {
        let (_value, metrics) = measure_allocations(|| vec![0u8; 256]);
        assert!(metrics.alloc_ops > 0);
        assert!(metrics.alloc_bytes >= 256);
    }

    #[test]
    fn stats_conversion_tracks_net_alloc_bytes() {
        // `measure_allocations` observes a process-global allocator, so parallel
        // test activity can perturb live net bytes. Cover the net-byte math with
        // a deterministic `Stats` conversion instead.
        let metrics = PerfAllocMetrics::from(Stats {
            allocations: 3,
            deallocations: 1,
            reallocations: 2,
            bytes_allocated: 512,
            bytes_deallocated: 128,
            bytes_reallocated: 64,
        });

        assert_eq!(metrics.alloc_ops, 3);
        assert_eq!(metrics.dealloc_ops, 1);
        assert_eq!(metrics.realloc_ops, 2);
        assert_eq!(metrics.alloc_bytes, 512);
        assert_eq!(metrics.dealloc_bytes, 128);
        assert_eq!(metrics.realloc_bytes_delta, 64);
        assert_eq!(metrics.net_alloc_bytes, 384);
    }

    #[test]
    fn append_to_payload_with_prefix_uses_stable_metric_names() {
        let mut payload = Map::new();
        PerfAllocMetrics {
            alloc_ops: 3,
            dealloc_ops: 1,
            realloc_ops: 2,
            alloc_bytes: 512,
            dealloc_bytes: 128,
            realloc_bytes_delta: 64,
            net_alloc_bytes: 384,
        }
        .append_to_payload_with_prefix(&mut payload, "first_interactive_");

        assert_eq!(payload.get("first_interactive_alloc_ops"), Some(&json!(3)));
        assert_eq!(
            payload.get("first_interactive_dealloc_bytes"),
            Some(&json!(128))
        );
        assert_eq!(
            payload.get("first_interactive_net_alloc_bytes"),
            Some(&json!(384))
        );
    }
}
