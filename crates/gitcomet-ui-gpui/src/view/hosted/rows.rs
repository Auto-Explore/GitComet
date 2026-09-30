//! A hosted diff pane's rows: file lines with their numbers on each side,
//! built from a session's file text (preferred) or its patch.

use gitcomet_core::domain::{Diff, DiffLineKind};
use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};
use gitcomet_extension_api::DiffLineSide;
use gpui::SharedString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneRowKind {
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaneRow {
    /// Side/line API anchors are meaningful only when the target names a file.
    /// Whole-target patches may repeat the same numbers in several files.
    file_scoped: bool,
    pub(crate) kind: PaneRowKind,
    pub(crate) old_line: Option<u32>,
    pub(crate) new_line: Option<u32>,
    pub(crate) text: SharedString,
}

impl PaneRow {
    /// The file line this row shows: its new side unless it only has an old
    /// one.
    pub(crate) fn anchor(&self) -> Option<(DiffLineSide, u32)> {
        match (self.line(DiffLineSide::New), self.line(DiffLineSide::Old)) {
            (Some(line), _) => Some((DiffLineSide::New, line)),
            (None, Some(line)) => Some((DiffLineSide::Old, line)),
            (None, None) => None,
        }
    }

    pub(crate) fn line(&self, side: DiffLineSide) -> Option<u32> {
        if !self.file_scoped {
            return None;
        }
        match side {
            DiffLineSide::Old => self.old_line,
            DiffLineSide::New => self.new_line,
        }
    }
}

/// Every line of both sides, a change's removals before its additions.
#[cfg(test)]
pub(crate) fn rows_from_file_text(old: &str, new: &str) -> Vec<PaneRow> {
    rows_from_file_rows(gitcomet_core::file_diff::side_by_side_rows(old, new))
}

pub(crate) fn rows_from_file_rows(source: impl IntoIterator<Item = FileDiffRow>) -> Vec<PaneRow> {
    let mut rows = Vec::new();
    for row in source {
        let old_text = row
            .old
            .as_ref()
            .map(|text| SharedString::from(text.to_string()));
        let new_text = row
            .new
            .as_ref()
            .map(|text| SharedString::from(text.to_string()));
        match row.kind {
            FileDiffRowKind::Context => rows.push(PaneRow {
                file_scoped: true,
                kind: PaneRowKind::Context,
                old_line: row.old_line,
                new_line: row.new_line,
                text: new_text.or(old_text).unwrap_or_default(),
            }),
            FileDiffRowKind::Remove => rows.push(PaneRow {
                file_scoped: true,
                kind: PaneRowKind::Removed,
                old_line: row.old_line,
                new_line: None,
                text: old_text.unwrap_or_default(),
            }),
            FileDiffRowKind::Add => rows.push(PaneRow {
                file_scoped: true,
                kind: PaneRowKind::Added,
                old_line: None,
                new_line: row.new_line,
                text: new_text.unwrap_or_default(),
            }),
            FileDiffRowKind::Modify => {
                rows.push(PaneRow {
                    file_scoped: true,
                    kind: PaneRowKind::Removed,
                    old_line: row.old_line,
                    new_line: None,
                    text: old_text.unwrap_or_default(),
                });
                rows.push(PaneRow {
                    file_scoped: true,
                    kind: PaneRowKind::Added,
                    old_line: None,
                    new_line: row.new_line,
                    text: new_text.unwrap_or_default(),
                });
            }
        }
    }
    rows
}

/// A patch's lines, numbered from its hunk headers.
pub(crate) fn rows_from_patch(diff: &Diff) -> Vec<PaneRow> {
    let (mut old, mut new) = (0u32, 0u32);
    let mut rows = Vec::with_capacity(diff.lines.len());
    for line in &diff.lines {
        let text: &str = line.text.as_ref();
        let (kind, old_line, new_line) = match line.kind {
            DiffLineKind::Header => (PaneRowKind::Header, None, None),
            DiffLineKind::Hunk => {
                if let Some((old_start, new_start)) = hunk_starts(text) {
                    old = old_start;
                    new = new_start;
                }
                (PaneRowKind::Hunk, None, None)
            }
            DiffLineKind::Context => {
                let numbers = (Some(old), Some(new));
                old += 1;
                new += 1;
                (PaneRowKind::Context, numbers.0, numbers.1)
            }
            DiffLineKind::Remove => {
                let number = Some(old);
                old += 1;
                (PaneRowKind::Removed, number, None)
            }
            DiffLineKind::Add => {
                let number = Some(new);
                new += 1;
                (PaneRowKind::Added, None, number)
            }
        };
        // Patch lines keep their `+`/`-`/` ` marker; the gutter shows it.
        let body = match kind {
            PaneRowKind::Context | PaneRowKind::Added | PaneRowKind::Removed => {
                text.get(1..).unwrap_or("")
            }
            _ => text,
        };
        rows.push(PaneRow {
            file_scoped: diff.target.file_path().is_some(),
            kind,
            old_line,
            new_line,
            text: SharedString::from(body.to_string()),
        });
    }
    rows
}

/// `@@ -a,b +c,d @@` → `(a, c)`.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |range: &str| range.split(',').next()?.parse().ok();
    Some((start(old)?, start(new)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{DiffArea, DiffTarget};

    #[test]
    fn file_text_rows_number_both_sides_and_split_modifications() {
        let rows = rows_from_file_text("a\nb\nc\n", "a\nB\nc\nd\n");
        let summary: Vec<_> = rows
            .iter()
            .map(|row| (row.kind, row.old_line, row.new_line, row.text.to_string()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (PaneRowKind::Context, Some(1), Some(1), "a".into()),
                (PaneRowKind::Removed, Some(2), None, "b".into()),
                (PaneRowKind::Added, None, Some(2), "B".into()),
                (PaneRowKind::Context, Some(3), Some(3), "c".into()),
                (PaneRowKind::Added, None, Some(4), "d".into()),
            ]
        );
        assert_eq!(rows[1].anchor(), Some((DiffLineSide::Old, 2)));
        assert_eq!(rows[2].anchor(), Some((DiffLineSide::New, 2)));
    }

    #[test]
    fn patch_rows_follow_hunk_headers() {
        let target = DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged);
        let diff = Diff::from_unified(
            target,
            "diff --git a/a.rs b/a.rs\n@@ -10,2 +10,3 @@\n ctx\n-old\n+new\n+more\n",
        );
        let numbered: Vec<_> = rows_from_patch(&diff)
            .into_iter()
            .filter(|row| {
                matches!(
                    row.kind,
                    PaneRowKind::Context | PaneRowKind::Added | PaneRowKind::Removed
                )
            })
            .map(|row| (row.old_line, row.new_line, row.text.to_string()))
            .collect();
        assert_eq!(
            numbered,
            vec![
                (Some(10), Some(10), "ctx".into()),
                (Some(11), None, "old".into()),
                (None, Some(11), "new".into()),
                (None, Some(12), "more".into()),
            ]
        );
    }
}
