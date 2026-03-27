use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;

use super::ConflictSegment;

pub(super) const CONFLICT_SPLIT_PAGE_SIZE: usize = 256;
pub(super) const CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES: usize = 8;
const CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE: usize = CONFLICT_SPLIT_PAGE_SIZE;

/// Sparse line-start checkpoints for lazy row materialization.
///
/// Startup only needs line counts and occasional random access into the
/// visible window, so storing every line start for giant blocks is wasted
/// work.  Instead we keep one byte offset every N lines and rescan the small
/// local window from the nearest checkpoint when a page is requested.
#[derive(Clone, Debug, Default)]
struct SparseLineIndex {
    line_count: usize,
    checkpoints: Vec<usize>,
    widest_line_ix: usize,
    widest_line_len: usize,
}

impl SparseLineIndex {
    fn for_text(text: &str) -> Self {
        if text.is_empty() {
            return Self::default();
        }

        let bytes = text.as_bytes();
        let mut checkpoints =
            Vec::with_capacity(bytes.len().saturating_div(4096).saturating_add(1));
        checkpoints.push(0usize);
        let mut line_count = 0usize;
        let mut prev_pos = 0usize;
        let mut widest_line_ix = 0usize;
        let mut widest_line_len = 0usize;

        for pos in memchr::memchr_iter(b'\n', bytes) {
            let line_len = pos - prev_pos;
            if line_len > widest_line_len {
                widest_line_len = line_len;
                widest_line_ix = line_count;
            }
            line_count += 1;
            prev_pos = pos + 1;
            // Record checkpoint at stride boundaries when there is more content.
            if prev_pos < bytes.len()
                && line_count.is_multiple_of(CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE)
            {
                checkpoints.push(prev_pos);
            }
        }

        // Handle last line (no trailing newline).
        if prev_pos < bytes.len() {
            let line_len = bytes.len() - prev_pos;
            if line_len > widest_line_len {
                widest_line_len = line_len;
                widest_line_ix = line_count;
            }
            line_count += 1;
        }

        Self {
            line_count,
            checkpoints,
            widest_line_ix,
            widest_line_len,
        }
    }

    fn line_count(&self) -> usize {
        self.line_count
    }

    fn widest_line(&self) -> Option<(usize, usize)> {
        (self.line_count > 0).then_some((self.widest_line_ix, self.widest_line_len))
    }

    fn matching_lines_from_positions(
        &self,
        bytes: &[u8],
        line_limit: usize,
        positions: impl Iterator<Item = usize>,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        let mut newlines = memchr::memchr_iter(b'\n', bytes).peekable();
        let mut line_ix = 0usize;
        let mut last_line = usize::MAX;

        for pos in positions {
            while let Some(&newline_pos) = newlines.peek() {
                if newline_pos >= pos {
                    break;
                }
                newlines.next();
                line_ix = line_ix.saturating_add(1);
            }
            if line_ix >= line_limit {
                break;
            }
            if line_ix != last_line {
                out.push(line_ix);
                last_line = line_ix;
            }
        }

        out
    }

    /// Collect the 0-based line indices of all lines that contain `needle`
    /// (case-sensitive) within the first `line_limit` lines.  Uses SIMD
    /// `memmem::find_iter` on the full text and maps match positions back
    /// to line indices via checkpoints, avoiding per-line extraction.
    #[cfg(any(test, feature = "benchmarks"))]
    fn lines_containing(&self, text: &str, needle: &[u8], line_limit: usize) -> Vec<usize> {
        let bytes = text.as_bytes();
        let finder = memchr::memmem::Finder::new(needle);
        self.matching_lines_from_positions(bytes, line_limit, finder.find_iter(bytes))
    }

    /// Collect the 0-based line indices of all lines that contain `needle`
    /// using ASCII case-insensitive matching.
    fn lines_containing_ascii_case_insensitive(
        &self,
        text: &str,
        needle: &[u8],
        line_limit: usize,
    ) -> Vec<usize> {
        let bytes = text.as_bytes();
        let Some((&first, &last)) = needle.first().zip(needle.last()) else {
            return Vec::new();
        };
        let first_lower = first.to_ascii_lowercase();
        let first_upper = first.to_ascii_uppercase();

        if needle.len() == 1 {
            return self.matching_lines_from_positions(
                bytes,
                line_limit,
                memchr::memchr2_iter(first_lower, first_upper, bytes),
            );
        }

        let Some(last_start) = bytes.len().checked_sub(needle.len()) else {
            return Vec::new();
        };
        let last_lower = last.to_ascii_lowercase();
        let last_upper = last.to_ascii_uppercase();
        let middle = &needle[1..needle.len() - 1];
        let positions = memchr::memchr2_iter(first_lower, first_upper, &bytes[..=last_start])
            .filter(move |&start| {
                let last = bytes[start + needle.len() - 1];
                (last == last_lower || last == last_upper)
                    && bytes[start + 1..start + needle.len() - 1].eq_ignore_ascii_case(middle)
            });
        self.matching_lines_from_positions(bytes, line_limit, positions)
    }

    #[cfg(test)]
    fn line_range(&self, text: &str, line_ix: usize) -> Option<Range<usize>> {
        if line_ix >= self.line_count {
            return None;
        }

        let checkpoint_line = (line_ix / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE)
            * CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let checkpoint_ix = checkpoint_line / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let mut byte_ix = self.checkpoints.get(checkpoint_ix).copied()?;
        let bytes = text.as_bytes();
        let mut current_line = checkpoint_line;

        while current_line <= line_ix && byte_ix <= bytes.len() {
            let line_start = byte_ix;
            while byte_ix < bytes.len() && bytes[byte_ix] != b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            let line_end = byte_ix;
            if byte_ix < bytes.len() && bytes[byte_ix] == b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            if current_line == line_ix {
                return Some(line_start..line_end);
            }
            current_line = current_line.saturating_add(1);
        }

        None
    }

    fn line_ranges(&self, text: &str, start_line_ix: usize, max_lines: usize) -> Vec<Range<usize>> {
        if start_line_ix >= self.line_count || max_lines == 0 {
            return Vec::new();
        }

        let target_len = (self.line_count - start_line_ix).min(max_lines);
        let checkpoint_line = (start_line_ix / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE)
            * CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let checkpoint_ix = checkpoint_line / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let Some(mut byte_ix) = self.checkpoints.get(checkpoint_ix).copied() else {
            return Vec::new();
        };

        let bytes = text.as_bytes();
        let mut current_line = checkpoint_line;
        let mut ranges = Vec::with_capacity(target_len);
        while current_line < self.line_count && byte_ix <= bytes.len() && ranges.len() < target_len
        {
            let line_start = byte_ix;
            while byte_ix < bytes.len() && bytes[byte_ix] != b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            let line_end = byte_ix;
            if byte_ix < bytes.len() && bytes[byte_ix] == b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            if current_line >= start_line_ix {
                ranges.push(line_start..line_end);
            }
            current_line = current_line.saturating_add(1);
        }
        ranges
    }

    #[cfg(test)]
    fn line_text<'a>(&self, text: &'a str, line_ix: usize) -> Option<&'a str> {
        let range = self.line_range(text, line_ix)?;
        text.get(range)
    }
}

/// Pre-computed segment layout entry for lazy two-way split row generation.
#[derive(Clone, Debug)]
enum SplitLayoutKind {
    /// Boundary context lines from a `Text` segment.
    Context {
        line_index: SparseLineIndex,
        /// Number of leading context rows included from the start of the text.
        leading_row_count: usize,
        /// Source line index where the trailing context window begins.
        trailing_row_start: usize,
        /// 1-based starting ours line number.
        ours_start_line: u32,
        /// 1-based starting theirs line number.
        theirs_start_line: u32,
    },
    /// Plain split rows from a conflict block.
    Block {
        ours_line_index: SparseLineIndex,
        theirs_line_index: SparseLineIndex,
        ours_start_line: u32,
        theirs_start_line: u32,
    },
}

#[derive(Clone, Debug)]
struct SplitLayoutEntry {
    /// First row index in the flat row space.
    row_start: usize,
    /// Number of rows this entry contributes.
    row_count: usize,
    /// Index into the original `marker_segments` slice.
    segment_ix: usize,
    /// Conflict index (for block entries only).
    conflict_ix: Option<usize>,
    kind: SplitLayoutKind,
}

#[derive(Debug, Default)]
struct ConflictSplitPageCache {
    pages: FxHashMap<usize, Arc<[gitcomet_core::file_diff::FileDiffRow]>>,
    lru: std::collections::VecDeque<usize>,
}

impl ConflictSplitPageCache {
    fn touch(&mut self, page_ix: usize) {
        if let Some(pos) = self.lru.iter().position(|&cached_ix| cached_ix == page_ix) {
            self.lru.remove(pos);
        }
        self.lru.push_back(page_ix);
    }

    fn get(&mut self, page_ix: usize) -> Option<Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        let page = self.pages.get(&page_ix).cloned()?;
        self.touch(page_ix);
        Some(page)
    }

    fn insert(
        &mut self,
        page_ix: usize,
        page: Arc<[gitcomet_core::file_diff::FileDiffRow]>,
    ) -> Arc<[gitcomet_core::file_diff::FileDiffRow]> {
        self.pages.insert(page_ix, Arc::clone(&page));
        self.touch(page_ix);
        while self.pages.len() > CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES {
            if let Some(evicted) = self.lru.pop_front() {
                self.pages.remove(&evicted);
            }
        }
        page
    }
}

/// Pre-computed index for lazy two-way split row access in giant mode.
///
/// Instead of eagerly building all `FileDiffRow` objects for every conflict block,
/// this stores compact per-segment metadata and generates rows on demand.
#[derive(Clone, Debug)]
pub struct ConflictSplitRowIndex {
    entries: Vec<SplitLayoutEntry>,
    total_rows: usize,
    page_size: usize,
    pages: Arc<std::sync::Mutex<ConflictSplitPageCache>>,
}

impl Default for ConflictSplitRowIndex {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            total_rows: 0,
            page_size: CONFLICT_SPLIT_PAGE_SIZE,
            pages: Arc::new(std::sync::Mutex::new(ConflictSplitPageCache::default())),
        }
    }
}

impl ConflictSplitRowIndex {
    /// Build the layout from conflict segments.
    pub fn new(segments: &[ConflictSegment], context_lines: usize) -> Self {
        let mut entries = Vec::new();
        let mut total_rows = 0usize;
        let mut ours_line = 1u32;
        let mut theirs_line = 1u32;
        let mut conflict_ix = 0usize;

        for (segment_ix, segment) in segments.iter().enumerate() {
            match segment {
                ConflictSegment::Text(text) => {
                    let line_index = SparseLineIndex::for_text(text);
                    let line_count_usize = line_index.line_count();
                    let line_count = u32::try_from(line_count_usize).unwrap_or(u32::MAX);

                    let has_prev_block = segment_ix > 0
                        && matches!(
                            segments.get(segment_ix - 1),
                            Some(ConflictSegment::Block(_))
                        );
                    let has_next_block = matches!(
                        segments.get(segment_ix + 1),
                        Some(ConflictSegment::Block(_))
                    );

                    let leading = if has_prev_block {
                        context_lines.min(line_count_usize)
                    } else {
                        0
                    };
                    let trailing = if has_next_block {
                        context_lines.min(line_count_usize)
                    } else {
                        0
                    };
                    let trailing_row_start = leading.max(line_count_usize.saturating_sub(trailing));
                    let row_count =
                        leading.saturating_add(line_count_usize.saturating_sub(trailing_row_start));

                    if row_count > 0 {
                        entries.push(SplitLayoutEntry {
                            row_start: total_rows,
                            row_count,
                            segment_ix,
                            conflict_ix: None,
                            kind: SplitLayoutKind::Context {
                                line_index,
                                leading_row_count: leading,
                                trailing_row_start,
                                ours_start_line: ours_line,
                                theirs_start_line: theirs_line,
                            },
                        });
                        total_rows += row_count;
                    }

                    ours_line = ours_line.saturating_add(line_count);
                    theirs_line = theirs_line.saturating_add(line_count);
                }
                ConflictSegment::Block(block) => {
                    let ours_line_index = SparseLineIndex::for_text(&block.ours);
                    let theirs_line_index = SparseLineIndex::for_text(&block.theirs);
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    let row_count = ours_count.max(theirs_count);

                    entries.push(SplitLayoutEntry {
                        row_start: total_rows,
                        row_count,
                        segment_ix,
                        conflict_ix: Some(conflict_ix),
                        kind: SplitLayoutKind::Block {
                            ours_line_index,
                            theirs_line_index,
                            ours_start_line: ours_line,
                            theirs_start_line: theirs_line,
                        },
                    });
                    total_rows += row_count;

                    let ours_count_u32 = u32::try_from(ours_count).unwrap_or(u32::MAX);
                    let theirs_count_u32 = u32::try_from(theirs_count).unwrap_or(u32::MAX);
                    ours_line = ours_line.saturating_add(ours_count_u32);
                    theirs_line = theirs_line.saturating_add(theirs_count_u32);
                    conflict_ix += 1;
                }
            }
        }

        Self {
            entries,
            total_rows,
            page_size: CONFLICT_SPLIT_PAGE_SIZE,
            pages: Arc::new(std::sync::Mutex::new(ConflictSplitPageCache::default())),
        }
    }

    /// Total number of rows across all segments (before visibility filtering).
    pub fn total_rows(&self) -> usize {
        self.total_rows
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.total_rows).then(|| {
            let end = start.saturating_add(self.page_size).min(self.total_rows);
            (start, end)
        })
    }

    /// Find the layout entry that contains `row_ix`.
    fn entry_for_row(&self, row_ix: usize) -> Option<(usize, &SplitLayoutEntry)> {
        if row_ix >= self.total_rows {
            return None;
        }
        // Binary search: find the last entry where row_start <= row_ix.
        let pos = self
            .entries
            .partition_point(|e| e.row_start <= row_ix)
            .saturating_sub(1);
        let entry = self.entries.get(pos)?;
        if row_ix >= entry.row_start && row_ix < entry.row_start + entry.row_count {
            Some((pos, entry))
        } else {
            None
        }
    }

    fn build_page(
        &self,
        segments: &[ConflictSegment],
        page_ix: usize,
    ) -> Option<Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        let (start, end) = self.page_bounds(page_ix)?;
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        let mut row_ix = start;
        while row_ix < end {
            let (_, entry) = self.entry_for_row(row_ix)?;
            let entry_row_end = (entry.row_start + entry.row_count).min(end);
            let local_start = row_ix.saturating_sub(entry.row_start);
            let local_end = entry_row_end.saturating_sub(entry.row_start);
            let segment = segments.get(entry.segment_ix)?;

            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ours_start_line,
                        theirs_start_line,
                    },
                    ConflictSegment::Text(text),
                ) => {
                    let leading_end = local_end.min(*leading_row_count);
                    if local_start < leading_end {
                        let line_ranges =
                            line_index.line_ranges(text, local_start, leading_end - local_start);
                        for (offset, range) in line_ranges.into_iter().enumerate() {
                            let line_ix = local_start.saturating_add(offset);
                            let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
                            let content = text.get(range).unwrap_or("");
                            rows.push(gitcomet_core::file_diff::FileDiffRow {
                                kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                                old_line: Some(ours_start_line.saturating_add(line_offset)),
                                new_line: Some(theirs_start_line.saturating_add(line_offset)),
                                old: Some(content.into()),
                                new: Some(content.into()),
                                eof_newline: None,
                            });
                        }
                    }

                    let trailing_local_start = local_start.max(*leading_row_count);
                    if trailing_local_start < local_end {
                        let trailing_line_start = trailing_row_start.saturating_add(
                            trailing_local_start.saturating_sub(*leading_row_count),
                        );
                        let line_ranges = line_index.line_ranges(
                            text,
                            trailing_line_start,
                            local_end - trailing_local_start,
                        );
                        for (offset, range) in line_ranges.into_iter().enumerate() {
                            let line_ix = trailing_line_start.saturating_add(offset);
                            let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
                            let content = text.get(range).unwrap_or("");
                            rows.push(gitcomet_core::file_diff::FileDiffRow {
                                kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                                old_line: Some(ours_start_line.saturating_add(line_offset)),
                                new_line: Some(theirs_start_line.saturating_add(line_offset)),
                                old: Some(content.into()),
                                new: Some(content.into()),
                                eof_newline: None,
                            });
                        }
                    }
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ours_start_line,
                        theirs_start_line,
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let row_count = local_end.saturating_sub(local_start);
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    let ours_ranges = if local_start < ours_count {
                        ours_line_index.line_ranges(
                            &block.ours,
                            local_start,
                            row_count.min(ours_count - local_start),
                        )
                    } else {
                        Vec::new()
                    };
                    let theirs_ranges = if local_start < theirs_count {
                        theirs_line_index.line_ranges(
                            &block.theirs,
                            local_start,
                            row_count.min(theirs_count - local_start),
                        )
                    } else {
                        Vec::new()
                    };

                    for offset in 0..row_count {
                        let source_line_ix = local_start.saturating_add(offset);
                        let old_line = (source_line_ix < ours_count).then(|| {
                            ours_start_line
                                .saturating_add(u32::try_from(source_line_ix).unwrap_or(u32::MAX))
                        });
                        let new_line = (source_line_ix < theirs_count).then(|| {
                            theirs_start_line
                                .saturating_add(u32::try_from(source_line_ix).unwrap_or(u32::MAX))
                        });
                        let old_text = ours_ranges
                            .get(offset)
                            .and_then(|range: &Range<usize>| block.ours.get(range.clone()))
                            .map(Arc::<str>::from);
                        let new_text = theirs_ranges
                            .get(offset)
                            .and_then(|range: &Range<usize>| block.theirs.get(range.clone()))
                            .map(Arc::<str>::from);

                        let kind = match (old_text.as_deref(), new_text.as_deref()) {
                            (Some(old), Some(new)) if old == new => {
                                gitcomet_core::file_diff::FileDiffRowKind::Context
                            }
                            (Some(_), Some(_)) => gitcomet_core::file_diff::FileDiffRowKind::Modify,
                            (Some(_), None) => gitcomet_core::file_diff::FileDiffRowKind::Remove,
                            (None, Some(_)) => gitcomet_core::file_diff::FileDiffRowKind::Add,
                            (None, None) => continue,
                        };

                        rows.push(gitcomet_core::file_diff::FileDiffRow {
                            kind,
                            old_line,
                            new_line,
                            old: old_text,
                            new: new_text,
                            eof_newline: None,
                        });
                    }
                }
                _ => return None,
            }

            row_ix = entry_row_end;
        }
        Some(Arc::from(rows))
    }

    fn load_page(
        &self,
        segments: &[ConflictSegment],
        page_ix: usize,
    ) -> Option<Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(page_ix)
        {
            return Some(page);
        }

        let page = self.build_page(segments, page_ix)?;
        if let Ok(mut pages) = self.pages.lock() {
            return Some(pages.insert(page_ix, page));
        }
        Some(page)
    }

    /// Generate a single `FileDiffRow` on demand from segment text.
    pub fn row_at(
        &self,
        segments: &[ConflictSegment],
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRow> {
        if row_ix >= self.total_rows {
            return None;
        }
        let page_ix = row_ix / self.page_size;
        let row_offset = row_ix % self.page_size;
        let page = self.load_page(segments, page_ix)?;
        page.get(row_offset).cloned()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn clear_cached_pages(&self) {
        if let Ok(mut pages) = self.pages.lock() {
            pages.pages.clear();
            pages.lru.clear();
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn cached_page_count(&self) -> usize {
        self.pages
            .lock()
            .map(|pages| pages.pages.len())
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(in crate::view) fn cached_page_indices(&self) -> Vec<usize> {
        let mut pages = self
            .pages
            .lock()
            .map(|pages| pages.pages.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        pages.sort_unstable();
        pages
    }

    /// Approximate heap bytes used by the index metadata,
    /// excluding the bounded page cache.
    #[cfg(test)]
    pub fn metadata_byte_size(&self) -> usize {
        let entry_overhead = self.entries.len() * std::mem::size_of::<SplitLayoutEntry>();
        let entry_vecs: usize = self
            .entries
            .iter()
            .map(|e| match &e.kind {
                SplitLayoutKind::Context { line_index, .. } => {
                    line_index.checkpoints.len() * std::mem::size_of::<usize>()
                }
                SplitLayoutKind::Block {
                    ours_line_index,
                    theirs_line_index,
                    ..
                } => {
                    ours_line_index.checkpoints.len() * std::mem::size_of::<usize>()
                        + theirs_line_index.checkpoints.len() * std::mem::size_of::<usize>()
                }
            })
            .sum();
        entry_overhead + entry_vecs
    }

    /// Look up the conflict index for a given source row.
    #[cfg(test)]
    pub fn conflict_ix_for_row(&self, row_ix: usize) -> Option<usize> {
        let (_, entry) = self.entry_for_row(row_ix)?;
        entry.conflict_ix
    }

    /// Find the first source row index belonging to a conflict block.
    #[cfg(test)]
    pub fn first_row_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        self.entries
            .iter()
            .find(|e| e.conflict_ix == Some(conflict_ix))
            .map(|e| e.row_start)
    }

    /// Find all source row indices whose text matches a predicate.
    ///
    /// Searches old (ours) and new (theirs) text for each row without
    /// allocating `FileDiffRow` objects, making this much cheaper than
    /// iterating `row_at()` for every row in a giant file.
    #[cfg(test)]
    pub fn search_matching_rows(
        &self,
        segments: &[ConflictSegment],
        predicate: impl Fn(&str) -> bool,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        for entry in &self.entries {
            let Some(segment) = segments.get(entry.segment_ix) else {
                continue;
            };
            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ..
                    },
                    ConflictSegment::Text(text),
                ) => {
                    for offset in 0..entry.row_count {
                        let line_ix = if offset < *leading_row_count {
                            offset
                        } else {
                            trailing_row_start
                                .saturating_add(offset.saturating_sub(*leading_row_count))
                        };
                        let Some(line) = line_index.line_text(text, line_ix) else {
                            continue;
                        };
                        if predicate(line) {
                            out.push(entry.row_start + offset);
                        }
                    }
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ..
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    for offset in 0..entry.row_count {
                        let ours_line_ix = (offset < ours_count).then_some(offset);
                        let theirs_line_ix = (offset < theirs_count).then_some(offset);
                        let ours_match = ours_line_ix.is_some_and(|line_ix| {
                            ours_line_index
                                .line_text(&block.ours, line_ix)
                                .is_some_and(&predicate)
                        });
                        let theirs_match = theirs_line_ix.is_some_and(|line_ix| {
                            theirs_line_index
                                .line_text(&block.theirs, line_ix)
                                .is_some_and(&predicate)
                        });
                        if ours_match || theirs_match {
                            out.push(entry.row_start + offset);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn extend_context_line_matches(
        out: &mut Vec<usize>,
        entry: &SplitLayoutEntry,
        leading_row_count: usize,
        trailing_row_start: usize,
        matching_lines: &[usize],
    ) {
        for &line_ix in matching_lines {
            if line_ix < leading_row_count {
                out.push(entry.row_start + line_ix);
            } else if line_ix >= trailing_row_start {
                let trailing_offset = line_ix - trailing_row_start;
                let offset = leading_row_count + trailing_offset;
                if offset < entry.row_count {
                    out.push(entry.row_start + offset);
                }
            }
        }
    }

    fn extend_block_line_matches(
        out: &mut Vec<usize>,
        entry: &SplitLayoutEntry,
        ours_hits: &[usize],
        theirs_hits: &[usize],
    ) {
        let mut oi = 0;
        let mut ti = 0;
        while oi < ours_hits.len() || ti < theirs_hits.len() {
            let o_off = ours_hits.get(oi).copied().unwrap_or(usize::MAX);
            let t_off = theirs_hits.get(ti).copied().unwrap_or(usize::MAX);
            let offset = o_off.min(t_off);
            if offset >= entry.row_count {
                break;
            }
            out.push(entry.row_start + offset);
            if o_off == offset {
                oi += 1;
            }
            if t_off == offset {
                ti += 1;
            }
        }
    }

    /// Find all source row indices whose text contains `needle` (case-sensitive
    /// byte substring).
    ///
    /// Uses SIMD-accelerated `memmem::find_iter` to search each segment's full
    /// text in a single pass and maps match byte positions to line indices
    /// via the sparse checkpoint structure.  This is dramatically faster than
    /// `search_matching_rows` with a per-line predicate for large segments
    /// because it avoids per-line byte-by-byte checkpoint walks.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn search_text_matching_rows(
        &self,
        segments: &[ConflictSegment],
        needle: &[u8],
    ) -> Vec<usize> {
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for entry in &self.entries {
            let Some(segment) = segments.get(entry.segment_ix) else {
                continue;
            };
            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ..
                    },
                    ConflictSegment::Text(text),
                ) => {
                    let matching_lines =
                        line_index.lines_containing(text, needle, line_index.line_count());
                    Self::extend_context_line_matches(
                        &mut out,
                        entry,
                        *leading_row_count,
                        *trailing_row_start,
                        &matching_lines,
                    );
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ..
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    let ours_hits =
                        ours_line_index.lines_containing(&block.ours, needle, ours_count);
                    let theirs_hits =
                        theirs_line_index.lines_containing(&block.theirs, needle, theirs_count);
                    Self::extend_block_line_matches(&mut out, entry, &ours_hits, &theirs_hits);
                }
                _ => {}
            }
        }
        out
    }

    /// Find all source row indices whose text contains `needle` using ASCII
    /// case-insensitive matching.
    pub fn search_ascii_case_insensitive_matching_rows(
        &self,
        segments: &[ConflictSegment],
        needle: &[u8],
    ) -> Vec<usize> {
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for entry in &self.entries {
            let Some(segment) = segments.get(entry.segment_ix) else {
                continue;
            };
            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ..
                    },
                    ConflictSegment::Text(text),
                ) => {
                    let matching_lines = line_index.lines_containing_ascii_case_insensitive(
                        text,
                        needle,
                        line_index.line_count(),
                    );
                    Self::extend_context_line_matches(
                        &mut out,
                        entry,
                        *leading_row_count,
                        *trailing_row_start,
                        &matching_lines,
                    );
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ..
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let ours_hits = ours_line_index.lines_containing_ascii_case_insensitive(
                        &block.ours,
                        needle,
                        ours_line_index.line_count(),
                    );
                    let theirs_hits = theirs_line_index.lines_containing_ascii_case_insensitive(
                        &block.theirs,
                        needle,
                        theirs_line_index.line_count(),
                    );
                    Self::extend_block_line_matches(&mut out, entry, &ours_hits, &theirs_hits);
                }
                _ => {}
            }
        }
        out
    }

    /// Find the source-row indices that contain the widest visible text for the
    /// left (ours) and right (theirs) sides of the split view.
    ///
    /// This scans the indexed source text directly instead of materializing
    /// `FileDiffRow`s for every row, which keeps measurement selection cheap even
    /// for large streamed conflicts.
    pub fn widest_source_rows_by_text_len(
        &self,
        segments: &[ConflictSegment],
        hide_resolved: bool,
    ) -> [Option<usize>; 2] {
        let mut best_rows = [None, None];
        let mut best_lens = [0usize, 0usize];

        let mut update_best = |side_ix: usize, source_row_ix: usize, width: usize| {
            if width > best_lens[side_ix] {
                best_lens[side_ix] = width;
                best_rows[side_ix] = Some(source_row_ix);
            }
        };

        for entry in &self.entries {
            let Some(segment) = segments.get(entry.segment_ix) else {
                continue;
            };
            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ..
                    },
                    ConflictSegment::Text(text),
                ) => {
                    for (offset, range) in line_index
                        .line_ranges(text, 0, *leading_row_count)
                        .into_iter()
                        .enumerate()
                    {
                        let width = range.len();
                        let source_row_ix = entry.row_start + offset;
                        update_best(0, source_row_ix, width);
                        update_best(1, source_row_ix, width);
                    }

                    let trailing_row_count = entry.row_count.saturating_sub(*leading_row_count);
                    for (offset, range) in line_index
                        .line_ranges(text, *trailing_row_start, trailing_row_count)
                        .into_iter()
                        .enumerate()
                    {
                        let width = range.len();
                        let source_row_ix =
                            entry.row_start + leading_row_count.saturating_add(offset);
                        update_best(0, source_row_ix, width);
                        update_best(1, source_row_ix, width);
                    }
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ..
                    },
                    ConflictSegment::Block(block),
                ) => {
                    if hide_resolved && block.resolved {
                        continue;
                    }

                    if let Some((line_ix, width)) = ours_line_index.widest_line() {
                        update_best(0, entry.row_start + line_ix, width);
                    }
                    if let Some((line_ix, width)) = theirs_line_index.widest_line() {
                        update_best(1, entry.row_start + line_ix, width);
                    }
                }
                _ => {}
            }
        }

        best_rows
    }
}

// ---------------------------------------------------------------------------
// Two-way split visible projection (analogous to ThreeWayVisibleProjection)
// ---------------------------------------------------------------------------

/// A contiguous span of visible split rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwoWaySplitSpan {
    /// First visible index for this span.
    pub visible_start: usize,
    /// First source row index.
    pub source_row_start: usize,
    /// Number of rows in this span.
    pub len: usize,
    /// Conflict index if all rows in this span belong to one block.
    pub conflict_ix: Option<usize>,
}

/// Materialized split-view row with its source-row and conflict metadata.
#[derive(Clone, Debug)]
pub struct TwoWaySplitVisibleRow {
    pub source_row_ix: usize,
    pub row: gitcomet_core::file_diff::FileDiffRow,
    pub conflict_ix: Option<usize>,
}

/// Span-based visible projection for the two-way split view in giant mode.
#[derive(Clone, Debug, Default)]
pub struct TwoWaySplitProjection {
    spans: Vec<TwoWaySplitSpan>,
    visible_len: usize,
}

impl TwoWaySplitProjection {
    /// Build a projection from the split row index, filtering out resolved blocks.
    pub fn new(
        index: &ConflictSplitRowIndex,
        segments: &[ConflictSegment],
        hide_resolved: bool,
    ) -> Self {
        let resolved_blocks: Vec<bool> = segments
            .iter()
            .filter_map(|s| match s {
                ConflictSegment::Block(b) => Some(b.resolved),
                _ => None,
            })
            .collect();

        let mut spans = Vec::new();
        let mut visible_len = 0usize;

        for entry in &index.entries {
            if hide_resolved
                && let Some(ci) = entry.conflict_ix
                && resolved_blocks.get(ci).copied().unwrap_or(false)
            {
                continue;
            }
            spans.push(TwoWaySplitSpan {
                visible_start: visible_len,
                source_row_start: entry.row_start,
                len: entry.row_count,
                conflict_ix: entry.conflict_ix,
            });
            visible_len += entry.row_count;
        }

        Self { spans, visible_len }
    }

    /// Total number of visible rows.
    pub fn visible_len(&self) -> usize {
        self.visible_len
    }

    /// Map a visible index to a source row index and conflict index.
    pub fn get(&self, visible_ix: usize) -> Option<(usize, Option<usize>)> {
        if visible_ix >= self.visible_len {
            return None;
        }
        let pos = self
            .spans
            .partition_point(|s| s.visible_start <= visible_ix)
            .saturating_sub(1);
        let span = self.spans.get(pos)?;
        let offset = visible_ix.checked_sub(span.visible_start)?;
        if offset >= span.len {
            return None;
        }
        Some((span.source_row_start + offset, span.conflict_ix))
    }

    /// Find the first visible index for a given conflict.
    pub fn visible_index_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        self.spans
            .iter()
            .find(|s| s.conflict_ix == Some(conflict_ix))
            .map(|s| s.visible_start)
    }

    /// Map a source row index back to a visible index.
    pub fn source_to_visible(&self, source_row_ix: usize) -> Option<usize> {
        let pos = self
            .spans
            .partition_point(|s| s.source_row_start <= source_row_ix)
            .saturating_sub(1);
        let span = self.spans.get(pos)?;
        let offset = source_row_ix.checked_sub(span.source_row_start)?;
        if offset >= span.len {
            return None;
        }
        Some(span.visible_start + offset)
    }

    /// Approximate heap bytes used by the projection metadata (spans vec).
    #[cfg(all(test, feature = "benchmarks"))]
    pub fn metadata_byte_size(&self) -> usize {
        self.spans.len() * std::mem::size_of::<TwoWaySplitSpan>()
    }
}
