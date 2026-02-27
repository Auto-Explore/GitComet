#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictChoice {
    #[allow(dead_code)]
    Base,
    Ours,
    Theirs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictDiffMode {
    Split,
    Inline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictResolverViewMode {
    ThreeWay,
    TwoWayDiff,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum ConflictPickSide {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictBlock {
    pub base: Option<String>,
    pub ours: String,
    pub theirs: String,
    pub choice: ConflictChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictSegment {
    Text(String),
    Block(ConflictBlock),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictInlineRow {
    pub side: ConflictPickSide,
    pub kind: gitgpui_core::domain::DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

pub fn parse_conflict_markers(text: &str) -> Vec<ConflictSegment> {
    let mut segments: Vec<ConflictSegment> = Vec::new();
    let mut buf = String::new();

    let mut it = text.split_inclusive('\n').peekable();
    while let Some(line) = it.next() {
        if !line.starts_with("<<<<<<<") {
            buf.push_str(line);
            continue;
        }

        // Flush prior text.
        if !buf.is_empty() {
            segments.push(ConflictSegment::Text(std::mem::take(&mut buf)));
        }

        let start_marker = line;

        let mut base_marker_line: Option<&str> = None;
        let mut base: Option<String> = None;
        let mut ours = String::new();
        let mut found_sep = false;

        while let Some(l) = it.next() {
            if l.starts_with("=======") {
                found_sep = true;
                break;
            }
            if l.starts_with("|||||||") {
                base_marker_line = Some(l);
                let mut base_buf = String::new();
                for l in it.by_ref() {
                    if l.starts_with("=======") {
                        found_sep = true;
                        break;
                    }
                    base_buf.push_str(l);
                }
                base = Some(base_buf);
                break;
            }
            ours.push_str(l);
        }

        if !found_sep {
            // Malformed marker; preserve as plain text.
            buf.push_str(start_marker);
            buf.push_str(&ours);
            if let Some(base_marker_line) = base_marker_line {
                buf.push_str(base_marker_line);
            }
            if let Some(base) = base.as_deref() {
                buf.push_str(base);
            }
            break;
        }

        let mut theirs = String::new();
        let mut found_end = false;
        for l in it.by_ref() {
            if l.starts_with(">>>>>>>") {
                found_end = true;
                break;
            }
            theirs.push_str(l);
        }

        if !found_end {
            // Malformed marker; preserve as plain text.
            buf.push_str(start_marker);
            buf.push_str(&ours);
            buf.push_str("=======\n");
            buf.push_str(&theirs);
            break;
        }

        segments.push(ConflictSegment::Block(ConflictBlock {
            base,
            ours,
            theirs,
            choice: ConflictChoice::Ours,
        }));
    }

    if !buf.is_empty() {
        segments.push(ConflictSegment::Text(buf));
    }

    segments
}

pub fn conflict_count(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Block(_)))
        .count()
}

pub fn generate_resolved_text(segments: &[ConflictSegment]) -> String {
    let approx_len: usize = segments
        .iter()
        .map(|seg| match seg {
            ConflictSegment::Text(t) => t.len(),
            ConflictSegment::Block(block) => match block.choice {
                ConflictChoice::Base => block.base.as_ref().map_or(0, |b| b.len()),
                ConflictChoice::Ours => block.ours.len(),
                ConflictChoice::Theirs => block.theirs.len(),
            },
        })
        .sum();
    let mut out = String::with_capacity(approx_len);
    for seg in segments {
        match seg {
            ConflictSegment::Text(t) => out.push_str(t),
            ConflictSegment::Block(block) => match block.choice {
                ConflictChoice::Base => {
                    if let Some(base) = block.base.as_deref() {
                        out.push_str(base)
                    }
                }
                ConflictChoice::Ours => out.push_str(&block.ours),
                ConflictChoice::Theirs => out.push_str(&block.theirs),
            },
        }
    }
    out
}

pub fn build_inline_rows(rows: &[gitgpui_core::file_diff::FileDiffRow]) -> Vec<ConflictInlineRow> {
    use gitgpui_core::domain::DiffLineKind as K;
    use gitgpui_core::file_diff::FileDiffRowKind as RK;

    let extra = rows.iter().filter(|r| matches!(r.kind, RK::Modify)).count();
    let mut out: Vec<ConflictInlineRow> = Vec::with_capacity(rows.len() + extra);
    for row in rows {
        match row.kind {
            RK::Context => out.push(ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: K::Context,
                old_line: row.old_line,
                new_line: row.new_line,
                content: row.old.as_deref().unwrap_or("").to_string(),
            }),
            RK::Add => out.push(ConflictInlineRow {
                side: ConflictPickSide::Theirs,
                kind: K::Add,
                old_line: None,
                new_line: row.new_line,
                content: row.new.as_deref().unwrap_or("").to_string(),
            }),
            RK::Remove => out.push(ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: K::Remove,
                old_line: row.old_line,
                new_line: None,
                content: row.old.as_deref().unwrap_or("").to_string(),
            }),
            RK::Modify => {
                out.push(ConflictInlineRow {
                    side: ConflictPickSide::Ours,
                    kind: K::Remove,
                    old_line: row.old_line,
                    new_line: None,
                    content: row.old.as_deref().unwrap_or("").to_string(),
                });
                out.push(ConflictInlineRow {
                    side: ConflictPickSide::Theirs,
                    kind: K::Add,
                    old_line: None,
                    new_line: row.new_line,
                    content: row.new.as_deref().unwrap_or("").to_string(),
                });
            }
        }
    }
    out
}

pub fn collect_split_selection(
    rows: &[gitgpui_core::file_diff::FileDiffRow],
    selected: &std::collections::BTreeSet<(usize, ConflictPickSide)>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(selected.len());
    for &(row_ix, side) in selected {
        let Some(row) = rows.get(row_ix) else {
            continue;
        };
        match side {
            ConflictPickSide::Ours => {
                if let Some(t) = row.old.as_deref() {
                    out.push(t.to_string());
                }
            }
            ConflictPickSide::Theirs => {
                if let Some(t) = row.new.as_deref() {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

pub fn collect_inline_selection(
    rows: &[ConflictInlineRow],
    selected: &std::collections::BTreeSet<usize>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(selected.len());
    for &ix in selected {
        if let Some(row) = rows.get(ix) {
            out.push(row.content.clone());
        }
    }
    out
}

pub fn compute_three_way_word_highlights(
    base_lines: &[gpui::SharedString],
    ours_lines: &[gpui::SharedString],
    theirs_lines: &[gpui::SharedString],
    conflict_ranges: &[std::ops::Range<usize>],
) -> (
    Vec<Option<Vec<std::ops::Range<usize>>>>,
    Vec<Option<Vec<std::ops::Range<usize>>>>,
    Vec<Option<Vec<std::ops::Range<usize>>>>,
) {
    let len = base_lines.len().max(ours_lines.len()).max(theirs_lines.len());
    let mut wh_base: Vec<Option<Vec<std::ops::Range<usize>>>> = vec![None; len];
    let mut wh_ours: Vec<Option<Vec<std::ops::Range<usize>>>> = vec![None; len];
    let mut wh_theirs: Vec<Option<Vec<std::ops::Range<usize>>>> = vec![None; len];

    for range in conflict_ranges {
        for i in range.clone() {
            if i >= len {
                break;
            }
            let base = base_lines.get(i).map(|s| s.as_ref()).unwrap_or("");
            let ours = ours_lines.get(i).map(|s| s.as_ref()).unwrap_or("");
            let theirs = theirs_lines.get(i).map(|s| s.as_ref()).unwrap_or("");

            let (base_vs_ours_base, ours_ranges) =
                super::word_diff::capped_word_diff_ranges(base, ours);
            let (base_vs_theirs_base, theirs_ranges) =
                super::word_diff::capped_word_diff_ranges(base, theirs);

            // Merge base ranges from both comparisons (union).
            let merged_base = merge_ranges(&base_vs_ours_base, &base_vs_theirs_base);

            if !merged_base.is_empty() {
                wh_base[i] = Some(merged_base);
            }
            if !ours_ranges.is_empty() {
                wh_ours[i] = Some(ours_ranges);
            }
            if !theirs_ranges.is_empty() {
                wh_theirs[i] = Some(theirs_ranges);
            }
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

fn merge_ranges(
    a: &[std::ops::Range<usize>],
    b: &[std::ops::Range<usize>],
) -> Vec<std::ops::Range<usize>> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let mut combined: Vec<std::ops::Range<usize>> = Vec::with_capacity(a.len() + b.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    combined.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(combined.len());
    for r in combined {
        if let Some(last) = out.last_mut() {
            if r.start <= last.end {
                last.end = last.end.max(r.end);
                continue;
            }
        }
        out.push(r);
    }
    out
}

pub fn compute_two_way_word_highlights(
    diff_rows: &[gitgpui_core::file_diff::FileDiffRow],
) -> Vec<Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)>> {
    diff_rows
        .iter()
        .map(|row| {
            if row.kind != gitgpui_core::file_diff::FileDiffRowKind::Modify {
                return None;
            }
            let old = row.old.as_deref().unwrap_or("");
            let new = row.new.as_deref().unwrap_or("");
            let (old_ranges, new_ranges) = super::word_diff::capped_word_diff_ranges(old, new);
            if old_ranges.is_empty() && new_ranges.is_empty() {
                None
            } else {
                Some((old_ranges, new_ranges))
            }
        })
        .collect()
}

pub fn append_lines_to_output(output: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return output.to_string();
    }

    let needs_leading_nl = !output.is_empty() && !output.ends_with('\n');
    let extra_len: usize =
        lines.iter().map(|l| l.len()).sum::<usize>() + lines.len() + usize::from(needs_leading_nl);
    let mut out = String::with_capacity(output.len() + extra_len);
    out.push_str(output);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitgpui_core::file_diff::FileDiffRow;
    use gitgpui_core::file_diff::FileDiffRowKind as RK;

    #[test]
    fn parses_and_generates_conflicts() {
        let input = "a\n<<<<<<< HEAD\none\ntwo\n=======\nuno\ndos\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        let ours = generate_resolved_text(&segments);
        assert_eq!(ours, "a\none\ntwo\nb\n");

        let ConflictSegment::Block(block) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
            .unwrap()
        else {
            panic!("expected a conflict block");
        };
        block.choice = ConflictChoice::Theirs;

        let theirs = generate_resolved_text(&segments);
        assert_eq!(theirs, "a\nuno\ndos\nb\n");
    }

    #[test]
    fn parses_diff3_style_markers() {
        let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
        let segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        let ConflictSegment::Block(block) = segments
            .iter()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
            .unwrap()
        else {
            panic!("expected a conflict block");
        };

        assert_eq!(block.ours, "one\n");
        assert_eq!(block.base.as_deref(), Some("orig\n"));
        assert_eq!(block.theirs, "uno\n");
    }

    #[test]
    fn malformed_markers_are_preserved() {
        let input = "a\n<<<<<<< HEAD\none\n";
        let segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 0);
        assert_eq!(generate_resolved_text(&segments), input);
    }

    #[test]
    fn inline_rows_expand_modify_into_remove_and_add() {
        let rows = vec![
            FileDiffRow {
                kind: RK::Context,
                old_line: Some(1),
                new_line: Some(1),
                old: Some("a".into()),
                new: Some("a".into()),
            },
            FileDiffRow {
                kind: RK::Modify,
                old_line: Some(2),
                new_line: Some(2),
                old: Some("b".into()),
                new: Some("b2".into()),
            },
        ];
        let inline = build_inline_rows(&rows);
        assert_eq!(inline.len(), 3);
        assert_eq!(inline[0].content, "a");
        assert_eq!(inline[1].kind, gitgpui_core::domain::DiffLineKind::Remove);
        assert_eq!(inline[2].kind, gitgpui_core::domain::DiffLineKind::Add);
    }

    #[test]
    fn append_lines_adds_newlines_safely() {
        let out = append_lines_to_output("a\n", &["b".into(), "c".into()]);
        assert_eq!(out, "a\nb\nc\n");
        let out = append_lines_to_output("a", &["b".into()]);
        assert_eq!(out, "a\nb\n");
    }
}
