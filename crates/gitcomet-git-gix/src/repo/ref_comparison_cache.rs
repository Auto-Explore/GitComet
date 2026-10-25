use gitcomet_core::domain::CommitId;
use gitcomet_core::ref_comparison::{
    COMPARISON_ALGORITHM_VERSION, COMPARISON_REPOSITORY_BYTES, RefComparison, RefComparisonDepth,
};
use gitcomet_core::ref_comparison_memory::{ComparisonMemory, ComparisonReservation};
use rustc_hash::FxHashMap;
use std::sync::Arc;

const CACHE_BYTES: usize = COMPARISON_REPOSITORY_BYTES / 4;
const ENTRY_BYTES: usize = 384;

#[derive(Default)]
pub(super) struct ComparisonCache {
    results: FxHashMap<(u32, CommitId, CommitId), (RefComparisonDepth, RefComparison)>,
    bytes: usize,
    reservation: Option<Arc<ComparisonReservation>>,
}

impl ComparisonCache {
    pub fn get(
        &self,
        target: &CommitId,
        subject: &CommitId,
        depth: RefComparisonDepth,
    ) -> Option<RefComparison> {
        self.results
            .get(&(
                COMPARISON_ALGORITHM_VERSION,
                target.clone(),
                subject.clone(),
            ))
            .filter(|(stored, result)| {
                depth == RefComparisonDepth::Counts
                    || *stored == depth
                    || *stored == RefComparisonDepth::Deep
                    || matches!(
                        result.integration,
                        gitcomet_core::ref_comparison::RefIntegration::Confirmed(_)
                    )
            })
            .map(|(_, result)| result.clone())
    }

    pub fn insert(
        &mut self,
        target: &CommitId,
        result: &RefComparison,
        depth: RefComparisonDepth,
        memory: &Arc<ComparisonMemory>,
    ) {
        let mut result = result.clone();
        let depth = if matches!(
            result.integration,
            gitcomet_core::ref_comparison::RefIntegration::NotConfirmed(
                gitcomet_core::ref_comparison::RefComparisonReason::IncompleteHistory
                    | gitcomet_core::ref_comparison::RefComparisonReason::Unsupported
            )
        ) {
            result.integration = gitcomet_core::ref_comparison::RefIntegration::Pending;
            RefComparisonDepth::Counts
        } else {
            depth
        };
        if matches!(
            result.counts,
            gitcomet_core::ref_comparison::RefComparisonCounts::Unavailable(
                gitcomet_core::ref_comparison::RefComparisonReason::IncompleteHistory
                    | gitcomet_core::ref_comparison::RefComparisonReason::Unsupported
            )
        ) {
            return;
        }
        let key = (
            COMPARISON_ALGORITHM_VERSION,
            target.clone(),
            result.subject.clone(),
        );
        if let Some(existing) = self.results.get_mut(&key) {
            *existing = (depth, result.clone());
            return;
        }
        if self.bytes + ENTRY_BYTES > CACHE_BYTES {
            self.clear();
        }
        let Some(reservation) = self.reservation.clone().or_else(|| memory.reserve(0)) else {
            return;
        };
        if !reservation.resize(self.bytes + ENTRY_BYTES) {
            return;
        }
        self.reservation = Some(reservation);
        self.bytes += ENTRY_BYTES;
        self.results.insert(key, (depth, result.clone()));
    }

    fn clear(&mut self) {
        self.results = FxHashMap::default();
        self.reservation = None;
        self.bytes = 0;
    }
}

impl Drop for ComparisonCache {
    fn drop(&mut self) {
        self.clear();
    }
}
