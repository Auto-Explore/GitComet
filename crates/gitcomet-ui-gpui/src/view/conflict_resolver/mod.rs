mod split_row_index;
mod word_highlight;

use super::CachedDiffStyledText;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use split_row_index::SparseLineIndex;
#[cfg(test)]
use split_row_index::{CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES, CONFLICT_SPLIT_PAGE_SIZE};
pub use split_row_index::{ConflictSplitRowIndex, TwoWaySplitProjection, TwoWaySplitVisibleRow};
#[cfg(any(test, feature = "benchmarks"))]
pub use word_highlight::compute_three_way_word_highlights;
#[cfg(feature = "benchmarks")]
pub use word_highlight::{TwoWayWordHighlights, compute_two_way_word_highlights};
pub use word_highlight::{compute_word_highlights_for_row, compute_word_highlights_for_texts};

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

pub use gitcomet_core::conflict_output::ConflictOutputChoice as ConflictChoice;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictResolverViewMode {
    ThreeWay,
    TwoWayDiff,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictRenderingMode {
    EagerSmallFile,
    StreamedLargeFile,
}

impl ConflictRenderingMode {
    pub fn is_streamed_large_file(self) -> bool {
        matches!(self, Self::StreamedLargeFile)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum ConflictPickSide {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, Default)]
struct ConflictSplitStyledTextCacheRow {
    ours: Option<CachedDiffStyledText>,
    theirs: Option<CachedDiffStyledText>,
}

const CONFLICT_SPLIT_STYLE_DENSE_ROWS: usize = 16_384;
const CONFLICT_SPLIT_STYLE_PAGE_ROWS: usize = 256;
const CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES: usize = 16;

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct ConflictSplitStyledTextCache {
    rows: Vec<ConflictSplitStyledTextCacheRow>,
    sparse_pages: FxHashMap<usize, Vec<ConflictSplitStyledTextCacheRow>>,
    sparse_page_order: VecDeque<usize>,
    entries: usize,
}

impl ConflictSplitStyledTextCache {
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn with_row_capacity(row_count: usize) -> Self {
        let mut cache = Self::default();
        cache.rows.resize_with(
            row_count.min(CONFLICT_SPLIT_STYLE_DENSE_ROWS),
            ConflictSplitStyledTextCacheRow::default,
        );
        cache
    }

    fn slot(
        row: &ConflictSplitStyledTextCacheRow,
        side: ConflictPickSide,
    ) -> &Option<CachedDiffStyledText> {
        match side {
            ConflictPickSide::Ours => &row.ours,
            ConflictPickSide::Theirs => &row.theirs,
        }
    }

    fn slot_mut(
        row: &mut ConflictSplitStyledTextCacheRow,
        side: ConflictPickSide,
    ) -> &mut Option<CachedDiffStyledText> {
        match side {
            ConflictPickSide::Ours => &mut row.ours,
            ConflictPickSide::Theirs => &mut row.theirs,
        }
    }

    fn sparse_page_key(row_ix: usize) -> usize {
        (row_ix - CONFLICT_SPLIT_STYLE_DENSE_ROWS) / CONFLICT_SPLIT_STYLE_PAGE_ROWS
    }

    fn sparse_page_offset(row_ix: usize) -> usize {
        (row_ix - CONFLICT_SPLIT_STYLE_DENSE_ROWS) % CONFLICT_SPLIT_STYLE_PAGE_ROWS
    }

    fn row_entry_count(row: &ConflictSplitStyledTextCacheRow) -> usize {
        usize::from(row.ours.is_some()) + usize::from(row.theirs.is_some())
    }

    fn ensure_row(&mut self, row_ix: usize) -> &mut ConflictSplitStyledTextCacheRow {
        if row_ix < CONFLICT_SPLIT_STYLE_DENSE_ROWS {
            if row_ix >= self.rows.len() {
                self.rows
                    .resize_with(row_ix + 1, ConflictSplitStyledTextCacheRow::default);
            }
            return &mut self.rows[row_ix];
        }

        let page_key = Self::sparse_page_key(row_ix);
        if !self.sparse_pages.contains_key(&page_key) {
            while self.sparse_pages.len() >= CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES {
                let Some(evicted_key) = self.sparse_page_order.pop_front() else {
                    break;
                };
                if let Some(evicted) = self.sparse_pages.remove(&evicted_key) {
                    let evicted_entries = evicted.iter().map(Self::row_entry_count).sum::<usize>();
                    self.entries = self.entries.saturating_sub(evicted_entries);
                }
            }
            self.sparse_pages.insert(
                page_key,
                vec![ConflictSplitStyledTextCacheRow::default(); CONFLICT_SPLIT_STYLE_PAGE_ROWS],
            );
            self.sparse_page_order.push_back(page_key);
        }
        &mut self
            .sparse_pages
            .get_mut(&page_key)
            .expect("inserted conflict style cache page")[Self::sparse_page_offset(row_ix)]
    }

    pub(in crate::view) fn get(
        &self,
        key: &(usize, ConflictPickSide),
    ) -> Option<&CachedDiffStyledText> {
        let (row_ix, side) = *key;
        let row = if row_ix < CONFLICT_SPLIT_STYLE_DENSE_ROWS {
            self.rows.get(row_ix)?
        } else {
            self.sparse_pages
                .get(&Self::sparse_page_key(row_ix))?
                .get(Self::sparse_page_offset(row_ix))?
        };
        Self::slot(row, side).as_ref()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn contains_key(&self, key: &(usize, ConflictPickSide)) -> bool {
        self.get(key).is_some()
    }

    pub(in crate::view) fn insert(
        &mut self,
        key: (usize, ConflictPickSide),
        value: CachedDiffStyledText,
    ) -> Option<CachedDiffStyledText> {
        let (row_ix, side) = key;
        let slot = Self::slot_mut(self.ensure_row(row_ix), side);
        let previous = slot.replace(value);
        if previous.is_none() {
            self.entries = self.entries.saturating_add(1);
        }
        previous
    }

    pub(in crate::view) fn clear(&mut self) {
        self.rows.clear();
        self.sparse_pages.clear();
        self.sparse_page_order.clear();
        self.entries = 0;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn len(&self) -> usize {
        self.entries
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn is_empty(&self) -> bool {
        self.entries == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveTraceMode {
    /// The safe rules and the subchunk split, applied automatically when the
    /// file opened. Whitespace-only, regex and history merges are never
    /// automatic — kdiff3 parity, see the section 30 auto-solve policy.
    OnOpen,
    #[cfg(test)]
    History,
}

mod autosolve;
mod block_diff;
mod generated_text;
mod markers;
mod nav;
mod projection;
mod provenance;
mod resolution;
mod three_way;

pub use autosolve::*;
pub use block_diff::*;
pub use generated_text::*;
pub use markers::*;
pub use nav::*;
pub use projection::*;
pub use provenance::*;
pub use resolution::*;
pub use three_way::*;

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests;
