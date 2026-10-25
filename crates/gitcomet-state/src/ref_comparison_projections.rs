use crate::ref_comparisons::MAX_COMPARISON_PROJECTIONS;
use gitcomet_core::domain::{Branch, CommitId, Worktree};
use gitcomet_core::ref_comparison_memory::{
    ComparisonMemory, ComparisonReservation, hash_table_bytes, vector_bytes,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct ComparisonProjections {
    pub branches: FxHashMap<String, CommitId>,
    pub worktrees: FxHashMap<PathBuf, CommitId>,
    pub subjects: Vec<CommitId>,
    pub members: Arc<FxHashSet<CommitId>>,
    pub worktree_heads: FxHashSet<CommitId>,
    pub active: Option<CommitId>,
    _reservation: Option<Arc<ComparisonReservation>>,
}

impl ComparisonProjections {
    pub(crate) fn capture(
        memory: &Arc<ComparisonMemory>,
        branches: &[Branch],
        worktrees: &[Worktree],
        visible: &[CommitId],
        active: Option<CommitId>,
        metadata_bytes: usize,
    ) -> Option<Arc<Self>> {
        let tips = branches.len().saturating_add(worktrees.len());
        if tips > MAX_COMPARISON_PROJECTIONS {
            return None;
        }
        let bytes = hash_table_bytes::<String, CommitId>(branches.len())
            .saturating_add(metadata_bytes)
            .saturating_add(
                tips.saturating_mul(gitcomet_core::ref_comparison_memory::COMPARISON_OID_BYTES),
            )
            .saturating_add(hash_table_bytes::<PathBuf, CommitId>(worktrees.len()))
            .saturating_add(hash_table_bytes::<CommitId, ()>(worktrees.len()))
            .saturating_add(vector_bytes::<CommitId>(tips))
            .saturating_add(
                branches
                    .iter()
                    .map(|branch| branch.name.len())
                    .sum::<usize>(),
            )
            .saturating_add(
                worktrees
                    .iter()
                    .map(|worktree| worktree.path.as_os_str().len())
                    .sum::<usize>(),
            );
        let scratch = hash_table_bytes::<CommitId, ()>(tips);
        let reservation = memory.reserve(bytes.saturating_add(scratch))?;
        let mut projections = Self {
            branches: FxHashMap::with_capacity_and_hasher(branches.len(), Default::default()),
            worktrees: FxHashMap::with_capacity_and_hasher(worktrees.len(), Default::default()),
            subjects: Vec::with_capacity(tips),
            members: Arc::default(),
            worktree_heads: FxHashSet::with_capacity_and_hasher(
                worktrees.len(),
                Default::default(),
            ),
            active,
            _reservation: Some(reservation.clone()),
        };
        for branch in branches {
            projections
                .branches
                .insert(branch.name.clone(), branch.target.clone());
        }
        for worktree in worktrees {
            if let Some(head) = &worktree.head {
                projections
                    .worktrees
                    .insert(worktree.path.clone(), head.clone());
                projections.worktree_heads.insert(head.clone());
            }
        }
        let mut available: FxHashSet<CommitId> =
            FxHashSet::with_capacity_and_hasher(tips, Default::default());
        available.extend(
            projections
                .worktrees
                .values()
                .chain(projections.branches.values())
                .cloned(),
        );
        let mut push = |id: &CommitId| {
            if available.remove(id) {
                projections.subjects.push(id.clone());
            }
        };
        for id in visible.iter().take(128) {
            push(id);
        }
        if let Some(active) = &projections.active {
            push(active);
        }
        for head in projections.worktrees.values() {
            push(head);
        }
        for head in projections.branches.values() {
            push(head);
        }
        available.clear();
        available.extend(projections.subjects.iter().cloned());
        reservation.resize(
            bytes.saturating_add(scratch).saturating_sub(
                tips.saturating_sub(projections.subjects.len())
                    .saturating_mul(gitcomet_core::ref_comparison_memory::COMPARISON_OID_BYTES),
            ),
        );
        projections.members = Arc::new(available);
        Some(Arc::new(projections))
    }
}
