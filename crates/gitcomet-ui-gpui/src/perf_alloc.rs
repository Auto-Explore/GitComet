use gitcomet_tree_sitter_alloc::{
    AllocMetrics as TreeSitterAllocMetrics,
    install_tracking_allocator as install_tree_sitter_tracking_allocator_impl,
    measure_allocations as measure_tree_sitter_allocations,
};
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PerfAllocChannels {
    pub rust: PerfAllocMetrics,
    pub tree_sitter: PerfAllocMetrics,
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

    pub fn is_zero(self) -> bool {
        self.alloc_ops == 0
            && self.dealloc_ops == 0
            && self.realloc_ops == 0
            && self.alloc_bytes == 0
            && self.dealloc_bytes == 0
            && self.realloc_bytes_delta == 0
            && self.net_alloc_bytes == 0
    }

    pub fn delta_since(self, earlier: Self) -> Self {
        let alloc_bytes = self.alloc_bytes.saturating_sub(earlier.alloc_bytes);
        let dealloc_bytes = self.dealloc_bytes.saturating_sub(earlier.dealloc_bytes);
        Self {
            alloc_ops: self.alloc_ops.saturating_sub(earlier.alloc_ops),
            dealloc_ops: self.dealloc_ops.saturating_sub(earlier.dealloc_ops),
            realloc_ops: self.realloc_ops.saturating_sub(earlier.realloc_ops),
            alloc_bytes,
            dealloc_bytes,
            realloc_bytes_delta: clamp_i128_to_i64(
                i128::from(self.realloc_bytes_delta) - i128::from(earlier.realloc_bytes_delta),
            ),
            net_alloc_bytes: clamp_i128_to_i64(i128::from(alloc_bytes) - i128::from(dealloc_bytes)),
        }
    }

    pub fn saturating_add(self, other: Self) -> Self {
        let alloc_bytes = self.alloc_bytes.saturating_add(other.alloc_bytes);
        let dealloc_bytes = self.dealloc_bytes.saturating_add(other.dealloc_bytes);
        Self {
            alloc_ops: self.alloc_ops.saturating_add(other.alloc_ops),
            dealloc_ops: self.dealloc_ops.saturating_add(other.dealloc_ops),
            realloc_ops: self.realloc_ops.saturating_add(other.realloc_ops),
            alloc_bytes,
            dealloc_bytes,
            realloc_bytes_delta: clamp_i128_to_i64(
                i128::from(self.realloc_bytes_delta) + i128::from(other.realloc_bytes_delta),
            ),
            net_alloc_bytes: clamp_i128_to_i64(i128::from(alloc_bytes) - i128::from(dealloc_bytes)),
        }
    }
}

pub fn current_alloc_metrics() -> PerfAllocMetrics {
    TRACKING_MIMALLOC.stats().into()
}

pub fn install_tree_sitter_tracking_allocator() {
    install_tree_sitter_tracking_allocator_impl();
}

pub fn measure_allocation_channels<T>(f: impl FnOnce() -> T) -> (T, PerfAllocChannels) {
    let rust_region = Region::new(&TRACKING_MIMALLOC);
    let (value, tree_sitter) = measure_tree_sitter_allocations(f);
    let rust = rust_region.change().into();
    (
        value,
        PerfAllocChannels {
            rust,
            tree_sitter: tree_sitter.into(),
        },
    )
}

pub fn measure_allocations<T>(f: impl FnOnce() -> T) -> (T, PerfAllocMetrics) {
    let (value, channels) = measure_allocation_channels(f);
    (value, channels.rust)
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

fn clamp_i128_to_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

impl From<TreeSitterAllocMetrics> for PerfAllocMetrics {
    fn from(value: TreeSitterAllocMetrics) -> Self {
        Self {
            alloc_ops: value.alloc_ops,
            dealloc_ops: value.dealloc_ops,
            realloc_ops: value.realloc_ops,
            alloc_bytes: value.alloc_bytes,
            dealloc_bytes: value.dealloc_bytes,
            realloc_bytes_delta: value.realloc_bytes_delta,
            net_alloc_bytes: value.net_alloc_bytes,
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

    #[test]
    fn delta_since_subtracts_metric_snapshots() {
        let current = PerfAllocMetrics {
            alloc_ops: 7,
            dealloc_ops: 4,
            realloc_ops: 3,
            alloc_bytes: 1_024,
            dealloc_bytes: 256,
            realloc_bytes_delta: 128,
            net_alloc_bytes: 768,
        };
        let earlier = PerfAllocMetrics {
            alloc_ops: 2,
            dealloc_ops: 1,
            realloc_ops: 1,
            alloc_bytes: 128,
            dealloc_bytes: 64,
            realloc_bytes_delta: 32,
            net_alloc_bytes: 64,
        };

        let delta = current.delta_since(earlier);

        assert_eq!(delta.alloc_ops, 5);
        assert_eq!(delta.dealloc_ops, 3);
        assert_eq!(delta.realloc_ops, 2);
        assert_eq!(delta.alloc_bytes, 896);
        assert_eq!(delta.dealloc_bytes, 192);
        assert_eq!(delta.realloc_bytes_delta, 96);
        assert_eq!(delta.net_alloc_bytes, 704);
    }

    #[test]
    fn saturating_add_combines_metric_channels() {
        let combined = PerfAllocMetrics {
            alloc_ops: 3,
            dealloc_ops: 1,
            realloc_ops: 1,
            alloc_bytes: 256,
            dealloc_bytes: 64,
            realloc_bytes_delta: 64,
            net_alloc_bytes: 192,
        }
        .saturating_add(PerfAllocMetrics {
            alloc_ops: 2,
            dealloc_ops: 3,
            realloc_ops: 4,
            alloc_bytes: 128,
            dealloc_bytes: 256,
            realloc_bytes_delta: -32,
            net_alloc_bytes: -128,
        });

        assert_eq!(combined.alloc_ops, 5);
        assert_eq!(combined.dealloc_ops, 4);
        assert_eq!(combined.realloc_ops, 5);
        assert_eq!(combined.alloc_bytes, 384);
        assert_eq!(combined.dealloc_bytes, 320);
        assert_eq!(combined.realloc_bytes_delta, 32);
        assert_eq!(combined.net_alloc_bytes, 64);
    }
}
