use gitcomet_core::merge::{MergeBlockId, MergePlan, MergeSource};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictPlanOrigin {
    base: Option<Arc<str>>,
    local: Arc<str>,
    remote: Arc<str>,
    blocks: Arc<[MergeBlockId]>,
}

impl ConflictPlanOrigin {
    pub fn from_plan(plan: &MergePlan) -> Self {
        Self {
            base: plan.base.clone(),
            local: Arc::clone(&plan.local),
            remote: Arc::clone(&plan.remote),
            blocks: plan.blocks.iter().map(|block| block.id).collect(),
        }
    }

    pub fn matches_plan(&self, plan: &MergePlan) -> bool {
        self.base == plan.base
            && self.local == plan.local
            && self.remote == plan.remote
            && self
                .blocks
                .iter()
                .copied()
                .eq(plan.blocks.iter().map(|block| block.id))
    }

    pub fn contains(&self, block_id: MergeBlockId) -> bool {
        self.blocks.contains(&block_id)
    }

    pub fn has_base(&self) -> bool {
        self.base.is_some()
    }

    pub fn local_source(&self) -> MergeSource {
        if self.has_base() {
            MergeSource::B
        } else {
            MergeSource::A
        }
    }

    pub fn remote_source(&self) -> MergeSource {
        if self.has_base() {
            MergeSource::C
        } else {
            MergeSource::B
        }
    }
}
