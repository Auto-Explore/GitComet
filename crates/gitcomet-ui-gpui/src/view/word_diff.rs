use std::ops::Range;

const WORD_DIFF_MAX_BYTES_PER_SIDE: usize = 4 * 1024;
const WORD_DIFF_MAX_TOTAL_BYTES: usize = 8 * 1024;

type WordDiffRangePair = (Vec<Range<usize>>, Vec<Range<usize>>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind {
    Whitespace,
    Other,
}

#[derive(Clone, Debug)]
struct Token {
    range: Range<usize>,
    kind: TokenKind,
}

/// Reusable buffers for `word_diff_ranges` to avoid per-call allocation overhead.
struct WordDiffBufs {
    old_tokens: Vec<Token>,
    new_tokens: Vec<Token>,
    v: Vec<isize>,
    /// Flat trace buffer: depth `d` stores `2d+1` values starting at offset `d*d`.
    trace: Vec<isize>,
    keep_old: Vec<bool>,
    keep_new: Vec<bool>,
    old_ranges: Vec<Range<usize>>,
    new_ranges: Vec<Range<usize>>,
}

impl WordDiffBufs {
    fn new() -> Self {
        Self {
            old_tokens: Vec::new(),
            new_tokens: Vec::new(),
            v: Vec::new(),
            trace: Vec::new(),
            keep_old: Vec::new(),
            keep_new: Vec::new(),
            old_ranges: Vec::new(),
            new_ranges: Vec::new(),
        }
    }
}

std::thread_local! {
    static WORD_DIFF_BUFS: std::cell::RefCell<WordDiffBufs> =
        std::cell::RefCell::new(WordDiffBufs::new());
}

/// Read trace value for depth `d` at diagonal `k`. Depth `d` stores `2d+1`
/// values (for `k` in `[-d, d]`) starting at flat offset `d*d`.
#[inline(always)]
fn trace_at(trace: &[isize], d: usize, k: isize) -> isize {
    trace[d * d + (k + d as isize) as usize]
}

#[inline(always)]
fn classify_byte(b: u8) -> u8 {
    if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
        0 // whitespace
    } else if b.is_ascii_alphanumeric() || b == b'_' {
        1 // word
    } else {
        2 // punctuation
    }
}

#[inline(always)]
fn token_range_eq(
    old_bytes: &[u8],
    old_range: &Range<usize>,
    new_bytes: &[u8],
    new_range: &Range<usize>,
) -> bool {
    old_bytes[old_range.start..old_range.end] == new_bytes[new_range.start..new_range.end]
}

#[inline(always)]
fn retreat_ascii_token_start(bytes: &[u8], mut ix: usize) -> usize {
    while ix > 0 {
        let prev_class = classify_byte(bytes[ix - 1]);
        let current_class = classify_byte(bytes[ix]);
        if prev_class == 0 || prev_class != current_class {
            break;
        }
        ix -= 1;
    }
    ix
}

#[inline(always)]
fn advance_ascii_token_end(bytes: &[u8], mut ix: usize) -> usize {
    while ix < bytes.len() {
        let prev_class = classify_byte(bytes[ix - 1]);
        let current_class = classify_byte(bytes[ix]);
        if current_class == 0 || prev_class != current_class {
            break;
        }
        ix += 1;
    }
    ix
}

#[inline(always)]
fn is_single_ascii_token_range(bytes: &[u8], range: Range<usize>) -> bool {
    if range.is_empty() {
        return true;
    }
    let class = classify_byte(bytes[range.start]);
    if class == 0 {
        return false;
    }
    bytes[range.start + 1..range.end]
        .iter()
        .all(|&byte| classify_byte(byte) == class)
}

fn ascii_single_token_diff_fast_ranges(
    old: &str,
    new: &str,
    bufs: &mut WordDiffBufs,
) -> Option<WordDiffRangePair> {
    if !old.is_ascii() || !new.is_ascii() || old.is_empty() || new.is_empty() {
        return None;
    }

    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();
    let mut prefix = 0usize;
    let shared_len = old_bytes.len().min(new_bytes.len());
    while prefix < shared_len && old_bytes[prefix] == new_bytes[prefix] {
        prefix += 1;
    }
    if prefix == old_bytes.len() && prefix == new_bytes.len() {
        return Some((Vec::new(), Vec::new()));
    }

    let mut old_end = old_bytes.len();
    let mut new_end = new_bytes.len();
    while old_end > prefix && new_end > prefix && old_bytes[old_end - 1] == new_bytes[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let mut old_start = prefix;
    if old_start < old_end {
        old_start = retreat_ascii_token_start(old_bytes, old_start);
        old_end = advance_ascii_token_end(old_bytes, old_end);
    }
    let mut new_start = prefix;
    if new_start < new_end {
        new_start = retreat_ascii_token_start(new_bytes, new_start);
        new_end = advance_ascii_token_end(new_bytes, new_end);
    }

    if !is_single_ascii_token_range(old_bytes, old_start..old_end)
        || !is_single_ascii_token_range(new_bytes, new_start..new_end)
    {
        return None;
    }

    bufs.old_ranges.clear();
    if old_start < old_end {
        bufs.old_ranges.push(old_start..old_end);
    }
    bufs.new_ranges.clear();
    if new_start < new_end {
        bufs.new_ranges.push(new_start..new_end);
    }
    Some((bufs.old_ranges.clone(), bufs.new_ranges.clone()))
}

fn push_all_non_whitespace_token_ranges(tokens: &[Token], out: &mut Vec<Range<usize>>) {
    out.clear();
    out.extend(
        tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Other)
            .map(|token| token.range.clone()),
    );
}

fn push_changed_token_ranges(tokens: &[Token], keep: &[bool], out: &mut Vec<Range<usize>>) {
    out.clear();
    out.extend(
        tokens
            .iter()
            .zip(keep.iter())
            .filter(|(token, is_kept)| token.kind == TokenKind::Other && !**is_kept)
            .map(|(token, _)| token.range.clone()),
    );
}

fn tokenize_for_word_diff_into(s: &str, max_tokens: usize, out: &mut Vec<Token>) {
    out.clear();
    if max_tokens == 0 {
        return;
    }

    let bytes = s.as_bytes();
    if s.is_ascii() {
        // Fast path: byte-level tokenization for ASCII strings.
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let start = i;
            let class = classify_byte(bytes[i]);
            let kind = if class == 0 {
                TokenKind::Whitespace
            } else {
                TokenKind::Other
            };
            i += 1;
            while i < len && classify_byte(bytes[i]) == class {
                i += 1;
            }
            out.push(Token {
                range: start..i,
                kind,
            });
            if out.len() >= max_tokens {
                return;
            }
        }
        return;
    }

    // Slow path: char-based tokenization for non-ASCII strings.
    fn classify_char(c: char) -> (u8, TokenKind) {
        if c.is_whitespace() {
            return (0, TokenKind::Whitespace);
        }
        if c.is_alphanumeric() || c == '_' {
            return (1, TokenKind::Other);
        }
        (2, TokenKind::Other)
    }

    let mut it = s.char_indices().peekable();
    while let Some((start, ch)) = it.next() {
        let (class, kind) = classify_char(ch);
        let mut end = start + ch.len_utf8();
        while let Some(&(next_start, next_ch)) = it.peek() {
            let (next_class, _) = classify_char(next_ch);
            if next_class != class {
                break;
            }
            it.next();
            end = next_start + next_ch.len_utf8();
        }
        out.push(Token {
            range: start..end,
            kind,
        });
        if out.len() >= max_tokens {
            return;
        }
    }
}

/// In-place coalescing: sorts `ranges`, deduplicates overlaps, returns a cloned Vec.
fn coalesce_ranges_in_place(ranges: &mut Vec<Range<usize>>) -> Vec<Range<usize>> {
    if ranges.len() <= 1 {
        return ranges.clone();
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut write = 0usize;
    for read in 1..ranges.len() {
        if ranges[read].start <= ranges[write].end {
            let new_end = ranges[read].end;
            ranges[write].end = ranges[write].end.max(new_end);
        } else {
            write += 1;
            ranges[write] = ranges[read].clone();
        }
    }
    ranges.truncate(write + 1);
    ranges.clone()
}

pub(super) fn word_diff_ranges(old: &str, new: &str) -> WordDiffRangePair {
    WORD_DIFF_BUFS.with(|cell| {
        let mut bufs = cell.borrow_mut();
        word_diff_ranges_with_bufs(old, new, &mut bufs)
    })
}

fn word_diff_ranges_with_bufs(old: &str, new: &str, bufs: &mut WordDiffBufs) -> WordDiffRangePair {
    const MAX_TOKENS: usize = 128;
    /// Maximum Myers edit depth before falling back to affix diff. When the
    /// edit distance exceeds this, the token-level diff is mostly noise rather
    /// than useful word highlighting. Capping at 48 bounds worst-case work per
    /// line pair to O(48 * 257) ≈ 12K operations instead of O(256²) ≈ 65K.
    const MAX_EDIT_DEPTH: usize = 48;
    if let Some(ranges) = ascii_single_token_diff_fast_ranges(old, new, bufs) {
        return ranges;
    }
    tokenize_for_word_diff_into(old, MAX_TOKENS + 1, &mut bufs.old_tokens);
    tokenize_for_word_diff_into(new, MAX_TOKENS + 1, &mut bufs.new_tokens);
    if bufs.old_tokens.len() > MAX_TOKENS || bufs.new_tokens.len() > MAX_TOKENS {
        return fallback_affix_diff_ranges(old, new);
    }
    if bufs.old_tokens.is_empty() || bufs.new_tokens.is_empty() {
        return fallback_affix_diff_ranges(old, new);
    }

    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    // Most edited code lines still share long token prefixes/suffixes. Trim
    // them so the Myers core only sees the changed middle.
    let mut prefix = 0usize;
    let shared_prefix_limit = bufs.old_tokens.len().min(bufs.new_tokens.len());
    while prefix < shared_prefix_limit
        && token_range_eq(
            old_bytes,
            &bufs.old_tokens[prefix].range,
            new_bytes,
            &bufs.new_tokens[prefix].range,
        )
    {
        prefix += 1;
    }

    let mut old_end = bufs.old_tokens.len();
    let mut new_end = bufs.new_tokens.len();
    while old_end > prefix
        && new_end > prefix
        && token_range_eq(
            old_bytes,
            &bufs.old_tokens[old_end - 1].range,
            new_bytes,
            &bufs.new_tokens[new_end - 1].range,
        )
    {
        old_end -= 1;
        new_end -= 1;
    }

    let old_tokens = &bufs.old_tokens[prefix..old_end];
    let new_tokens = &bufs.new_tokens[prefix..new_end];
    if old_tokens.is_empty() || new_tokens.is_empty() {
        push_all_non_whitespace_token_ranges(old_tokens, &mut bufs.old_ranges);
        push_all_non_whitespace_token_ranges(new_tokens, &mut bufs.new_ranges);
        return (
            coalesce_ranges_in_place(&mut bufs.old_ranges),
            coalesce_ranges_in_place(&mut bufs.new_ranges),
        );
    }

    let n = old_tokens.len() as isize;
    let m = new_tokens.len() as isize;
    let max = (n + m) as usize;
    let depth_limit = max.min(MAX_EDIT_DEPTH);
    let offset = max as isize;

    let Some(v_size) = max.checked_mul(2).and_then(|v| v.checked_add(1)) else {
        return fallback_affix_diff_ranges(old, new);
    };

    // Reuse v buffer.
    bufs.v.clear();
    bufs.v.resize(v_size, 0);
    let v = &mut bufs.v;

    // Flat trace buffer: depth d stores 2d+1 values starting at offset d*d.
    // Total for depth 0..D: (D+1)^2.
    bufs.trace.clear();

    let mut final_d = 0usize;
    let mut done = false;
    for d in 0..=depth_limit {
        for k in (-(d as isize)..=(d as isize)).step_by(2) {
            let k_ix = (k + offset) as usize;
            let x = if k == -(d as isize)
                || (k != d as isize && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
            {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };

            let mut x = x;
            let mut y = x - k;
            while x < n
                && y < m
                && token_range_eq(
                    old_bytes,
                    &old_tokens[x as usize].range,
                    new_bytes,
                    &new_tokens[y as usize].range,
                )
            {
                x += 1;
                y += 1;
            }

            v[k_ix] = x;
            if x >= n && y >= m {
                done = true;
                break;
            }
        }

        // Append this depth's v-values to the flat trace buffer.
        let trace = &mut bufs.trace;
        let d_isize = d as isize;
        for k in -d_isize..=d_isize {
            trace.push(v[(k + offset) as usize]);
        }
        final_d = d;
        if done {
            break;
        }
    }

    // If we hit the depth limit without finding the full edit path, the lines
    // are too dissimilar for useful word-level highlighting — fall back to the
    // cheaper prefix/suffix diff.
    if !done {
        return fallback_affix_diff_ranges(old, new);
    }

    // Backtrace to find kept tokens.
    bufs.keep_old.clear();
    bufs.keep_old.resize(bufs.old_tokens.len(), false);
    bufs.keep_new.clear();
    bufs.keep_new.resize(bufs.new_tokens.len(), false);

    let mut x = n;
    let mut y = m;

    for d in (1..=final_d).rev() {
        let d_isize = d as isize;
        // Read from trace at depth d-1.
        let prev_d = d - 1;
        let k = x - y;
        let prev_k = if k == -d_isize
            || (k != d_isize
                && trace_at(&bufs.trace, prev_d, k - 1) < trace_at(&bufs.trace, prev_d, k + 1))
        {
            k + 1
        } else {
            k - 1
        };

        let prev_x = trace_at(&bufs.trace, prev_d, prev_k);
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            bufs.keep_old[(x - 1) as usize] = true;
            bufs.keep_new[(y - 1) as usize] = true;
            x -= 1;
            y -= 1;
        }

        if x == prev_x {
            y -= 1;
        } else {
            x -= 1;
        }
    }

    while x > 0 && y > 0 {
        if !token_range_eq(
            old_bytes,
            &old_tokens[(x - 1) as usize].range,
            new_bytes,
            &new_tokens[(y - 1) as usize].range,
        ) {
            break;
        }
        bufs.keep_old[(x - 1) as usize] = true;
        bufs.keep_new[(y - 1) as usize] = true;
        x -= 1;
        y -= 1;
    }

    push_changed_token_ranges(old_tokens, &bufs.keep_old, &mut bufs.old_ranges);
    push_changed_token_ranges(new_tokens, &bufs.keep_new, &mut bufs.new_ranges);

    (
        coalesce_ranges_in_place(&mut bufs.old_ranges),
        coalesce_ranges_in_place(&mut bufs.new_ranges),
    )
}

pub(super) fn capped_word_diff_ranges(
    old: &str,
    new: &str,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old.len() > WORD_DIFF_MAX_BYTES_PER_SIDE
        || new.len() > WORD_DIFF_MAX_BYTES_PER_SIDE
        || old.len().saturating_add(new.len()) > WORD_DIFF_MAX_TOTAL_BYTES
    {
        return (Vec::new(), Vec::new());
    }
    word_diff_ranges(old, new)
}

fn fallback_affix_diff_ranges(old: &str, new: &str) -> WordDiffRangePair {
    let mut prefix = 0usize;
    for ((old_ix, old_ch), (_new_ix, new_ch)) in old.char_indices().zip(new.char_indices()) {
        if old_ch != new_ch {
            break;
        }
        prefix = old_ix + old_ch.len_utf8();
    }

    let mut suffix = 0usize;
    let old_tail = &old[prefix.min(old.len())..];
    let new_tail = &new[prefix.min(new.len())..];
    for (old_ch, new_ch) in old_tail.chars().rev().zip(new_tail.chars().rev()) {
        if old_ch != new_ch {
            break;
        }
        suffix += old_ch.len_utf8();
    }

    let old_mid_start = prefix.min(old.len());
    let old_mid_end = old.len().saturating_sub(suffix).max(old_mid_start);
    let new_mid_start = prefix.min(new.len());
    let new_mid_end = new.len().saturating_sub(suffix).max(new_mid_start);

    let old_ranges = if old_mid_end > old_mid_start {
        vec![Range {
            start: old_mid_start,
            end: old_mid_end,
        }]
    } else {
        Vec::new()
    };
    let new_ranges = if new_mid_end > new_mid_start {
        vec![Range {
            start: new_mid_start,
            end: new_mid_end,
        }]
    } else {
        Vec::new()
    };
    (old_ranges, new_ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn word_diff_ranges_highlights_changed_tokens() {
        let (old, new) = ("let x = 1;", "let x = 2;");
        let (old_ranges, new_ranges) = word_diff_ranges(old, new);
        assert_eq!(
            old_ranges
                .iter()
                .map(|r| &old[r.clone()])
                .collect::<Vec<_>>(),
            vec!["1"]
        );
        assert_eq!(
            new_ranges
                .iter()
                .map(|r| &new[r.clone()])
                .collect::<Vec<_>>(),
            vec!["2"]
        );
    }

    #[test]
    fn capped_word_diff_ranges_matches_word_diff_for_small_inputs() {
        let (old, new) = ("let x = 1;", "let x = 2;");
        let (a_old, a_new) = word_diff_ranges(old, new);
        let (b_old, b_new) = capped_word_diff_ranges(old, new);
        assert_eq!(a_old, b_old);
        assert_eq!(a_new, b_new);
    }

    #[test]
    fn capped_word_diff_ranges_skips_huge_inputs() {
        let old = "a".repeat(WORD_DIFF_MAX_TOTAL_BYTES + 1);
        let new = format!("{old}x");
        let (old_ranges, new_ranges) = capped_word_diff_ranges(&old, &new);
        assert!(old_ranges.is_empty());
        assert!(new_ranges.is_empty());
    }

    #[test]
    fn word_diff_ranges_handles_unicode_safely() {
        let (old, new) = ("aé", "aê");
        let (old_ranges, new_ranges) = word_diff_ranges(old, new);
        assert_eq!(
            old_ranges
                .iter()
                .map(|r| &old[r.clone()])
                .collect::<Vec<_>>(),
            vec!["aé"]
        );
        assert_eq!(
            new_ranges
                .iter()
                .map(|r| &new[r.clone()])
                .collect::<Vec<_>>(),
            vec!["aê"]
        );
    }

    #[test]
    fn word_diff_ranges_falls_back_for_large_inputs() {
        let old = "a".repeat(2048);
        let new = format!("{old}x");
        let (old_ranges, new_ranges) = word_diff_ranges(&old, &new);
        assert!(old_ranges.len() <= 1);
        assert!(new_ranges.len() <= 1);
    }

    #[test]
    fn word_diff_ranges_outputs_are_ordered_and_utf8_safe() {
        let (old, new) = ("aé b", "aê  b");
        let (old_ranges, new_ranges) = word_diff_ranges(old, new);

        for r in &old_ranges {
            assert!(r.start <= r.end);
            assert!(r.end <= old.len());
            assert!(old.is_char_boundary(r.start));
            assert!(old.is_char_boundary(r.end));
        }
        for w in old_ranges.windows(2) {
            assert!(w[0].end <= w[1].start);
        }

        for r in &new_ranges {
            assert!(r.start <= r.end);
            assert!(r.end <= new.len());
            assert!(new.is_char_boundary(r.start));
            assert!(new.is_char_boundary(r.end));
        }
        for w in new_ranges.windows(2) {
            assert!(w[0].end <= w[1].start);
        }
    }

    #[test]
    fn word_diff_ranges_empty_inputs_do_not_panic() {
        let (old_ranges, new_ranges) = word_diff_ranges("", "");
        assert!(old_ranges.is_empty());
        assert!(new_ranges.is_empty());
    }

    #[test]
    fn word_diff_ranges_many_edits_near_token_limit() {
        fn words(prefix: &str, count: usize) -> String {
            (0..count)
                .map(|ix| format!("{prefix}{ix}"))
                .collect::<Vec<_>>()
                .join(" ")
        }

        // 64 words produce 127 tokens (words + spaces), staying below the 128-token limit.
        // All 64 words differ → edit distance = 128 (delete 64 + insert 64), which exceeds
        // MAX_EDIT_DEPTH. The depth-bounded Myers falls back to affix diff, returning
        // one range covering the entire text (correct: the whole line is changed).
        let old = words("old", 64);
        let new = words("new", 64);
        let (old_ranges, new_ranges) = word_diff_ranges(&old, &new);

        // Affix fallback strips any common suffix (here: "63") and returns
        // one range covering the differing middle.
        assert_eq!(old_ranges.len(), 1, "affix fallback produces one range");
        assert_eq!(new_ranges.len(), 1, "affix fallback produces one range");
        assert!(old_ranges[0].start == 0);
        assert!(new_ranges[0].start == 0);
        // The exact end depends on common-suffix stripping, but both ranges
        // should cover most of the text.
        assert!(old_ranges[0].end > old.len() / 2);
        assert!(new_ranges[0].end > new.len() / 2);
    }

    #[test]
    fn word_diff_ranges_moderate_edits_within_depth_limit() {
        fn words(prefix: &str, count: usize) -> String {
            (0..count)
                .map(|ix| format!("{prefix}{ix}"))
                .collect::<Vec<_>>()
                .join(" ")
        }

        // 16 different words + 16 identical words: edit distance is small enough
        // that Myers completes within the depth limit.
        let old_part = words("old", 16);
        let shared_part = words("shared", 16);
        let old = format!("{old_part} {shared_part}");
        let new_part = words("new", 16);
        let new = format!("{new_part} {shared_part}");
        let (old_ranges, new_ranges) = word_diff_ranges(&old, &new);

        // Should produce 16 individual word ranges (the non-shared words).
        assert_eq!(old_ranges.len(), 16);
        assert_eq!(new_ranges.len(), 16);
        assert_eq!(&old[old_ranges[0].clone()], "old0");
        assert_eq!(&new[new_ranges[0].clone()], "new0");
    }

    #[test]
    fn word_diff_ranges_long_shared_affixes_stay_precise() {
        fn words(prefix: &str, count: usize) -> String {
            (0..count)
                .map(|ix| format!("{prefix}{ix}"))
                .collect::<Vec<_>>()
                .join(" ")
        }

        let shared_prefix = words("shared_prefix_", 24);
        let shared_suffix = words("shared_suffix_", 24);
        let old = format!("{shared_prefix} changed_old {shared_suffix}");
        let new = format!("{shared_prefix} changed_new {shared_suffix}");
        let (old_ranges, new_ranges) = word_diff_ranges(&old, &new);

        assert_eq!(old_ranges.len(), 1);
        assert_eq!(new_ranges.len(), 1);
        assert_eq!(&old[old_ranges[0].clone()], "changed_old");
        assert_eq!(&new[new_ranges[0].clone()], "changed_new");
    }

    #[test]
    fn word_diff_ranges_ignores_whitespace_only_edits() {
        let old = "let x = 1;";
        let new = "let  x = 1;";
        let (old_ranges, new_ranges) = word_diff_ranges(old, new);
        assert!(old_ranges.is_empty());
        assert!(new_ranges.is_empty());
    }

    #[test]
    #[ignore]
    fn perf_word_diff_ranges_smoke() {
        let old = "fn foo(a: i32, b: i32) -> i32 { a + b }";
        let new = "fn foo(a: i32, b: i32) -> i32 { a - b }";
        let start = Instant::now();
        for _ in 0..200_000 {
            let _ = word_diff_ranges(old, new);
        }
        eprintln!("word_diff_ranges: {:?}", start.elapsed());
    }
}
