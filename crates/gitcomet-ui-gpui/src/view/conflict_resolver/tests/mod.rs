pub(super) use super::*;
pub(super) use gitcomet_core::conflict_output::{
    ConflictMarkerLabels, GenerateResolvedTextOptions, UnresolvedConflictMode,
};
pub(super) use gitcomet_core::file_diff::FileDiffRow;
pub(super) use gitcomet_core::file_diff::FileDiffRowKind as RK;

pub(super) fn mark_block_resolved(segments: &mut [ConflictSegment], target: usize) {
    let mut seen = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if seen == target {
            block.resolved = true;
            return;
        }
        seen += 1;
    }
    panic!("missing block index {target}");
}

mod block_diff;
mod parsing;
mod resolution;
mod split_row_index;
mod visibility;
