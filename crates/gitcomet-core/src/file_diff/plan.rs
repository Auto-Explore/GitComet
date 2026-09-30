//! Side-by-side planning: replacement alignment and plan construction.

use super::*;

/// Compact plan for a streamed side-by-side diff.
///
/// Runs carry only line-index spans into the old/new source documents. UI code
/// can materialize rows page-by-page without cloning the entire file into a
/// `Vec<FileDiffRow>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDiffPlanRun {
    Context {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
    Remove {
        old_start: usize,
        len: usize,
    },
    Add {
        new_start: usize,
        len: usize,
    },
    Modify {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
}

impl FileDiffPlanRun {
    pub fn row_len(&self) -> usize {
        match self {
            Self::Context { len, .. }
            | Self::Remove { len, .. }
            | Self::Add { len, .. }
            | Self::Modify { len, .. } => *len,
        }
    }

    pub fn inline_row_len(&self) -> usize {
        match self {
            Self::Modify { len, .. } => len.saturating_mul(2),
            _ => self.row_len(),
        }
    }

    pub fn kind(&self) -> FileDiffRowKind {
        match self {
            Self::Context { .. } => FileDiffRowKind::Context,
            Self::Remove { .. } => FileDiffRowKind::Remove,
            Self::Add { .. } => FileDiffRowKind::Add,
            Self::Modify { .. } => FileDiffRowKind::Modify,
        }
    }
}

/// Compact whole-file plan used by the streamed UI runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffPlan {
    pub runs: Vec<FileDiffPlanRun>,
    pub row_count: usize,
    pub inline_row_count: usize,
    pub eof_newline: Option<FileDiffEofNewline>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditKind {
    Equal,
    Insert,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Edit<'a> {
    pub(crate) kind: EditKind,
    pub(crate) old: Option<&'a str>,
    pub(crate) new: Option<&'a str>,
}

/// A contiguous edit span relative to base lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiffHunk<T> {
    pub(crate) base_start: usize,
    pub(crate) base_end: usize,
    pub(crate) new_lines: Vec<T>,
}

/// Convert an edit script into base-relative change hunks.
pub(crate) fn edits_to_hunks_with<'a, T, F>(
    edits: &[Edit<'a>],
    mut map_insert: F,
) -> Vec<DiffHunk<T>>
where
    F: FnMut(&'a str) -> T,
{
    let mut hunks = Vec::new();
    let mut base_ix = 0usize;
    let mut i = 0usize;

    while i < edits.len() {
        if edits[i].kind == EditKind::Equal {
            base_ix += 1;
            i += 1;
            continue;
        }

        let hunk_base_start = base_ix;
        let mut new_lines = Vec::new();

        while i < edits.len() && edits[i].kind != EditKind::Equal {
            match edits[i].kind {
                EditKind::Delete => {
                    base_ix += 1;
                }
                EditKind::Insert => {
                    new_lines.push(map_insert(edits[i].new.unwrap_or_default()));
                }
                EditKind::Equal => unreachable!(),
            }
            i += 1;
        }

        hunks.push(DiffHunk {
            base_start: hunk_base_start,
            base_end: base_ix,
            new_lines,
        });
    }

    hunks
}

/// Reconstruct one side's sequence for a base range by applying hunks.
pub(crate) fn reconstruct_side_with<'a, T, FBase>(
    base_lines: &'a [&'a str],
    range: std::ops::Range<usize>,
    hunks: &[DiffHunk<T>],
    output: &mut Vec<T>,
    mut map_base_line: FBase,
) where
    T: Clone,
    FBase: FnMut(&'a str) -> T,
{
    let range_end = range.end.min(base_lines.len());
    let mut pos = range.start.min(range_end);

    for hunk in hunks {
        let base_limit = hunk.base_start.min(range_end).max(pos);
        for &line in &base_lines[pos..base_limit] {
            output.push(map_base_line(line));
        }
        output.extend(hunk.new_lines.iter().cloned());
        pos = hunk.base_end.min(range_end).max(pos);
    }

    for &line in &base_lines[pos..range_end] {
        output.push(map_base_line(line));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplacementAlignStep {
    None,
    Pair,
    Delete,
    Insert,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BenchmarkReplacementDistanceBackend {
    Scratch,
    Strsim,
}

pub fn side_by_side_plan(old: &str, new: &str) -> FileDiffPlan {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    side_by_side_plan_from_lines(old, new, old_lines.as_slice(), new_lines.as_slice())
}

/// Build a side-by-side diff plan from precomputed `str::lines()` slices.
///
/// Callers must ensure `old_lines` and `new_lines` were derived from
/// `old_text`/`new_text` using the same line-splitting semantics as
/// [`str::lines`], because EOF newline handling still comes from the full
/// source texts.
pub fn side_by_side_plan_from_lines(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
) -> FileDiffPlan {
    build_side_by_side_plan_with_pair_cost(
        old_text,
        new_text,
        old_lines,
        new_lines,
        replacement_pair_cost,
    )
}

#[cfg(feature = "benchmarks")]
pub fn benchmark_side_by_side_plan_with_replacement_backend(
    old: &str,
    new: &str,
    backend: BenchmarkReplacementDistanceBackend,
) -> FileDiffPlan {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    match backend {
        BenchmarkReplacementDistanceBackend::Scratch => build_side_by_side_plan_with_pair_cost(
            old,
            new,
            old_lines.as_slice(),
            new_lines.as_slice(),
            replacement_pair_cost_with_scratch,
        ),
        BenchmarkReplacementDistanceBackend::Strsim => build_side_by_side_plan_with_pair_cost(
            old,
            new,
            old_lines.as_slice(),
            new_lines.as_slice(),
            replacement_pair_cost_with_strsim,
        ),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PlannedReplacementOp {
    Pair,
    Delete,
    Insert,
}

pub(super) struct PreparedReplacementLine<'a> {
    pub(super) text: &'a str,
    pub(super) ascii_bytes: Option<&'a [u8]>,
    pub(super) chars: OnceCell<Box<[char]>>,
}

impl<'a> PreparedReplacementLine<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        if text.is_ascii() {
            Self {
                text,
                ascii_bytes: Some(text.as_bytes()),
                chars: OnceCell::new(),
            }
        } else {
            let prepared_chars = text.chars().collect::<Vec<_>>().into_boxed_slice();
            let chars = OnceCell::new();
            assert!(
                chars.set(prepared_chars).is_ok(),
                "fresh OnceCell should accept prepared chars"
            );
            Self {
                text,
                ascii_bytes: None,
                chars,
            }
        }
    }

    pub(super) fn ascii_bytes(&self) -> Option<&[u8]> {
        self.ascii_bytes
    }

    pub(super) fn chars(&self) -> &[char] {
        self.chars
            .get_or_init(|| self.text.chars().collect::<Vec<_>>().into_boxed_slice())
            .as_ref()
    }
}

// Calls to `strsim::generic_levenshtein` with these wrappers need explicit type
// args: on macOS, objc2's recursive `IntoIterator for &Retained<T>` impl
// overflows inference (E0275).
#[cfg(feature = "benchmarks")]
pub(super) struct CharSlice<'a>(pub(super) &'a [char]);

#[cfg(feature = "benchmarks")]
impl<'b> IntoIterator for &CharSlice<'b> {
    type Item = char;
    type IntoIter = std::iter::Copied<std::slice::Iter<'b, char>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().copied()
    }
}

#[cfg(all(feature = "benchmarks", test))]
pub(super) struct ByteSlice<'a>(pub(super) &'a [u8]);

#[cfg(all(feature = "benchmarks", test))]
impl<'b> IntoIterator for &ByteSlice<'b> {
    type Item = u8;
    type IntoIter = std::iter::Copied<std::slice::Iter<'b, u8>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().copied()
    }
}

#[derive(Default)]
pub(super) struct LevenshteinScratch {
    pub(super) cache: Vec<usize>,
}

impl LevenshteinScratch {
    pub(super) fn distance<T: Eq>(&mut self, a: &[T], b: &[T]) -> usize {
        if a == b {
            return 0;
        }
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }

        self.distance_non_empty_unequal(a, b)
    }

    pub(super) fn distance_non_empty_unequal<T: Eq>(&mut self, a: &[T], b: &[T]) -> usize {
        distance_non_empty_unequal_with_cache(&mut self.cache, a, b)
    }

    pub(super) fn distance_bytes(&mut self, a: &[u8], b: &[u8]) -> usize {
        if a == b {
            return 0;
        }
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }

        if let Some(distance) = bitparallel_levenshtein_bytes(a, b) {
            return distance;
        }

        distance_non_empty_unequal_with_cache(&mut self.cache, a, b)
    }
}

pub(super) fn distance_non_empty_unequal_with_cache<T: Eq>(
    cache: &mut Vec<usize>,
    a: &[T],
    b: &[T],
) -> usize {
    debug_assert!(!a.is_empty());
    debug_assert!(!b.is_empty());

    let (a, b) = if b.len() > a.len() { (a, b) } else { (b, a) };
    debug_assert!(a != b);

    let b_len = b.len();
    cache.resize(b_len, 0);
    let cache = &mut cache[..b_len];
    for (ix, slot) in cache.iter_mut().enumerate() {
        *slot = ix + 1;
    }

    let mut result = b_len;
    for (i, a_ch) in a.iter().enumerate() {
        result = i + 1;
        let mut distance_b = i;
        for (j, b_ch) in b.iter().enumerate() {
            let cost = usize::from(a_ch != b_ch);
            let distance_a = distance_b + cost;
            distance_b = cache[j];
            result = (result + 1).min(distance_a).min(distance_b + 1);
            cache[j] = result;
        }
    }

    result
}

pub(super) fn bitparallel_levenshtein_bytes(a: &[u8], b: &[u8]) -> Option<usize> {
    let (pattern, text) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let pattern_len = pattern.len();
    if pattern_len == 0 {
        return Some(text.len());
    }
    if pattern_len > ASCII_BITPARALLEL_MAX_PATTERN_LEN {
        return None;
    }

    let mut eq_masks = [0u128; 256];
    for (ix, &byte) in pattern.iter().enumerate() {
        eq_masks[byte as usize] |= 1u128 << ix;
    }

    let mask = if pattern_len == ASCII_BITPARALLEL_MAX_PATTERN_LEN {
        u128::MAX
    } else {
        (1u128 << pattern_len) - 1
    };
    let high_bit = 1u128 << (pattern_len - 1);
    let mut positive = mask;
    let mut negative = 0u128;
    let mut distance = pattern_len;

    for &byte in text {
        let eq = eq_masks[byte as usize];
        let xv = eq | negative;
        let xh = (((eq & positive).wrapping_add(positive)) ^ positive) | eq;
        let mut positive_h = negative | !(xh | positive);
        let mut negative_h = positive & xh;

        if (positive_h & high_bit) != 0 {
            distance += 1;
        } else if (negative_h & high_bit) != 0 {
            distance -= 1;
        }

        positive_h = ((positive_h << 1) | 1) & mask;
        negative_h = (negative_h << 1) & mask;
        positive = (negative_h | !(xv | positive_h)) & mask;
        negative = positive_h & xv;
    }

    Some(distance)
}

pub(super) fn prepare_replacement_lines<'a>(lines: &[&'a str]) -> Vec<PreparedReplacementLine<'a>> {
    lines
        .iter()
        .map(|line| PreparedReplacementLine::new(line))
        .collect()
}

pub(super) struct ReplacementTextCacheIds {
    pub(super) ids: Vec<usize>,
    pub(super) unique_texts: usize,
    pub(super) has_duplicates: bool,
}

pub(super) fn prepare_replacement_text_cache_ids(
    lines: &[PreparedReplacementLine<'_>],
) -> ReplacementTextCacheIds {
    let mut text_ids = FxHashMap::default();
    text_ids.reserve(lines.len());
    let mut ids = Vec::with_capacity(lines.len());

    for line in lines {
        let next_id = text_ids.len();
        let id = *text_ids.entry(line.text).or_insert(next_id);
        ids.push(id);
    }

    let unique_texts = text_ids.len();
    ReplacementTextCacheIds {
        ids,
        unique_texts,
        has_duplicates: unique_texts < lines.len(),
    }
}

#[cfg(test)]
pub(super) fn replacement_alignment_ops(
    deletes: &[PreparedReplacementLine<'_>],
    inserts: &[PreparedReplacementLine<'_>],
) -> Vec<PlannedReplacementOp> {
    replacement_alignment_ops_with_pair_cost(deletes, inserts, replacement_pair_cost)
}

pub(super) fn replacement_alignment_ops_with_pair_cost<'a, F>(
    deletes: &[PreparedReplacementLine<'a>],
    inserts: &[PreparedReplacementLine<'a>],
    pair_cost_fn: F,
) -> Vec<PlannedReplacementOp>
where
    F: Copy
        + Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let n = deletes.len();
    let m = inserts.len();
    let width = m + 1;
    let mut prev_costs = vec![0; width];
    let mut curr_costs = vec![0; width];
    let mut step = vec![ReplacementAlignStep::None; (n + 1) * width];
    #[allow(clippy::default_constructed_unit_structs)]
    let mut scratch = LevenshteinScratch::default();
    let delete_text_ids = prepare_replacement_text_cache_ids(deletes);
    let insert_text_ids = prepare_replacement_text_cache_ids(inserts);
    // Cache pair costs only when either side actually repeats line text within
    // this replacement block; otherwise every pair is unique and the cache
    // adds extra hashing/allocation work without any reuse.
    let mut pair_cost_cache = (delete_text_ids.has_duplicates || insert_text_ids.has_duplicates)
        .then(|| vec![u32::MAX; delete_text_ids.unique_texts * insert_text_ids.unique_texts]);
    for j in 1..=m {
        prev_costs[j] = (j as u32) * REPLACEMENT_GAP_COST;
        step[j] = ReplacementAlignStep::Insert;
    }

    for i in 1..=n {
        curr_costs[0] = (i as u32) * REPLACEMENT_GAP_COST;
        step[i * width] = ReplacementAlignStep::Delete;
        for j in 1..=m {
            let idx = i * width + j;
            let pair_cost_value = if let Some(pair_cost_cache) = pair_cost_cache.as_mut() {
                let pair_cache_idx = delete_text_ids.ids[i - 1] * insert_text_ids.unique_texts
                    + insert_text_ids.ids[j - 1];
                let cached_pair_cost = &mut pair_cost_cache[pair_cache_idx];
                if *cached_pair_cost == u32::MAX {
                    let computed = pair_cost_fn(&deletes[i - 1], &inserts[j - 1], &mut scratch);
                    *cached_pair_cost = computed;
                    computed
                } else {
                    *cached_pair_cost
                }
            } else {
                pair_cost_fn(&deletes[i - 1], &inserts[j - 1], &mut scratch)
            };
            let pair_cost = prev_costs[j - 1].saturating_add(pair_cost_value);
            let insert_cost = curr_costs[j - 1].saturating_add(REPLACEMENT_GAP_COST);
            let delete_cost = prev_costs[j].saturating_add(REPLACEMENT_GAP_COST);

            let mut best_cost = pair_cost;
            let mut best_step = ReplacementAlignStep::Pair;

            if insert_cost < best_cost {
                best_cost = insert_cost;
                best_step = ReplacementAlignStep::Insert;
            }
            if delete_cost < best_cost {
                best_cost = delete_cost;
                best_step = ReplacementAlignStep::Delete;
            }

            curr_costs[j] = best_cost;
            step[idx] = best_step;
        }
        std::mem::swap(&mut prev_costs, &mut curr_costs);
    }

    let mut i = n;
    let mut j = m;
    let mut aligned_rev = Vec::with_capacity(n + m);
    while i > 0 || j > 0 {
        let idx = i * width + j;
        match step[idx] {
            ReplacementAlignStep::Pair if i > 0 && j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Pair);
                i -= 1;
                j -= 1;
            }
            ReplacementAlignStep::Insert if j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Insert);
                j -= 1;
            }
            ReplacementAlignStep::Delete if i > 0 => {
                aligned_rev.push(PlannedReplacementOp::Delete);
                i -= 1;
            }
            _ if j > 0 => {
                aligned_rev.push(PlannedReplacementOp::Insert);
                j -= 1;
            }
            _ if i > 0 => {
                aligned_rev.push(PlannedReplacementOp::Delete);
                i -= 1;
            }
            _ => break,
        }
    }

    aligned_rev.reverse();
    aligned_rev
}

pub(super) fn select_side_by_side_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    let combined = old.len().saturating_add(new.len());
    if combined >= SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD {
        return myers_fallback_edits(old, new);
    }
    if combined >= SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD {
        return histogram_edits(old, new);
    }
    myers_edits(old, new)
}

pub(super) fn push_plan_run(runs: &mut Vec<FileDiffPlanRun>, run: FileDiffPlanRun) {
    let len = run.row_len();
    if len == 0 {
        return;
    }

    let merged = match (runs.last_mut(), &run) {
        (
            Some(FileDiffPlanRun::Context {
                old_start: last_old_start,
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            },
        ) if last_old_start.saturating_add(*last_len) == *old_start
            && last_new_start.saturating_add(*last_len) == *new_start =>
        {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Remove {
                old_start: last_old_start,
                len: last_len,
            }),
            FileDiffPlanRun::Remove { old_start, len },
        ) if last_old_start.saturating_add(*last_len) == *old_start => {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Add {
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Add { new_start, len },
        ) if last_new_start.saturating_add(*last_len) == *new_start => {
            *last_len = last_len.saturating_add(*len);
            true
        }
        (
            Some(FileDiffPlanRun::Modify {
                old_start: last_old_start,
                new_start: last_new_start,
                len: last_len,
            }),
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            },
        ) if last_old_start.saturating_add(*last_len) == *old_start
            && last_new_start.saturating_add(*last_len) == *new_start =>
        {
            *last_len = last_len.saturating_add(*len);
            true
        }
        _ => false,
    };

    if !merged {
        runs.push(run);
    }
}

pub(super) fn push_plan_run_with_counts(
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
    run: FileDiffPlanRun,
) {
    let len = run.row_len();
    if len == 0 {
        return;
    }
    *row_count = row_count.saturating_add(len);
    *inline_row_count = inline_row_count.saturating_add(run.inline_row_len());
    push_plan_run(runs, run);
}

pub(super) fn apply_eof_newline_to_plan(
    runs: &mut Vec<FileDiffPlanRun>,
    eof_newline: Option<FileDiffEofNewline>,
) -> bool {
    if eof_newline.is_none() {
        return false;
    }

    let Some(last_run) = runs.pop() else {
        return false;
    };
    match last_run {
        FileDiffPlanRun::Context {
            old_start,
            new_start,
            len,
        } if len > 1 => {
            runs.push(FileDiffPlanRun::Context {
                old_start,
                new_start,
                len: len.saturating_sub(1),
            });
            runs.push(FileDiffPlanRun::Modify {
                old_start: old_start.saturating_add(len.saturating_sub(1)),
                new_start: new_start.saturating_add(len.saturating_sub(1)),
                len: 1,
            });
            true
        }
        FileDiffPlanRun::Context {
            old_start,
            new_start,
            ..
        } => {
            runs.push(FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len: 1,
            });
            true
        }
        other => {
            runs.push(other);
            false
        }
    }
}

pub(super) fn build_sparse_positional_side_by_side_plan(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
) -> Option<FileDiffPlan> {
    if old_lines.len() != new_lines.len() {
        return None;
    }
    let line_count = old_lines.len();
    if line_count == 0
        || line_count.saturating_add(line_count) < SIDE_BY_SIDE_HISTOGRAM_LINE_THRESHOLD
    {
        return None;
    }

    let mut runs = Vec::with_capacity(line_count / 8 + 1);
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;
    let mut changed_lines = 0usize;
    let mut ix = 0usize;

    while ix < line_count {
        if old_lines[ix] == new_lines[ix] {
            let start = ix;
            ix += 1;
            while ix < line_count && old_lines[ix] == new_lines[ix] {
                ix += 1;
            }
            push_plan_run_with_counts(
                &mut runs,
                &mut row_count,
                &mut inline_row_count,
                FileDiffPlanRun::Context {
                    old_start: start,
                    new_start: start,
                    len: ix.saturating_sub(start),
                },
            );
            continue;
        }

        let block_start = ix;
        ix += 1;
        while ix < line_count && old_lines[ix] != new_lines[ix] {
            ix += 1;
        }
        let block_len = ix.saturating_sub(block_start);
        changed_lines = changed_lines.saturating_add(block_len);
        if block_len > SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_BLOCK_LEN
            || changed_lines
                .saturating_mul(SIDE_BY_SIDE_SPARSE_POSITIONAL_MAX_CHANGED_RATIO_DENOMINATOR)
                > line_count
        {
            return None;
        }

        push_plan_run_with_counts(
            &mut runs,
            &mut row_count,
            &mut inline_row_count,
            FileDiffPlanRun::Modify {
                old_start: block_start,
                new_start: block_start,
                len: block_len,
            },
        );
    }

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }

    Some(FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    })
}

pub(super) fn push_paired_replacement_runs_by_position_to_plan(
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
) {
    let paired = old_len.min(new_len);
    if paired > 0 {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len: paired,
            },
        );
    }
    if old_len > paired {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Remove {
                old_start: old_start.saturating_add(paired),
                len: old_len.saturating_sub(paired),
            },
        );
    }
    if new_len > paired {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Add {
                new_start: new_start.saturating_add(paired),
                len: new_len.saturating_sub(paired),
            },
        );
    }
}

pub(super) fn push_aligned_replacement_runs_to_plan_with_pair_cost<F>(
    old_lines: &[&str],
    new_lines: &[&str],
    old_range: std::ops::Range<usize>,
    new_range: std::ops::Range<usize>,
    runs: &mut Vec<FileDiffPlanRun>,
    row_count: &mut usize,
    inline_row_count: &mut usize,
    pair_cost_fn: F,
) where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let old_start = old_range.start;
    let new_start = new_range.start;
    let deletes = &old_lines[old_range.start..old_range.end];
    let inserts = &new_lines[new_range.start..new_range.end];

    if deletes.is_empty() {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Add {
                new_start,
                len: inserts.len(),
            },
        );
        return;
    }
    if inserts.is_empty() {
        push_plan_run_with_counts(
            runs,
            row_count,
            inline_row_count,
            FileDiffPlanRun::Remove {
                old_start,
                len: deletes.len(),
            },
        );
        return;
    }

    if deletes.len().saturating_mul(inserts.len()) > REPLACEMENT_ALIGN_CELL_BUDGET {
        push_paired_replacement_runs_by_position_to_plan(
            old_start,
            deletes.len(),
            new_start,
            inserts.len(),
            runs,
            row_count,
            inline_row_count,
        );
        return;
    }

    let deletes = prepare_replacement_lines(deletes);
    let inserts = prepare_replacement_lines(inserts);

    let mut local_old = 0usize;
    let mut local_new = 0usize;
    for op in replacement_alignment_ops_with_pair_cost(&deletes, &inserts, pair_cost_fn) {
        match op {
            PlannedReplacementOp::Pair => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Modify {
                        old_start: old_start.saturating_add(local_old),
                        new_start: new_start.saturating_add(local_new),
                        len: 1,
                    },
                );
                local_old += 1;
                local_new += 1;
            }
            PlannedReplacementOp::Delete => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Remove {
                        old_start: old_start.saturating_add(local_old),
                        len: 1,
                    },
                );
                local_old += 1;
            }
            PlannedReplacementOp::Insert => {
                push_plan_run_with_counts(
                    runs,
                    row_count,
                    inline_row_count,
                    FileDiffPlanRun::Add {
                        new_start: new_start.saturating_add(local_new),
                        len: 1,
                    },
                );
                local_new += 1;
            }
        }
    }
}

pub(super) fn build_linear_fallback_side_by_side_plan_with_pair_cost<F>(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
    pair_cost_fn: F,
) -> FileDiffPlan
where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old_lines.len()
        && prefix + suffix < new_lines.len()
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid_end = old_lines.len().saturating_sub(suffix);
    let new_mid_end = new_lines.len().saturating_sub(suffix);
    let mut runs = Vec::with_capacity(3);
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;

    push_plan_run_with_counts(
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        FileDiffPlanRun::Context {
            old_start: 0,
            new_start: 0,
            len: prefix,
        },
    );

    push_aligned_replacement_runs_to_plan_with_pair_cost(
        old_lines,
        new_lines,
        prefix..old_mid_end,
        prefix..new_mid_end,
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        pair_cost_fn,
    );

    push_plan_run_with_counts(
        &mut runs,
        &mut row_count,
        &mut inline_row_count,
        FileDiffPlanRun::Context {
            old_start: old_mid_end,
            new_start: new_mid_end,
            len: suffix,
        },
    );

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }
    FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    }
}

pub(super) fn build_side_by_side_plan_with_pair_cost<F>(
    old_text: &str,
    new_text: &str,
    old_lines: &[&str],
    new_lines: &[&str],
    pair_cost_fn: F,
) -> FileDiffPlan
where
    F: Copy
        + for<'a> Fn(
            &PreparedReplacementLine<'a>,
            &PreparedReplacementLine<'a>,
            &mut LevenshteinScratch,
        ) -> u32,
{
    if old_lines.len().saturating_add(new_lines.len())
        >= SIDE_BY_SIDE_LINEAR_FALLBACK_LINE_THRESHOLD
    {
        return build_linear_fallback_side_by_side_plan_with_pair_cost(
            old_text,
            new_text,
            old_lines,
            new_lines,
            pair_cost_fn,
        );
    }
    if let Some(plan) =
        build_sparse_positional_side_by_side_plan(old_text, new_text, old_lines, new_lines)
    {
        return plan;
    }

    let edits = select_side_by_side_edits(old_lines, new_lines);
    let mut runs = Vec::with_capacity(edits.len());
    let mut row_count = 0usize;
    let mut inline_row_count = 0usize;
    let mut old_ix = 0usize;
    let mut new_ix = 0usize;
    let mut i = 0usize;

    while i < edits.len() {
        match edits[i].kind {
            EditKind::Equal => {
                let run_old_start = old_ix;
                let run_new_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Equal {
                    old_ix += 1;
                    new_ix += 1;
                    i += 1;
                }
                push_plan_run_with_counts(
                    &mut runs,
                    &mut row_count,
                    &mut inline_row_count,
                    FileDiffPlanRun::Context {
                        old_start: run_old_start,
                        new_start: run_new_start,
                        len: old_ix.saturating_sub(run_old_start),
                    },
                );
            }
            EditKind::Delete => {
                let delete_start = old_ix;
                while i < edits.len() && edits[i].kind == EditKind::Delete {
                    old_ix += 1;
                    i += 1;
                }

                let insert_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Insert {
                    new_ix += 1;
                    i += 1;
                }

                if insert_start == new_ix {
                    push_plan_run_with_counts(
                        &mut runs,
                        &mut row_count,
                        &mut inline_row_count,
                        FileDiffPlanRun::Remove {
                            old_start: delete_start,
                            len: old_ix.saturating_sub(delete_start),
                        },
                    );
                } else {
                    push_aligned_replacement_runs_to_plan_with_pair_cost(
                        old_lines,
                        new_lines,
                        delete_start..old_ix,
                        insert_start..new_ix,
                        &mut runs,
                        &mut row_count,
                        &mut inline_row_count,
                        pair_cost_fn,
                    );
                }
            }
            EditKind::Insert => {
                let insert_start = new_ix;
                while i < edits.len() && edits[i].kind == EditKind::Insert {
                    new_ix += 1;
                    i += 1;
                }
                push_plan_run_with_counts(
                    &mut runs,
                    &mut row_count,
                    &mut inline_row_count,
                    FileDiffPlanRun::Add {
                        new_start: insert_start,
                        len: new_ix.saturating_sub(insert_start),
                    },
                );
            }
        }
    }

    let eof_newline = eof_newline_delta(old_text, new_text);
    if apply_eof_newline_to_plan(&mut runs, eof_newline) {
        inline_row_count = inline_row_count.saturating_add(1);
    }
    FileDiffPlan {
        runs,
        row_count,
        inline_row_count,
        eof_newline,
    }
}

#[cfg(test)]
pub(super) fn pair_replacements(rows: Vec<FileDiffRow>) -> Vec<FileDiffRow> {
    let mut out = Vec::with_capacity(rows.len());
    let mut ix = 0usize;

    while ix < rows.len() {
        if rows[ix].kind != FileDiffRowKind::Remove {
            out.push(rows[ix].clone());
            ix += 1;
            continue;
        }

        let del_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Remove {
            ix += 1;
        }
        let del_end = ix;

        let ins_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Add {
            ix += 1;
        }
        let ins_end = ix;

        if ins_start == ins_end {
            out.extend(rows[del_start..del_end].iter().cloned());
            continue;
        }

        out.extend(align_replacement_runs(
            &rows[del_start..del_end],
            &rows[ins_start..ins_end],
        ));
    }

    out
}

#[cfg(test)]
pub(super) fn align_replacement_runs(
    deletes: &[FileDiffRow],
    inserts: &[FileDiffRow],
) -> Vec<FileDiffRow> {
    if deletes.is_empty() {
        return inserts.to_vec();
    }
    if inserts.is_empty() {
        return deletes.to_vec();
    }

    if deletes.len().saturating_mul(inserts.len()) > REPLACEMENT_ALIGN_CELL_BUDGET {
        return pair_replacement_runs_by_position(deletes, inserts);
    }

    let delete_meta: Vec<_> = deletes
        .iter()
        .map(|row| PreparedReplacementLine::new(row.old.as_deref().unwrap_or_default()))
        .collect();
    let insert_meta: Vec<_> = inserts
        .iter()
        .map(|row| PreparedReplacementLine::new(row.new.as_deref().unwrap_or_default()))
        .collect();

    let mut out = Vec::with_capacity(deletes.len() + inserts.len());
    let mut delete_ix = 0usize;
    let mut insert_ix = 0usize;
    for op in replacement_alignment_ops(&delete_meta, &insert_meta) {
        match op {
            PlannedReplacementOp::Pair => {
                out.push(make_modify_row(&deletes[delete_ix], &inserts[insert_ix]));
                delete_ix += 1;
                insert_ix += 1;
            }
            PlannedReplacementOp::Insert => {
                out.push(inserts[insert_ix].clone());
                insert_ix += 1;
            }
            PlannedReplacementOp::Delete => {
                out.push(deletes[delete_ix].clone());
                delete_ix += 1;
            }
        }
    }

    out
}

#[cfg(test)]
pub(super) fn pair_replacement_runs_by_position(
    deletes: &[FileDiffRow],
    inserts: &[FileDiffRow],
) -> Vec<FileDiffRow> {
    let paired = deletes.len().min(inserts.len());
    let mut out = Vec::with_capacity(deletes.len() + inserts.len());

    for i in 0..paired {
        out.push(make_modify_row(&deletes[i], &inserts[i]));
    }
    if deletes.len() > paired {
        out.extend(deletes[paired..].iter().cloned());
    }
    if inserts.len() > paired {
        out.extend(inserts[paired..].iter().cloned());
    }
    out
}

#[cfg(test)]
pub(super) fn make_modify_row(delete: &FileDiffRow, insert: &FileDiffRow) -> FileDiffRow {
    FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: delete.old_line,
        new_line: insert.new_line,
        old: delete.old.clone(),
        new: insert.new.clone(),
        eof_newline: None,
    }
}

pub(super) fn replacement_pair_cost(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    if old.text == new.text {
        return 0;
    }

    if let (Some(old_bytes), Some(new_bytes)) = (old.ascii_bytes(), new.ascii_bytes()) {
        let (shared_prefix, shared_suffix) = shared_boundary_bytes(old_bytes, new_bytes);
        return replacement_pair_cost_with_shared_boundary(
            old_bytes,
            new_bytes,
            shared_prefix,
            shared_suffix,
            |old_trimmed, new_trimmed| scratch.distance_bytes(old_trimmed, new_trimmed) as u32,
        );
    }

    replacement_pair_cost_with_distance(old.chars(), new.chars(), |old_trimmed, new_trimmed| {
        scratch.distance(old_trimmed, new_trimmed) as u32
    })
}

#[cfg(feature = "benchmarks")]
pub(super) fn replacement_pair_cost_with_scratch(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    replacement_pair_cost(old, new, scratch)
}

#[cfg(feature = "benchmarks")]
pub(super) fn replacement_pair_cost_with_strsim(
    old: &PreparedReplacementLine<'_>,
    new: &PreparedReplacementLine<'_>,
    scratch: &mut LevenshteinScratch,
) -> u32 {
    if old.text == new.text {
        return 0;
    }

    if let (Some(old_bytes), Some(new_bytes)) = (old.ascii_bytes(), new.ascii_bytes()) {
        let (shared_prefix, shared_suffix) = shared_boundary_bytes(old_bytes, new_bytes);
        return replacement_pair_cost_with_shared_boundary(
            old_bytes,
            new_bytes,
            shared_prefix,
            shared_suffix,
            |old_trimmed, new_trimmed| {
                u32::try_from(scratch.distance_bytes(old_trimmed, new_trimmed)).unwrap_or(u32::MAX)
            },
        );
    }

    replacement_pair_cost_with_distance(old.chars(), new.chars(), |old_trimmed, new_trimmed| {
        let old_trimmed_wrapper = CharSlice(old_trimmed);
        let new_trimmed_wrapper = CharSlice(new_trimmed);
        let distance = strsim::generic_levenshtein::<CharSlice<'_>, CharSlice<'_>, char, char>(
            &old_trimmed_wrapper,
            &new_trimmed_wrapper,
        );
        u32::try_from(distance).unwrap_or(u32::MAX)
    })
}

pub(super) fn replacement_pair_cost_with_distance<T: Eq>(
    old_units: &[T],
    new_units: &[T],
    distance_fn: impl FnOnce(&[T], &[T]) -> u32,
) -> u32 {
    let (shared_prefix, shared_suffix) = shared_boundary_units(old_units, new_units);
    replacement_pair_cost_with_shared_boundary(
        old_units,
        new_units,
        shared_prefix,
        shared_suffix,
        distance_fn,
    )
}

pub(super) fn replacement_pair_cost_with_shared_boundary<T: Eq>(
    old_units: &[T],
    new_units: &[T],
    shared_prefix: usize,
    shared_suffix: usize,
    distance_fn: impl FnOnce(&[T], &[T]) -> u32,
) -> u32 {
    let max_len_usize = old_units.len().max(new_units.len()).max(1);
    let max_len = max_len_usize as u32;
    let old_trimmed = &old_units[shared_prefix..old_units.len().saturating_sub(shared_suffix)];
    let new_trimmed = &new_units[shared_prefix..new_units.len().saturating_sub(shared_suffix)];
    let trimmed_cells = old_trimmed.len().saturating_mul(new_trimmed.len());

    // Fast path: if either trimmed side is empty, the distance is exactly
    // the length of the other side — skip the O(n*m) Levenshtein DP.
    let distance = if old_trimmed.is_empty() || new_trimmed.is_empty() {
        (old_trimmed.len() + new_trimmed.len()) as u32
    } else if trimmed_cells > REPLACEMENT_ALIGN_CELL_BUDGET {
        u32::try_from(old_trimmed.len().max(new_trimmed.len())).unwrap_or(u32::MAX)
    } else {
        distance_fn(old_trimmed, new_trimmed)
    };

    let mut cost = REPLACEMENT_PAIR_BASE_COST
        + distance
            .min(max_len)
            .saturating_mul(REPLACEMENT_PAIR_SCALE_COST)
            / max_len;
    if shared_prefix == 0
        && shared_suffix == 0
        && max_len_usize >= REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN
    {
        cost = cost.saturating_add(REPLACEMENT_DISSIMILAR_PENALTY_COST);
    }

    cost
}

pub(super) fn shared_boundary_bytes(a: &[u8], b: &[u8]) -> (usize, usize) {
    let min_len = a.len().min(b.len());
    let word_bytes = std::mem::size_of::<usize>();
    let mut prefix = 0usize;
    while prefix.saturating_add(word_bytes) <= min_len
        && a[prefix..prefix + word_bytes] == b[prefix..prefix + word_bytes]
    {
        prefix += word_bytes;
    }
    while prefix < min_len && a[prefix] == b[prefix] {
        prefix += 1;
    }

    let max_suffix = min_len.saturating_sub(prefix);
    let mut suffix = 0usize;
    while suffix < max_suffix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }

    (prefix, suffix)
}

pub(super) fn shared_boundary_units<T: Eq>(a: &[T], b: &[T]) -> (usize, usize) {
    let mut prefix = 0usize;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }

    let max_suffix = a.len().min(b.len()).saturating_sub(prefix);
    let mut suffix = 0usize;
    while suffix < max_suffix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }

    (prefix, suffix)
}

#[cfg(test)]
pub(super) fn shared_boundary_chars(a: &[char], b: &[char]) -> (usize, usize) {
    shared_boundary_units(a, b)
}
