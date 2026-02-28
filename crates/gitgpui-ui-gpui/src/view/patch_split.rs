use super::*;

pub(super) fn build_patch_split_rows(diff: &[AnnotatedDiffLine]) -> Vec<PatchSplitRow> {
    use gitgpui_core::domain::DiffLineKind as DK;
    use gitgpui_core::file_diff::FileDiffRowKind as K;

    let mut out: Vec<PatchSplitRow> = Vec::with_capacity(diff.len());
    let mut ix = 0usize;

    let mut pending_removes: Vec<usize> = Vec::new();
    let mut pending_adds: Vec<usize> = Vec::new();

    fn flush_pending(
        out: &mut Vec<PatchSplitRow>,
        diff: &[AnnotatedDiffLine],
        pending_removes: &mut Vec<usize>,
        pending_adds: &mut Vec<usize>,
    ) {
        let pairs = pending_removes.len().max(pending_adds.len());
        for i in 0..pairs {
            let left_ix = pending_removes.get(i).copied();
            let right_ix = pending_adds.get(i).copied();
            let left = left_ix.and_then(|ix| diff.get(ix));
            let right = right_ix.and_then(|ix| diff.get(ix));

            let kind = match (left_ix.is_some(), right_ix.is_some()) {
                (true, true) => gitgpui_core::file_diff::FileDiffRowKind::Modify,
                (true, false) => gitgpui_core::file_diff::FileDiffRowKind::Remove,
                (false, true) => gitgpui_core::file_diff::FileDiffRowKind::Add,
                (false, false) => gitgpui_core::file_diff::FileDiffRowKind::Context,
            };
            let row = FileDiffRow {
                kind,
                old_line: left.and_then(|l| l.old_line),
                new_line: right.and_then(|l| l.new_line),
                old: left.map(|l| diff_content_text(l).to_string()),
                new: right.map(|l| diff_content_text(l).to_string()),
                eof_newline: None,
            };
            out.push(PatchSplitRow::Aligned {
                row,
                old_src_ix: left_ix,
                new_src_ix: right_ix,
            });
        }
        pending_removes.clear();
        pending_adds.clear();
    }

    while ix < diff.len() {
        let line = &diff[ix];
        let is_file_header =
            matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");

        if is_file_header {
            flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
            out.push(PatchSplitRow::Raw {
                src_ix: ix,
                click_kind: DiffClickKind::FileHeader,
            });
            ix += 1;
            continue;
        }

        if matches!(line.kind, DK::Hunk) {
            flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
            out.push(PatchSplitRow::Raw {
                src_ix: ix,
                click_kind: DiffClickKind::HunkHeader,
            });
            ix += 1;

            while ix < diff.len() {
                let line = &diff[ix];
                let is_next_file_header =
                    matches!(line.kind, DK::Header) && line.text.starts_with("diff --git ");
                if is_next_file_header || matches!(line.kind, DK::Hunk) {
                    break;
                }

                match line.kind {
                    DK::Context => {
                        flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
                        let text = diff_content_text(line).to_string();
                        out.push(PatchSplitRow::Aligned {
                            row: FileDiffRow {
                                kind: K::Context,
                                old_line: line.old_line,
                                new_line: line.new_line,
                                old: Some(text.clone()),
                                new: Some(text),
                                eof_newline: None,
                            },
                            old_src_ix: Some(ix),
                            new_src_ix: Some(ix),
                        });
                    }
                    DK::Remove => pending_removes.push(ix),
                    DK::Add => pending_adds.push(ix),
                    DK::Header | DK::Hunk => {
                        flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
                        out.push(PatchSplitRow::Raw {
                            src_ix: ix,
                            click_kind: DiffClickKind::Line,
                        });
                    }
                }
                ix += 1;
            }

            flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
            continue;
        }

        // Headers outside hunks, e.g. `index`, `---`, `+++`, etc.
        out.push(PatchSplitRow::Raw {
            src_ix: ix,
            click_kind: DiffClickKind::Line,
        });
        ix += 1;
    }

    flush_pending(&mut out, diff, &mut pending_removes, &mut pending_adds);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitgpui_core::domain::DiffLineKind as DK;
    use gitgpui_core::file_diff::FileDiffRowKind as RK;

    fn line(
        kind: DK,
        text: &str,
        old_line: Option<u32>,
        new_line: Option<u32>,
    ) -> AnnotatedDiffLine {
        AnnotatedDiffLine {
            kind,
            text: text.into(),
            old_line,
            new_line,
        }
    }

    #[test]
    fn pairs_remove_and_add_into_modify_row() {
        let diff = vec![
            line(DK::Header, "diff --git a/a.txt b/a.txt", None, None),
            line(DK::Hunk, "@@ -1 +1 @@", None, None),
            line(DK::Remove, "-old", Some(1), None),
            line(DK::Add, "+new", None, Some(1)),
        ];

        let rows = build_patch_split_rows(&diff);
        assert_eq!(rows.len(), 3);
        assert!(matches!(
            rows[0],
            PatchSplitRow::Raw {
                src_ix: 0,
                click_kind: DiffClickKind::FileHeader
            }
        ));
        assert!(matches!(
            rows[1],
            PatchSplitRow::Raw {
                src_ix: 1,
                click_kind: DiffClickKind::HunkHeader
            }
        ));
        match &rows[2] {
            PatchSplitRow::Aligned {
                row,
                old_src_ix,
                new_src_ix,
            } => {
                assert_eq!(row.kind, RK::Modify);
                assert_eq!(row.old.as_deref(), Some("old"));
                assert_eq!(row.new.as_deref(), Some("new"));
                assert_eq!(*old_src_ix, Some(2));
                assert_eq!(*new_src_ix, Some(3));
            }
            _ => panic!("expected aligned row"),
        }
    }

    #[test]
    fn pads_unbalanced_edits() {
        let diff = vec![
            line(DK::Header, "diff --git a/a.txt b/a.txt", None, None),
            line(DK::Hunk, "@@ -1,2 +1,1 @@", None, None),
            line(DK::Remove, "-old1", Some(1), None),
            line(DK::Remove, "-old2", Some(2), None),
            line(DK::Add, "+new1", None, Some(1)),
        ];

        let rows = build_patch_split_rows(&diff);
        assert_eq!(rows.len(), 4);
        match &rows[2] {
            PatchSplitRow::Aligned { row, .. } => {
                assert_eq!(row.kind, RK::Modify);
                assert_eq!(row.old.as_deref(), Some("old1"));
                assert_eq!(row.new.as_deref(), Some("new1"));
            }
            _ => panic!("expected aligned row"),
        }
        match &rows[3] {
            PatchSplitRow::Aligned { row, .. } => {
                assert_eq!(row.kind, RK::Remove);
                assert_eq!(row.old.as_deref(), Some("old2"));
                assert!(row.new.is_none());
            }
            _ => panic!("expected aligned row"),
        }
    }
}
