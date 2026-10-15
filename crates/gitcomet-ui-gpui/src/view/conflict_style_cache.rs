//! Styled text for the conflict resolver's split rows, cached per row and side.
//! Rows past a dense prefix live in a bounded set of sparse pages.
use super::CachedDiffStyledText;
use super::conflict_resolver::ConflictPickSide;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
