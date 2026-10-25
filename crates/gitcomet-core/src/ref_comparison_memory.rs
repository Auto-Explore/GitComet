use crate::ref_comparison::{COMPARISON_GLOBAL_BYTES, COMPARISON_REPOSITORY_BYTES};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static GLOBAL_BYTES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default)]
pub struct ComparisonMemory {
    bytes: AtomicUsize,
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
        self.bytes
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
        Some(())
    }

    pub fn used_bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
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
