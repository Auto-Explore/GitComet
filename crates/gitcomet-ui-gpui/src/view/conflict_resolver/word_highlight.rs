use super::TwoWayWordHighlightPair;

#[cfg(feature = "benchmarks")]
use std::num::NonZeroU32;
#[cfg(any(test, feature = "benchmarks"))]
use {
    super::{
        ConflictSegment, LARGE_CONFLICT_BLOCK_WORD_HIGHLIGHT_MAX_LINES, WordHighlights,
        block_max_line_count, indexed_line_text, text_line_count,
    },
    std::ops::Range,
};

#[cfg(any(test, feature = "benchmarks"))]
fn should_skip_large_block_word_highlights(block: &super::ConflictBlock) -> bool {
    block_max_line_count(block) > LARGE_CONFLICT_BLOCK_WORD_HIGHLIGHT_MAX_LINES
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn compute_three_way_word_highlights(
    base_text: &str,
    base_line_starts: &[usize],
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
    marker_segments: &[ConflictSegment],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let mut wh_base: WordHighlights = WordHighlights::default();
    let mut wh_ours: WordHighlights = WordHighlights::default();
    let mut wh_theirs: WordHighlights = WordHighlights::default();

    fn merge_line_ranges(
        highlights: &mut WordHighlights,
        line_ix: usize,
        ranges: Vec<Range<usize>>,
    ) {
        if ranges.is_empty() {
            return;
        }
        highlights
            .entry(line_ix)
            .and_modify(|existing| {
                *existing = merge_ranges(existing, &ranges);
            })
            .or_insert(ranges);
    }

    fn line_index(start: usize, line_no: Option<u32>) -> Option<usize> {
        let local = usize::try_from(line_no?).ok()?.checked_sub(1)?;
        start.checked_add(local)
    }

    fn full_line_range(text: &str, line_starts: &[usize], line_ix: usize) -> Vec<Range<usize>> {
        let Some(line) = indexed_line_text(text, line_starts, line_ix) else {
            return Vec::new();
        };
        if line.is_empty() {
            return Vec::new();
        }
        std::iter::once(0..line.len()).collect()
    }

    struct HighlightSide<'a> {
        global_start: usize,
        text: &'a str,
        line_starts: &'a [usize],
    }

    fn apply_aligned_word_highlights(
        old_text: &str,
        new_text: &str,
        old_side: HighlightSide<'_>,
        new_side: HighlightSide<'_>,
        old_highlights: &mut WordHighlights,
        new_highlights: &mut WordHighlights,
    ) {
        use gitcomet_core::file_diff::PlanRowView;

        gitcomet_core::file_diff::for_each_side_by_side_row(
            old_text,
            new_text,
            |view| match view {
                PlanRowView::Modify {
                    old_line,
                    new_line,
                    old_text: old,
                    new_text: new,
                } => {
                    let (old_ranges, new_ranges) =
                        crate::view::word_diff::capped_word_diff_ranges(old, new);

                    if let Some(ix) = line_index(old_side.global_start, Some(old_line)) {
                        merge_line_ranges(old_highlights, ix, old_ranges);
                    }
                    if let Some(ix) = line_index(new_side.global_start, Some(new_line)) {
                        merge_line_ranges(new_highlights, ix, new_ranges);
                    }
                }
                PlanRowView::Remove { old_line, .. } => {
                    if let Some(ix) = line_index(old_side.global_start, Some(old_line)) {
                        merge_line_ranges(
                            old_highlights,
                            ix,
                            full_line_range(old_side.text, old_side.line_starts, ix),
                        );
                    }
                }
                PlanRowView::Add { new_line, .. } => {
                    if let Some(ix) = line_index(new_side.global_start, Some(new_line)) {
                        merge_line_ranges(
                            new_highlights,
                            ix,
                            full_line_range(new_side.text, new_side.line_starts, ix),
                        );
                    }
                }
                PlanRowView::Context { .. } => {}
            },
        );
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    for seg in marker_segments {
        match seg {
            ConflictSegment::Text(text) => {
                let n = usize::try_from(text_line_count(text)).unwrap_or(0);
                base_offset = base_offset.saturating_add(n);
                ours_offset = ours_offset.saturating_add(n);
                theirs_offset = theirs_offset.saturating_add(n);
            }
            ConflictSegment::Block(block) => {
                let base_count =
                    usize::try_from(text_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let ours_count = usize::try_from(text_line_count(&block.ours)).unwrap_or(0);
                let theirs_count = usize::try_from(text_line_count(&block.theirs)).unwrap_or(0);
                if should_skip_large_block_word_highlights(block) {
                    base_offset = base_offset.saturating_add(base_count);
                    ours_offset = ours_offset.saturating_add(ours_count);
                    theirs_offset = theirs_offset.saturating_add(theirs_count);
                    continue;
                }

                if let Some(base) = block.base.as_deref() {
                    apply_aligned_word_highlights(
                        base,
                        &block.ours,
                        HighlightSide {
                            global_start: base_offset,
                            text: base_text,
                            line_starts: base_line_starts,
                        },
                        HighlightSide {
                            global_start: ours_offset,
                            text: ours_text,
                            line_starts: ours_line_starts,
                        },
                        &mut wh_base,
                        &mut wh_ours,
                    );
                    apply_aligned_word_highlights(
                        base,
                        &block.theirs,
                        HighlightSide {
                            global_start: base_offset,
                            text: base_text,
                            line_starts: base_line_starts,
                        },
                        HighlightSide {
                            global_start: theirs_offset,
                            text: theirs_text,
                            line_starts: theirs_line_starts,
                        },
                        &mut wh_base,
                        &mut wh_theirs,
                    );
                }
                // Local/Remote highlighting must align by diff rows, not absolute same-row index.
                apply_aligned_word_highlights(
                    &block.ours,
                    &block.theirs,
                    HighlightSide {
                        global_start: ours_offset,
                        text: ours_text,
                        line_starts: ours_line_starts,
                    },
                    HighlightSide {
                        global_start: theirs_offset,
                        text: theirs_text,
                        line_starts: theirs_line_starts,
                    },
                    &mut wh_ours,
                    &mut wh_theirs,
                );
                base_offset = base_offset.saturating_add(base_count);
                ours_offset = ours_offset.saturating_add(ours_count);
                theirs_offset = theirs_offset.saturating_add(theirs_count);
            }
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

#[cfg(any(test, feature = "benchmarks"))]
fn merge_ranges(a: &[Range<usize>], b: &[Range<usize>]) -> Vec<Range<usize>> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let mut combined: Vec<Range<usize>> = Vec::with_capacity(a.len() + b.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    combined.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<Range<usize>> = Vec::with_capacity(combined.len());
    for r in combined {
        if let Some(last) = out.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        out.push(r);
    }
    out
}

/// Per-line pair of (old, new) word-highlight ranges for two-way diff.
#[cfg(feature = "benchmarks")]
#[derive(Clone, Debug, Default)]
pub struct TwoWayWordHighlights {
    row_to_entry: Box<[Option<NonZeroU32>]>,
    entries: Box<[TwoWayWordHighlightPair]>,
}

#[cfg(feature = "benchmarks")]
impl TwoWayWordHighlights {
    pub fn len(&self) -> usize {
        self.row_to_entry.len()
    }

    pub fn get(&self, row_ix: usize) -> Option<&TwoWayWordHighlightPair> {
        let entry_ix = self
            .row_to_entry
            .get(row_ix)?
            .map(|ix| ix.get() as usize - 1)?;
        self.entries.get(entry_ix)
    }

    pub fn iter(&self) -> impl Iterator<Item = Option<&TwoWayWordHighlightPair>> + '_ {
        self.row_to_entry
            .iter()
            .copied()
            .map(|entry_ix| entry_ix.and_then(|ix| self.entries.get(ix.get() as usize - 1)))
    }

    #[cfg(test)]
    pub(super) fn highlighted_rows(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(feature = "benchmarks")]
pub fn compute_two_way_word_highlights(
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
) -> TwoWayWordHighlights {
    let modify_rows = diff_rows
        .iter()
        .filter(|row| row.kind == gitcomet_core::file_diff::FileDiffRowKind::Modify)
        .count();
    let mut row_to_entry = vec![None; diff_rows.len()];
    let mut entries = Vec::with_capacity(modify_rows);

    for (row_ix, row) in diff_rows.iter().enumerate() {
        if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
            continue;
        }
        let old = row.old.as_deref().unwrap_or("");
        let new = row.new.as_deref().unwrap_or("");
        let (old_ranges, new_ranges) =
            crate::view::word_diff::compact_capped_word_diff_ranges(old, new);
        if old_ranges.is_empty() && new_ranges.is_empty() {
            continue;
        }

        entries.push((old_ranges, new_ranges));
        let entry_ix = NonZeroU32::new(
            u32::try_from(entries.len()).expect("two-way word highlights should fit in u32"),
        )
        .expect("stored highlight entry index should be non-zero");
        row_to_entry[row_ix] = Some(entry_ix);
    }

    TwoWayWordHighlights {
        row_to_entry: row_to_entry.into_boxed_slice(),
        entries: entries.into_boxed_slice(),
    }
}

/// Compute word-level highlights for a single `FileDiffRow` on the fly.
///
/// Used in giant/streamed mode where word highlights are not pre-computed for
/// all rows. Only produces highlights for `Modify` rows (both sides present,
/// text differs).
pub fn compute_word_highlights_for_row(
    row: &gitcomet_core::file_diff::FileDiffRow,
) -> Option<TwoWayWordHighlightPair> {
    if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
        return None;
    }
    let old = row.old.as_deref().unwrap_or("");
    let new = row.new.as_deref().unwrap_or("");
    let (old_ranges, new_ranges) =
        crate::view::word_diff::compact_capped_word_diff_ranges(old, new);
    if old_ranges.is_empty() && new_ranges.is_empty() {
        None
    } else {
        Some((old_ranges, new_ranges))
    }
}

#[cfg(all(test, feature = "benchmarks"))]
mod tests {
    use super::*;
    use gitcomet_core::file_diff::{FileDiffLineText, FileDiffRow, FileDiffRowKind};
    use std::sync::Arc;

    fn modify_row(old: &'static str, new: &'static str) -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffRowKind::Modify,
            old_line: Some(1),
            new_line: Some(1),
            old: Some(FileDiffLineText::shared(Arc::<str>::from(old))),
            new: Some(FileDiffLineText::shared(Arc::<str>::from(new))),
            eof_newline: None,
        }
    }

    #[test]
    fn two_way_word_highlights_store_only_rows_with_ranges() {
        let rows = vec![
            FileDiffRow {
                kind: FileDiffRowKind::Context,
                old_line: Some(1),
                new_line: Some(1),
                old: Some(FileDiffLineText::shared(Arc::<str>::from("same"))),
                new: Some(FileDiffLineText::shared(Arc::<str>::from("same"))),
                eof_newline: None,
            },
            modify_row(
                "let value = compute_local(1);",
                "let value = compute_remote(1);",
            ),
            modify_row(
                "let shared_alpha = compute_local(2);",
                "let shared_alpha_tail = compute_remote(2);",
            ),
        ];

        let highlights = compute_two_way_word_highlights(&rows);

        assert_eq!(highlights.len(), rows.len());
        assert_eq!(highlights.highlighted_rows(), 2);
        assert!(highlights.get(0).is_none());
        assert!(highlights.get(1).is_some());
        assert!(highlights.get(2).is_some());
        assert_eq!(highlights.iter().filter(|entry| entry.is_some()).count(), 2);
    }
}
