//! Three-way file merge algorithm.
//!
//! Takes base, local (ours), and remote (theirs) file contents and produces
//! merged output, potentially with conflict markers where the two sides
//! changed the same region differently.
//!
//! Compatible with `git merge-file` marker format.

use crate::file_diff::{
    DiffHunk, Edit, edits_to_hunks_with, histogram_edits, myers_edits, reconstruct_side_with,
    split_lines,
};
use std::borrow::Cow;
use std::fmt;

/// Default conflict marker width (matches git's default).
pub const DEFAULT_MARKER_SIZE: usize = 7;

/// How to render the base content in conflict markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ConflictStyle {
    /// Two-section markers: `<<<<<<<` / `=======` / `>>>>>>>`.
    #[default]
    Merge,
    /// Three-section markers showing ancestor: `<<<<<<<` / `|||||||` / `=======` / `>>>>>>>`.
    Diff3,
    /// Like diff3 but strips common prefix/suffix lines from conflict blocks.
    Zdiff3,
}

/// How to automatically resolve conflicts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum MergeStrategy {
    /// Leave conflict markers in output.
    #[default]
    Normal,
    /// Auto-resolve conflicts by picking ours (local).
    Ours,
    /// Auto-resolve conflicts by picking theirs (remote).
    Theirs,
    /// Auto-resolve conflicts by including both sides (ours then theirs).
    Union,
}

/// Which diff algorithm to use for computing edit scripts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum DiffAlgorithm {
    /// Classic Myers O(ND) algorithm. Fast and minimal edit distance.
    #[default]
    Myers,
    /// Patience/histogram algorithm. Anchors on unique lines to produce
    /// semantically cleaner diffs, especially for code with repetitive
    /// structural tokens (braces, returns). Falls back to Myers for
    /// regions with no unique lines.
    Histogram,
}

/// Labels for the three merge sides.
#[derive(Clone, Debug, Default)]
pub struct MergeLabels {
    pub ours: Option<String>,
    pub base: Option<String>,
    pub theirs: Option<String>,
}

/// Options controlling merge behavior.
#[derive(Clone, Debug)]
pub struct MergeOptions {
    pub style: ConflictStyle,
    pub strategy: MergeStrategy,
    pub labels: MergeLabels,
    pub marker_size: usize,
    pub diff_algorithm: DiffAlgorithm,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            style: ConflictStyle::default(),
            strategy: MergeStrategy::default(),
            labels: MergeLabels::default(),
            marker_size: DEFAULT_MARKER_SIZE,
            diff_algorithm: DiffAlgorithm::default(),
        }
    }
}

/// Result of a three-way merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeResult {
    /// The merged output text.
    pub output: String,
    /// Number of conflict regions (0 = clean merge).
    pub conflict_count: usize,
}

impl MergeResult {
    /// Returns `true` if the merge completed without conflicts.
    pub fn is_clean(&self) -> bool {
        self.conflict_count == 0
    }
}

/// Error from a three-way merge operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeError {
    /// One or more inputs contain binary content (null bytes or non-UTF-8).
    BinaryContent,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::BinaryContent => write!(f, "cannot merge binary files"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Perform a three-way merge on raw byte inputs with binary detection.
///
/// Returns `Err(MergeError::BinaryContent)` if any input contains null bytes
/// or is not valid UTF-8. Otherwise delegates to [`merge_file`].
pub fn merge_file_bytes(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    options: &MergeOptions,
) -> Result<MergeResult, MergeError> {
    fn check_binary(data: &[u8]) -> Result<&str, MergeError> {
        if data.contains(&0) {
            return Err(MergeError::BinaryContent);
        }
        std::str::from_utf8(data).map_err(|_| MergeError::BinaryContent)
    }

    let base_str = check_binary(base)?;
    let ours_str = check_binary(ours)?;
    let theirs_str = check_binary(theirs)?;

    Ok(merge_file(base_str, ours_str, theirs_str, options))
}

/// Perform a three-way merge of text files.
///
/// Diffs `base` against both `ours` and `theirs`, then walks the two edit
/// scripts to produce a merged result. Where both sides changed the same
/// base region differently, a conflict is emitted (or auto-resolved per
/// the chosen strategy).
pub fn merge_file(base: &str, ours: &str, theirs: &str, options: &MergeOptions) -> MergeResult {
    let base_lines = split_lines(base);
    let ours_lines = split_lines(ours);
    let theirs_lines = split_lines(theirs);

    let diff_fn = match options.diff_algorithm {
        DiffAlgorithm::Myers => myers_edits,
        DiffAlgorithm::Histogram => histogram_edits,
    };
    let edits_ours = diff_fn(&base_lines, &ours_lines);
    let edits_theirs = diff_fn(&base_lines, &theirs_lines);

    let hunks_ours = edits_to_hunks(&edits_ours);
    let hunks_theirs = edits_to_hunks(&edits_theirs);

    let merged_hunks = merge_hunks(&base_lines, &hunks_ours, &hunks_theirs);
    let merged_hunks = coalesce_zealous_conflicts(&base_lines, merged_hunks);
    render_merged(&base_lines, &merged_hunks, base, ours, theirs, options)
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A contiguous change from one side's diff against the base.
type Hunk<'a> = DiffHunk<Cow<'a, str>>;

/// A merged hunk — either cleanly resolved or a conflict.
#[derive(Clone, Debug)]
enum MergedHunk<'a> {
    /// Resolved: output these lines.
    Resolved {
        base_start: usize,
        base_end: usize,
        lines: Vec<Cow<'a, str>>,
    },
    /// Conflict: both sides changed the same base region differently.
    Conflict {
        base_start: usize,
        base_end: usize,
        ours_lines: Vec<Cow<'a, str>>,
        theirs_lines: Vec<Cow<'a, str>>,
    },
}

impl MergedHunk<'_> {
    fn base_start(&self) -> usize {
        match self {
            MergedHunk::Resolved { base_start, .. } => *base_start,
            MergedHunk::Conflict { base_start, .. } => *base_start,
        }
    }

    fn base_end(&self) -> usize {
        match self {
            MergedHunk::Resolved { base_end, .. } => *base_end,
            MergedHunk::Conflict { base_end, .. } => *base_end,
        }
    }
}

// ---------------------------------------------------------------------------
// Diff → Hunk conversion
// ---------------------------------------------------------------------------

fn edits_to_hunks<'a>(edits: &[Edit<'a>]) -> Vec<Hunk<'a>> {
    edits_to_hunks_with(edits, Cow::Borrowed)
}

// ---------------------------------------------------------------------------
// Hunk merging
// ---------------------------------------------------------------------------

/// Merge two hunk lists into a sequence of resolved/conflict hunks.
fn merge_hunks<'a>(
    base_lines: &'a [&'a str],
    ours: &[Hunk<'a>],
    theirs: &[Hunk<'a>],
) -> Vec<MergedHunk<'a>> {
    let mut result = Vec::new();
    let mut oi = 0;
    let mut ti = 0;

    loop {
        let oh_start = ours.get(oi).map(|h| h.base_start).unwrap_or(usize::MAX);
        let th_start = theirs.get(ti).map(|h| h.base_start).unwrap_or(usize::MAX);

        if oh_start == usize::MAX && th_start == usize::MAX {
            break;
        }

        // Determine the start of the next change region.
        let change_start = oh_start.min(th_start);

        // Expand the region to include all overlapping hunks from both sides.
        let mut region_end = change_start;
        let oi_start = oi;
        let ti_start = ti;

        // Consume initial hunks at change_start.
        while let Some(oh) = ours.get(oi) {
            if oh.base_start <= region_end {
                region_end = region_end.max(oh.base_end);
                oi += 1;
            } else {
                break;
            }
        }
        while let Some(th) = theirs.get(ti) {
            if th.base_start <= region_end {
                region_end = region_end.max(th.base_end);
                ti += 1;
            } else {
                break;
            }
        }

        // Keep expanding while hunks overlap.
        loop {
            let mut extended = false;
            while let Some(oh) = ours.get(oi) {
                if oh.base_start <= region_end {
                    region_end = region_end.max(oh.base_end);
                    oi += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            while let Some(th) = theirs.get(ti) {
                if th.base_start <= region_end {
                    region_end = region_end.max(th.base_end);
                    ti += 1;
                    extended = true;
                } else {
                    break;
                }
            }
            if !extended {
                break;
            }
        }

        let ours_involved = oi > oi_start;
        let theirs_involved = ti > ti_start;

        if ours_involved && theirs_involved {
            // Both sides changed the same region.
            let ours_hunks = &ours[oi_start..oi];
            let theirs_hunks = &theirs[ti_start..ti];

            if ours_hunks == theirs_hunks {
                // Identical hunk structure — skip reconstructing theirs entirely.
                let ours_content =
                    reconstruct_side(base_lines, change_start, region_end, ours_hunks);
                result.push(MergedHunk::Resolved {
                    base_start: change_start,
                    base_end: region_end,
                    lines: ours_content,
                });
            } else {
                let ours_content =
                    reconstruct_side(base_lines, change_start, region_end, ours_hunks);
                let theirs_content =
                    reconstruct_side(base_lines, change_start, region_end, theirs_hunks);

                if ours_content == theirs_content {
                    // Different hunks but same result — resolved.
                    result.push(MergedHunk::Resolved {
                        base_start: change_start,
                        base_end: region_end,
                        lines: ours_content,
                    });
                } else {
                    result.push(MergedHunk::Conflict {
                        base_start: change_start,
                        base_end: region_end,
                        ours_lines: ours_content,
                        theirs_lines: theirs_content,
                    });
                }
            }
        } else if ours_involved {
            let content =
                reconstruct_side(base_lines, change_start, region_end, &ours[oi_start..oi]);
            result.push(MergedHunk::Resolved {
                base_start: change_start,
                base_end: region_end,
                lines: content,
            });
        } else if theirs_involved {
            let content =
                reconstruct_side(base_lines, change_start, region_end, &theirs[ti_start..ti]);
            result.push(MergedHunk::Resolved {
                base_start: change_start,
                base_end: region_end,
                lines: content,
            });
        }
    }

    result
}

/// Coalesce consecutive conflict hunks when the unchanged base context between
/// them is adjacent or blank-only. This mirrors git's "zealous" behavior for
/// reducing noisy back-to-back conflict markers.
fn coalesce_zealous_conflicts<'a>(
    base_lines: &'a [&'a str],
    hunks: Vec<MergedHunk<'a>>,
) -> Vec<MergedHunk<'a>> {
    let mut out = Vec::with_capacity(hunks.len());

    for hunk in hunks {
        let mut merged_into_previous = false;

        if let Some(last) = out.last_mut()
            && let (
                MergedHunk::Conflict {
                    base_end: last_base_end,
                    ours_lines: last_ours,
                    theirs_lines: last_theirs,
                    ..
                },
                MergedHunk::Conflict {
                    base_start: next_base_start,
                    base_end: next_base_end,
                    ours_lines: next_ours,
                    theirs_lines: next_theirs,
                    ..
                },
            ) = (last, &hunk)
            && blank_only_or_adjacent_separator(base_lines, *last_base_end, *next_base_start)
        {
            let start = (*last_base_end).min(base_lines.len());
            let end = (*next_base_start).min(base_lines.len());
            for &line in &base_lines[start..end] {
                last_ours.push(Cow::Borrowed(line));
                last_theirs.push(Cow::Borrowed(line));
            }
            last_ours.extend(next_ours.iter().cloned());
            last_theirs.extend(next_theirs.iter().cloned());
            *last_base_end = *next_base_end;
            merged_into_previous = true;
        }

        if !merged_into_previous {
            out.push(hunk);
        }
    }

    out
}

fn blank_only_or_adjacent_separator(base_lines: &[&str], from: usize, to: usize) -> bool {
    if to < from {
        return false;
    }

    let start = from.min(base_lines.len());
    let end = to.min(base_lines.len());
    base_lines[start..end]
        .iter()
        .all(|line| line.trim().is_empty())
}

/// Reconstruct the content of one side for a base line range, applying hunks.
fn reconstruct_side<'a>(
    base_lines: &'a [&'a str],
    range_start: usize,
    range_end: usize,
    hunks: &[Hunk<'a>],
) -> Vec<Cow<'a, str>> {
    let mut lines = Vec::new();
    reconstruct_side_with(
        base_lines,
        range_start..range_end,
        hunks,
        &mut lines,
        Cow::Borrowed,
    );
    lines
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render merged hunks into final output text.
fn render_merged(
    base_lines: &[&str],
    merged_hunks: &[MergedHunk<'_>],
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    options: &MergeOptions,
) -> MergeResult {
    let line_ending = detect_line_ending(ours_text, theirs_text, base_text);
    let mut output = String::new();
    let mut conflict_count = 0;
    let mut base_pos = 0;

    for hunk in merged_hunks {
        // Emit unchanged base lines before this hunk.
        let ctx_end = hunk.base_start().min(base_lines.len());
        emit_context_lines(&mut output, base_lines, base_pos, ctx_end, line_ending);
        base_pos = hunk.base_end();

        match hunk {
            MergedHunk::Resolved { lines, .. } => {
                for line in lines {
                    output.push_str(line.as_ref());
                    output.push_str(line_ending);
                }
            }
            MergedHunk::Conflict {
                base_start,
                base_end,
                ours_lines,
                theirs_lines,
            } => {
                let base_conflict_lines =
                    &base_lines[*base_start..(*base_end).min(base_lines.len())];

                match options.strategy {
                    MergeStrategy::Ours => {
                        for line in ours_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Theirs => {
                        for line in theirs_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Union => {
                        for line in ours_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                        for line in theirs_lines {
                            output.push_str(line.as_ref());
                            output.push_str(line_ending);
                        }
                    }
                    MergeStrategy::Normal => {
                        emit_conflict_markers(
                            &mut output,
                            ours_lines,
                            theirs_lines,
                            base_conflict_lines,
                            options,
                            line_ending,
                        );
                        conflict_count += 1;
                    }
                }
            }
        }
    }

    // Remaining base lines after all hunks.
    emit_context_lines(
        &mut output,
        base_lines,
        base_pos,
        base_lines.len(),
        line_ending,
    );

    apply_trailing_newline_decision(&mut output, base_text, base_lines, ours_text, theirs_text);

    MergeResult {
        output,
        conflict_count,
    }
}

fn emit_context_lines(
    output: &mut String,
    base_lines: &[&str],
    from: usize,
    to: usize,
    line_ending: &str,
) {
    for &line in &base_lines[from..to] {
        output.push_str(line);
        output.push_str(line_ending);
    }
}

/// 3-way merge decision for whether the output should end with a trailing
/// newline. Checks which input(s) contributed the output's last line, then
/// applies merge logic to the trailing-LF "bit".
fn apply_trailing_newline_decision(
    output: &mut String,
    base_text: &str,
    base_lines: &[&str],
    ours_text: &str,
    theirs_text: &str,
) {
    let ours_has_trailing = ours_text.is_empty() || ours_text.ends_with('\n');
    let theirs_has_trailing = theirs_text.is_empty() || theirs_text.ends_with('\n');
    let base_has_trailing = base_text.is_empty() || base_text.ends_with('\n');

    let ours_lines_all = split_lines(ours_text);
    let theirs_lines_all = split_lines(theirs_text);

    let output_last = output
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .rsplit('\n')
        .next()
        .unwrap_or("");

    let ours_last_matches = ours_lines_all.last().is_some_and(|l| *l == output_last);
    let theirs_last_matches = theirs_lines_all.last().is_some_and(|l| *l == output_last);
    let base_last_matches = base_lines.last().is_some_and(|l| *l == output_last);

    // Each branch has distinct semantics even when the result expression
    // happens to be the same (`ours_has_trailing`):
    //   - agree    → both match, pick either
    //   - ours-only→ only ours diverged from base, pick ours
    //   - conflict → both diverged, prefer ours
    #[allow(clippy::if_same_then_else)]
    let want_trailing = if ours_last_matches && theirs_last_matches {
        if ours_has_trailing == theirs_has_trailing {
            ours_has_trailing
        } else if base_last_matches && base_has_trailing == theirs_has_trailing {
            ours_has_trailing // only ours changed
        } else if base_last_matches && base_has_trailing == ours_has_trailing {
            theirs_has_trailing // only theirs changed
        } else {
            ours_has_trailing // both changed; prefer ours
        }
    } else if ours_last_matches {
        ours_has_trailing
    } else if theirs_last_matches {
        theirs_has_trailing
    } else if base_last_matches {
        base_has_trailing
    } else {
        true // conflict marker or union content — keep trailing LF
    };

    if !want_trailing {
        if output.ends_with("\r\n") {
            output.truncate(output.len() - 2);
        } else if output.ends_with('\n') {
            output.truncate(output.len() - 1);
        }
    }
}

fn emit_conflict_markers(
    output: &mut String,
    ours_lines: &[Cow<'_, str>],
    theirs_lines: &[Cow<'_, str>],
    base_lines: &[&str],
    options: &MergeOptions,
    line_ending: &str,
) {
    let ms = options.marker_size;

    match options.style {
        ConflictStyle::Zdiff3 => {
            // Strip common prefix and suffix lines from the conflict.
            let (prefix_len, suffix_len) = common_prefix_suffix_lines(ours_lines, theirs_lines);

            // Emit common prefix as resolved.
            for line in &ours_lines[..prefix_len] {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }

            let ours_end = ours_lines.len().saturating_sub(suffix_len).max(prefix_len);
            let theirs_end = theirs_lines
                .len()
                .saturating_sub(suffix_len)
                .max(prefix_len);
            let ours_conflict = &ours_lines[prefix_len..ours_end];
            let theirs_conflict = &theirs_lines[prefix_len..theirs_end];

            // Emit conflict markers for the remaining inner region.
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_conflict {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '|', ms, options.labels.base.as_deref(), line_ending);
            // In zdiff3, the base section shows the trimmed base content.
            let base_conflict = if base_lines.len() > prefix_len + suffix_len {
                &base_lines[prefix_len..base_lines.len() - suffix_len]
            } else {
                &[] as &[&str]
            };
            for line in base_conflict {
                output.push_str(line);
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_conflict {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );

            // Emit common suffix as resolved.
            for line in &ours_lines[ours_lines.len() - suffix_len..] {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
        }
        ConflictStyle::Diff3 => {
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '|', ms, options.labels.base.as_deref(), line_ending);
            for line in base_lines {
                output.push_str(line);
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );
        }
        ConflictStyle::Merge => {
            emit_marker(output, '<', ms, options.labels.ours.as_deref(), line_ending);
            for line in ours_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(output, '=', ms, None, line_ending);
            for line in theirs_lines {
                output.push_str(line.as_ref());
                output.push_str(line_ending);
            }
            emit_marker(
                output,
                '>',
                ms,
                options.labels.theirs.as_deref(),
                line_ending,
            );
        }
    }
}

fn emit_marker(output: &mut String, ch: char, size: usize, label: Option<&str>, le: &str) {
    for _ in 0..size {
        output.push(ch);
    }
    if let Some(lbl) = label {
        output.push(' ');
        output.push_str(lbl);
    }
    output.push_str(le);
}

/// Find common prefix and suffix lines between two line sequences.
fn common_prefix_suffix_lines<T: PartialEq>(a: &[T], b: &[T]) -> (usize, usize) {
    let max = a.len().min(b.len());
    let mut prefix = 0;
    while prefix < max && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let remaining = max - prefix;
    let mut suffix = 0;
    while suffix < remaining && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }
    (prefix, suffix)
}

/// Detect the dominant line ending in the full-file merge inputs.
///
/// This remains a local counting heuristic so merge-file output keeps its
/// historical full-text behavior even as other modules share
/// `text_utils::detect_line_ending_from_texts` with context-specific modes.
fn detect_line_ending(ours: &str, theirs: &str, base: &str) -> &'static str {
    let crlf_count = ours.matches("\r\n").count()
        + theirs.matches("\r\n").count()
        + base.matches("\r\n").count();
    let lf_only_count =
        ours.matches('\n').count() + theirs.matches('\n').count() + base.matches('\n').count()
            - crlf_count;

    if crlf_count > lf_only_count {
        "\r\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_diff::EditKind;
    use std::borrow::Cow;

    fn default_opts() -> MergeOptions {
        MergeOptions::default()
    }

    fn opts_with_labels(ours: &str, base: &str, theirs: &str) -> MergeOptions {
        MergeOptions {
            labels: MergeLabels {
                ours: Some(ours.to_string()),
                base: Some(base.to_string()),
                theirs: Some(theirs.to_string()),
            },
            ..Default::default()
        }
    }

    fn opts_with_strategy(strategy: MergeStrategy) -> MergeOptions {
        MergeOptions {
            strategy,
            ..Default::default()
        }
    }

    fn opts_with_style(style: ConflictStyle) -> MergeOptions {
        MergeOptions {
            style,
            ..Default::default()
        }
    }

    #[test]
    fn edits_to_hunks_inserts_use_borrowed_cow() {
        let inserted_line = String::from("inserted");
        let edits = vec![Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(inserted_line.as_str()),
        }];

        let hunks = edits_to_hunks(&edits);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].new_lines.len(), 1);
        assert!(matches!(
            &hunks[0].new_lines[0],
            Cow::Borrowed(line) if *line == "inserted"
        ));
    }

    #[test]
    fn reconstruct_side_uses_borrowed_base_and_insert_lines() {
        let base_lines = split_lines("base-1\nbase-2\n");
        let inserted_lines = split_lines("inserted\n");
        let hunks = vec![Hunk {
            base_start: 1,
            base_end: 1,
            new_lines: vec![Cow::Borrowed(inserted_lines[0])],
        }];

        let lines = reconstruct_side(&base_lines, 0, 2, &hunks);
        assert_eq!(lines.len(), 3);
        assert!(matches!(&lines[0], Cow::Borrowed(line) if *line == "base-1"));
        assert!(matches!(&lines[1], Cow::Borrowed(line) if *line == "inserted"));
        assert!(matches!(&lines[2], Cow::Borrowed(line) if *line == "base-2"));
    }

    #[test]
    fn coalesce_zealous_conflicts_reuses_borrowed_separator_lines() {
        let base_lines = split_lines("top\n\nbottom\n");
        let hunks = vec![
            MergedHunk::Conflict {
                base_start: 0,
                base_end: 1,
                ours_lines: vec![Cow::Borrowed("ours-1")],
                theirs_lines: vec![Cow::Borrowed("theirs-1")],
            },
            MergedHunk::Conflict {
                base_start: 2,
                base_end: 3,
                ours_lines: vec![Cow::Borrowed("ours-2")],
                theirs_lines: vec![Cow::Borrowed("theirs-2")],
            },
        ];

        let coalesced = coalesce_zealous_conflicts(&base_lines, hunks);
        assert_eq!(coalesced.len(), 1);

        let MergedHunk::Conflict {
            ours_lines,
            theirs_lines,
            ..
        } = &coalesced[0]
        else {
            panic!("expected coalesced conflict hunk");
        };

        assert_eq!(ours_lines.len(), 3);
        assert_eq!(theirs_lines.len(), 3);
        assert!(matches!(&ours_lines[1], Cow::Borrowed(line) if line.is_empty()));
        assert!(matches!(&theirs_lines[1], Cow::Borrowed(line) if line.is_empty()));
    }

    // -----------------------------------------------------------------------
    // Identity and clean merge
    // -----------------------------------------------------------------------

    #[test]
    fn merge_identity() {
        let text = "line1\nline2\nline3\n";
        let result = merge_file(text, text, text, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, text);
    }

    #[test]
    fn merge_nonoverlapping_clean() {
        let base = "line1\nline2\nline3\n";
        let ours = "LINE1\nline2\nline3\n";
        let theirs = "line1\nline2\nLINE3\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "LINE1\nline2\nLINE3\n");
    }

    #[test]
    fn merge_nonoverlapping_additions() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nbbb\nccc\nours_added\n";
        let theirs = "theirs_added\naaa\nbbb\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "theirs_added\naaa\nbbb\nccc\nours_added\n");
    }

    // -----------------------------------------------------------------------
    // Conflict detection and marker format
    // -----------------------------------------------------------------------

    #[test]
    fn merge_overlapping_conflict() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        assert_eq!(result.conflict_count, 1);
        assert!(result.output.contains("<<<<<<<"));
        assert!(result.output.contains("======="));
        assert!(result.output.contains(">>>>>>>"));
        assert!(result.output.contains("OURS"));
        assert!(result.output.contains("THEIRS"));
    }

    #[test]
    fn merge_conflict_markers_with_labels() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let opts = opts_with_labels("local", "ancestor", "remote");
        let result = merge_file(base, ours, theirs, &opts);
        assert!(!result.is_clean());
        assert!(result.output.contains("<<<<<<< local"));
        assert!(result.output.contains(">>>>>>> remote"));
    }

    #[test]
    fn merge_delete_vs_modify_conflict() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\n";
        let theirs = "aaa\nBBB\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
    }

    // -----------------------------------------------------------------------
    // Conflict resolution strategies
    // -----------------------------------------------------------------------

    #[test]
    fn merge_ours_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &opts_with_strategy(MergeStrategy::Ours));
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nOURS\nccc\n");
    }

    #[test]
    fn merge_theirs_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(
            base,
            ours,
            theirs,
            &opts_with_strategy(MergeStrategy::Theirs),
        );
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
    }

    #[test]
    fn merge_union_strategy() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(
            base,
            ours,
            theirs,
            &opts_with_strategy(MergeStrategy::Union),
        );
        assert!(result.is_clean());
        assert!(result.output.contains("OURS"));
        assert!(result.output.contains("THEIRS"));
        // Union: ours comes before theirs.
        let ours_pos = result.output.find("OURS").unwrap();
        let theirs_pos = result.output.find("THEIRS").unwrap();
        assert!(ours_pos < theirs_pos);
    }

    // -----------------------------------------------------------------------
    // Diff3 and zdiff3 conflict styles
    // -----------------------------------------------------------------------

    #[test]
    fn merge_diff3_output() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Diff3));
        assert!(!result.is_clean());
        assert!(result.output.contains("|||||||"));
        assert!(result.output.contains("bbb"));
    }

    #[test]
    fn zdiff3_extracts_common_prefix_suffix() {
        // Both sides share prefix "A" and suffix "E" around the conflict.
        let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let ours = "1\n2\n3\n4\nA\nB\nC\nD\nE\n7\n8\n9\n";
        let theirs = "1\n2\n3\n4\nA\nX\nC\nY\nE\n7\n8\n9\n";
        let result = merge_file(base, ours, theirs, &opts_with_style(ConflictStyle::Zdiff3));
        assert!(!result.is_clean());
        // "A" should appear before the conflict marker, not inside.
        let marker_start = result.output.find("<<<<<<<").unwrap();
        let a_positions: Vec<_> = result
            .output
            .match_indices("\nA\n")
            .map(|(pos, _)| pos)
            .collect();
        // At least one "A" occurrence should be before the conflict.
        assert!(
            a_positions.iter().any(|&pos| pos < marker_start),
            "Common prefix 'A' should be before conflict markers"
        );
    }

    // -----------------------------------------------------------------------
    // Marker size
    // -----------------------------------------------------------------------

    #[test]
    fn merge_marker_size_10() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let opts = MergeOptions {
            marker_size: 10,
            ..Default::default()
        };
        let result = merge_file(base, ours, theirs, &opts);
        assert!(result.output.contains("<<<<<<<<<<"));
        assert!(result.output.contains("=========="));
        assert!(result.output.contains(">>>>>>>>>>"));
    }

    // -----------------------------------------------------------------------
    // Trailing newline / EOF edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn merge_preserves_trailing_newline() {
        let base = "aaa\nbbb\n";
        let ours = "aaa\nbbb\n";
        let theirs = "aaa\nBBB\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert!(result.output.ends_with('\n'));
    }

    #[test]
    fn merge_no_trailing_newline_when_inputs_lack_it() {
        let base = "aaa";
        let ours = "aaa";
        let theirs = "aaa";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert!(!result.output.ends_with('\n'));
    }

    // -----------------------------------------------------------------------
    // CRLF handling
    // -----------------------------------------------------------------------

    #[test]
    fn merge_crlf_conflict_markers() {
        let base = "1\r\n2\r\n3\r\n";
        let ours = "1\r\n2\r\n4\r\n";
        let theirs = "1\r\n2\r\n5\r\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        // Conflict markers should use CRLF too.
        assert!(result.output.contains("<<<<<<<\r\n"));
        assert!(result.output.contains("=======\r\n"));
        assert!(result.output.contains(">>>>>>>\r\n"));
    }

    #[test]
    fn merge_lf_conflict_markers() {
        let base = "1\n2\n3\n";
        let ours = "1\n2\n4\n";
        let theirs = "1\n2\n5\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(!result.is_clean());
        assert!(result.output.contains("<<<<<<<\n"));
        assert!(result.output.contains("=======\n"));
        assert!(result.output.contains(">>>>>>>\n"));
        // Ensure no CRLF.
        assert!(!result.output.contains("\r\n"));
    }

    // -----------------------------------------------------------------------
    // Multiple conflicts
    // -----------------------------------------------------------------------

    #[test]
    fn merge_multiple_conflicts() {
        let base = "a\nb\nc\nd\ne\n";
        let ours = "A\nb\nC\nd\ne\n";
        let theirs = "X\nb\nY\nd\ne\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert_eq!(result.conflict_count, 2);
    }

    // -----------------------------------------------------------------------
    // Identical changes
    // -----------------------------------------------------------------------

    #[test]
    fn merge_identical_changes_are_clean() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nXXX\nccc\n";
        let theirs = "aaa\nXXX\nccc\n";
        let result = merge_file(base, ours, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nXXX\nccc\n");
    }

    // -----------------------------------------------------------------------
    // Empty inputs
    // -----------------------------------------------------------------------

    #[test]
    fn merge_all_empty() {
        let result = merge_file("", "", "", &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "");
    }

    #[test]
    fn merge_base_empty_both_add_same() {
        let result = merge_file("", "added\n", "added\n", &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "added\n");
    }

    #[test]
    fn merge_base_empty_both_add_different() {
        let result = merge_file("", "ours\n", "theirs\n", &default_opts());
        assert!(!result.is_clean());
    }

    // -----------------------------------------------------------------------
    // Only one side changes
    // -----------------------------------------------------------------------

    #[test]
    fn merge_only_ours_changes() {
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nOURS\nccc\n";
        let result = merge_file(base, ours, base, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nOURS\nccc\n");
    }

    #[test]
    fn merge_only_theirs_changes() {
        let base = "aaa\nbbb\nccc\n";
        let theirs = "aaa\nTHEIRS\nccc\n";
        let result = merge_file(base, base, theirs, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "aaa\nTHEIRS\nccc\n");
    }

    #[test]
    fn merge_identical_changes_both_sides_resolves_cleanly() {
        // When ours and theirs make the exact same change, the hunk-level
        // short-circuit avoids reconstructing the theirs side entirely.
        let base = "first\nsecond\nthird\n";
        let both = "first\nreplaced\nthird\n";
        let result = merge_file(base, both, both, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "first\nreplaced\nthird\n");
    }

    #[test]
    fn merge_identical_multi_hunk_changes_resolves_cleanly() {
        let base = "a\nb\nc\nd\ne\n";
        let both = "a\nX\nc\nY\ne\n";
        let result = merge_file(base, both, both, &default_opts());
        assert!(result.is_clean());
        assert_eq!(result.output, "a\nX\nc\nY\ne\n");
    }
}
