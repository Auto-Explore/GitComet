//! Line edit scripts: histogram/patience and Myers with positional fallbacks.

use super::*;

/// Patience/histogram diff algorithm.
///
/// Uses unique lines as anchors via the longest increasing subsequence,
/// then recursively diffs the regions between anchors. Falls back to
/// Myers for regions with no unique lines. This produces cleaner diffs
/// for code with repetitive structural tokens (braces, returns, etc.)
/// by preferring semantically unique lines (function signatures) as
/// alignment points.
/// Maximum recursion depth for histogram/patience diff before falling back to Myers.
pub(super) const PATIENCE_MAX_DEPTH: usize = 32;

pub(crate) fn histogram_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    patience_recurse(old, new, 0, old.len(), 0, new.len(), 0)
}

pub(super) fn patience_recurse<'a>(
    old: &[&'a str],
    new: &[&'a str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    depth: usize,
) -> Vec<Edit<'a>> {
    // Fall back to Myers if recursion is too deep.
    if depth >= PATIENCE_MAX_DEPTH {
        return myers_edits(&old[old_start..old_end], &new[new_start..new_end]);
    }

    // Strip common prefix.
    let mut prefix = 0;
    while old_start + prefix < old_end
        && new_start + prefix < new_end
        && old[old_start + prefix] == new[new_start + prefix]
    {
        prefix += 1;
    }

    // Strip common suffix.
    let mut suffix = 0;
    while old_start + prefix + suffix < old_end
        && new_start + prefix + suffix < new_end
        && old[old_end - 1 - suffix] == new[new_end - 1 - suffix]
    {
        suffix += 1;
    }

    let inner_old_start = old_start + prefix;
    let inner_old_end = old_end - suffix;
    let inner_new_start = new_start + prefix;
    let inner_new_end = new_end - suffix;

    let mut edits = Vec::new();

    // Emit prefix equals.
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_start + i]),
            new: Some(new[new_start + i]),
        });
    }

    if inner_old_start == inner_old_end && inner_new_start == inner_new_end {
        // Nothing between prefix and suffix.
    } else if inner_old_start == inner_old_end {
        // Pure insertions.
        for &item in &new[inner_new_start..inner_new_end] {
            edits.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(item),
            });
        }
    } else if inner_new_start == inner_new_end {
        // Pure deletions.
        for &item in &old[inner_old_start..inner_old_end] {
            edits.push(Edit {
                kind: EditKind::Delete,
                old: Some(item),
                new: None,
            });
        }
    } else {
        // Find unique-line anchors via patience matching.
        let anchors = find_patience_anchors(
            old,
            new,
            inner_old_start,
            inner_old_end,
            inner_new_start,
            inner_new_end,
        );

        if anchors.is_empty() {
            let old_inner = &old[inner_old_start..inner_old_end];
            let new_inner = &new[inner_new_start..inner_new_end];
            // Large anchorless regions with matching line counts tend to be
            // structurally aligned already; preserve same-position context
            // lines linearly instead of paying for a full Myers trace.
            if should_use_patience_positional_fallback(old_inner, new_inner) {
                edits.extend(positional_fallback_edits(old_inner, new_inner));
            } else {
                // No unique anchors — fall back to Myers for this region.
                edits.extend(myers_edits(old_inner, new_inner));
            }
        } else {
            // Recursively diff between anchors.
            let mut oi = inner_old_start;
            let mut ni = inner_new_start;

            for &(old_idx, new_idx) in &anchors {
                if oi < old_idx || ni < new_idx {
                    edits.extend(patience_recurse(
                        old,
                        new,
                        oi,
                        old_idx,
                        ni,
                        new_idx,
                        depth + 1,
                    ));
                }
                edits.push(Edit {
                    kind: EditKind::Equal,
                    old: Some(old[old_idx]),
                    new: Some(new[new_idx]),
                });
                oi = old_idx + 1;
                ni = new_idx + 1;
            }

            // Region after the last anchor.
            if oi < inner_old_end || ni < inner_new_end {
                edits.extend(patience_recurse(
                    old,
                    new,
                    oi,
                    inner_old_end,
                    ni,
                    inner_new_end,
                    depth + 1,
                ));
            }
        }
    }

    // Emit suffix equals.
    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[inner_old_end + i]),
            new: Some(new[inner_new_end + i]),
        });
    }

    edits
}

pub(super) fn should_use_patience_positional_fallback(old: &[&str], new: &[&str]) -> bool {
    old.len() == new.len()
        && old.len().saturating_add(new.len()) >= PATIENCE_POSITIONAL_FALLBACK_LINE_THRESHOLD
}

/// Find lines that are unique in both old and new within the given ranges,
/// then compute the longest increasing subsequence of their positions to
/// produce patience anchors.
pub(super) fn find_patience_anchors(
    old: &[&str],
    new: &[&str],
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
) -> Vec<(usize, usize)> {
    // Count occurrences and record position for old lines.
    let mut old_info: FxHashMap<&str, (usize, usize)> = FxHashMap::default();
    for (i, &line) in old.iter().enumerate().take(old_end).skip(old_start) {
        let entry = old_info.entry(line).or_insert((0, i));
        entry.0 += 1;
        entry.1 = i;
    }

    // Count occurrences and record position for new lines.
    let mut new_info: FxHashMap<&str, (usize, usize)> = FxHashMap::default();
    for (j, &line) in new.iter().enumerate().take(new_end).skip(new_start) {
        let entry = new_info.entry(line).or_insert((0, j));
        entry.0 += 1;
        entry.1 = j;
    }

    // Collect lines that appear exactly once in both old and new.
    let mut unique_pairs: Vec<(usize, usize)> = Vec::new();
    for (line, &(old_count, old_idx)) in &old_info {
        if old_count != 1 {
            continue;
        }
        if let Some(&(new_count, new_idx)) = new_info.get(line)
            && new_count == 1
        {
            unique_pairs.push((old_idx, new_idx));
        }
    }

    // Sort by position in old; positions are unique, so stability is moot.
    unique_pairs.sort_unstable_by_key(|&(oi, _)| oi);

    // Find longest increasing subsequence by new-index.
    patience_lis(&unique_pairs)
}

/// Longest increasing subsequence by the second element (new-index).
pub(super) fn patience_lis(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }

    let n = pairs.len();
    // `tails[i]` stores the index in `pairs` of the smallest tail element
    // for an increasing subsequence of length `i+1`.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        let new_idx = pairs[i].1;
        let pos = tails.partition_point(|&t| pairs[t].1 < new_idx);
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
        if pos > 0 {
            prev[i] = Some(tails[pos - 1]);
        }
    }

    // Reconstruct.
    let mut result = Vec::with_capacity(tails.len());
    // SAFETY: loop above runs at least once (n >= 1) and always pushes to `tails`.
    let mut idx = *tails
        .last()
        .expect("tails is non-empty after processing pairs");
    loop {
        result.push(pairs[idx]);
        match prev[idx] {
            Some(p) => idx = p,
            None => break,
        }
    }
    result.reverse();
    result
}

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    // Keep row tokenization line-oriented; EOF newline deltas are annotated separately.
    text.lines().collect()
}

pub(super) fn eof_newline_delta(old_text: &str, new_text: &str) -> Option<FileDiffEofNewline> {
    let old_has_newline = old_text.ends_with('\n');
    let new_has_newline = new_text.ends_with('\n');
    match (old_has_newline, new_has_newline) {
        (false, true) => Some(FileDiffEofNewline::MissingInOld),
        (true, false) => Some(FileDiffEofNewline::MissingInNew),
        _ => None,
    }
}

pub(super) fn myers_fallback_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    // Keep fallback linear by only preserving common prefix/suffix and
    // representing the interior as delete/insert spans.
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old.len()
        && prefix + suffix < new.len()
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old.len().saturating_sub(suffix);
    let new_mid_end = new.len().saturating_sub(suffix);

    let mut edits = Vec::with_capacity(
        old_mid_end
            .saturating_add(new_mid_end)
            .saturating_sub(prefix)
            .saturating_add(suffix),
    );
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[i]),
            new: Some(new[i]),
        });
    }
    for &line in &old[prefix..old_mid_end] {
        edits.push(Edit {
            kind: EditKind::Delete,
            old: Some(line),
            new: None,
        });
    }
    for &line in &new[prefix..new_mid_end] {
        edits.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(line),
        });
    }
    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_mid_end + i]),
            new: Some(new[new_mid_end + i]),
        });
    }
    edits
}

pub(super) fn positional_fallback_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old.len()
        && prefix + suffix < new.len()
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old.len().saturating_sub(suffix);
    let new_mid_end = new.len().saturating_sub(suffix);
    let old_mid = &old[prefix..old_mid_end];
    let new_mid = &new[prefix..new_mid_end];
    let paired = old_mid.len().min(new_mid.len());

    let mut edits = Vec::with_capacity(
        old_mid_end
            .saturating_add(new_mid_end)
            .saturating_sub(prefix)
            .saturating_add(suffix),
    );
    for i in 0..prefix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[i]),
            new: Some(new[i]),
        });
    }

    for i in 0..paired {
        if old_mid[i] == new_mid[i] {
            edits.push(Edit {
                kind: EditKind::Equal,
                old: Some(old_mid[i]),
                new: Some(new_mid[i]),
            });
        } else {
            edits.push(Edit {
                kind: EditKind::Delete,
                old: Some(old_mid[i]),
                new: None,
            });
            edits.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(new_mid[i]),
            });
        }
    }

    for &line in &old_mid[paired..] {
        edits.push(Edit {
            kind: EditKind::Delete,
            old: Some(line),
            new: None,
        });
    }
    for &line in &new_mid[paired..] {
        edits.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(line),
        });
    }

    for i in 0..suffix {
        edits.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[old_mid_end + i]),
            new: Some(new[new_mid_end + i]),
        });
    }

    edits
}

pub(crate) fn myers_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    // An empty side has a fully determined edit script, so take GNU diff's
    // "handle simple cases" branch instead of searching for it. The search
    // would find the same answer at depth D = max(n, m) while storing a
    // D*(D+1)/2 trace, which is gigabytes for a file-sized deletion.
    if old.is_empty() || new.is_empty() {
        return myers_fallback_edits(old, new);
    }

    // Guard against overflow: if n + m exceeds isize::MAX, use linear fallback.
    let Some(sum) = old.len().checked_add(new.len()) else {
        return myers_fallback_edits(old, new);
    };
    if sum > isize::MAX as usize {
        return myers_fallback_edits(old, new);
    }
    if sum > u32::MAX as usize {
        return myers_fallback_edits(old, new);
    }

    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m) as usize;
    let offset = max as isize;

    let Some(v_size) = max.checked_mul(2).and_then(|v| v.checked_add(1)) else {
        return myers_fallback_edits(old, new);
    };
    let mut v = vec![0u32; v_size];

    // Compact trace: store only active diagonals per depth.
    // At depth d, active diagonals are -d, -d+2, ..., d (d+1 values).
    // Depth d starts at flat index d*(d+1)/2.
    // This eliminates per-depth Vec allocations and reduces trace memory from
    // d * (2*(n+m)+1) to d*(d+1)/2 elements.
    let mut trace = Vec::<u32>::new();

    {
        let mut x = 0isize;
        let mut y = 0isize;
        while x < n && y < m && old[x as usize] == new[y as usize] {
            x += 1;
            y += 1;
        }
        v[offset as usize] = x as u32;
    }
    // Store depth 0: only diagonal 0
    trace.push(v[offset as usize]);

    let mut last_d = 0usize;
    if v[offset as usize] >= n as u32 && v[offset as usize] >= m as u32 {
        last_d = 0;
    } else {
        'outer: for d in 1..=max {
            let d_isize = d as isize;

            // Update v in-place: at depth d, writes go to diagonals with
            // the same parity as d, reads come from the opposite parity
            // (depth d-1 values), so there is no aliasing.
            for k in (-d_isize..=d_isize).step_by(2) {
                let k_idx = (offset + k) as usize;

                let x = if k == -d_isize
                    || (k != d_isize && v[(offset + k - 1) as usize] < v[(offset + k + 1) as usize])
                {
                    v[(offset + k + 1) as usize]
                } else {
                    v[(offset + k - 1) as usize] + 1
                };

                let mut x = x as isize;
                let mut y = x - k;
                while x < n && y < m && old[x as usize] == new[y as usize] {
                    x += 1;
                    y += 1;
                }
                v[k_idx] = x as u32;

                if x >= n && y >= m {
                    for k2 in (-d_isize..=d_isize).step_by(2) {
                        trace.push(v[(offset + k2) as usize]);
                    }
                    last_d = d;
                    break 'outer;
                }
            }

            for k2 in (-d_isize..=d_isize).step_by(2) {
                trace.push(v[(offset + k2) as usize]);
            }
        }
    }

    if n == 0 && m == 0 {
        return Vec::new();
    }

    if last_d == 0 && n == m && v[offset as usize] == n as u32 {
        return old
            .iter()
            .map(|&s| Edit {
                kind: EditKind::Equal,
                old: Some(s),
                new: Some(s),
            })
            .collect();
    }

    let mut x = n;
    let mut y = m;
    let mut rev: Vec<Edit<'a>> = Vec::with_capacity(last_d + (n + m) as usize);

    for d in (1..=last_d).rev() {
        let prev_depth = d - 1;
        let d_isize = d as isize;
        let k = x - y;

        let prev_k = if k == -d_isize
            || (k != d_isize
                && compact_trace_get(&trace, prev_depth, k - 1)
                    < compact_trace_get(&trace, prev_depth, k + 1))
        {
            k + 1
        } else {
            k - 1
        };

        let prev_x = compact_trace_get(&trace, prev_depth, prev_k) as isize;
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            rev.push(Edit {
                kind: EditKind::Equal,
                old: Some(old[(x - 1) as usize]),
                new: Some(new[(y - 1) as usize]),
            });
            x -= 1;
            y -= 1;
        }

        if x == prev_x {
            rev.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(new[(y - 1) as usize]),
            });
            y -= 1;
        } else {
            rev.push(Edit {
                kind: EditKind::Delete,
                old: Some(old[(x - 1) as usize]),
                new: None,
            });
            x -= 1;
        }
    }

    while x > 0 && y > 0 {
        rev.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[(x - 1) as usize]),
            new: Some(new[(y - 1) as usize]),
        });
        x -= 1;
        y -= 1;
    }
    while x > 0 {
        rev.push(Edit {
            kind: EditKind::Delete,
            old: Some(old[(x - 1) as usize]),
            new: None,
        });
        x -= 1;
    }
    while y > 0 {
        rev.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(new[(y - 1) as usize]),
        });
        y -= 1;
    }

    rev.reverse();
    rev
}

/// Access diagonal `k` at `depth` in the compact trace buffer.
/// At depth d, active diagonals are -d, -d+2, ..., d (d+1 values).
/// Base offset = d*(d+1)/2, index within depth = (k+d)/2.
#[inline]
pub(super) fn compact_trace_get(trace: &[u32], depth: usize, k: isize) -> u32 {
    let d = depth as isize;
    let base = depth * (depth + 1) / 2;
    let idx = ((k + d) / 2) as usize;
    trace[base + idx]
}
