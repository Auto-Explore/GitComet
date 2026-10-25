use crate::ref_comparison::{COMPARISON_GLOBAL_BYTES, COMPARISON_REPOSITORY_BYTES};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static GLOBAL_BYTES: AtomicUsize = AtomicUsize::new(0);
pub const COMPARISON_OID_BYTES: usize = 64 + 4 * std::mem::size_of::<usize>();

pub fn hash_table_bytes<K, V>(entries: usize) -> usize {
    if entries == 0 {
        return 0;
    }
    entries
        .saturating_add(entries / 2)
        .max(4)
        .checked_next_power_of_two()
        .unwrap_or(usize::MAX)
        .saturating_mul(std::mem::size_of::<(K, V)>().saturating_add(16))
        .saturating_add(64)
}

pub fn vector_bytes<T>(entries: usize) -> usize {
    entries
        .saturating_mul(std::mem::size_of::<T>())
        .saturating_add(64)
}

#[derive(Debug, Default)]
pub struct ComparisonMemory {
    bytes: AtomicUsize,
    peak: AtomicUsize,
    native_peak: AtomicUsize,
}

impl ComparisonMemory {
    pub fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Arc<ComparisonReservation>> {
        self.claim(bytes)?;
        Some(Arc::new(ComparisonReservation {
            owner: self.clone(),
            bytes: Mutex::new(bytes),
        }))
    }

    fn claim(&self, bytes: usize) -> Option<()> {
        let previous = self
            .bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= COMPARISON_REPOSITORY_BYTES)
            })
            .ok()?;
        if GLOBAL_BYTES
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= COMPARISON_GLOBAL_BYTES)
            })
            .is_err()
        {
            self.bytes.fetch_sub(bytes, Ordering::AcqRel);
            return None;
        }
        self.peak.fetch_max(previous + bytes, Ordering::Relaxed);
        Some(())
    }

    pub fn used_bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }
    pub fn peak_bytes(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
    pub fn available_bytes(&self) -> usize {
        COMPARISON_REPOSITORY_BYTES
            .saturating_sub(self.used_bytes())
            .min(COMPARISON_GLOBAL_BYTES.saturating_sub(Self::global_used_bytes()))
    }
    pub fn observe_native_bytes(&self, bytes: usize) {
        self.native_peak.fetch_max(bytes, Ordering::Relaxed);
    }
    pub fn native_peak_bytes(&self) -> usize {
        self.native_peak.load(Ordering::Acquire)
    }
    pub fn global_used_bytes() -> usize {
        GLOBAL_BYTES.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
pub struct ComparisonReservation {
    owner: Arc<ComparisonMemory>,
    bytes: Mutex<usize>,
}

impl ComparisonReservation {
    pub fn grow(&self, additional: usize) -> bool {
        let mut charged = self.bytes.lock().unwrap_or_else(|error| error.into_inner());
        let Some(next) = charged.checked_add(additional) else {
            return false;
        };
        if self.owner.claim(additional).is_none() {
            return false;
        }
        *charged = next;
        true
    }

    pub fn resize(&self, bytes: usize) -> bool {
        let mut charged = self.bytes.lock().unwrap_or_else(|error| error.into_inner());
        let previous = *charged;
        if bytes > previous && self.owner.claim(bytes - previous).is_none() {
            return false;
        }
        if bytes < previous {
            self.owner
                .bytes
                .fetch_sub(previous - bytes, Ordering::AcqRel);
            GLOBAL_BYTES.fetch_sub(previous - bytes, Ordering::AcqRel);
        }
        *charged = bytes;
        true
    }
}

impl Drop for ComparisonReservation {
    fn drop(&mut self) {
        let bytes = *self
            .bytes
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        self.owner.bytes.fetch_sub(bytes, Ordering::AcqRel);
        GLOBAL_BYTES.fetch_sub(bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_and_global_reservations_include_shared_owners_and_release_on_last_drop() {
        let owners: Vec<_> = (0..4)
            .map(|_| Arc::new(ComparisonMemory::default()))
            .collect();
        let reservations: Vec<_> = owners
            .iter()
            .map(|owner| owner.reserve(32 * 1024 * 1024).unwrap())
            .collect();
        assert_eq!(ComparisonMemory::global_used_bytes(), 128 * 1024 * 1024);
        assert!(owners[0].reserve(1).is_none());
        assert!(Arc::new(ComparisonMemory::default()).reserve(1).is_none());
        let retained = reservations[0].clone();
        drop(reservations);
        assert_eq!(ComparisonMemory::global_used_bytes(), 32 * 1024 * 1024);
        assert!(retained.resize(16 * 1024 * 1024));
        assert_eq!(owners[0].used_bytes(), 16 * 1024 * 1024);
        drop(retained);
        assert_eq!(owners[0].used_bytes(), 0);
        assert_eq!(ComparisonMemory::global_used_bytes(), 0);
    }
}
