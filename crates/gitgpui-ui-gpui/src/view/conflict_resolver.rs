#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictChoice {
    Base,
    Ours,
    Theirs,
    Both,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveTraceMode {
    Safe,
    Regex,
    History,
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

/// Per-line word-highlight ranges. `None` means no highlights for that line.
pub type WordHighlights = Vec<Option<Vec<std::ops::Range<usize>>>>;

/// Build a user-facing summary for the most recent autosolve run.
///
/// The summary is shown in the resolver UI so autosolve behavior remains
/// auditable without opening command logs.
pub fn format_autosolve_trace_summary(
    mode: AutosolveTraceMode,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: &gitgpui_state::msg::ConflictAutosolveStats,
) -> String {
    let resolved = unresolved_before.saturating_sub(unresolved_after);
    let blocks_word = if resolved == 1 { "block" } else { "blocks" };
    match mode {
        AutosolveTraceMode::Safe => format!(
            "Last autosolve (safe): resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, pass1-after-split {}).",
            unresolved_before,
            unresolved_after,
            stats.pass1,
            stats.pass2_split,
            stats.pass1_after_split
        ),
        AutosolveTraceMode::Regex => format!(
            "Last autosolve (regex): resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, pass1-after-split {}, regex {}).",
            unresolved_before,
            unresolved_after,
            stats.pass1,
            stats.pass2_split,
            stats.pass1_after_split,
            stats.regex
        ),
        AutosolveTraceMode::History => format!(
            "Last autosolve (history): resolved {resolved} {blocks_word}, unresolved {} -> {} (history {}).",
            unresolved_before, unresolved_after, stats.history
        ),
    }
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

/// Return conflict indices for currently unresolved blocks in queue order.
pub fn unresolved_conflict_indices(segments: &[ConflictSegment]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut conflict_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if !block.resolved {
            out.push(conflict_ix);
        }
        conflict_ix += 1;
    }
    out
}

/// Apply a choice to all unresolved conflict blocks.
///
/// Already-resolved blocks are preserved. Choosing `Base` skips unresolved
/// 2-way blocks that don't have an ancestor section.
///
/// Returns the number of blocks updated.
pub fn apply_choice_to_unresolved_segments(
    segments: &mut [ConflictSegment],
    choice: ConflictChoice,
) -> usize {
    let mut updated = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }
        if matches!(choice, ConflictChoice::Base) && block.base.is_none() {
            continue;
        }
        block.choice = choice;
        block.resolved = true;
        updated += 1;
    }
    updated
}

/// Find the next unresolved conflict index after `current`.
/// Wraps around to the first unresolved conflict.
pub fn next_unresolved_conflict_index(
    segments: &[ConflictSegment],
    current: usize,
) -> Option<usize> {
    let unresolved = unresolved_conflict_indices(segments);
    unresolved
        .iter()
        .copied()
        .find(|&ix| ix > current)
        .or_else(|| unresolved.first().copied())
}

/// Find the previous unresolved conflict index before `current`.
/// Wraps around to the last unresolved conflict.
pub fn prev_unresolved_conflict_index(
    segments: &[ConflictSegment],
    current: usize,
) -> Option<usize> {
    let unresolved = unresolved_conflict_indices(segments);
    unresolved
        .iter()
        .rev()
        .copied()
        .find(|&ix| ix < current)
        .or_else(|| unresolved.last().copied())
}

/// Apply safe auto-resolve rules (Pass 1) to all unresolved conflict blocks.
///
/// Safe rules:
/// 1. `ours == theirs` — both sides made the same change → pick ours.
/// 2. `ours == base` and `theirs != base` — only theirs changed → pick theirs.
/// 3. `theirs == base` and `ours != base` — only ours changed → pick ours.
/// 4. (if `whitespace_normalize`) whitespace-only difference → pick ours.
///
/// Returns the number of blocks auto-resolved.
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_resolve_segments(segments: &mut [ConflictSegment]) -> usize {
    auto_resolve_segments_with_options(segments, false)
}

/// Like [`auto_resolve_segments`] but with an optional whitespace-normalization toggle.
pub fn auto_resolve_segments_with_options(
    segments: &mut [ConflictSegment],
    whitespace_normalize: bool,
) -> usize {
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
        if let Some(base) = block.base.as_deref() {
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

        // Rule 4 (optional): whitespace-only difference.
        if whitespace_normalize
            && gitgpui_core::conflict_session::is_whitespace_only_diff(&block.ours, &block.theirs)
        {
            block.choice = ConflictChoice::Ours;
            block.resolved = true;
            count += 1;
        }
    }
    count
}

/// Apply Pass 3 regex-assisted auto-resolve rules (opt-in) to unresolved blocks.
///
/// This mode uses regex normalization rules from core and only performs
/// side-picks (`Ours` / `Theirs`), never synthetic text rewrites.
pub fn auto_resolve_segments_regex(
    segments: &mut [ConflictSegment],
    options: &gitgpui_core::conflict_session::RegexAutosolveOptions,
) -> usize {
    use gitgpui_core::conflict_session::{AutosolvePickSide, regex_assisted_auto_resolve_pick};

    let mut count = 0;
    for seg in segments.iter_mut() {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }

        let Some((_, pick)) = regex_assisted_auto_resolve_pick(
            block.base.as_deref(),
            &block.ours,
            &block.theirs,
            options,
        ) else {
            continue;
        };

        block.choice = match pick {
            AutosolvePickSide::Ours => ConflictChoice::Ours,
            AutosolvePickSide::Theirs => ConflictChoice::Theirs,
        };
        block.resolved = true;
        count += 1;
    }
    count
}

/// Apply history-aware auto-resolve to unresolved conflict blocks.
///
/// Detects history/changelog sections and merges entries by deduplication.
/// When a block is resolved by history merge, it is replaced with a `Text`
/// segment containing the merged content.
///
/// Returns the number of blocks resolved.
pub fn auto_resolve_segments_history(
    segments: &mut Vec<ConflictSegment>,
    options: &gitgpui_core::conflict_session::HistoryAutosolveOptions,
) -> usize {
    use gitgpui_core::conflict_session::history_merge_region;

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(ref block) if !block.resolved => {
                if let Some(merged) =
                    history_merge_region(block.base.as_deref(), &block.ours, &block.theirs, options)
                {
                    // Merge adjacent Text segments for cleanliness.
                    if let Some(ConflictSegment::Text(prev)) = new_segments.last_mut() {
                        prev.push_str(&merged);
                    } else {
                        new_segments.push(ConflictSegment::Text(merged));
                    }
                    count += 1;
                } else {
                    new_segments.push(seg);
                }
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    count
}

/// Apply Pass 2 (heuristic subchunk splitting) to unresolved conflict blocks.
///
/// For each unresolved block that has a base, attempts to split it into
/// line-level subchunks via 3-way diff/merge. Non-conflicting subchunks
/// become `Text` segments; remaining conflicts become smaller `Block` segments.
///
/// Returns the number of original blocks that were split.
pub fn auto_resolve_segments_pass2(segments: &mut Vec<ConflictSegment>) -> usize {
    use gitgpui_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut split_count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(ref block) if !block.resolved && block.base.is_some() => {
                let base = block.base.as_deref().unwrap();
                if let Some(subchunks) =
                    split_conflict_into_subchunks(base, &block.ours, &block.theirs)
                {
                    split_count += 1;
                    for subchunk in subchunks {
                        match subchunk {
                            Subchunk::Resolved(text) => {
                                // Merge adjacent Text segments for cleanliness.
                                if let Some(ConflictSegment::Text(prev)) = new_segments.last_mut() {
                                    prev.push_str(&text);
                                } else {
                                    new_segments.push(ConflictSegment::Text(text));
                                }
                            }
                            Subchunk::Conflict { base, ours, theirs } => {
                                new_segments.push(ConflictSegment::Block(ConflictBlock {
                                    base: Some(base),
                                    ours,
                                    theirs,
                                    choice: ConflictChoice::Ours,
                                    resolved: false,
                                }));
                            }
                        }
                    }
                    // If all subchunks resolved, no Block segments remain
                    // from this split (all became Text above).
                } else {
                    new_segments.push(seg);
                }
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    split_count
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
                ConflictChoice::Both => block.ours.len() + block.theirs.len(),
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
                ConflictChoice::Both => {
                    out.push_str(&block.ours);
                    out.push_str(&block.theirs);
                }
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

fn text_line_count(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

fn build_two_way_conflict_line_ranges(
    segments: &[ConflictSegment],
) -> Vec<(std::ops::Range<u32>, std::ops::Range<u32>)> {
    let mut ranges = Vec::new();
    let mut ours_line = 1u32;
    let mut theirs_line = 1u32;

    for seg in segments {
        match seg {
            ConflictSegment::Text(text) => {
                let count = text_line_count(text);
                ours_line = ours_line.saturating_add(count);
                theirs_line = theirs_line.saturating_add(count);
            }
            ConflictSegment::Block(block) => {
                let ours_count = text_line_count(&block.ours);
                let theirs_count = text_line_count(&block.theirs);
                let ours_end = ours_line.saturating_add(ours_count);
                let theirs_end = theirs_line.saturating_add(theirs_count);
                ranges.push((ours_line..ours_end, theirs_line..theirs_end));
                ours_line = ours_end;
                theirs_line = theirs_end;
            }
        }
    }

    ranges
}

fn row_conflict_index_for_lines(
    old_line: Option<u32>,
    new_line: Option<u32>,
    ranges: &[(std::ops::Range<u32>, std::ops::Range<u32>)],
) -> Option<usize> {
    ranges.iter().position(|(ours, theirs)| {
        old_line.is_some_and(|line| ours.contains(&line))
            || new_line.is_some_and(|line| theirs.contains(&line))
    })
}

/// Build conflict-index maps for two-way split and inline rows.
///
/// Each output entry is `Some(conflict_index)` when the row belongs to a marker
/// conflict block, or `None` for non-conflict context rows.
pub fn map_two_way_rows_to_conflicts(
    segments: &[ConflictSegment],
    diff_rows: &[gitgpui_core::file_diff::FileDiffRow],
    inline_rows: &[ConflictInlineRow],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let ranges = build_two_way_conflict_line_ranges(segments);
    let split = diff_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    let inline = inline_rows
        .iter()
        .map(|row| row_conflict_index_for_lines(row.old_line, row.new_line, &ranges))
        .collect();
    (split, inline)
}

/// Build visible row indices for two-way views.
///
/// When `hide_resolved` is true, rows belonging to resolved conflict blocks are
/// removed from the visible list. Non-conflict rows are always kept visible.
pub fn build_two_way_visible_indices(
    row_conflict_map: &[Option<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<usize> {
    if !hide_resolved {
        return (0..row_conflict_map.len()).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    row_conflict_map
        .iter()
        .enumerate()
        .filter_map(|(ix, conflict_ix)| match conflict_ix {
            Some(ci) if resolved_blocks.get(*ci).copied().unwrap_or(false) => None,
            _ => Some(ix),
        })
        .collect()
}

/// Find the visible list index for the first row that belongs to `conflict_ix`.
///
/// `visible_row_indices` maps visible list rows to source row indices. This helper
/// resolves conflict index -> visible row index so callers can scroll/focus a
/// specific conflict in two-way resolver modes.
pub fn visible_index_for_two_way_conflict(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    conflict_ix: usize,
) -> Option<usize> {
    visible_row_indices.iter().position(|&row_ix| {
        row_conflict_map
            .get(row_ix)
            .copied()
            .flatten()
            .is_some_and(|ix| ix == conflict_ix)
    })
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

/// Represents a visible row in the three-way view when hide-resolved is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleItem {
    /// A normal line at the given index in the three-way data.
    Line(usize),
    /// A collapsed summary row for a resolved conflict block (by conflict index).
    CollapsedBlock(usize),
}

/// Build the mapping from visible row indices to actual three-way data items.
///
/// When `hide_resolved` is false, every line maps directly.
/// When true, resolved conflict ranges are collapsed to a single summary row.
pub fn build_three_way_visible_map(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    let mut visible = Vec::with_capacity(total_lines);
    let mut line = 0usize;
    while line < total_lines {
        if let Some((range_ix, range)) = conflict_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(&line))
            .filter(|(ri, _)| resolved_blocks.get(*ri).copied().unwrap_or(false))
        {
            // Emit one collapsed summary row and skip the rest of the range.
            visible.push(ThreeWayVisibleItem::CollapsedBlock(range_ix));
            line = range.end;
            continue;
        }
        visible.push(ThreeWayVisibleItem::Line(line));
        line += 1;
    }
    visible
}

/// Find the visible index for the first line of a conflict range, or the
/// collapsed block entry. Returns `None` if the range is not visible.
pub fn visible_index_for_conflict(
    visible_map: &[ThreeWayVisibleItem],
    conflict_ranges: &[std::ops::Range<usize>],
    range_ix: usize,
) -> Option<usize> {
    let range = conflict_ranges.get(range_ix)?;
    visible_map.iter().position(|item| match item {
        ThreeWayVisibleItem::Line(ix) => range.contains(ix),
        ThreeWayVisibleItem::CollapsedBlock(ci) => *ci == range_ix,
    })
}

pub fn compute_three_way_word_highlights(
    base_lines: &[gpui::SharedString],
    ours_lines: &[gpui::SharedString],
    theirs_lines: &[gpui::SharedString],
    conflict_ranges: &[std::ops::Range<usize>],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let len = base_lines
        .len()
        .max(ours_lines.len())
        .max(theirs_lines.len());
    let mut wh_base: WordHighlights = vec![None; len];
    let mut wh_ours: WordHighlights = vec![None; len];
    let mut wh_theirs: WordHighlights = vec![None; len];

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
        if let Some(last) = out.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        out.push(r);
    }
    out
}

/// Per-line pair of (old, new) word-highlight ranges for two-way diff.
pub type TwoWayWordHighlights =
    Vec<Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)>>;

pub fn compute_two_way_word_highlights(
    diff_rows: &[gitgpui_core::file_diff::FileDiffRow],
) -> TwoWayWordHighlights {
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
pub fn populate_block_bases_from_ancestor(segments: &mut [ConflictSegment], ancestor_text: &str) {
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
    gitgpui_core::services::validate_conflict_resolution_text(text).has_conflict_markers
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictStageSafetyCheck {
    pub has_conflict_markers: bool,
    pub unresolved_blocks: usize,
}

impl ConflictStageSafetyCheck {
    pub fn requires_confirmation(self) -> bool {
        self.has_conflict_markers || self.unresolved_blocks > 0
    }
}

/// Compute stage-safety status for the current conflict resolver output/state.
///
/// This gate is stricter than marker-only checks: unresolved conflict blocks
/// should still require explicit confirmation even if the current output text
/// no longer contains marker lines.
pub fn conflict_stage_safety_check(
    output_text: &str,
    segments: &[ConflictSegment],
) -> ConflictStageSafetyCheck {
    let total_blocks = conflict_count(segments);
    let resolved_blocks = resolved_conflict_count(segments);
    ConflictStageSafetyCheck {
        has_conflict_markers: text_contains_conflict_markers(output_text),
        unresolved_blocks: total_blocks.saturating_sub(resolved_blocks),
    }
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
#[allow(clippy::single_range_in_vec_init)]
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

        {
            let ConflictSegment::Block(block) = segments
                .iter_mut()
                .find(|s| matches!(s, ConflictSegment::Block(_)))
                .unwrap()
            else {
                panic!("expected a conflict block");
            };
            block.choice = ConflictChoice::Theirs;
        }

        let theirs = generate_resolved_text(&segments);
        assert_eq!(theirs, "a\nuno\ndos\nb\n");

        {
            let ConflictSegment::Block(block) = segments
                .iter_mut()
                .find(|s| matches!(s, ConflictSegment::Block(_)))
                .unwrap()
            else {
                panic!("expected a conflict block");
            };
            block.choice = ConflictChoice::Both;
        }
        let both = generate_resolved_text(&segments);
        assert_eq!(both, "a\none\ntwo\nuno\ndos\nb\n");
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
                eof_newline: None,
            },
            FileDiffRow {
                kind: RK::Modify,
                old_line: Some(2),
                new_line: Some(2),
                old: Some("b".into()),
                new: Some("b2".into()),
                eof_newline: None,
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
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert!(block.base.is_none());

        // Populate base from ancestor file
        let ancestor = "a\norig\nb\n";
        populate_block_bases_from_ancestor(&mut segments, ancestor);

        // Now the block should have base content extracted from the ancestor
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n"));
    }

    #[test]
    fn populate_block_bases_preserves_existing_base() {
        // 3-way conflict markers (with base section)
        let input = "a\n<<<<<<< ours\none\n||||||| base\norig\n=======\nuno\n>>>>>>> theirs\nb\n";
        let mut segments = parse_conflict_markers(input);

        // Block already has base from markers
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n"));

        // populate should not overwrite existing base
        populate_block_bases_from_ancestor(&mut segments, "a\nDIFFERENT\nb\n");
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.base.as_deref(), Some("orig\n")); // unchanged
    }

    #[test]
    fn populate_block_bases_multiple_conflicts() {
        let input = "a\n<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> other\nb\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\nc\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 2);

        let ancestor = "a\norig_foo\nb\norig_x\nc\n";
        populate_block_bases_from_ancestor(&mut segments, ancestor);

        let blocks: Vec<_> = segments
            .iter()
            .filter_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .collect();
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
        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
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
        assert!(!text_contains_conflict_markers(
            "some text with < and > arrows"
        ));
        assert!(!text_contains_conflict_markers("====== not quite seven"));
    }

    #[test]
    fn stage_safety_requires_confirmation_for_unresolved_blocks_without_markers() {
        let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
        let segments = parse_conflict_markers(input);
        let output_text = generate_resolved_text(&segments);

        let safety = conflict_stage_safety_check(&output_text, &segments);
        assert!(!safety.has_conflict_markers);
        assert_eq!(safety.unresolved_blocks, 1);
        assert!(safety.requires_confirmation());
    }

    #[test]
    fn stage_safety_does_not_require_confirmation_when_fully_resolved_and_clean() {
        let input = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n";
        let mut segments = parse_conflict_markers(input);
        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
        }
        let output_text = generate_resolved_text(&segments);

        let safety = conflict_stage_safety_check(&output_text, &segments);
        assert!(!safety.has_conflict_markers);
        assert_eq!(safety.unresolved_blocks, 0);
        assert!(!safety.requires_confirmation());
    }

    #[test]
    fn stage_safety_requires_confirmation_when_markers_remain() {
        let safety = conflict_stage_safety_check("<<<<<<< HEAD\nours\n", &[]);
        assert!(safety.has_conflict_markers);
        assert_eq!(safety.unresolved_blocks, 0);
        assert!(safety.requires_confirmation());
    }

    #[test]
    fn autosolve_trace_summary_safe_mode() {
        let stats = gitgpui_state::msg::ConflictAutosolveStats {
            pass1: 2,
            pass2_split: 1,
            pass1_after_split: 0,
            regex: 0,
            history: 0,
        };
        let summary = format_autosolve_trace_summary(AutosolveTraceMode::Safe, 5, 2, &stats);
        assert!(summary.contains("Last autosolve (safe)"));
        assert!(summary.contains("resolved 3 blocks"));
        assert!(summary.contains("unresolved 5 -> 2"));
        assert!(summary.contains("pass1 2"));
        assert!(summary.contains("split 1"));
    }

    #[test]
    fn autosolve_trace_summary_history_mode_uses_history_stat() {
        let stats = gitgpui_state::msg::ConflictAutosolveStats {
            pass1: 0,
            pass2_split: 0,
            pass1_after_split: 0,
            regex: 0,
            history: 3,
        };
        let summary = format_autosolve_trace_summary(AutosolveTraceMode::History, 4, 1, &stats);
        assert!(summary.contains("Last autosolve (history)"));
        assert!(summary.contains("resolved 3 blocks"));
        assert!(summary.contains("history 3"));
        assert!(!summary.contains("pass1"));
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

    fn mark_block_resolved(segments: &mut [ConflictSegment], target: usize) {
        let mut seen = 0usize;
        for seg in segments {
            let ConflictSegment::Block(block) = seg else {
                continue;
            };
            if seen == target {
                block.resolved = true;
                return;
            }
            seen += 1;
        }
        panic!("missing block index {target}");
    }

    #[test]
    fn next_unresolved_wraps_to_first() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
            "<<<<<<< HEAD\nthree\n=======\ntres\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        mark_block_resolved(&mut segments, 1);

        assert_eq!(next_unresolved_conflict_index(&segments, 2), Some(0));
        assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
    }

    #[test]
    fn prev_unresolved_wraps_to_last() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
            "<<<<<<< HEAD\nthree\n=======\ntres\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        mark_block_resolved(&mut segments, 1);

        assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));
        assert_eq!(prev_unresolved_conflict_index(&segments, 2), Some(0));
    }

    #[test]
    fn unresolved_navigation_returns_none_when_fully_resolved() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        mark_block_resolved(&mut segments, 0);
        mark_block_resolved(&mut segments, 1);

        assert_eq!(next_unresolved_conflict_index(&segments, 0), None);
        assert_eq!(prev_unresolved_conflict_index(&segments, 0), None);
    }

    #[test]
    fn unresolved_navigation_can_jump_from_resolved_active_conflict() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        mark_block_resolved(&mut segments, 0);

        assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(1));
        assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(1));
    }

    #[test]
    fn bulk_pick_updates_only_unresolved_blocks() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);

        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
        }

        let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Ours);
        assert_eq!(updated, 1);
        assert_eq!(resolved_conflict_count(&segments), 2);

        let mut blocks = segments.iter().filter_map(|s| match s {
            ConflictSegment::Block(block) => Some(block),
            ConflictSegment::Text(_) => None,
        });
        let first = blocks.next().expect("missing first block");
        let second = blocks.next().expect("missing second block");
        assert_eq!(first.choice, ConflictChoice::Theirs);
        assert!(first.resolved);
        assert_eq!(second.choice, ConflictChoice::Ours);
        assert!(second.resolved);
    }

    #[test]
    fn bulk_pick_both_concatenates_for_unresolved_blocks() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n=======\ndos\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Both);
        assert_eq!(updated, 2);
        assert_eq!(resolved_conflict_count(&segments), 2);
        let resolved = generate_resolved_text(&segments);
        assert_eq!(resolved, "one\nuno\ntwo\ndos\n");
    }

    #[test]
    fn bulk_pick_base_skips_unresolved_blocks_without_base() {
        let input = concat!(
            "<<<<<<< HEAD\none\n=======\nuno\n>>>>>>> other\n",
            "<<<<<<< HEAD\ntwo\n||||||| base\ntwo\n=======\ndos\n>>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Base);
        assert_eq!(updated, 1);
        assert_eq!(resolved_conflict_count(&segments), 1);

        let mut blocks = segments.iter().filter_map(|s| match s {
            ConflictSegment::Block(block) => Some(block),
            ConflictSegment::Text(_) => None,
        });
        let first = blocks.next().expect("missing first block");
        let second = blocks.next().expect("missing second block");

        assert_eq!(first.choice, ConflictChoice::Ours);
        assert!(!first.resolved);
        assert_eq!(second.choice, ConflictChoice::Base);
        assert!(second.resolved);
    }

    // -- auto_resolve_segments tests --

    #[test]
    fn auto_resolve_identical_sides() {
        let input = "a\n<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);
        assert_eq!(resolved_conflict_count(&segments), 1);

        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.choice, ConflictChoice::Ours);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_only_theirs_changed() {
        let input =
            "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);

        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.choice, ConflictChoice::Theirs);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_only_ours_changed() {
        let input =
            "a\n<<<<<<< HEAD\nchanged\n||||||| base\norig\n=======\norig\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        assert_eq!(auto_resolve_segments(&mut segments), 1);

        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.choice, ConflictChoice::Ours);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_both_changed_differently_not_resolved() {
        let input =
            "a\n<<<<<<< HEAD\nours\n||||||| base\norig\n=======\ntheirs\n>>>>>>> other\nb\n";
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
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
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
        let input =
            "a\n<<<<<<< HEAD\norig\n||||||| base\norig\n=======\nchanged\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        auto_resolve_segments(&mut segments);
        let text = generate_resolved_text(&segments);
        assert_eq!(text, "a\nchanged\nb\n");
    }

    #[test]
    fn auto_resolve_regex_equivalent_sides() {
        use gitgpui_core::conflict_session::RegexAutosolveOptions;

        let input = "a\n<<<<<<< HEAD\nlet  answer = 42;\n||||||| base\nlet answer = 42;\n=======\nlet answer\t=\t42;\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        let options = RegexAutosolveOptions::whitespace_insensitive();

        assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 1);
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.choice, ConflictChoice::Ours);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_regex_only_theirs_changed_from_normalized_base() {
        use gitgpui_core::conflict_session::RegexAutosolveOptions;

        let input = "a\n<<<<<<< HEAD\nlet answer=42;\n||||||| base\nlet answer = 42;\n=======\nlet answer = 43;\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        let options = RegexAutosolveOptions::whitespace_insensitive();

        assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 1);
        let block = segments
            .iter()
            .find_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .unwrap();
        assert_eq!(block.choice, ConflictChoice::Theirs);
        assert!(block.resolved);
    }

    #[test]
    fn auto_resolve_regex_invalid_pattern_noops() {
        use gitgpui_core::conflict_session::RegexAutosolveOptions;

        let input = "a\n<<<<<<< HEAD\nlet answer=42;\n||||||| base\nlet answer = 42;\n=======\nlet answer = 43;\n>>>>>>> other\nb\n";
        let mut segments = parse_conflict_markers(input);
        let options = RegexAutosolveOptions::default().with_pattern("(", "");

        assert_eq!(auto_resolve_segments_regex(&mut segments, &options), 0);
        assert_eq!(resolved_conflict_count(&segments), 0);
    }

    #[test]
    fn map_two_way_rows_to_conflicts_tracks_conflict_indices() {
        let markers = concat!(
            "a\n",
            "<<<<<<< HEAD\n",
            "b\n",
            "=======\n",
            "B\n",
            ">>>>>>> other\n",
            "mid\n",
            "<<<<<<< HEAD\n",
            "c\n",
            "=======\n",
            "C\n",
            ">>>>>>> other\n",
            "z\n",
        );
        let segments = parse_conflict_markers(markers);
        let diff_rows =
            gitgpui_core::file_diff::side_by_side_rows("a\nb\nmid\nc\nz\n", "a\nB\nmid\nC\nz\n");
        let inline_rows = build_inline_rows(&diff_rows);
        let (split_map, inline_map) =
            map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

        let split_conflicts: Vec<usize> = split_map.iter().flatten().copied().collect();
        let inline_conflicts: Vec<usize> = inline_map.iter().flatten().copied().collect();

        assert_eq!(split_conflicts, vec![0, 1]);
        assert_eq!(inline_conflicts, vec![0, 0, 1, 1]);
    }

    #[test]
    fn map_two_way_rows_to_conflicts_maps_single_sided_rows() {
        let markers = "<<<<<<< HEAD\n=======\nadd\n>>>>>>> other\n";
        let segments = parse_conflict_markers(markers);
        let diff_rows = gitgpui_core::file_diff::side_by_side_rows("", "add\n");
        let inline_rows = build_inline_rows(&diff_rows);
        let (split_map, inline_map) =
            map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

        assert_eq!(split_map, vec![Some(0)]);
        assert_eq!(inline_map, vec![Some(0)]);
    }

    #[test]
    fn two_way_visible_indices_hide_only_resolved_conflict_rows() {
        let segments = vec![
            ConflictSegment::Text("a\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\n".into(),
                theirs: "B\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "c\n".into(),
                theirs: "C\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
        ];
        let row_conflict_map = vec![None, Some(0), Some(0), None, Some(1), Some(1)];

        assert_eq!(
            build_two_way_visible_indices(&row_conflict_map, &segments, false),
            vec![0, 1, 2, 3, 4, 5]
        );
        assert_eq!(
            build_two_way_visible_indices(&row_conflict_map, &segments, true),
            vec![0, 3, 4, 5]
        );
    }

    // -- hide-resolved visible map tests --

    #[test]
    fn visible_map_identity_when_not_hiding() {
        // 3 lines of text, 1 conflict with 2 lines = 5 total lines
        // conflict range: 1..3
        let segments = vec![
            ConflictSegment::Text("a\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".into(),
                theirs: "x\ny\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("d\ne\n".into()),
        ];
        let ranges = [1..3];
        let map = build_three_way_visible_map(5, &ranges, &segments, false);
        assert_eq!(map.len(), 5);
        for (i, item) in map.iter().enumerate() {
            assert_eq!(*item, ThreeWayVisibleItem::Line(i));
        }
    }

    #[test]
    fn visible_map_collapses_resolved_block() {
        let segments = vec![
            ConflictSegment::Text("a\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".into(),
                theirs: "x\ny\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true, // resolved
            }),
            ConflictSegment::Text("d\ne\n".into()),
        ];
        let ranges = [1..3];
        let map = build_three_way_visible_map(5, &ranges, &segments, true);
        // Should be: Line(0), CollapsedBlock(0), Line(3), Line(4)
        assert_eq!(map.len(), 4);
        assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
        assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
        assert_eq!(map[2], ThreeWayVisibleItem::Line(3));
        assert_eq!(map[3], ThreeWayVisibleItem::Line(4));
    }

    #[test]
    fn visible_map_keeps_unresolved_blocks_expanded() {
        let segments = vec![
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a\nb\n".into(),
                theirs: "x\ny\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false, // unresolved — keep expanded
            }),
            ConflictSegment::Text("c\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "d\n".into(),
                theirs: "z\n".into(),
                choice: ConflictChoice::Theirs,
                resolved: true, // resolved — collapse
            }),
        ];
        let ranges = vec![0..2, 3..4];
        let map = build_three_way_visible_map(4, &ranges, &segments, true);
        // Unresolved block: Line(0), Line(1)
        // Text: Line(2)
        // Resolved block: CollapsedBlock(1)
        assert_eq!(map.len(), 4);
        assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
        assert_eq!(map[1], ThreeWayVisibleItem::Line(1));
        assert_eq!(map[2], ThreeWayVisibleItem::Line(2));
        assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
    }

    #[test]
    fn visible_index_for_conflict_finds_collapsed() {
        let segments = vec![
            ConflictSegment::Text("a\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".into(),
                theirs: "x\ny\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("d\n".into()),
        ];
        let ranges = [1..3];
        let map = build_three_way_visible_map(4, &ranges, &segments, true);
        // map: Line(0), CollapsedBlock(0), Line(3)
        let vi = visible_index_for_conflict(&map, &ranges, 0);
        assert_eq!(vi, Some(1)); // CollapsedBlock is at visible index 1
    }

    #[test]
    fn visible_index_for_conflict_finds_expanded() {
        let segments = vec![
            ConflictSegment::Text("a\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "b\nc\n".into(),
                theirs: "x\ny\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
        ];
        let ranges = [1..3];
        let map = build_three_way_visible_map(3, &ranges, &segments, false);
        // map: Line(0), Line(1), Line(2)
        let vi = visible_index_for_conflict(&map, &ranges, 0);
        assert_eq!(vi, Some(1)); // First line of conflict at visible index 1
    }

    // -- Pass 2 subchunk splitting tests --

    #[test]
    fn pass2_splits_block_with_nonoverlapping_changes() {
        // 3-way conflict: ours changes line 1, theirs changes line 3.
        // Line 2 is context. Should split into resolved parts.
        let input = concat!(
            "ctx\n",
            "<<<<<<< HEAD\n",
            "AAA\nbbb\nccc\n",
            "||||||| base\n",
            "aaa\nbbb\nccc\n",
            "=======\n",
            "aaa\nbbb\nCCC\n",
            ">>>>>>> other\n",
            "end\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        // Pass 1 can't resolve (both sides changed differently).
        assert_eq!(auto_resolve_segments(&mut segments), 0);

        // Pass 2 should split the block.
        let split = auto_resolve_segments_pass2(&mut segments);
        assert_eq!(split, 1);

        // Original 1-block conflict is now gone (split into text + smaller blocks or all text).
        // Since ours changes line 1 and theirs changes line 3, non-overlapping →
        // all subchunks resolved → no more Block segments.
        assert_eq!(conflict_count(&segments), 0);

        // Resolved text should be the merged result.
        let text = generate_resolved_text(&segments);
        assert_eq!(text, "ctx\nAAA\nbbb\nCCC\nend\n");
    }

    #[test]
    fn pass2_splits_block_with_partial_conflict() {
        // Both sides change line 2, but line 1 and 3 are only changed by one side.
        let input = concat!(
            "<<<<<<< HEAD\n",
            "AAA\nBBB\nccc\n",
            "||||||| base\n",
            "aaa\nbbb\nccc\n",
            "=======\n",
            "aaa\nYYY\nCCC\n",
            ">>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        let split = auto_resolve_segments_pass2(&mut segments);
        assert_eq!(split, 1);

        // Should now have 1 smaller conflict block (line 2: BBB vs YYY)
        // and resolved text for lines 1 and 3.
        let blocks: Vec<_> = segments
            .iter()
            .filter_map(|s| match s {
                ConflictSegment::Block(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(blocks.len(), 1, "should have 1 remaining conflict");
        assert_eq!(blocks[0].ours, "BBB\n");
        assert_eq!(blocks[0].theirs, "YYY\n");
        assert_eq!(blocks[0].base.as_deref(), Some("bbb\n"));
    }

    #[test]
    fn pass2_no_base_skips_block() {
        // 2-way markers (no base) — Pass 2 can't split without a base.
        let input = "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n";
        let mut segments = parse_conflict_markers(input);
        let split = auto_resolve_segments_pass2(&mut segments);
        assert_eq!(split, 0);
        assert_eq!(conflict_count(&segments), 1);
    }

    #[test]
    fn pass2_skips_already_resolved() {
        let input = concat!(
            "<<<<<<< HEAD\n",
            "AAA\nbbb\nccc\n",
            "||||||| base\n",
            "aaa\nbbb\nccc\n",
            "=======\n",
            "aaa\nbbb\nCCC\n",
            ">>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);

        // Resolve manually first.
        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
            block.resolved = true;
        }

        // Pass 2 should skip resolved blocks.
        let split = auto_resolve_segments_pass2(&mut segments);
        assert_eq!(split, 0);
    }

    #[test]
    fn pass2_merges_adjacent_text_segments() {
        // After splitting, resolved subchunks adjacent to existing Text segments
        // should be merged for cleanliness.
        let input = concat!(
            "before\n",
            "<<<<<<< HEAD\n",
            "AAA\nbbb\n",
            "||||||| base\n",
            "aaa\nbbb\n",
            "=======\n",
            "aaa\nBBB\n",
            ">>>>>>> other\n",
            "after\n",
        );
        let mut segments = parse_conflict_markers(input);
        auto_resolve_segments_pass2(&mut segments);

        // Non-overlapping changes → fully merged → no blocks remain.
        assert_eq!(conflict_count(&segments), 0);

        // All text should be merged into as few Text segments as possible.
        let text_count = segments
            .iter()
            .filter(|s| matches!(s, ConflictSegment::Text(_)))
            .count();
        // "before\n" + merged subchunks + "after\n" — exact count depends on
        // merging, but should be compact.
        assert!(text_count <= 3, "should have at most 3 text segments");
    }

    // -- History-aware auto-resolve tests --

    #[test]
    fn history_auto_resolve_merges_changelog_block() {
        use gitgpui_core::conflict_session::HistoryAutosolveOptions;

        // Simulate a conflict in a changelog section.
        let input = concat!(
            "# README\n",
            "<<<<<<< HEAD\n",
            "# Changes\n",
            "- Added feature A\n",
            "- Existing entry\n",
            "||||||| base\n",
            "# Changes\n",
            "- Existing entry\n",
            "=======\n",
            "# Changes\n",
            "- Fixed bug B\n",
            "- Existing entry\n",
            ">>>>>>> other\n",
            "# Footer\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 1);

        let options = HistoryAutosolveOptions::bullet_list();
        let resolved = auto_resolve_segments_history(&mut segments, &options);
        assert_eq!(resolved, 1);
        assert_eq!(conflict_count(&segments), 0);

        let text = generate_resolved_text(&segments);
        assert!(text.contains("- Added feature A"), "ours' new entry");
        assert!(text.contains("- Fixed bug B"), "theirs' new entry");
        assert!(text.contains("- Existing entry"), "common entry");
        assert_eq!(
            text.matches("- Existing entry").count(),
            1,
            "deduped common entry"
        );
    }

    #[test]
    fn history_auto_resolve_skips_non_changelog_blocks() {
        use gitgpui_core::conflict_session::HistoryAutosolveOptions;

        // Regular code conflict, no changelog markers.
        let input = concat!(
            "<<<<<<< HEAD\n",
            "let x = 1;\n",
            "=======\n",
            "let x = 2;\n",
            ">>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        let options = HistoryAutosolveOptions::bullet_list();
        let resolved = auto_resolve_segments_history(&mut segments, &options);
        assert_eq!(resolved, 0);
        assert_eq!(conflict_count(&segments), 1);
    }

    #[test]
    fn history_auto_resolve_skips_already_resolved() {
        use gitgpui_core::conflict_session::HistoryAutosolveOptions;

        let input = concat!(
            "<<<<<<< HEAD\n",
            "# Changes\n- New\n",
            "=======\n",
            "# Changes\n- Other\n",
            ">>>>>>> other\n",
        );
        let mut segments = parse_conflict_markers(input);
        // Resolve manually first.
        if let Some(ConflictSegment::Block(block)) = segments
            .iter_mut()
            .find(|s| matches!(s, ConflictSegment::Block(_)))
        {
            block.resolved = true;
        }

        let options = HistoryAutosolveOptions::bullet_list();
        let resolved = auto_resolve_segments_history(&mut segments, &options);
        assert_eq!(resolved, 0);
    }

    // -- bulk-pick + hide-resolved interaction tests --

    #[test]
    fn bulk_pick_then_three_way_visible_map_collapses_all_resolved() {
        // Scenario: 3 conflicts with context. Resolve block 0 manually, then bulk-pick
        // remaining. The three-way visible map should collapse all 3 blocks.
        let input = concat!(
            "ctx\n",                                    // line 0
            "<<<<<<< HEAD\nA\n=======\na\n>>>>>>> o\n", // conflict 0, lines 1..2
            "mid\n",                                    // line 3 (after conflict)
            "<<<<<<< HEAD\nB\n=======\nb\n>>>>>>> o\n", // conflict 1, lines 4..5
            "mid2\n",                                   // line 6
            "<<<<<<< HEAD\nC\n=======\nc\n>>>>>>> o\n", // conflict 2, lines 7..8
            "end\n",                                    // line 9
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 3);

        // Manually resolve block 0
        mark_block_resolved(&mut segments, 0);

        // Bulk-pick remaining → blocks 1 and 2 become resolved
        let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Ours);
        assert_eq!(updated, 2);
        assert_eq!(resolved_conflict_count(&segments), 3);

        // Now rebuild the three-way visible map with hide_resolved=true.
        // Each conflict block is 2 lines (ours side), ranges are:
        //   block 0: 1..3, block 1: 4..6, block 2: 7..9
        // Total lines in the three-way view: 10
        let conflict_ranges = [1..3, 4..6, 7..9];
        let map = build_three_way_visible_map(10, &conflict_ranges, &segments, true);

        // Expect: Line(0), Collapsed(0), Line(3), Collapsed(1), Line(6), Collapsed(2), Line(9)
        assert_eq!(map.len(), 7);
        assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
        assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
        assert_eq!(map[2], ThreeWayVisibleItem::Line(3));
        assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
        assert_eq!(map[4], ThreeWayVisibleItem::Line(6));
        assert_eq!(map[5], ThreeWayVisibleItem::CollapsedBlock(2));
        assert_eq!(map[6], ThreeWayVisibleItem::Line(9));
    }

    #[test]
    fn bulk_pick_then_two_way_visible_indices_hides_all_resolved() {
        // Two-way variant: after bulk pick, all conflict rows should be hidden.
        let mut segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "A\n".into(),
                theirs: "a\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "B\n".into(),
                theirs: "b\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("end\n".into()),
        ];
        // row indices: 0=ctx, 1,2=block0(ours+theirs), 3=mid, 4,5=block1, 6=end
        let row_conflict_map: Vec<Option<usize>> =
            vec![None, Some(0), Some(0), None, Some(1), Some(1), None];

        // Before bulk pick: all rows visible
        assert_eq!(
            build_two_way_visible_indices(&row_conflict_map, &segments, true).len(),
            7
        );

        // Bulk pick resolves both blocks
        let updated = apply_choice_to_unresolved_segments(&mut segments, ConflictChoice::Theirs);
        assert_eq!(updated, 2);

        // After bulk pick with hide_resolved=true: conflict rows hidden
        let visible = build_two_way_visible_indices(&row_conflict_map, &segments, true);
        assert_eq!(visible, vec![0, 3, 6]); // only context rows
    }

    #[test]
    fn autosolve_then_three_way_visible_map_collapses_autoresolved() {
        // Auto-resolve should cause the same collapse behavior as manual picks
        // when hide_resolved is active.
        let input = concat!(
            "ctx\n",
            "<<<<<<< HEAD\nsame\n||||||| base\norig\n=======\nsame\n>>>>>>> o\n",
            "mid\n",
            "<<<<<<< HEAD\nX\n||||||| base\norig2\n=======\nY\n>>>>>>> o\n",
            "end\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 2);

        // Block 0: ours==theirs → autosolve resolves it
        // Block 1: both changed differently → stays unresolved
        let resolved = auto_resolve_segments(&mut segments);
        assert_eq!(resolved, 1);
        assert_eq!(resolved_conflict_count(&segments), 1);

        // Three-way: ctx(0), block0(1), mid(2), block1(3), end(4) → total 5
        let conflict_ranges = [1..2, 3..4];
        let map = build_three_way_visible_map(5, &conflict_ranges, &segments, true);
        assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
        assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0)); // autoresolved
        assert_eq!(map[2], ThreeWayVisibleItem::Line(2)); // mid
        assert_eq!(map[3], ThreeWayVisibleItem::Line(3)); // unresolved block stays expanded
        assert_eq!(map[4], ThreeWayVisibleItem::Line(4)); // end
    }

    // -- counter/navigation correctness after sequential picks --

    #[test]
    fn navigation_updates_correctly_after_sequential_picks() {
        // Start with 3 unresolved blocks, resolve them one-by-one,
        // verify navigation at each step.
        let input = concat!(
            "<<<<<<< HEAD\nA\n=======\na\n>>>>>>> o\n",
            "<<<<<<< HEAD\nB\n=======\nb\n>>>>>>> o\n",
            "<<<<<<< HEAD\nC\n=======\nc\n>>>>>>> o\n",
        );
        let mut segments = parse_conflict_markers(input);
        assert_eq!(conflict_count(&segments), 3);

        // All unresolved: next from 0 → 1, prev from 0 → 2 (wrap)
        assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(1));
        assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));

        // Resolve block 1 (middle)
        mark_block_resolved(&mut segments, 1);
        assert_eq!(resolved_conflict_count(&segments), 1);
        // Next from 0 should skip block 1, go to 2
        assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
        // Prev from 2 should skip block 1, go to 0
        assert_eq!(prev_unresolved_conflict_index(&segments, 2), Some(0));

        // Resolve block 0 (first)
        mark_block_resolved(&mut segments, 0);
        assert_eq!(resolved_conflict_count(&segments), 2);
        // Only block 2 is unresolved
        assert_eq!(next_unresolved_conflict_index(&segments, 0), Some(2));
        assert_eq!(next_unresolved_conflict_index(&segments, 1), Some(2));
        assert_eq!(next_unresolved_conflict_index(&segments, 2), Some(2));
        assert_eq!(prev_unresolved_conflict_index(&segments, 0), Some(2));

        // Resolve last block
        mark_block_resolved(&mut segments, 2);
        assert_eq!(resolved_conflict_count(&segments), 3);
        assert_eq!(next_unresolved_conflict_index(&segments, 0), None);
        assert_eq!(prev_unresolved_conflict_index(&segments, 0), None);
    }

    #[test]
    fn resolved_counter_consistent_with_visible_map_after_incremental_picks() {
        // Ensure the resolved count and visible map stay in sync as
        // conflicts are resolved one by one. Uses multi-line conflicts so
        // collapsing them visibly reduces the visible row count.
        let mut segments = vec![
            ConflictSegment::Text("pre\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("orig1\norig1b\n".into()),
                ours: "A\nA2\n".into(),
                theirs: "a\na2\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("orig2\norig2b\norig2c\n".into()),
                ours: "B\nB2\nB3\n".into(),
                theirs: "b\nb2\nb3\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("post\n".into()),
        ];
        // Layout: pre(0), block0(1..3), mid(3), block1(4..7), post(7) → total 8
        let conflict_ranges = [1..3, 4..7];
        let total_lines = 8;

        // Step 0: nothing resolved — all lines visible
        assert_eq!(resolved_conflict_count(&segments), 0);
        let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
        assert_eq!(map.len(), 8);
        assert!(
            map.iter()
                .all(|item| matches!(item, ThreeWayVisibleItem::Line(_)))
        );

        // Step 1: resolve block 0 (2 lines → 1 collapsed row)
        mark_block_resolved(&mut segments, 0);
        assert_eq!(resolved_conflict_count(&segments), 1);
        let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
        // pre(0), [collapsed0], mid(3), block1-lines(4,5,6), post(7) = 7 items
        assert_eq!(map.len(), 7);
        assert_eq!(map[0], ThreeWayVisibleItem::Line(0));
        assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
        assert_eq!(map[2], ThreeWayVisibleItem::Line(3));

        // Step 2: resolve block 1 (3 lines → 1 collapsed row)
        mark_block_resolved(&mut segments, 1);
        assert_eq!(resolved_conflict_count(&segments), 2);
        let map = build_three_way_visible_map(total_lines, &conflict_ranges, &segments, true);
        // pre(0), [collapsed0], mid(3), [collapsed1], post(7) = 5 items
        assert_eq!(map.len(), 5);
        assert_eq!(map[1], ThreeWayVisibleItem::CollapsedBlock(0));
        assert_eq!(map[3], ThreeWayVisibleItem::CollapsedBlock(1));
    }

    // -- split vs inline row list consistency --

    #[test]
    fn split_and_inline_views_have_consistent_conflict_counts() {
        // Verify that both split and inline row conflict maps produce the
        // same set of conflict indices (the same number of distinct conflicts).
        let markers = concat!(
            "ctx\n",
            "<<<<<<< HEAD\n",
            "alpha\nbeta\n",
            "=======\n",
            "ALPHA\nBETA\n",
            ">>>>>>> other\n",
            "mid\n",
            "<<<<<<< HEAD\n",
            "gamma\n",
            "=======\n",
            "GAMMA\nDELTA\n",
            ">>>>>>> other\n",
            "end\n",
        );
        let segments = parse_conflict_markers(markers);
        assert_eq!(conflict_count(&segments), 2);

        let ours_text = "ctx\nalpha\nbeta\nmid\ngamma\nend\n";
        let theirs_text = "ctx\nALPHA\nBETA\nmid\nGAMMA\nDELTA\nend\n";
        let diff_rows = gitgpui_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = build_inline_rows(&diff_rows);

        let (split_map, inline_map) =
            map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

        // Both maps should contain the same set of distinct conflict indices
        let split_indices: std::collections::BTreeSet<usize> =
            split_map.iter().flatten().copied().collect();
        let inline_indices: std::collections::BTreeSet<usize> =
            inline_map.iter().flatten().copied().collect();
        assert_eq!(split_indices, inline_indices);

        // And that set should match the actual conflict count
        assert_eq!(split_indices.len(), 2);
        assert!(split_indices.contains(&0));
        assert!(split_indices.contains(&1));
    }

    #[test]
    fn split_and_inline_hide_resolved_filter_same_conflicts() {
        // After resolving one conflict, both split and inline visible indices
        // should filter out the same conflict's rows.
        let segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "A\nB\n".into(),
                theirs: "a\nb\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true, // resolved
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "C\n".into(),
                theirs: "c\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false, // unresolved
            }),
            ConflictSegment::Text("end\n".into()),
        ];

        // Build split and inline maps
        let ours_text = "ctx\nA\nB\nmid\nC\nend\n";
        let theirs_text = "ctx\na\nb\nmid\nc\nend\n";
        let diff_rows = gitgpui_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = build_inline_rows(&diff_rows);
        let (split_map, inline_map) =
            map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

        // With hide_resolved=true, both views should hide block 0 rows
        let split_visible = build_two_way_visible_indices(&split_map, &segments, true);
        let inline_visible = build_two_way_visible_indices(&inline_map, &segments, true);

        // Split visible should not contain any rows mapped to conflict 0
        for &ix in &split_visible {
            if let Some(ci) = split_map[ix] {
                assert_ne!(ci, 0, "split view should hide resolved conflict 0 rows");
            }
        }
        // Inline visible should not contain any rows mapped to conflict 0
        for &ix in &inline_visible {
            if let Some(ci) = inline_map[ix] {
                assert_ne!(ci, 0, "inline view should hide resolved conflict 0 rows");
            }
        }

        // Both should still show the unresolved conflict 1 rows
        let split_has_conflict_1 = split_visible.iter().any(|&ix| split_map[ix] == Some(1));
        let inline_has_conflict_1 = inline_visible.iter().any(|&ix| inline_map[ix] == Some(1));
        assert!(
            split_has_conflict_1,
            "split should show unresolved conflict 1"
        );
        assert!(
            inline_has_conflict_1,
            "inline should show unresolved conflict 1"
        );
    }

    #[test]
    fn unresolved_conflict_indices_match_queue_order() {
        let segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "A\n".into(),
                theirs: "a\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "B\n".into(),
                theirs: "b\n".into(),
                choice: ConflictChoice::Theirs,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "C\n".into(),
                theirs: "c\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
        ];

        assert_eq!(unresolved_conflict_indices(&segments), vec![0, 2]);
    }

    #[test]
    fn visible_index_for_two_way_conflict_respects_hide_resolved_filter() {
        let segments = vec![
            ConflictSegment::Text("ctx\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "A\n".into(),
                theirs: "a\n".into(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("mid\n".into()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "B\n".into(),
                theirs: "b\n".into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("end\n".into()),
        ];
        let ours_text = "ctx\nA\nmid\nB\nend\n";
        let theirs_text = "ctx\na\nmid\nb\nend\n";
        let diff_rows = gitgpui_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = build_inline_rows(&diff_rows);
        let (split_map, inline_map) =
            map_two_way_rows_to_conflicts(&segments, &diff_rows, &inline_rows);

        let split_visible = build_two_way_visible_indices(&split_map, &segments, true);
        let inline_visible = build_two_way_visible_indices(&inline_map, &segments, true);

        assert_eq!(
            visible_index_for_two_way_conflict(&split_map, &split_visible, 0),
            None
        );
        assert_eq!(
            visible_index_for_two_way_conflict(&inline_map, &inline_visible, 0),
            None
        );
        assert!(
            visible_index_for_two_way_conflict(&split_map, &split_visible, 1).is_some(),
            "unresolved conflict should remain visible in split mode"
        );
        assert!(
            visible_index_for_two_way_conflict(&inline_map, &inline_visible, 1).is_some(),
            "unresolved conflict should remain visible in inline mode"
        );
    }
}
