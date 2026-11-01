use super::GixRepo;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::services::Result;

impl GixRepo {
    pub(super) fn require_diff_target(&self, target: &DiffTarget) -> Result<()> {
        match target {
            DiffTarget::WorkingTree { .. }
            | DiffTarget::CommitRange {
                to_commit_id: None, ..
            } => self.require_working_tree(),
            DiffTarget::Commit { .. }
            | DiffTarget::CommitRange {
                to_commit_id: Some(_),
                ..
            } => Ok(()),
        }
    }
}
