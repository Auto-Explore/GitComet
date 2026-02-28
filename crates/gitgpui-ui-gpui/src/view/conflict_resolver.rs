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
    /// Whether this block has been explicitly resolved (by user pick or auto-resolve).
    /// Blocks start unresolved; becomes `true` when the user picks a side or auto-resolve runs.
    pub resolved: bool,
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
            resolved: false,
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

/// Count how many conflict blocks have been explicitly resolved.
pub fn resolved_conflict_count(segments: &[ConflictSegment]) -> usize {
    segments
        .iter()
        .filter(|s| matches!(s, ConflictSegment::Block(b) if b.resolved))
        .count()
}

/// Apply safe auto-resolve rules (Pass 1) to all unresolved conflict blocks.
///
/// Safe rules:
/// 1. `ours == theirs` — both sides made the same change → pick ours.
/// 2. `ours == base` and `theirs != base` — only theirs changed → pick theirs.
/// 3. `theirs == base` and `ours != base` — only ours changed → pick ours.
///
/// Returns the number of blocks auto-resolved.
pub fn auto_resolve_segments(segments: &mut [ConflictSegment]) -> usize {
    let mut count = 0;
    for seg in segments.iter_mut() {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }

        // Rule 1: both sides identical.
        if block.ours == block.theirs {
            block.choice = ConflictChoice::Ours;
            block.resolved = true;
            count += 1;
            continue;
        }

        // Rules 2 & 3 require a base.
        let Some(base) = block.base.as_deref() else {
            continue;
        };

        // Rule 2: only theirs changed (ours == base).
        if block.ours == base && block.theirs != base {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
            count += 1;
            continue;
        }

        // Rule 3: only ours changed (theirs == base).
        if block.theirs == base && block.ours != base {
            block.choice = ConflictChoice::Ours;
            block.resolved = true;
            count += 1;
            continue;
        }
    }
    count
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

/// When conflict markers use 2-way style (no `|||||||` base section), `block.base`
/// will be `None` even though the git ancestor content (index stage :1:) is available.
/// This function populates `block.base` by using the Text segments as anchors to
/// locate the corresponding base content in the ancestor file.
pub fn populate_block_bases_from_ancestor(
    segments: &mut [ConflictSegment],
    ancestor_text: &str,
) {
    if ancestor_text.is_empty() {
        return;
    }
    let any_missing = segments
        .iter()
        .any(|s| matches!(s, ConflictSegment::Block(b) if b.base.is_none()));
    if !any_missing {
        return;
    }

    // Find each Text segment's byte position in the ancestor file.
    // Text segments are the non-conflicting parts that exist in all three versions.
    let mut text_byte_ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut cursor = 0usize;
    for seg in segments.iter() {
        if let ConflictSegment::Text(text) = seg {
            if let Some(rel) = ancestor_text[cursor..].find(text.as_str()) {
                let start = cursor + rel;
                let end = start + text.len();
                text_byte_ranges.push(start..end);
                cursor = end;
            } else {
                // Text not found in ancestor – bail out.
                return;
            }
        }
    }

    // Extract base content for each block from the gaps between text positions.
    let mut text_idx = 0usize;
    let mut prev_end = 0usize;
    for seg in segments.iter_mut() {
        match seg {
            ConflictSegment::Text(_) => {
                prev_end = text_byte_ranges[text_idx].end;
                text_idx += 1;
            }
            ConflictSegment::Block(block) => {
                if block.base.is_some() {
                    continue;
                }
                let next_start = text_byte_ranges
                    .get(text_idx)
                    .map(|r| r.start)
                    .unwrap_or(ancestor_text.len());
                block.base = Some(ancestor_text[prev_end..next_start].to_string());
            }
        }
    }
}

/// Check whether the given text still contains git conflict markers.
/// Used as a safety gate before "Save & stage" to warn the user about unresolved conflicts.
pub fn text_contains_conflict_markers(text: &str) -> bool {
    for line in text.lines() {
        if line.starts_with("<<<<<<<")
            || line.starts_with(">>>>>>>")
            || line.starts_with("=======")
            || line.starts_with("|||||||")
        {
            return true;
        }
    }
    false
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

    #[test]
    fn populate_block_bases_from_ancestor_fills_missing_base() {
        // 2-way conflict markers (no base section)
        let input = "a\n<<<<<<< HEAD\none\ntwo\n=======\nuno\ndos\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        // The block has no base initially (2-way markers)
        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert!(block.base.is_none());

        // Populate base from ancestor file
        let ancestor = "a\norig\nb\n";
        populate_block_bases_from_ancestor(&mut segments, ancestor);

        // Now the block should have base content extracted from the ancestor
        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n"));
    }

    #[test]
    fn populate_block_bases_preserves_existing_base() {
        // 3-way conflict markers (with base section)
        let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
        let mut segments = parse_conflict_markers(input);

        // Block already has base from markers
        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n"));

        // populate should not overwrite existing base
        populate_block_bases_from_ancestor(&mut segments, "a\nDIFFERENT\nb\n");
        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n")); // unchanged
    }

    #[test]
    fn populate_block_bases_multiple_conflicts() {
        let input = "a\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> other\nb\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\nc\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 2);

        let ancestor = "a\norig_foo\nb\norig_x\nc\n";
        populate_block_bases_from_ancestor(&mut segments, ancestor);

        let blocks: Vec<_> = segments.iter().filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].base.as_deref(), Some("orig_foo\n"));
        assert_eq!(blocks[1].base.as_deref(), Some("orig_x\n"));
    }

    #[test]
    fn populate_block_bases_generates_correct_resolved_text() {
        let input = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);

        let ancestor = "a\norig\nb\n";
        populate_block_bases_from_ancestor(&mut segments, ancestor);

        // Pick Base and generate resolved text
        if let Some(ConflictSegment::Block(block)) = segments.iter_mut().find(|s| matches!(s, ConflictSegment::Block(_))) {
            block.choice = ConflictChoice::Base;
        }
        let resolved = generate_resolved_text(&segments);
        assert_eq!(resolved, "a\norig\nb\n");
    }

    #[test]
    fn detects_conflict_markers_in_text() {
        assert!(text_contains_conflict_markers(
            "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n"
        ));
        assert!(text_contains_conflict_markers("<<<<<<< HEAD\n"));
        assert!(text_contains_conflict_markers("=======\n"));
        assert!(text_contains_conflict_markers(">>>>>>> branch\n"));
        assert!(text_contains_conflict_markers("||||||| base\n"));
    }

    #[test]
    fn no_false_positives_for_clean_text() {
        assert!(!text_contains_conflict_markers("a\nb\nc\n"));
        assert!(!text_contains_conflict_markers(""));
        assert!(!text_contains_conflict_markers("some text with < and > arrows"));
        assert!(!text_contains_conflict_markers("====== not quite seven"));
    }

    // -- resolved_conflict_count tests --

    #[test]
    fn resolved_count_starts_at_zero() {
        let input = "a\n<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\nb\n";
        let segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);
        assert_eq!(resolved_conflict_count(&segments), 0);
    }

    #[test]
    fn resolved_count_tracks_picks() {
        let input = "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 2);
        assert_eq!(resolved_conflict_count(&segments), 0);

        // Resolve first block.
        if let ConflictSegment::Block(block) = &mut segments[0] {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
        }
        assert_eq!(resolved_conflict_count(&segments), 1);
    }

    // -- auto_resolve_segments tests --

    #[test]
    fn auto_resolve_identical_sides() {
        let input = "a\n<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);
        assert_eq!(resolved_conflict_count(&segments), 1);

        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.choice, ConflictChoice::Ours);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_only_theirs_changed() {
        let input = "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);

        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.choice, ConflictChoice::Theirs);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_only_ours_changed() {
        let input = "a\n<<<<<<< HEAD\nchanged\n||||||| base\norig\n=======\norig\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);

        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.choice, ConflictChoice::Ours);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_both_changed_differently_not_resolved() {
        let input = "a\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 0);
        assert_eq!(resolved_conflict_count(&segments), 0);
    }

    #[test]
    fn auto_resolve_no_base_identical_sides() {
        // 2-way markers (no base section) — identical sides should still resolve.
        let input = "a\n<<<<<<< HEAD\nsame\n=======\nsame\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);
        assert_eq!(resolved_conflict_count(&segments), 1);
    }

    #[test]
    fn auto_resolve_no_base_different_sides_not_resolved() {
        let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 0);
    }

    #[test]
    fn auto_resolve_skips_already_resolved() {
        let input = "a\n<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);

        // Manually resolve first.
        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
        }

        // Auto-resolve should skip it.
        assert_eq!(auto_resolve_segments(&mut segments), 0);
        // Choice should remain Theirs (not overwritten).
        let block = segments.iter().find_map(|s| match s {
            ConflictSegment::Block(b) => Some(b),
            _ => None,
        }).unwrap();
        assert_eq!(block.choice, ConflictChoice::Theirs);
    }

    #[test]
    fn auto_resolve_multiple_blocks_mixed() {
        let input = concat!(
            "<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\n",
            "<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\n",
            "<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 3);

        let resolved = auto_resolve_segments(&mut segments);
        assert_eq!(resolved, 2); // blocks 0 (identical) and 2 (only theirs changed)
        assert_eq!(resolved_conflict_count(&segments), 2);
    }

    #[test]
    fn auto_resolve_generates_correct_text() {
        let input = "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        auto_resolve_segments(&mut segments);
        let text = generate_resolved_text(&segments);
        assert_eq!(text, "a\nchanged\nb\n");
    }
}
