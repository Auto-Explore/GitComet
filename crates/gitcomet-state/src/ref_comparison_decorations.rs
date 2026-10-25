use crate::ref_comparisons::MAX_COMPARISON_PROJECTIONS;
use gitcomet_core::domain::CommitId;
use gitcomet_core::ref_comparison::{RefComparison, RefComparisonReason};
use gitcomet_core::ref_comparison_memory::{
    ComparisonMemory, ComparisonReservation, hash_table_bytes, vector_bytes,
};
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const DECORATION_PAGES: usize = 256;
const DIRECTORY_BYTES: usize = DECORATION_PAGES * std::mem::size_of::<Arc<DecorationPage>>() + 256;

#[derive(Debug, Default)]
struct DecorationPage {
    entries: FxHashMap<CommitId, RefComparison>,
    _reservation: Option<Arc<ComparisonReservation>>,
}

#[derive(Debug)]
struct DecorationDirectory {
    pages: Vec<Arc<DecorationPage>>,
    _reservation: Arc<ComparisonReservation>,
}

#[derive(Clone, Debug, Default)]
pub struct ComparisonDecorations {
    directory: Option<Arc<DecorationDirectory>>,
    len: usize,
}

impl ComparisonDecorations {
    fn page(oid: &CommitId) -> usize {
        let mut hasher = FxHasher::default();
        oid.hash(&mut hasher);
        hasher.finish() as usize % DECORATION_PAGES
    }

    pub fn get(&self, oid: &CommitId) -> Option<&RefComparison> {
        self.directory.as_ref()?.pages[Self::page(oid)]
            .entries
            .get(oid)
    }

    pub(crate) fn retain(
        &mut self,
        memory: &Arc<ComparisonMemory>,
        members: &rustc_hash::FxHashSet<CommitId>,
    ) -> Result<(), RefComparisonReason> {
        let Some(directory) = &self.directory else {
            return Ok(());
        };
        if directory
            .pages
            .iter()
            .all(|page| page.entries.keys().all(|id| members.contains(id)))
        {
            return Ok(());
        }
        let reservation = memory
            .reserve(DIRECTORY_BYTES)
            .ok_or(RefComparisonReason::Budget)?;
        let mut pages = directory.pages.clone();
        let empty = Arc::new(DecorationPage::default());
        let mut len = 0;
        for page in &mut pages {
            let count = page
                .entries
                .keys()
                .filter(|id| members.contains(*id))
                .count();
            len += count;
            if count == page.entries.len() {
                continue;
            }
            if count == 0 {
                *page = empty.clone();
                continue;
            }
            let reservation = memory
                .reserve(hash_table_bytes::<CommitId, RefComparison>(count))
                .ok_or(RefComparisonReason::Budget)?;
            let mut entries = FxHashMap::with_capacity_and_hasher(count, Default::default());
            entries.extend(
                page.entries
                    .iter()
                    .filter(|(id, _)| members.contains(*id))
                    .map(|(id, result)| (id.clone(), result.clone())),
            );
            *page = Arc::new(DecorationPage {
                entries,
                _reservation: Some(reservation),
            });
        }
        self.directory = Some(Arc::new(DecorationDirectory {
            pages,
            _reservation: reservation,
        }));
        self.len = len;
        Ok(())
    }

    pub fn publish(
        &mut self,
        memory: &Arc<ComparisonMemory>,
        results: Vec<RefComparison>,
    ) -> Result<(), RefComparisonReason> {
        if results.is_empty() {
            return Ok(());
        }
        if results.len() > gitcomet_core::ref_comparison::MAX_COMPARISON_SUBJECTS {
            return Err(RefComparisonReason::Budget);
        }
        let scratch_bytes = hash_table_bytes::<usize, Vec<RefComparison>>(results.len())
            .saturating_add(hash_table_bytes::<CommitId, ()>(results.len()))
            .saturating_add(vector_bytes::<RefComparison>(
                results.len().saturating_mul(4),
            ));
        let _scratch = memory
            .reserve(scratch_bytes)
            .ok_or(RefComparisonReason::Budget)?;
        let mut subjects =
            rustc_hash::FxHashSet::with_capacity_and_hasher(results.len(), Default::default());
        if results
            .iter()
            .any(|result| !subjects.insert(result.subject.clone()))
        {
            return Err(RefComparisonReason::IncompleteHistory);
        }
        let directory_memory = memory
            .reserve(DIRECTORY_BYTES)
            .ok_or(RefComparisonReason::Budget)?;
        let mut pages = self.directory.as_ref().map_or_else(
            || {
                let empty = Arc::new(DecorationPage::default());
                vec![empty; DECORATION_PAGES]
            },
            |directory| directory.pages.clone(),
        );
        let mut batches: FxHashMap<usize, Vec<RefComparison>> = FxHashMap::default();
        for result in results {
            batches
                .entry(Self::page(&result.subject))
                .or_default()
                .push(result);
        }
        let mut len = self.len;
        for (page, batch) in batches {
            let previous = &pages[page];
            let additional = batch
                .iter()
                .filter(|result| !previous.entries.contains_key(&result.subject))
                .count();
            if len.saturating_add(additional) > MAX_COMPARISON_PROJECTIONS {
                return Err(RefComparisonReason::Budget);
            }
            let capacity = previous.entries.len().saturating_add(additional);
            let reservation = memory
                .reserve(hash_table_bytes::<CommitId, RefComparison>(capacity))
                .ok_or(RefComparisonReason::Budget)?;
            let mut entries = FxHashMap::with_capacity_and_hasher(capacity, Default::default());
            entries.extend(
                previous
                    .entries
                    .iter()
                    .map(|(oid, result)| (oid.clone(), result.clone())),
            );
            for result in batch {
                entries.insert(result.subject.clone(), result);
            }
            pages[page] = Arc::new(DecorationPage {
                entries,
                _reservation: Some(reservation),
            });
            len += additional;
        }
        self.directory = Some(Arc::new(DecorationDirectory {
            pages,
            _reservation: directory_memory,
        }));
        self.len = len;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::ref_comparison::{
        COMPARISON_REPOSITORY_BYTES, IntegrationEvidence, RefComparisonCounts, RefIntegration,
    };

    fn result(ahead: u64) -> RefComparison {
        RefComparison {
            subject: CommitId("a".repeat(40).into()),
            counts: RefComparisonCounts::Exact { ahead, behind: 0 },
            integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
        }
    }

    #[test]
    fn retained_snapshots_keep_their_memory_charge_and_immutable_values() {
        let memory = Arc::new(ComparisonMemory::default());
        let mut decorations = ComparisonDecorations::default();
        decorations.publish(&memory, vec![result(0)]).unwrap();
        let first_bytes = memory.used_bytes();
        assert!(first_bytes > 0);
        let retained = decorations.clone();
        decorations.publish(&memory, vec![result(1)]).unwrap();
        assert!(memory.used_bytes() > first_bytes + DIRECTORY_BYTES);
        assert_eq!(
            retained.get(&result(0).subject).unwrap().counts,
            RefComparisonCounts::Exact {
                ahead: 0,
                behind: 0
            }
        );
        drop(retained);
        assert_eq!(memory.used_bytes(), first_bytes);
        drop(decorations);
        assert_eq!(memory.used_bytes(), 0);
    }

    #[test]
    fn memory_exhaustion_refuses_the_entire_publication_and_releases_temporary_pages() {
        let _guard = crate::ref_comparisons::MEMORY_TESTS.lock().unwrap();
        let memory = Arc::new(ComparisonMemory::default());
        let mut decorations = ComparisonDecorations::default();
        decorations.publish(&memory, vec![result(0)]).unwrap();
        let first_bytes = memory.used_bytes();
        let retained = memory
            .reserve(COMPARISON_REPOSITORY_BYTES - first_bytes)
            .unwrap();
        assert_eq!(
            decorations.publish(&memory, vec![result(1)]),
            Err(RefComparisonReason::Budget)
        );
        assert_eq!(
            decorations.get(&result(0).subject).unwrap().counts,
            RefComparisonCounts::Exact {
                ahead: 0,
                behind: 0
            }
        );
        drop(retained);
        assert_eq!(memory.used_bytes(), first_bytes);
        decorations.publish(&memory, vec![result(1)]).unwrap();
        drop(decorations);
        assert_eq!(memory.used_bytes(), 0);
    }
}
