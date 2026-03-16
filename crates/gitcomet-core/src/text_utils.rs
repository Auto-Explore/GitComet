//! Text processing utilities for diff, merge, and text editing operations.
//!
//! Provides:
//! - Matching block extraction from sequence diffs
//! - Interval coalescing for overlapping ranges
//! - Newline-aware text manipulation

#[cfg(test)]
use crate::file_diff::{Edit, EditKind, myers_edits};
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::fmt;

/// A contiguous matching block between two sequences.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MatchingBlock {
    /// Start position in sequence A.
    pub a_start: usize,
    /// Start position in sequence B.
    pub b_start: usize,
    /// Length of the matching block.
    pub length: usize,
}

/// Heuristic used by [`detect_line_ending_from_texts`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineEndingDetectionMode {
    /// Prefer CRLF if any side contains it, then CR, otherwise LF.
    ///
    /// This mirrors the focused-merge marker renderer behavior.
    Presence,
    /// Pick the dominant ending between CRLF and LF by counting occurrences.
    ///
    /// Ties default to LF, matching existing autosolve/subchunk behavior.
    DominantCrlfVsLf,
}

/// Detect a line ending style from multiple text inputs.
///
/// The caller selects the detection mode to preserve context-specific legacy
/// behavior while sharing one implementation.
pub fn detect_line_ending_from_texts<'a, I>(texts: I, mode: LineEndingDetectionMode) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    match mode {
        LineEndingDetectionMode::Presence => detect_by_presence(texts),
        LineEndingDetectionMode::DominantCrlfVsLf => detect_by_dominant_counts(texts),
    }
}

fn detect_by_presence<'a, I>(texts: I) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    let mut saw_cr = false;

    for text in texts {
        if text.contains("\r\n") {
            return "\r\n";
        }
        if text.contains('\r') {
            saw_cr = true;
        }
    }

    if saw_cr { "\r" } else { "\n" }
}

fn detect_by_dominant_counts<'a, I>(texts: I) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    let mut crlf_count = 0usize;
    let mut lf_only_count = 0usize;
    for text in texts {
        let crlf = text.matches("\r\n").count();
        crlf_count += crlf;
        lf_only_count += text.matches('\n').count().saturating_sub(crlf);
    }
    if crlf_count > lf_only_count {
        "\r\n"
    } else {
        "\n"
    }
}

/// Validation error for sync-point-constrained matching.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SyncPointError {
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

#[cfg(test)]
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

#[cfg(test)]
impl std::error::Error for SyncPointError {}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatcherMode {
    Standard,
    InlineTrigram,
}

#[cfg(test)]
#[derive(Debug)]
struct PreprocessedSequences<'a> {
    a: Vec<&'a str>,
    b: Vec<&'a str>,
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
#[cfg(test)]
pub(crate) fn matching_blocks_chars(a: &str, b: &str) -> Vec<MatchingBlock> {
    let a_tokens = chars_to_tokens(a);
    let b_tokens = chars_to_tokens(b);
    matching_blocks_for_tokens(&a_tokens, &b_tokens, MatcherMode::Standard)
}

/// Extract matching blocks between two strings using Meld-style inline
/// trigram filtering before Myers matching.
#[cfg(test)]
pub(crate) fn matching_blocks_chars_inline(a: &str, b: &str) -> Vec<MatchingBlock> {
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
#[cfg(test)]
pub(crate) fn matching_blocks_chars_with_sync_points(
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
#[cfg(test)]
pub(crate) fn matching_blocks_lines<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<MatchingBlock> {
    matching_blocks_for_tokens(a, b, MatcherMode::Standard)
}

/// Extract matching blocks between two line sequences with sync-point
/// constraints.
#[cfg(test)]
pub(crate) fn matching_blocks_lines_with_sync_points<'a>(
    a: &[&'a str],
    b: &[&'a str],
    sync_points: &[(usize, usize)],
) -> Result<Vec<MatchingBlock>, SyncPointError> {
    matching_blocks_with_sync_points(a, b, sync_points, MatcherMode::Standard)
}

#[cfg(test)]
fn matching_blocks_with_sync_points<'a>(
    a: &[&'a str],
    b: &[&'a str],
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

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn append_segment_blocks<'a>(
    a: &[&'a str],
    b: &[&'a str],
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

#[cfg(test)]
fn chars_to_tokens(input: &str) -> Vec<&str> {
    input
        .char_indices()
        .map(|(i, c)| &input[i..i + c.len_utf8()])
        .collect()
}

#[cfg(test)]
fn matching_blocks_for_tokens<'a>(
    a: &[&'a str],
    b: &[&'a str],
    mode: MatcherMode,
) -> Vec<MatchingBlock> {
    let preprocessed = preprocess_tokens(a, b, mode);
    let mut blocks = matching_blocks_from_preprocessed(a, b, &preprocessed);
    postprocess_blocks(&mut blocks, a, b);
    blocks
}

#[cfg(test)]
fn preprocess_tokens<'a>(
    a: &[&'a str],
    b: &[&'a str],
    mode: MatcherMode,
) -> PreprocessedSequences<'a> {
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

#[cfg(test)]
fn preprocess_discard_nonmatching<'a>(
    a: Vec<&'a str>,
    b: Vec<&'a str>,
    mode: MatcherMode,
) -> (Vec<&'a str>, Vec<&'a str>, Vec<usize>, Vec<usize>, bool) {
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

#[cfg(test)]
fn index_matching<'a>(a: &[&'a str], b: &[&'a str]) -> (Vec<&'a str>, Vec<usize>) {
    let aset: HashSet<&str> = a.iter().copied().collect();
    let mut matches = Vec::new();
    let mut index = Vec::new();
    for (i, &token) in b.iter().enumerate() {
        if aset.contains(token) {
            matches.push(token);
            index.push(i);
        }
    }
    (matches, index)
}

#[cfg(test)]
fn index_matching_kmers<'a>(a: &[&'a str], b: &[&'a str]) -> (Vec<&'a str>, Vec<usize>) {
    let mut aset: HashSet<(&str, &str, &str)> = HashSet::new();
    for i in 0..a.len().saturating_sub(2) {
        aset.insert((a[i], a[i + 1], a[i + 2]));
    }

    let mut matches = Vec::new();
    let mut index = Vec::new();
    let mut next_poss_match = 0usize;

    for i in 2..b.len() {
        let kmer = (b[i - 2], b[i - 1], b[i]);
        if !aset.contains(&kmer) {
            continue;
        }

        for (j, &item) in b
            .iter()
            .enumerate()
            .take(i + 1)
            .skip(next_poss_match.max(i - 2))
        {
            matches.push(item);
            index.push(j);
        }
        next_poss_match = i + 1;
    }

    (matches, index)
}

#[cfg(test)]
fn find_common_prefix_tokens(a: &[&str], b: &[&str]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
fn find_common_suffix_tokens(a: &[&str], b: &[&str]) -> usize {
    let mut suffix = 0usize;
    while suffix < a.len() && suffix < b.len() && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    suffix
}

#[cfg(test)]
fn matching_blocks_from_preprocessed(
    original_a: &[&str],
    original_b: &[&str],
    preprocessed: &PreprocessedSequences<'_>,
) -> Vec<MatchingBlock> {
    let edits = myers_edits(&preprocessed.a, &preprocessed.b);
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

#[cfg(test)]
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

        let Some(&first_a_raw) = aindex.get(block.a_start) else {
            continue;
        };
        let Some(&first_b_raw) = bindex.get(block.b_start) else {
            continue;
        };
        let first_a = first_a_raw + common_prefix;
        let first_b = first_b_raw + common_prefix;
        let mut run_start_a = first_a;
        let mut run_start_b = first_b;
        let mut run_length = 1usize;

        for offset in 1..block.length {
            let Some(&cur_a_raw) = aindex.get(block.a_start + offset) else {
                break;
            };
            let Some(&cur_b_raw) = bindex.get(block.b_start + offset) else {
                break;
            };
            let Some(&prev_a_raw) = aindex.get(block.a_start + offset - 1) else {
                break;
            };
            let Some(&prev_b_raw) = bindex.get(block.b_start + offset - 1) else {
                break;
            };
            let current_a = cur_a_raw + common_prefix;
            let current_b = cur_b_raw + common_prefix;
            let prev_a = prev_a_raw + common_prefix;
            let prev_b = prev_b_raw + common_prefix;

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

#[cfg(test)]
fn postprocess_blocks(blocks: &mut Vec<MatchingBlock>, a: &[&str], b: &[&str]) {
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

#[cfg(test)]
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

#[cfg(test)]
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
#[cfg(test)]
pub(crate) fn merge_intervals(intervals: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if intervals.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<(usize, usize)> = intervals.to_vec();
    sorted.sort_unstable();

    let mut result = vec![sorted[0]];

    for &(start, end) in &sorted[1..] {
        // SAFETY: `result` is initialized with `vec![sorted[0]]` and only grows.
        let last = result
            .last_mut()
            .expect("result is non-empty by construction");
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
#[cfg(test)]
pub(crate) fn delete_last_line(text: &str) -> &str {
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

#[cfg(test)]
mod tests {
    use super::{LineEndingDetectionMode, chars_to_tokens, detect_line_ending_from_texts};

    #[test]
    fn chars_to_tokens_splits_unicode_scalars() {
        assert_eq!(chars_to_tokens(""), Vec::<&str>::new());
        assert_eq!(
            chars_to_tokens("a😀e\u{301}"),
            vec!["a", "😀", "e", "\u{301}"]
        );
    }

    #[test]
    fn detect_line_ending_presence_prefers_crlf_when_present() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["ours\n", "theirs\r\n", "base\n"],
                LineEndingDetectionMode::Presence,
            ),
            "\r\n"
        );
    }

    #[test]
    fn detect_line_ending_presence_prefers_cr_when_no_crlf() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["ours\rmore", "theirs\n", "base"],
                LineEndingDetectionMode::Presence,
            ),
            "\r"
        );
    }

    #[test]
    fn detect_line_ending_presence_defaults_to_lf() {
        assert_eq!(
            detect_line_ending_from_texts(["no separators"], LineEndingDetectionMode::Presence),
            "\n"
        );
        assert_eq!(
            detect_line_ending_from_texts([], LineEndingDetectionMode::Presence),
            "\n"
        );
    }

    #[test]
    fn detect_line_ending_dominant_prefers_majority() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["a\r\nb\r\nc\n"],
                LineEndingDetectionMode::DominantCrlfVsLf,
            ),
            "\r\n"
        );
        assert_eq!(
            detect_line_ending_from_texts(
                ["a\r\nb\nc\n"],
                LineEndingDetectionMode::DominantCrlfVsLf,
            ),
            "\n"
        );
    }

    #[test]
    fn detect_line_ending_dominant_tie_defaults_to_lf() {
        assert_eq!(
            detect_line_ending_from_texts(["a\r\nb\n"], LineEndingDetectionMode::DominantCrlfVsLf,),
            "\n"
        );
        assert_eq!(
            detect_line_ending_from_texts([], LineEndingDetectionMode::DominantCrlfVsLf),
            "\n"
        );
    }
}

#[cfg(test)]
mod meld_tests {
    //! Meld-derived algorithm tests (Phase 5A/5B/5C).
    //!
    //! Ports test concepts from Meld's `test_matchers.py`, `test_misc.py`,
    //! and `test_chunk_actions.py` as verification tests for gitcomet's diff
    //! engine, interval utilities, and newline-aware text operations.

    use super::{
        MatchingBlock, SyncPointError, delete_last_line, matching_blocks_chars,
        matching_blocks_chars_inline, matching_blocks_chars_with_sync_points,
        matching_blocks_lines, matching_blocks_lines_with_sync_points, merge_intervals,
    };

    // ---------------------------------------------------------------------------
    // Invariant helpers
    // ---------------------------------------------------------------------------

    /// Verify that matching blocks are valid, ordered, and non-overlapping.
    fn verify_matching_blocks_chars(a: &str, b: &str, blocks: &[MatchingBlock]) {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();

        for (i, block) in blocks.iter().enumerate() {
            assert!(
                block.a_start + block.length <= a_chars.len(),
                "block {i} exceeds bounds of sequence A"
            );
            assert!(
                block.b_start + block.length <= b_chars.len(),
                "block {i} exceeds bounds of sequence B"
            );

            // Content at indicated positions must match.
            for j in 0..block.length {
                assert_eq!(
                    a_chars[block.a_start + j],
                    b_chars[block.b_start + j],
                    "block {i} content mismatch at offset {j}"
                );
            }
        }

        // Blocks must be in strictly increasing order and non-overlapping.
        for w in blocks.windows(2) {
            assert!(
                w[0].a_start + w[0].length <= w[1].a_start,
                "blocks overlap in sequence A: {:?} and {:?}",
                w[0],
                w[1]
            );
            assert!(
                w[0].b_start + w[0].length <= w[1].b_start,
                "blocks overlap in sequence B: {:?} and {:?}",
                w[0],
                w[1]
            );
        }
    }

    fn total_matched(blocks: &[MatchingBlock]) -> usize {
        blocks.iter().map(|b| b.length).sum()
    }

    // ===========================================================================
    // Phase 5A — Myers matching blocks
    // ===========================================================================

    /// Meld `myers_basic` exact parity case.
    #[test]
    fn myers_matching_blocks_basic() {
        let a = "abcbdefgabcdefg";
        let b = "gfabcdefcd";

        let blocks = matching_blocks_chars(a, b);
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 2,
                    length: 3,
                },
                MatchingBlock {
                    a_start: 4,
                    b_start: 5,
                    length: 3,
                },
                MatchingBlock {
                    a_start: 10,
                    b_start: 8,
                    length: 2,
                },
            ]
        );
        verify_matching_blocks_chars(a, b, &blocks);
        assert_eq!(total_matched(&blocks), 8);
    }

    /// Meld `myers_postprocess` exact parity case.
    #[test]
    fn myers_matching_blocks_postprocess() {
        let a = "abcfabgcd";
        let b = "afabcgabgcabcd";

        let blocks = matching_blocks_chars(a, b);
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 2,
                    length: 3,
                },
                MatchingBlock {
                    a_start: 4,
                    b_start: 6,
                    length: 3,
                },
                MatchingBlock {
                    a_start: 7,
                    b_start: 12,
                    length: 2,
                },
            ]
        );
        verify_matching_blocks_chars(a, b, &blocks);
        assert_eq!(total_matched(&blocks), 8);
    }

    /// Meld `myers_inline_trigram` exact parity case.
    #[test]
    fn myers_matching_blocks_inline() {
        let a = "red, blue, yellow, white";
        let b = "black green, hue, white";

        let blocks = matching_blocks_chars_inline(a, b);
        assert_eq!(
            blocks,
            vec![MatchingBlock {
                a_start: 17,
                b_start: 16,
                length: 7
            }]
        );
        verify_matching_blocks_chars(a, b, &blocks);
        assert_eq!(total_matched(&blocks), 7);
    }

    /// Covers the short-input fast path where inline k-mer filtering is skipped.
    #[test]
    fn myers_matching_blocks_inline_short_inputs() {
        let a = "ab";
        let b = "ac";

        let blocks = matching_blocks_chars_inline(a, b);
        assert_eq!(
            blocks,
            vec![MatchingBlock {
                a_start: 0,
                b_start: 0,
                length: 1
            }]
        );
        verify_matching_blocks_chars(a, b, &blocks);
    }

    /// Covers the inline k-mer indexing path with multiple trigram anchors.
    #[test]
    fn myers_matching_blocks_inline_kmer_path() {
        let a = "0123456789";
        let b = "xx012yy789zz";

        let blocks = matching_blocks_chars_inline(a, b);
        verify_matching_blocks_chars(a, b, &blocks);
        assert!(
            blocks.iter().any(|block| block.length >= 3),
            "expected at least one trigram-sized match, got {blocks:?}"
        );
    }

    /// Meld `sync_point_none` test concept — same inputs as `basic` with no
    /// sync points. Since our algorithm never uses sync points, the result
    /// should be identical to the basic test.
    #[test]
    fn myers_matching_blocks_no_sync_points_same_as_basic() {
        let a = "abcbdefgabcdefg";
        let b = "gfabcdefcd";

        let blocks_basic = matching_blocks_chars(a, b);
        let blocks_no_sync =
            matching_blocks_chars_with_sync_points(a, b, &[]).expect("empty sync points");

        assert_eq!(blocks_basic, blocks_no_sync);
    }

    /// Meld `sync_point_one` concept: one sync point forces a different
    /// (but deterministic) alignment than plain Myers.
    #[test]
    fn myers_matching_blocks_one_sync_point() {
        let a = "012a3456c789";
        let b = "0a3412b5678";

        let blocks =
            matching_blocks_chars_with_sync_points(a, b, &[(3, 6)]).expect("valid sync point");
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 0,
                    length: 1
                },
                MatchingBlock {
                    a_start: 1,
                    b_start: 4,
                    length: 2
                },
                MatchingBlock {
                    a_start: 6,
                    b_start: 7,
                    length: 2
                },
                MatchingBlock {
                    a_start: 9,
                    b_start: 9,
                    length: 2
                },
            ]
        );
    }

    /// Meld `sync_point_two` concept: two sync points force chunk-local
    /// matching and produce a stable block layout.
    #[test]
    fn myers_matching_blocks_two_sync_points() {
        let a = "012a3456c789";
        let b = "02a341b5678";

        let blocks = matching_blocks_chars_with_sync_points(a, b, &[(3, 2), (8, 6)])
            .expect("valid sync points");
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 0,
                    length: 1
                },
                MatchingBlock {
                    a_start: 2,
                    b_start: 1,
                    length: 1
                },
                MatchingBlock {
                    a_start: 3,
                    b_start: 2,
                    length: 3
                },
                MatchingBlock {
                    a_start: 9,
                    b_start: 9,
                    length: 2
                },
            ]
        );
    }

    #[test]
    fn myers_matching_blocks_sync_point_validation() {
        let err = matching_blocks_chars_with_sync_points("abc", "abc", &[(4, 1)])
            .expect_err("sync point should be out of bounds");
        assert!(matches!(
            err,
            SyncPointError::OutOfBounds {
                index: 0,
                a_pos: 4,
                b_pos: 1,
                a_len: 3,
                b_len: 3
            }
        ));

        let err = matching_blocks_chars_with_sync_points("abcdef", "abcdef", &[(2, 2), (2, 3)])
            .expect_err("sync points should be strictly increasing");
        assert!(matches!(
            err,
            SyncPointError::NotStrictlyIncreasing {
                index: 1,
                prev_a: 2,
                prev_b: 2,
                a_pos: 2,
                b_pos: 3
            }
        ));
    }

    #[test]
    fn sync_point_error_display_formats_both_variants() {
        let out_of_bounds = SyncPointError::OutOfBounds {
            index: 2,
            a_pos: 9,
            b_pos: 4,
            a_len: 5,
            b_len: 6,
        };
        assert_eq!(
            out_of_bounds.to_string(),
            "sync point #2 (9, 4) is out of bounds for lengths (5, 6)"
        );

        let not_increasing = SyncPointError::NotStrictlyIncreasing {
            index: 3,
            prev_a: 4,
            prev_b: 7,
            a_pos: 4,
            b_pos: 8,
        };
        assert_eq!(
            not_increasing.to_string(),
            "sync point #3 (4, 8) is not strictly increasing after (4, 7)"
        );
    }

    /// Line-level matching blocks for simple sequences.
    #[test]
    fn matching_blocks_lines_basic() {
        let a = &["aaa", "bbb", "ccc", "ddd"][..];
        let b = &["aaa", "xxx", "ccc", "ddd"][..];

        let blocks = matching_blocks_lines(a, b);
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 0,
                    length: 1
                },
                MatchingBlock {
                    a_start: 2,
                    b_start: 2,
                    length: 2
                },
            ]
        );
    }

    /// Line-level matching blocks with completely disjoint sequences.
    #[test]
    fn matching_blocks_lines_no_common() {
        let a = &["aaa", "bbb"][..];
        let b = &["xxx", "yyy"][..];

        let blocks = matching_blocks_lines(a, b);
        assert!(blocks.is_empty());
    }

    /// Line-level matching blocks with identical sequences.
    #[test]
    fn matching_blocks_lines_identical() {
        let a = &["aaa", "bbb", "ccc"][..];
        let blocks = matching_blocks_lines(a, a);
        assert_eq!(
            blocks,
            vec![MatchingBlock {
                a_start: 0,
                b_start: 0,
                length: 3
            }]
        );
    }

    /// Line-level matching blocks with empty inputs.
    #[test]
    fn matching_blocks_lines_empty() {
        let empty: &[&str] = &[];
        let a = &["aaa"][..];

        assert!(matching_blocks_lines(empty, empty).is_empty());
        assert!(matching_blocks_lines(empty, a).is_empty());
        assert!(matching_blocks_lines(a, empty).is_empty());
    }

    #[test]
    fn matching_blocks_lines_sync_points_respected() {
        let a = &["a0", "a1", "a2", "a3"][..];
        let b = &["a0", "b1", "a2", "a3"][..];

        let blocks = matching_blocks_lines_with_sync_points(a, b, &[(1, 1)]).expect("valid sync");
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 0,
                    length: 1
                },
                MatchingBlock {
                    a_start: 2,
                    b_start: 2,
                    length: 2
                }
            ]
        );
    }

    #[test]
    fn matching_blocks_lines_remap_sparse_matches_after_discarding_noise() {
        let mut a = vec!["head".to_string()];
        for idx in 0..11 {
            a.push(format!("noise-{idx}"));
        }
        a.push("tail".to_string());
        let a_refs: Vec<&str> = a.iter().map(String::as_str).collect();

        let b = ["head", "tail"];
        let blocks = matching_blocks_lines(&a_refs, &b);
        assert_eq!(
            blocks,
            vec![
                MatchingBlock {
                    a_start: 0,
                    b_start: 0,
                    length: 1
                },
                MatchingBlock {
                    a_start: 12,
                    b_start: 1,
                    length: 1
                }
            ]
        );
    }

    #[test]
    fn matching_blocks_lines_flushes_trailing_match_at_end() {
        let a = ["x", "same"];
        let b = ["same"];
        assert_eq!(
            matching_blocks_lines(&a, &b),
            vec![MatchingBlock {
                a_start: 1,
                b_start: 0,
                length: 1
            }]
        );
    }

    // ===========================================================================
    // Phase 5B — Interval merging
    // ===========================================================================

    /// Meld `intervals_dominated`: one interval dominates all others.
    #[test]
    fn intervals_dominated() {
        let input = [(1, 5), (5, 9), (10, 11), (0, 20)];
        assert_eq!(merge_intervals(&input), vec![(0, 20)]);
    }

    /// Meld `intervals_disjoint`: no intervals overlap or touch.
    #[test]
    fn intervals_disjoint() {
        let input = [(1, 5), (7, 9), (11, 13)];
        assert_eq!(merge_intervals(&input), vec![(1, 5), (7, 9), (11, 13)]);
    }

    /// Meld `intervals_two_groups`: two pairs of touching intervals.
    #[test]
    fn intervals_two_groups() {
        let input = [(1, 5), (5, 9), (10, 12), (11, 20)];
        assert_eq!(merge_intervals(&input), vec![(1, 9), (10, 20)]);
    }

    /// Meld `intervals_unsorted`: same as two_groups but unsorted input.
    #[test]
    fn intervals_unsorted() {
        let input = [(11, 20), (5, 9), (10, 12), (1, 5)];
        assert_eq!(merge_intervals(&input), vec![(1, 9), (10, 20)]);
    }

    /// Meld `intervals_duplicate`: duplicated intervals are deduplicated.
    #[test]
    fn intervals_duplicate() {
        let input = [(1, 5), (7, 8), (1, 5)];
        assert_eq!(merge_intervals(&input), vec![(1, 5), (7, 8)]);
    }

    /// Meld `intervals_chain`: overlapping chain merges into one.
    #[test]
    fn intervals_chain() {
        let input = [(1, 5), (4, 10), (9, 15)];
        assert_eq!(merge_intervals(&input), vec![(1, 15)]);
    }

    /// Edge case: empty input.
    #[test]
    fn intervals_empty() {
        let input: &[(usize, usize)] = &[];
        assert!(merge_intervals(input).is_empty());
    }

    /// Edge case: single interval.
    #[test]
    fn intervals_single() {
        let input = [(3, 7)];
        assert_eq!(merge_intervals(&input), vec![(3, 7)]);
    }

    // ===========================================================================
    // Phase 5C — Newline-aware operations
    // ===========================================================================

    /// Meld `delete_last_line_crlf`: CRLF-separated, no trailing newline.
    #[test]
    fn delete_last_line_crlf() {
        assert_eq!(delete_last_line("ree\r\neee"), "ree");
    }

    /// Meld `delete_last_line_crlf_trail`: CRLF-separated, trailing CRLF.
    #[test]
    fn delete_last_line_crlf_trail() {
        assert_eq!(delete_last_line("ree\r\neee\r\n"), "ree\r\neee");
    }

    /// Meld `delete_last_line_lf`: LF-separated, no trailing newline.
    #[test]
    fn delete_last_line_lf() {
        assert_eq!(delete_last_line("ree\neee"), "ree");
    }

    /// Meld `delete_last_line_lf_trail`: LF-separated, trailing LF.
    #[test]
    fn delete_last_line_lf_trail() {
        assert_eq!(delete_last_line("ree\neee\n"), "ree\neee");
    }

    /// Meld `delete_last_line_cr`: CR-separated, no trailing newline.
    #[test]
    fn delete_last_line_cr() {
        assert_eq!(delete_last_line("ree\reee"), "ree");
    }

    /// Meld `delete_last_line_cr_trail`: CR-separated, trailing CR.
    #[test]
    fn delete_last_line_cr_trail() {
        assert_eq!(delete_last_line("ree\reee\r"), "ree\reee");
    }

    /// Meld `delete_last_line_mixed`: mixed line endings.
    #[test]
    fn delete_last_line_mixed() {
        assert_eq!(delete_last_line("ree\r\neee\nqqq"), "ree\r\neee");
    }

    /// Edge case: empty input.
    #[test]
    fn delete_last_line_empty() {
        assert_eq!(delete_last_line(""), "");
    }

    /// Edge case: single character (no newline).
    #[test]
    fn delete_last_line_single_char() {
        assert_eq!(delete_last_line("x"), "");
    }

    /// Edge case: multi-char single-line input with no separator.
    #[test]
    fn delete_last_line_two_chars_no_separator() {
        assert_eq!(delete_last_line("xy"), "");
    }

    /// Edge case: single newline.
    #[test]
    fn delete_last_line_single_newline() {
        assert_eq!(delete_last_line("\n"), "");
    }

    /// Edge case: single CRLF.
    #[test]
    fn delete_last_line_single_crlf() {
        assert_eq!(delete_last_line("\r\n"), "");
    }

    /// Edge case: only content on first line, newline at end.
    #[test]
    fn delete_last_line_single_line_with_newline() {
        assert_eq!(delete_last_line("hello\n"), "hello");
    }
}
