//! Change blocks for change navigation and its focus bar: each contiguous run
//! of visually changed rows (see `view::diff_navigation`).

use super::*;
use crate::view::diff_navigation::{
    change_block_ranges, change_block_ranges_with_transparent_rows,
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

    /// Source-visible rows. Hidden context always sits behind a hunk header
    /// row, so adjacent file rows are adjacent in the file too.
    pub(in crate::view) fn collapsed_change_blocks(&self) -> Vec<Range<usize>> {
        let rows = &self.collapsed_diff_visible_rows;
        change_block_ranges(rows.len(), |visible_ix| {
            rows[visible_ix]
                .row_ix()
                .is_some_and(|row_ix| self.file_diff_row_is_change(row_ix))
        })
    }

    /// Source-visible rows; hunk and file headers end a block.
    pub(in crate::view) fn patch_change_blocks(&self) -> Vec<Range<usize>> {
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
            DiffViewMode::Inline => change_block_ranges_with_transparent_rows(
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
                change_block_ranges_with_transparent_rows(
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

    /// What the rows were laid out from; a focused block captured under a
    /// different key points at rows that have since moved.
    pub(in crate::view) fn diff_visible_layout_key(&self) -> DiffVisibleLayoutKey {
        DiffVisibleLayoutKey {
            len: self.diff_visible_cache_len,
            view: self.diff_visible_view,
            is_file_view: self.diff_visible_is_file_view,
            projection_rev: self.diff_visible_cache_projection_rev,
        }
    }

    fn file_diff_row_change_sides(&self, row_ix: usize) -> DiffChangeSides {
        use gitcomet_core::domain::DiffLineKind as DK;
        match self.diff_view {
            DiffViewMode::Inline => match self.file_diff_inline_visual_kind(row_ix) {
                DK::Remove => DiffChangeSides {
                    removed: true,
                    added: false,
                },
                DK::Add => DiffChangeSides {
                    removed: false,
                    added: true,
                },
                DK::Context | DK::Header | DK::Hunk => DiffChangeSides::default(),
            },
            DiffViewMode::Split => split_row_change_sides(self.file_diff_split_visual_kind(row_ix)),
        }
    }

    fn patch_row_change_sides(&self, mapped_ix: usize) -> DiffChangeSides {
        use gitcomet_core::domain::DiffLineKind as DK;
        match self.diff_view {
            DiffViewMode::Inline => match self.patch_visual_line_kind(mapped_ix) {
                DK::Remove => DiffChangeSides {
                    removed: true,
                    added: false,
                },
                DK::Add => DiffChangeSides {
                    removed: false,
                    added: true,
                },
                DK::Context | DK::Header | DK::Hunk => DiffChangeSides::default(),
            },
            DiffViewMode::Split => match self.patch_diff_split_row(mapped_ix) {
                Some(row @ PatchSplitRow::Aligned { .. }) => {
                    split_row_change_sides(self.patch_split_visual_row_kind(&row))
                }
                _ => DiffChangeSides::default(),
            },
        }
    }

    /// What each source-visible row changes, with the layout resolved once so
    /// walking a large block does not redo the mode checks per row.
    fn diff_row_change_sides_lookup(&self) -> impl Fn(usize) -> DiffChangeSides + '_ {
        let collapsed = self.is_collapsed_diff_projection_active();
        let file_rows = collapsed || self.is_file_diff_view_active();
        move |source_visible_ix| {
            let mapped_ix = if collapsed {
                self.collapsed_diff_visible_rows
                    .get(source_visible_ix)
                    .and_then(|row| row.row_ix())
            } else {
                match self.diff_visible_inline_map.as_ref() {
                    Some(map) => map.src_ix_for_visible_ix(source_visible_ix),
                    None => self.diff_visible_indices.get(source_visible_ix).copied(),
                }
            };
            match mapped_ix {
                Some(ix) if file_rows => self.file_diff_row_change_sides(ix),
                Some(ix) => self.patch_row_change_sides(ix),
                None => DiffChangeSides::default(),
            }
        }
    }

    /// Everything a block of source-visible rows removes and adds.
    pub(in crate::view) fn diff_block_change_sides(&self, rows: Range<usize>) -> DiffChangeSides {
        let sides_of = self.diff_row_change_sides_lookup();
        let mut sides = DiffChangeSides::default();
        for source_visible_ix in rows {
            sides = sides.union(sides_of(source_visible_ix));
            if sides.removed && sides.added {
                break;
            }
        }
        sides
    }

    /// How visual row `visible_ix` paints its part of the focused change
    /// block's marks, while the selection and row layout they were captured
    /// with still hold.
    pub(in crate::view) fn diff_focused_change_block_row(
        &self,
        visible_ix: usize,
    ) -> Option<FocusedChangeBlockRow> {
        let focus = self.diff_focused_change_block.as_ref()?;
        if self.diff_selection_anchor != Some(focus.anchor)
            || focus.layout != self.diff_visible_layout_key()
        {
            return None;
        }
        let in_block = |visible_ix: usize| {
            self.diff_source_visible_ix_for_visible_ix(visible_ix)
                .filter(|source_visible_ix| focus.rows.contains(source_visible_ix))
        };
        let source_visible_ix = in_block(visible_ix)?;
        let sides_of = self.diff_row_change_sides_lookup();
        let mut row_sides = sides_of(source_visible_ix);
        // A `\ No newline` marker changes nothing itself; it belongs to the line
        // above it, and never starts a block.
        if row_sides == DiffChangeSides::default() && source_visible_ix > focus.rows.start {
            row_sides = sides_of(source_visible_ix - 1);
        }
        Some(FocusedChangeBlockRow {
            // Wrapped rows share a source row, so neighbours decide the edges.
            top: visible_ix.checked_sub(1).and_then(in_block).is_none(),
            bottom: visible_ix.checked_add(1).and_then(in_block).is_none(),
            row_sides,
            block_sides: focus.sides,
        })
    }
}

fn split_row_change_sides(kind: gitcomet_core::file_diff::FileDiffRowKind) -> DiffChangeSides {
    use gitcomet_core::file_diff::FileDiffRowKind as RK;
    DiffChangeSides {
        removed: matches!(kind, RK::Remove | RK::Modify),
        added: matches!(kind, RK::Add | RK::Modify),
    }
}
