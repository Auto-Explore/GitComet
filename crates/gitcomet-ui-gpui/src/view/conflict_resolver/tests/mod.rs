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
mod navigation;
mod overview;
mod parsing;
mod resolution;
mod split_row_index;
mod visibility;

#[test]
fn split_style_cache_keeps_far_rows_sparse_and_bounded() {
    let mut cache = ConflictSplitStyledTextCache::default();
    let far_row = CONFLICT_SPLIT_STYLE_DENSE_ROWS + CONFLICT_SPLIT_STYLE_PAGE_ROWS * 10_000;
    let _ = cache.ensure_row(far_row);

    assert!(cache.rows.len() <= CONFLICT_SPLIT_STYLE_DENSE_ROWS);
    assert_eq!(cache.sparse_pages.len(), 1);

    for page in 0..CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES + 4 {
        let row = CONFLICT_SPLIT_STYLE_DENSE_ROWS + page * CONFLICT_SPLIT_STYLE_PAGE_ROWS;
        let _ = cache.ensure_row(row);
    }
    assert_eq!(
        cache.sparse_pages.len(),
        CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES
    );
}
