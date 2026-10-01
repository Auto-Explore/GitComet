//! A hosted pane's display rows: the file's rows with insets after their
//! lines. Everything that works on file lines (search, selection, copy,
//! reveal, markers) goes through it, so inset rows are never taken for the
//! file's.

use super::rows::PaneRow;
use gitcomet_extension_api::{DiffAnnotations, DiffInset, DiffLineRange, DiffLineSide};
use gpui::Hsla;
use rustc_hash::FxHashMap;

/// Scrollbar markers per pane are merged into this many positions.
const MARKER_BUCKETS: f32 = 1000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplayRow {
    /// Row `.0` of the file's rows.
    Document(usize),
    /// Line `line` of inset `inset`.
    Inset { inset: usize, line: usize },
}

#[derive(Debug, Default)]
pub(crate) struct PaneProjection {
    pub(crate) display: Vec<DisplayRow>,
    /// The display row of each file line, by side.
    lines: FxHashMap<(DiffLineSide, u32), usize>,
}

impl PaneProjection {
    /// Insets on lines the rows do not show (outside a patch's hunks) are
    /// left out.
    pub(crate) fn build(rows: &[PaneRow], insets: &[DiffInset]) -> Self {
        let mut pending: FxHashMap<(DiffLineSide, u32), Vec<usize>> = FxHashMap::default();
        for (ix, inset) in insets.iter().enumerate() {
            pending
                .entry((inset.side, inset.line))
                .or_default()
                .push(ix);
        }
        let mut display = Vec::with_capacity(rows.len());
        let mut lines = FxHashMap::default();
        for (row_ix, row) in rows.iter().enumerate() {
            let keys = [
                row.line(DiffLineSide::Old)
                    .map(|line| (DiffLineSide::Old, line)),
                row.line(DiffLineSide::New)
                    .map(|line| (DiffLineSide::New, line)),
            ];
            let display_ix = display.len();
            display.push(DisplayRow::Document(row_ix));
            for key in keys.into_iter().flatten() {
                lines.entry(key).or_insert(display_ix);
            }
            if pending.is_empty() {
                continue;
            }
            for key in keys.into_iter().flatten() {
                for inset in pending.remove(&key).unwrap_or_default() {
                    display.extend(
                        (0..insets[inset].lines.len())
                            .map(|line| DisplayRow::Inset { inset, line }),
                    );
                }
            }
        }
        Self { display, lines }
    }

    pub(crate) fn len(&self) -> usize {
        self.display.len()
    }

    pub(crate) fn display_ix(&self, side: DiffLineSide, line: u32) -> Option<usize> {
        self.lines.get(&(side, line)).copied()
    }

    /// The file row shown at display row `ix`, if it is one.
    pub(crate) fn document_row(&self, ix: usize) -> Option<usize> {
        match self.display.get(ix)? {
            DisplayRow::Document(row) => Some(*row),
            DisplayRow::Inset { .. } => None,
        }
    }

    /// Scrollbar markers as (fraction of the pane's height, colour), one
    /// per annotated line the pane shows, merged by position.
    pub(crate) fn markers(&self, annotations: &DiffAnnotations) -> Vec<(f32, Hsla)> {
        if self.display.is_empty() {
            return Vec::new();
        }
        let total = self.display.len() as f32;
        let mut buckets: Vec<(u32, Hsla)> = annotations
            .iter()
            .filter_map(|(side, line, annotation)| {
                let ix = self.display_ix(side, line)?;
                Some((
                    (ix as f32 / total * MARKER_BUCKETS) as u32,
                    annotation.color,
                ))
            })
            .collect();
        buckets.sort_by_key(|(bucket, _)| *bucket);
        buckets.dedup_by_key(|(bucket, _)| *bucket);
        buckets
            .into_iter()
            .map(|(bucket, color)| (bucket as f32 / MARKER_BUCKETS, color))
            .collect()
    }
}

/// The selected lines' text, one per line; only file rows count.
pub(crate) fn selected_text(rows: &[PaneRow], range: DiffLineRange) -> String {
    let mut text = String::new();
    for row in rows {
        if row
            .line(range.side)
            .is_some_and(|line| range.contains(range.side, line))
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&row.text);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::super::rows::rows_from_file_text;
    use super::*;
    use gitcomet_extension_api::DiffAnnotation;

    fn inset(side: DiffLineSide, line: u32, lines: &[&'static str]) -> DiffInset {
        DiffInset::new(side, line, lines.iter().map(|line| (*line).into()))
    }

    #[test]
    fn insets_follow_their_line_and_are_not_file_lines() {
        // old: a b c   new: a B c
        let rows = rows_from_file_text("a\nb\nc\n", "a\nB\nc\n");
        let insets = [
            inset(DiffLineSide::Old, 2, &["on the removal"]),
            inset(DiffLineSide::New, 3, &["first", "second"]),
            inset(DiffLineSide::New, 40, &["nowhere"]),
        ];
        let projection = PaneProjection::build(&rows, &insets);
        assert_eq!(
            projection.display,
            vec![
                DisplayRow::Document(0),
                DisplayRow::Document(1),
                DisplayRow::Inset { inset: 0, line: 0 },
                DisplayRow::Document(2),
                DisplayRow::Document(3),
                DisplayRow::Inset { inset: 1, line: 0 },
                DisplayRow::Inset { inset: 1, line: 1 },
            ]
        );
        assert_eq!(projection.display_ix(DiffLineSide::New, 3), Some(4));
        assert_eq!(projection.display_ix(DiffLineSide::Old, 3), Some(4));
        assert_eq!(projection.document_row(2), None);
        assert_eq!(projection.document_row(3), Some(2));
    }

    #[test]
    fn markers_merge_by_position_and_skip_unshown_lines() {
        let old: String = (1..=10).map(|n| format!("{n}\n")).collect();
        let rows = rows_from_file_text(&old, &old);
        let projection = PaneProjection::build(&rows, &[]);
        let red = gpui::red();
        let annotations = DiffAnnotations::new()
            .with(DiffLineSide::New, 1, DiffAnnotation::new(red))
            .with(DiffLineSide::Old, 1, DiffAnnotation::new(red))
            .with(DiffLineSide::New, 6, DiffAnnotation::new(red))
            .with(DiffLineSide::New, 99, DiffAnnotation::new(red));
        assert_eq!(
            projection.markers(&annotations),
            vec![(0.0, red), (0.5, red)]
        );
    }

    #[test]
    fn copied_text_is_the_selected_side_only() {
        let rows = rows_from_file_text("a\nb\nc\n", "a\nB\nc\n");
        let range = |side, start, end| DiffLineRange { side, start, end };
        assert_eq!(selected_text(&rows, range(DiffLineSide::New, 1, 2)), "a\nB");
        assert_eq!(selected_text(&rows, range(DiffLineSide::Old, 2, 3)), "b\nc");
    }

    #[test]
    fn whole_commit_patches_do_not_alias_file_line_anchors() {
        use super::super::rows::rows_from_patch;
        use gitcomet_core::domain::{CommitId, Diff, DiffTarget};
        let diff = Diff::from_unified(
            DiffTarget::commit_range(CommitId("base".into()), Some(CommitId("head".into())), None),
            "diff --git a/a b/a\n@@ -1 +1 @@\n-old a\n+new a\ndiff --git a/b b/b\n@@ -1 +1 @@\n-old b\n+new b\n",
        );
        let rows = rows_from_patch(&diff);
        assert_eq!(rows.iter().filter(|row| row.new_line == Some(1)).count(), 2);
        let projection =
            PaneProjection::build(&rows, &[inset(DiffLineSide::New, 1, &["ambiguous"])]);
        assert_eq!(
            projection.len(),
            rows.len(),
            "no inset on an ambiguous line"
        );
        assert_eq!(projection.display_ix(DiffLineSide::New, 1), None);
        assert!(rows.iter().all(|row| row.anchor().is_none()));
        assert!(
            selected_text(
                &rows,
                DiffLineRange {
                    side: DiffLineSide::New,
                    start: 1,
                    end: 1
                }
            )
            .is_empty()
        );
        assert!(
            projection
                .markers(&DiffAnnotations::new().with(
                    DiffLineSide::New,
                    1,
                    DiffAnnotation::new(gpui::red())
                ))
                .is_empty()
        );
    }
}
