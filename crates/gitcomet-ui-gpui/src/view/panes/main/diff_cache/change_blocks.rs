//! Change-block starts for change navigation, one per contiguous run of
//! visually changed rows (see `view::diff_navigation`).

use super::*;
use crate::view::diff_navigation::{
    change_block_entries, change_block_entries_with_transparent_rows,
};

impl MainPaneView {
    /// Whether a Full/Collapsed file-diff row shows as a change in the current layout.
    pub(in crate::view) fn file_diff_row_is_change(&self, row_ix: usize) -> bool {
        match self.diff_view {
            DiffViewMode::Inline => matches!(
                self.file_diff_inline_visual_kind(row_ix),
                gitcomet_core::domain::DiffLineKind::Add
                    | gitcomet_core::domain::DiffLineKind::Remove
            ),
            DiffViewMode::Split => !matches!(
                self.file_diff_split_visual_kind(row_ix),
                gitcomet_core::file_diff::FileDiffRowKind::Context
            ),
        }
    }

    /// Source-visible indices. Hidden context always sits behind a hunk
    /// header row, so adjacent file rows are adjacent in the file too.
    pub(in crate::view) fn collapsed_change_block_starts(&self) -> Vec<usize> {
        let rows = &self.collapsed_diff_visible_rows;
        change_block_entries(rows.len(), |visible_ix| {
            rows[visible_ix]
                .row_ix()
                .is_some_and(|row_ix| self.file_diff_row_is_change(row_ix))
        })
    }

    /// Source-visible indices; hunk and file headers end a block.
    pub(in crate::view) fn patch_change_block_starts(&self) -> Vec<usize> {
        use gitcomet_core::domain::DiffLineKind as DK;

        // Mapped directly: `diff_source_mapped_ix_for_visible_ix` re-checks
        // the collapsed projection per row.
        let mapped_ix = |visible_ix: usize| match self.diff_visible_inline_map.as_ref() {
            Some(map) => map.src_ix_for_visible_ix(visible_ix),
            None => self.diff_visible_indices.get(visible_ix).copied(),
        };
        let is_marker = |src_ix: usize| {
            self.patch_diff_row(src_ix)
                .is_some_and(|line| is_unified_no_newline_marker(&line.text))
        };
        let len = self.diff_source_visible_len();

        match self.diff_view {
            DiffViewMode::Inline => change_block_entries_with_transparent_rows(
                len,
                |visible_ix| {
                    mapped_ix(visible_ix).is_some_and(|src_ix| {
                        matches!(self.patch_visual_line_kind(src_ix), DK::Add | DK::Remove)
                    })
                },
                |visible_ix| mapped_ix(visible_ix).is_some_and(is_marker),
            ),
            DiffViewMode::Split => {
                let aligned_row = |visible_ix: usize| {
                    mapped_ix(visible_ix)
                        .and_then(|row_ix| self.patch_diff_split_row(row_ix))
                        .filter(|row| matches!(row, PatchSplitRow::Aligned { .. }))
                };
                change_block_entries_with_transparent_rows(
                    len,
                    |visible_ix| {
                        aligned_row(visible_ix).is_some_and(|row| {
                            !matches!(
                                self.patch_split_visual_row_kind(&row),
                                gitcomet_core::file_diff::FileDiffRowKind::Context
                            )
                        })
                    },
                    |visible_ix| {
                        aligned_row(visible_ix).is_some_and(|row| match row {
                            PatchSplitRow::Aligned {
                                old_src_ix: Some(src_ix),
                                ..
                            } => is_marker(src_ix),
                            _ => false,
                        })
                    },
                )
            }
        }
    }
}
