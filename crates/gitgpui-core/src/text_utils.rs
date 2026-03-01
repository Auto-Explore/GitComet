//! Text processing utilities for diff, merge, and text editing operations.
//!
//! Provides:
//! - Matching block extraction from sequence diffs
//! - Interval coalescing for overlapping ranges
//! - Newline-aware text manipulation

use crate::file_diff::{Edit, EditKind, myers_edits};
use std::collections::HashSet;
use std::fmt;

/// A contiguous matching block between two sequences.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatchingBlock {
    /// Start position in sequence A.
    pub a_start: usize,
    /// Start position in sequence B.
    pub b_start: usize,
    /// Length of the matching block.
    pub length: usize,
}

/// Validation error for sync-point-constrained matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncPointError {
    /// A sync point references a position outside either input sequence.
    OutOfBounds {
        index: usize,
        a_pos: usize,
        b_pos: usize,
        a_len: usize,
        b_len: usize,
    },
    /// Sync points must be strictly increasing in both sequences.
    NotStrictlyIncreasing {
        index: usize,
        prev_a: usize,
        prev_b: usize,
        a_pos: usize,
        b_pos: usize,
    },
}

impl fmt::Display for SyncPointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncPointError::OutOfBounds {
                index,
                a_pos,
                b_pos,
                a_len,
                b_len,
            } => write!(
                f,
                "sync point #{index} ({a_pos}, {b_pos}) is out of bounds for lengths ({a_len}, {b_len})"
            ),
            SyncPointError::NotStrictlyIncreasing {
                index,
                prev_a,
                prev_b,
                a_pos,
                b_pos,
            } => write!(
                f,
                "sync point #{index} ({a_pos}, {b_pos}) is not strictly increasing after ({prev_a}, {prev_b})"
            ),
        }
    }
}

impl std::error::Error for SyncPointError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatcherMode {
    Standard,
    InlineTrigram,
}

#[derive(Debug)]
struct PreprocessedSequences {
    a: Vec<String>,
    b: Vec<String>,
    aindex: Vec<usize>,
    bindex: Vec<usize>,
    lines_discarded: bool,
    common_prefix: usize,
    common_suffix: usize,
}

/// Extract matching blocks between two strings at the character level.
///
/// Uses Myers diff to find an optimal alignment, then returns contiguous
/// runs of matching characters as blocks. Blocks are returned in order
/// and do not overlap.
pub fn matching_blocks_chars(a: &str, b: &str) -> Vec<MatchingBlock> {
    let a_tokens = chars_to_tokens(a);
    let b_tokens = chars_to_tokens(b);
    matching_blocks_for_tokens(&a_tokens, &b_tokens, MatcherMode::Standard)
}

/// Extract matching blocks between two strings using Meld-style inline
/// trigram filtering before Myers matching.
pub fn matching_blocks_chars_inline(a: &str, b: &str) -> Vec<MatchingBlock> {
    let a_tokens = chars_to_tokens(a);
    let b_tokens = chars_to_tokens(b);
    matching_blocks_for_tokens(&a_tokens, &b_tokens, MatcherMode::InlineTrigram)
}

/// Extract matching blocks between two strings at the character level, while
/// forcing alignment to respect caller-provided sync points.
///
/// Each sync point `(ai, bi)` splits the input into independent diff chunks:
/// `a[prev_ai..ai]` is matched only against `b[prev_bi..bi]`. This mirrors
/// Meld's sync-point matcher behavior and allows deterministic alignment in
/// ambiguous regions.
pub fn matching_blocks_chars_with_sync_points(
    a: &str,
    b: &str,
    sync_points: &[(usize, usize)],
) -> Result<Vec<MatchingBlock>, SyncPointError> {
    let a_tokens = chars_to_tokens(a);
    let b_tokens = chars_to_tokens(b);
    matching_blocks_with_sync_points(&a_tokens, &b_tokens, sync_points, MatcherMode::Standard)
}

/// Extract matching blocks between two line sequences.
///
/// Uses Myers diff on the line arrays and returns contiguous runs
/// of matching lines as blocks.
pub fn matching_blocks_lines<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<MatchingBlock> {
    let a_tokens = lines_to_tokens(a);
    let b_tokens = lines_to_tokens(b);
    matching_blocks_for_tokens(&a_tokens, &b_tokens, MatcherMode::Standard)
}

/// Extract matching blocks between two line sequences with sync-point
/// constraints.
pub fn matching_blocks_lines_with_sync_points<'a>(
    a: &[&'a str],
    b: &[&'a str],
    sync_points: &[(usize, usize)],
) -> Result<Vec<MatchingBlock>, SyncPointError> {
    let a_tokens = lines_to_tokens(a);
    let b_tokens = lines_to_tokens(b);
    matching_blocks_with_sync_points(&a_tokens, &b_tokens, sync_points, MatcherMode::Standard)
}

fn matching_blocks_with_sync_points(
    a: &[String],
    b: &[String],
    sync_points: &[(usize, usize)],
    mode: MatcherMode,
) -> Result<Vec<MatchingBlock>, SyncPointError> {
    validate_sync_points(sync_points, a.len(), b.len())?;

    if sync_points.is_empty() {
        return Ok(matching_blocks_for_tokens(a, b, mode));
    }

    let mut blocks = Vec::new();
    let mut a_start = 0usize;
    let mut b_start = 0usize;

    for &(a_end, b_end) in sync_points {
        append_segment_blocks(a, b, a_start, a_end, b_start, b_end, mode, &mut blocks);
        a_start = a_end;
        b_start = b_end;
    }

    append_segment_blocks(a, b, a_start, a.len(), b_start, b.len(), mode, &mut blocks);

    Ok(blocks)
}

#[allow(clippy::too_many_arguments)]
fn append_segment_blocks(
    a: &[String],
    b: &[String],
    a_start: usize,
    a_end: usize,
    b_start: usize,
    b_end: usize,
    mode: MatcherMode,
    out: &mut Vec<MatchingBlock>,
) {
    let segment_blocks = matching_blocks_for_tokens(&a[a_start..a_end], &b[b_start..b_end], mode);
    for block in segment_blocks {
        out.push(MatchingBlock {
            a_start: a_start + block.a_start,
            b_start: b_start + block.b_start,
            length: block.length,
        });
    }
}

fn chars_to_tokens(input: &str) -> Vec<String> {
    input.chars().map(|c| c.to_string()).collect()
}

fn lines_to_tokens(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_string()).collect()
}

fn matching_blocks_for_tokens(a: &[String], b: &[String], mode: MatcherMode) -> Vec<MatchingBlock> {
    let preprocessed = preprocess_tokens(a, b, mode);
    let mut blocks = matching_blocks_from_preprocessed(a, b, &preprocessed);
    postprocess_blocks(&mut blocks, a, b);
    blocks
}

fn preprocess_tokens(a: &[String], b: &[String], mode: MatcherMode) -> PreprocessedSequences {
    let common_prefix = find_common_prefix_tokens(a, b);
    let mut a_trimmed = a[common_prefix..].to_vec();
    let mut b_trimmed = b[common_prefix..].to_vec();

    let common_suffix = if !a_trimmed.is_empty() && !b_trimmed.is_empty() {
        let suffix = find_common_suffix_tokens(&a_trimmed, &b_trimmed);
        if suffix > 0 {
            a_trimmed.truncate(a_trimmed.len() - suffix);
            b_trimmed.truncate(b_trimmed.len() - suffix);
        }
        suffix
    } else {
        0
    };

    let (processed_a, processed_b, aindex, bindex, lines_discarded) =
        preprocess_discard_nonmatching(a_trimmed, b_trimmed, mode);

    PreprocessedSequences {
        a: processed_a,
        b: processed_b,
        aindex,
        bindex,
        lines_discarded,
        common_prefix,
        common_suffix,
    }
}

fn preprocess_discard_nonmatching(
    a: Vec<String>,
    b: Vec<String>,
    mode: MatcherMode,
) -> (Vec<String>, Vec<String>, Vec<usize>, Vec<usize>, bool) {
    if a.is_empty() || b.is_empty() {
        return (a, b, Vec::new(), Vec::new(), false);
    }

    if mode == MatcherMode::InlineTrigram && a.len() <= 2 && b.len() <= 2 {
        return (a, b, Vec::new(), Vec::new(), false);
    }

    let (indexed_b, bindex) = match mode {
        MatcherMode::Standard => index_matching(&a, &b),
        MatcherMode::InlineTrigram => index_matching_kmers(&a, &b),
    };
    let (indexed_a, aindex) = match mode {
        MatcherMode::Standard => index_matching(&b, &a),
        MatcherMode::InlineTrigram => index_matching_kmers(&b, &a),
    };

    let lines_discarded = (b.len() - indexed_b.len() > 10) || (a.len() - indexed_a.len() > 10);
    if lines_discarded {
        (indexed_a, indexed_b, aindex, bindex, true)
    } else {
        (a, b, aindex, bindex, false)
    }
}

fn index_matching(a: &[String], b: &[String]) -> (Vec<String>, Vec<usize>) {
    let aset: HashSet<&str> = a.iter().map(String::as_str).collect();
    let mut matches = Vec::new();
    let mut index = Vec::new();
    for (i, token) in b.iter().enumerate() {
        if aset.contains(token.as_str()) {
            matches.push(token.clone());
            index.push(i);
        }
    }
    (matches, index)
}

fn index_matching_kmers(a: &[String], b: &[String]) -> (Vec<String>, Vec<usize>) {
    let mut aset: HashSet<(String, String, String)> = HashSet::new();
    for i in 0..a.len().saturating_sub(2) {
        aset.insert((a[i].clone(), a[i + 1].clone(), a[i + 2].clone()));
    }

    let mut matches = Vec::new();
    let mut index = Vec::new();
    let mut next_poss_match = 0usize;

    for i in 2..b.len() {
        let kmer = (b[i - 2].clone(), b[i - 1].clone(), b[i].clone());
        if !aset.contains(&kmer) {
            continue;
        }

        for (j, item) in b.iter().enumerate().take(i + 1).skip(next_poss_match.max(i - 2)) {
            matches.push(item.clone());
            index.push(j);
        }
        next_poss_match = i + 1;
    }

    (matches, index)
}

fn find_common_prefix_tokens(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

fn find_common_suffix_tokens(a: &[String], b: &[String]) -> usize {
    let mut suffix = 0usize;
    while suffix < a.len() && suffix < b.len() && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    suffix
}

fn matching_blocks_from_preprocessed(
    original_a: &[String],
    original_b: &[String],
    preprocessed: &PreprocessedSequences,
) -> Vec<MatchingBlock> {
    let a_refs: Vec<&str> = preprocessed.a.iter().map(String::as_str).collect();
    let b_refs: Vec<&str> = preprocessed.b.iter().map(String::as_str).collect();
    let edits = myers_edits(&a_refs, &b_refs);
    let raw_blocks = edits_to_matching_blocks(&edits);

    let mut blocks = if preprocessed.lines_discarded {
        remap_discarded_blocks(
            &raw_blocks,
            &preprocessed.aindex,
            &preprocessed.bindex,
            preprocessed.common_prefix,
        )
    } else {
        raw_blocks
            .into_iter()
            .map(|block| MatchingBlock {
                a_start: block.a_start + preprocessed.common_prefix,
                b_start: block.b_start + preprocessed.common_prefix,
                length: block.length,
            })
            .collect()
    };

    if preprocessed.common_prefix > 0 {
        blocks.insert(
            0,
            MatchingBlock {
                a_start: 0,
                b_start: 0,
                length: preprocessed.common_prefix,
            },
        );
    }

    if preprocessed.common_suffix > 0 {
        blocks.push(MatchingBlock {
            a_start: original_a.len() - preprocessed.common_suffix,
            b_start: original_b.len() - preprocessed.common_suffix,
            length: preprocessed.common_suffix,
        });
    }

    blocks
}

fn remap_discarded_blocks(
    blocks: &[MatchingBlock],
    aindex: &[usize],
    bindex: &[usize],
    common_prefix: usize,
) -> Vec<MatchingBlock> {
    let mut remapped = Vec::new();

    for block in blocks {
        if block.length == 0 {
            continue;
        }

        let first_a = aindex[block.a_start] + common_prefix;
        let first_b = bindex[block.b_start] + common_prefix;
        let mut run_start_a = first_a;
        let mut run_start_b = first_b;
        let mut run_length = 1usize;

        for offset in 1..block.length {
            let current_a = aindex[block.a_start + offset] + common_prefix;
            let current_b = bindex[block.b_start + offset] + common_prefix;
            let prev_a = aindex[block.a_start + offset - 1] + common_prefix;
            let prev_b = bindex[block.b_start + offset - 1] + common_prefix;

            if current_a == prev_a + 1 && current_b == prev_b + 1 {
                run_length += 1;
                continue;
            }

            remapped.push(MatchingBlock {
                a_start: run_start_a,
                b_start: run_start_b,
                length: run_length,
            });
            run_start_a = current_a;
            run_start_b = current_b;
            run_length = 1;
        }

        remapped.push(MatchingBlock {
            a_start: run_start_a,
            b_start: run_start_b,
            length: run_length,
        });
    }

    remapped
}

fn postprocess_blocks(blocks: &mut Vec<MatchingBlock>, a: &[String], b: &[String]) {
    if blocks.is_empty() {
        return;
    }

    let mut merged = vec![*blocks.last().expect("non-empty checked above")];
    let mut i = blocks.len() as isize - 2;

    while i >= 0 {
        let mut current = blocks[i as usize];
        i -= 1;

        while i >= 0 {
            let prev = blocks[i as usize];
            if (prev.b_start + prev.length == current.b_start
                || prev.a_start + prev.length == current.a_start)
                && current.a_start >= prev.length
                && current.b_start >= prev.length
            {
                let prev_slice_a = &a[current.a_start - prev.length..current.a_start];
                let prev_slice_b = &b[current.b_start - prev.length..current.b_start];
                if prev_slice_a == prev_slice_b {
                    current.a_start -= prev.length;
                    current.b_start -= prev.length;
                    current.length += prev.length;
                    i -= 1;
                    continue;
                }
            }
            break;
        }

        merged.push(current);
    }

    merged.reverse();
    *blocks = merged;
}

fn validate_sync_points(
    sync_points: &[(usize, usize)],
    a_len: usize,
    b_len: usize,
) -> Result<(), SyncPointError> {
    let mut prev: Option<(usize, usize)> = None;
    for (index, &(a_pos, b_pos)) in sync_points.iter().enumerate() {
        if a_pos > a_len || b_pos > b_len {
            return Err(SyncPointError::OutOfBounds {
                index,
                a_pos,
                b_pos,
                a_len,
                b_len,
            });
        }

        if let Some((prev_a, prev_b)) = prev
            && (a_pos <= prev_a || b_pos <= prev_b)
        {
            return Err(SyncPointError::NotStrictlyIncreasing {
                index,
                prev_a,
                prev_b,
                a_pos,
                b_pos,
            });
        }
        prev = Some((a_pos, b_pos));
    }
    Ok(())
}

fn edits_to_matching_blocks(edits: &[Edit<'_>]) -> Vec<MatchingBlock> {
    let mut blocks = Vec::new();
    let mut a_pos = 0usize;
    let mut b_pos = 0usize;
    let mut match_start: Option<(usize, usize)> = None;
    let mut match_len = 0usize;

    for edit in edits {
        match edit.kind {
            EditKind::Equal => {
                if match_start.is_none() {
                    match_start = Some((a_pos, b_pos));
                    match_len = 0;
                }
                match_len += 1;
                a_pos += 1;
                b_pos += 1;
            }
            EditKind::Delete => {
                if let Some((sa, sb)) = match_start.take() {
                    blocks.push(MatchingBlock {
                        a_start: sa,
                        b_start: sb,
                        length: match_len,
                    });
                }
                a_pos += 1;
            }
            EditKind::Insert => {
                if let Some((sa, sb)) = match_start.take() {
                    blocks.push(MatchingBlock {
                        a_start: sa,
                        b_start: sb,
                        length: match_len,
                    });
                }
                b_pos += 1;
            }
        }
    }

    if let Some((sa, sb)) = match_start {
        blocks.push(MatchingBlock {
            a_start: sa,
            b_start: sb,
            length: match_len,
        });
    }

    blocks
}

/// Merge overlapping or adjacent intervals into non-overlapping intervals.
///
/// Each interval is `(start, end)` inclusive on both ends. Intervals that
/// touch (one ends where another starts) are merged. The result is sorted
/// by start position with no overlaps.
pub fn merge_intervals(intervals: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if intervals.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<(usize, usize)> = intervals.to_vec();
    sorted.sort_unstable();

    let mut result = vec![sorted[0]];

    for &(start, end) in &sorted[1..] {
        let last = result.last_mut().unwrap();
        if start <= last.1 {
            last.1 = last.1.max(end);
        } else {
            result.push((start, end));
        }
    }

    result
}

/// Delete the last line of text, respecting line endings.
///
/// If the text ends with a line ending (`\n`, `\r\n`, or `\r`), removes that
/// trailing line ending (effectively deleting the empty last line).
/// Otherwise, finds the last line ending and removes everything from there
/// to the end of the string (the last line and its preceding separator).
///
/// Returns an empty string if the text is empty or has no line endings
/// (single line).
pub fn delete_last_line(text: &str) -> &str {
    let bytes = text.as_bytes();
    let len = bytes.len();

    if len == 0 {
        return "";
    }

    // If text ends with a line ending, strip just that ending.
    if len >= 2 && bytes[len - 2] == b'\r' && bytes[len - 1] == b'\n' {
        return &text[..len - 2];
    }
    if bytes[len - 1] == b'\n' || bytes[len - 1] == b'\r' {
        return &text[..len - 1];
    }

    // Text doesn't end with a line ending.
    // Find the last line ending and remove from there to end.
    if len < 2 {
        return "";
    }

    let mut pos = len - 2;
    loop {
        match bytes[pos] {
            b'\n' => {
                if pos > 0 && bytes[pos - 1] == b'\r' {
                    return &text[..pos - 1];
                }
                return &text[..pos];
            }
            b'\r' => {
                return &text[..pos];
            }
            _ => {}
        }
        if pos == 0 {
            break;
        }
        pos -= 1;
    }

    // No line ending found — single line.
    ""
}
