//! KDiff3-compatible aligned merge planning.
//!
//! KDiff3 builds one aligned A/B/C row list first, then derives merge blocks
//! and their default selections from that list. Keeping the same separation
//! here gives the GUI and headless renderer one source of truth.

use super::{DiffAlgorithm, MergeOptions};
use crate::file_diff::{Edit, EditKind, histogram_edits, myers_edits, split_lines};
use rustc_hash::FxHashMap;
use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::sync::Arc;

/// A source in KDiff3's aligned merge space.
///
/// With a base, A is the base, B is local/ours, and C is remote/theirs.
/// Without a base, true two-input behavior is used: A is local and B is
/// remote; C is absent.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MergeSource {
    A,
    B,
    C,
}

/// A user-supplied alignment constraint over one line range per source.
///
/// KDiff3 calls this a "manual diff help" entry: the escape hatch for when the
/// automatic alignment pairs the wrong blocks. The planner must line the pinned
/// ranges up with one another, and diffs the text between consecutive entries
/// independently. An empty range is meaningful — it says the other sources'
/// lines align against nothing at that position.
///
/// Ranges are half-open, zero-based line indices. Without a base the `base`
/// range is unused and should be left empty.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManualAlignment {
    pub base: Range<usize>,
    pub local: Range<usize>,
    pub remote: Range<usize>,
}

impl ManualAlignment {
    /// Build an entry, normalizing any inverted range to an empty one.
    pub fn new(base: Range<usize>, local: Range<usize>, remote: Range<usize>) -> Self {
        Self {
            base: normalize_range(base),
            local: normalize_range(local),
            remote: normalize_range(remote),
        }
    }

    /// Build a two-input entry, leaving the unused base range empty.
    pub fn two_input(local: Range<usize>, remote: Range<usize>) -> Self {
        Self::new(0..0, local, remote)
    }

    /// The range pinned for `source`, in the plan's A/B/C space.
    pub fn source_range(&self, source: MergeSource, three_way: bool) -> Range<usize> {
        let (a, b, c) = self.plan_ranges(three_way);
        match source {
            MergeSource::A => a,
            MergeSource::B => b,
            MergeSource::C => c,
        }
    }

    /// An entry that pins nothing constrains nothing.
    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.local.is_empty() && self.remote.is_empty()
    }

    /// The A/B/C ranges this entry pins, following the plan's source mapping.
    fn plan_ranges(&self, three_way: bool) -> (Range<usize>, Range<usize>, Range<usize>) {
        if three_way {
            (self.base.clone(), self.local.clone(), self.remote.clone())
        } else {
            (self.local.clone(), self.remote.clone(), 0..0)
        }
    }

    /// Whether every range of `self` ends at or before the matching range of
    /// `other` starts, with at least one source strictly separated.
    fn strictly_precedes(&self, other: &Self) -> bool {
        let pairs = [
            (&self.base, &other.base),
            (&self.local, &other.local),
            (&self.remote, &other.remote),
        ];
        pairs.iter().all(|(left, right)| left.end <= right.start)
            && pairs.iter().any(|(left, right)| left.end < right.start)
    }
}

fn normalize_range(range: Range<usize>) -> Range<usize> {
    if range.start <= range.end {
        range
    } else {
        range.start..range.start
    }
}

/// An ordered, non-overlapping set of manual alignment constraints.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManualAlignmentList {
    entries: Vec<ManualAlignment>,
}

impl ManualAlignmentList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn as_slice(&self) -> &[ManualAlignment] {
        &self.entries
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ManualAlignment> {
        self.entries.iter()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Add an entry, keeping the list ordered.
    ///
    /// Rejects an entry that pins nothing, or that overlaps or interleaves with
    /// an existing one — those cannot be satisfied by a segmented diff.
    pub fn insert(&mut self, entry: ManualAlignment) -> bool {
        if entry.is_empty() {
            return false;
        }
        let before = self
            .entries
            .iter()
            .take_while(|existing| existing.strictly_precedes(&entry))
            .count();
        if self.entries[before..]
            .iter()
            .any(|existing| !entry.strictly_precedes(existing))
        {
            return false;
        }
        self.entries.insert(before, entry);
        true
    }

    /// Drop the entry whose `source` range contains or abuts `line`.
    ///
    /// Returns whether anything was removed. An empty pinned range matches the
    /// line it sits at, so a pin against nothing stays removable.
    pub fn remove_at(&mut self, source: MergeSource, three_way: bool, line: usize) -> bool {
        let found = self.entries.iter().position(|entry| {
            let range = entry.source_range(source, three_way);
            range.contains(&line) || (range.is_empty() && range.start == line)
        });
        match found {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }
}

/// Conservative preflight limits for an interactive merge plan.
///
/// The regular plan builders remain available to callers that explicitly
/// accept their algorithmic cost. Resolver sessions use this budget before
/// invoking Myers so opening a file cannot allocate an effectively quadratic
/// trace for large, unrelated inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InteractiveMergePlanBudget {
    pub always_plan_total_lines: usize,
    pub max_total_lines: usize,
    pub minimum_shared_line_numerator: usize,
    pub minimum_shared_line_denominator: usize,
}

impl Default for InteractiveMergePlanBudget {
    fn default() -> Self {
        Self {
            always_plan_total_lines: 2_000,
            max_total_lines: 100_000,
            minimum_shared_line_numerator: 1,
            minimum_shared_line_denominator: 4,
        }
    }
}

/// Whether an interactive merge plan is safe enough to construct.
pub fn interactive_merge_plan_is_practical(
    base: Option<&str>,
    local: &str,
    remote: &str,
    budget: InteractiveMergePlanBudget,
) -> bool {
    let base_count = base.map_or(0, |text| text.lines().count());
    let local_count = local.lines().count();
    let remote_count = remote.lines().count();
    let total = base_count
        .saturating_add(local_count)
        .saturating_add(remote_count);
    if total > budget.max_total_lines {
        return false;
    }
    if total <= budget.always_plan_total_lines {
        return true;
    }
    if budget.minimum_shared_line_denominator == 0 {
        return false;
    }

    let shared_enough = |shared: usize, side_count: usize| {
        side_count == 0
            || shared.saturating_mul(budget.minimum_shared_line_denominator)
                >= side_count.saturating_mul(budget.minimum_shared_line_numerator)
    };

    if let Some(base) = base {
        // `std`'s seeded `HashSet`, not `FxHashSet`: the keys are raw file
        // lines an untrusted repository controls, and this is the guard that
        // bounds pathological input -- it must not be what degrades on it.
        let mut base_lines = HashSet::with_capacity(base_count);
        base_lines.extend(base.lines());
        let local_shared = local
            .lines()
            .filter(|line| base_lines.contains(line))
            .count();
        let remote_shared = remote
            .lines()
            .filter(|line| base_lines.contains(line))
            .count();
        shared_enough(local_shared, local_count) && shared_enough(remote_shared, remote_count)
    } else {
        let mut local_lines = HashSet::with_capacity(local_count);
        local_lines.extend(local.lines());
        let remote_shared = remote
            .lines()
            .filter(|line| local_lines.contains(line))
            .count();
        shared_enough(remote_shared, remote_count)
    }
}

/// Build an interactive plan only when its preflight budget permits it.
pub fn try_build_interactive_merge_plan_with_optional_base(
    base: Option<&str>,
    local: &str,
    remote: &str,
    options: &MergeOptions,
    budget: InteractiveMergePlanBudget,
) -> Option<MergePlan> {
    try_build_interactive_merge_plan_with_alignments(
        base,
        local,
        remote,
        options,
        budget,
        &ManualAlignmentList::new(),
    )
}

/// Build a budgeted interactive plan honoring manual alignment constraints.
pub fn try_build_interactive_merge_plan_with_alignments(
    base: Option<&str>,
    local: &str,
    remote: &str,
    options: &MergeOptions,
    budget: InteractiveMergePlanBudget,
    alignments: &ManualAlignmentList,
) -> Option<MergePlan> {
    interactive_merge_plan_is_practical(base, local, remote, budget)
        .then(|| build_merge_plan_with_alignments(base, local, remote, options, alignments))
}

/// A unique, ordered set of selected merge sources.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OrderedSelection {
    sources: Vec<MergeSource>,
}

impl OrderedSelection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_source(source: MergeSource) -> Self {
        Self {
            sources: vec![source],
        }
    }

    pub fn from_sources(sources: impl IntoIterator<Item = MergeSource>) -> Self {
        let mut selection = Self::new();
        for source in sources {
            selection.append(source);
        }
        selection
    }

    pub fn as_slice(&self) -> &[MergeSource] {
        &self.sources
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = MergeSource> + '_ {
        self.sources.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn contains(&self, source: MergeSource) -> bool {
        self.sources.contains(&source)
    }

    /// Append a source if it is not already selected.
    pub fn append(&mut self, source: MergeSource) {
        if !self.contains(source) {
            self.sources.push(source);
        }
    }

    /// Toggle a source, appending newly selected sources at the end.
    pub fn toggle(&mut self, source: MergeSource) {
        if let Some(index) = self
            .sources
            .iter()
            .position(|candidate| *candidate == source)
        {
            self.sources.remove(index);
        } else {
            self.sources.push(source);
        }
    }

    pub fn clear(&mut self) {
        self.sources.clear();
    }
}

impl<const N: usize> From<[MergeSource; N]> for OrderedSelection {
    fn from(value: [MergeSource; N]) -> Self {
        Self::from_sources(value)
    }
}

impl From<MergeSource> for OrderedSelection {
    fn from(value: MergeSource) -> Self {
        Self::from_source(value)
    }
}

/// One row in the shared aligned A/B/C space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignedRow {
    pub a: Option<usize>,
    pub b: Option<usize>,
    pub c: Option<usize>,
    /// Exact line equality after final alignment.
    pub equal_ab: bool,
    pub equal_ac: bool,
    pub equal_bc: bool,
    /// Equality after removing whitespace, matching the equality metadata
    /// KDiff3 uses while aligning whole lines.
    pub whitespace_equal_ab: bool,
    pub whitespace_equal_ac: bool,
    pub whitespace_equal_bc: bool,
    /// Missing lines count as white, as they do in KDiff3's merge metadata.
    pub whitespace_a: bool,
    pub whitespace_b: bool,
    pub whitespace_c: bool,
}

impl Default for AlignedRow {
    fn default() -> Self {
        Self {
            a: None,
            b: None,
            c: None,
            equal_ab: false,
            equal_ac: false,
            equal_bc: false,
            whitespace_equal_ab: false,
            whitespace_equal_ac: false,
            whitespace_equal_bc: false,
            whitespace_a: true,
            whitespace_b: true,
            whitespace_c: true,
        }
    }
}

impl AlignedRow {
    pub fn line(&self, source: MergeSource) -> Option<usize> {
        match source {
            MergeSource::A => self.a,
            MergeSource::B => self.b,
            MergeSource::C => self.c,
        }
    }
}

/// KDiff3's per-row merge detail classifications.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MergeBlockClassification {
    Default,
    NoChange,
    BChanged,
    CChanged,
    BCChanged,
    BCChangedAndEqual,
    BDeleted,
    CDeleted,
    BCDeleted,
    BChangedCDeleted,
    CChangedBDeleted,
    BAdded,
    CAdded,
    BCAdded,
    BCAddedAndEqual,
}

/// Stable content identity for a merge block.
///
/// `occurrence` distinguishes repeated blocks for navigation within one plan.
/// It is not sufficient proof of cross-plan identity after an identical block
/// is inserted or removed; restoration across changed plans must treat
/// duplicate fingerprints as ambiguous.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MergeBlockId {
    pub fingerprint: u64,
    pub occurrence: u32,
}

/// A grouped range of aligned rows and its current output decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeBlock {
    pub id: MergeBlockId,
    pub rows: Range<usize>,
    pub classification: MergeBlockClassification,
    pub is_delta: bool,
    pub original_conflict: bool,
    pub whitespace_conflict: bool,
    pub automatic_selection: OrderedSelection,
    pub selection: OrderedSelection,
    pub manual_content: Option<String>,
}

impl MergeBlock {
    pub fn is_resolved(&self) -> bool {
        self.manual_content.is_some() || !self.selection.is_empty()
    }

    pub fn toggle_source(&mut self, source: MergeSource) {
        self.manual_content = None;
        self.selection.toggle(source);
    }

    pub fn replace_selection(&mut self, selection: OrderedSelection) {
        self.manual_content = None;
        self.selection = selection;
    }

    pub fn set_manual_content(&mut self, content: String) {
        self.selection.clear();
        self.manual_content = Some(content);
    }

    pub fn reset_to_automatic(&mut self) {
        self.manual_content = None;
        self.selection = self.automatic_selection.clone();
    }
}

/// Complete shared plan used by aligned views and merge rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergePlan {
    pub base: Option<Arc<str>>,
    pub local: Arc<str>,
    pub remote: Arc<str>,
    pub rows: Vec<AlignedRow>,
    pub blocks: Vec<MergeBlock>,
    pub unresolved_blocks: Vec<usize>,
    pub line_ending: &'static str,
}

impl MergePlan {
    pub fn has_base(&self) -> bool {
        self.base.is_some()
    }

    pub fn local_source(&self) -> MergeSource {
        if self.has_base() {
            MergeSource::B
        } else {
            MergeSource::A
        }
    }

    pub fn remote_source(&self) -> MergeSource {
        if self.has_base() {
            MergeSource::C
        } else {
            MergeSource::B
        }
    }

    pub fn source_text(&self, source: MergeSource) -> Option<&str> {
        match (self.has_base(), source) {
            (true, MergeSource::A) => self.base.as_deref(),
            (true, MergeSource::B) => Some(self.local.as_ref()),
            (true, MergeSource::C) => Some(self.remote.as_ref()),
            (false, MergeSource::A) => Some(self.local.as_ref()),
            (false, MergeSource::B) => Some(self.remote.as_ref()),
            (false, MergeSource::C) => None,
        }
    }

    pub fn source_lines(&self, source: MergeSource) -> Vec<&str> {
        self.source_text(source)
            .map(split_lines)
            .unwrap_or_default()
    }

    pub fn block_source_lines<'a>(
        &'a self,
        block: &MergeBlock,
        source: MergeSource,
    ) -> Vec<&'a str> {
        let lines = self.source_lines(source);
        self.rows[block.rows.clone()]
            .iter()
            .filter_map(|row| row.line(source).and_then(|index| lines.get(index).copied()))
            .collect()
    }

    pub fn block_source_text(&self, block: &MergeBlock, source: MergeSource) -> String {
        let mut text = String::new();
        for line in self.block_source_lines(block, source) {
            text.push_str(line);
            text.push_str(self.line_ending);
        }
        text
    }

    /// Contiguous source-line coordinates covered by a block.
    ///
    /// A deletion has an empty range at its source insertion boundary.
    pub fn block_source_line_range(&self, block: &MergeBlock, source: MergeSource) -> Range<usize> {
        let mut indices = self.rows[block.rows.clone()]
            .iter()
            .filter_map(|row| row.line(source));
        if let Some(first) = indices.next() {
            let last = indices.next_back().unwrap_or(first);
            return first..last.saturating_add(1);
        }

        let boundary = self.rows[..block.rows.start]
            .iter()
            .rev()
            .find_map(|row| row.line(source))
            .map_or(0, |line| line.saturating_add(1));
        boundary..boundary
    }

    /// The ancestor line range belonging to the delta region around `block`.
    ///
    /// Contributor↔contributor realignment can place a base line on a nearby
    /// automatically resolved row. Diff3 markers still need that ancestor
    /// line, so the range is bounded by the nearest unchanged three-way
    /// anchors rather than by base cells physically present in the block.
    pub fn block_ancestor_range(&self, block: &MergeBlock) -> Option<Range<usize>> {
        self.has_base().then(|| {
            let start = self.rows[..block.rows.start]
                .iter()
                .rev()
                .find(|row| row.equal_ab && row.equal_ac)
                .and_then(|row| row.a)
                .map_or(0, |line| line + 1);
            let end = self.rows[block.rows.end..]
                .iter()
                .find(|row| row.equal_ab && row.equal_ac)
                .and_then(|row| row.a)
                .unwrap_or_else(|| self.base.as_deref().map_or(0, line_count));
            start.min(end)..end
        })
    }

    /// [`Self::block_ancestor_range`] for every block at once: one forward
    /// pass over the rows instead of a backward and a forward scan per block.
    pub fn block_ancestor_ranges(&self) -> Vec<Option<Range<usize>>> {
        if !self.has_base() {
            return vec![None; self.blocks.len()];
        }
        let base_len = self.base.as_deref().map_or(0, line_count);
        // `before[i]`: one past the base line of the last three-way anchor
        // strictly before row `i`, or 0. `after[i]`: the base line of the
        // first anchor at or after row `i`, or the base length.
        let mut before = Vec::with_capacity(self.rows.len() + 1);
        let mut last = 0usize;
        for row in &self.rows {
            before.push(last);
            if row.equal_ab
                && row.equal_ac
                && let Some(a) = row.a
            {
                last = a + 1;
            }
        }
        before.push(last);
        let mut after = vec![base_len; self.rows.len() + 1];
        let mut next = base_len;
        for (ix, row) in self.rows.iter().enumerate().rev() {
            if row.equal_ab
                && row.equal_ac
                && let Some(a) = row.a
            {
                next = a;
            }
            after[ix] = next;
        }
        self.blocks
            .iter()
            .map(|block| {
                let start = before[block.rows.start];
                let end = after[block.rows.end];
                Some(start.min(end)..end)
            })
            .collect()
    }

    pub fn block_ancestor_lines<'a>(&'a self, block: &MergeBlock) -> Vec<&'a str> {
        let Some(range) = self.block_ancestor_range(block) else {
            return Vec::new();
        };
        let lines = self.source_lines(MergeSource::A);
        lines[range].to_vec()
    }

    pub fn refresh_unresolved_blocks(&mut self) {
        self.unresolved_blocks.clear();
        self.unresolved_blocks.extend(
            self.blocks
                .iter()
                .enumerate()
                .filter_map(|(index, block)| (!block.is_resolved()).then_some(index)),
        );
    }

    pub fn unresolved_count(&self) -> usize {
        self.unresolved_blocks.len()
    }

    /// Indices of all changed merge blocks, including automatically selected
    /// changes and conflicts.
    pub fn delta_block_indices(&self) -> Vec<usize> {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.is_delta.then_some(index))
            .collect()
    }

    /// Indices of blocks that were conflicts when the plan was built.
    ///
    /// This set is stable as selections change.
    pub fn original_conflict_block_indices(&self) -> Vec<usize> {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.original_conflict.then_some(index))
            .collect()
    }

    /// Indices of original conflicts that currently have no output decision.
    pub fn unresolved_block_indices(&self) -> &[usize] {
        &self.unresolved_blocks
    }

    pub fn toggle_source(&mut self, block_index: usize, source: MergeSource) -> bool {
        if self.source_text(source).is_none() {
            return false;
        }
        let Some(block) = self.blocks.get_mut(block_index) else {
            return false;
        };
        block.toggle_source(source);
        self.refresh_unresolved_blocks();
        true
    }

    pub fn replace_selection(&mut self, block_index: usize, selection: OrderedSelection) -> bool {
        let changed = self.replace_selection_deferred(block_index, selection);
        if changed {
            self.refresh_unresolved_blocks();
        }
        changed
    }

    pub fn set_manual_content(&mut self, block_index: usize, content: String) -> bool {
        let changed = self.set_manual_content_deferred(block_index, content);
        if changed {
            self.refresh_unresolved_blocks();
        }
        changed
    }

    /// As [`Self::replace_selection`], but leaves the unresolved index stale.
    ///
    /// `refresh_unresolved_blocks` walks every block, so doing it per call
    /// makes a bulk decision quadratic in the block count. Callers that touch
    /// many blocks use this and refresh once at the end.
    pub(crate) fn replace_selection_deferred(
        &mut self,
        block_index: usize,
        selection: OrderedSelection,
    ) -> bool {
        if selection
            .iter()
            .any(|source| self.source_text(source).is_none())
        {
            return false;
        }
        let Some(block) = self.blocks.get_mut(block_index) else {
            return false;
        };
        block.replace_selection(selection);
        true
    }

    /// As [`Self::set_manual_content`], but leaves the unresolved index stale.
    /// See [`Self::replace_selection_deferred`].
    pub(crate) fn set_manual_content_deferred(
        &mut self,
        block_index: usize,
        content: String,
    ) -> bool {
        let Some(block) = self.blocks.get_mut(block_index) else {
            return false;
        };
        block.set_manual_content(content);
        true
    }

    fn refresh_block_ids(&mut self) {
        let local = Arc::clone(&self.local);
        let remote = Arc::clone(&self.remote);
        let base = self.base.clone();
        let (a_text, b_text, c_text) = match base.as_ref() {
            Some(base) => (base.as_ref(), local.as_ref(), remote.as_ref()),
            None => (local.as_ref(), remote.as_ref(), ""),
        };
        let a_lines = split_lines(a_text);
        let b_lines = split_lines(b_text);
        let c_lines = split_lines(c_text);
        let mut occurrences = FxHashMap::<u64, u32>::default();

        for block in &mut self.blocks {
            let fingerprint = block_fingerprint(
                &self.rows[block.rows.clone()],
                block.classification,
                &a_lines,
                &b_lines,
                &c_lines,
            );
            let occurrence = occurrences.entry(fingerprint).or_default();
            block.id = MergeBlockId {
                fingerprint,
                occurrence: *occurrence,
            };
            *occurrence = occurrence.saturating_add(1);
        }
    }

    /// Split one plan block into adjacent row ranges whose A/B/C line counts
    /// match the newly split conflict regions.
    pub(crate) fn split_block_by_source_line_counts(
        &mut self,
        block_index: usize,
        part_counts: &[[usize; 3]],
    ) -> Option<usize> {
        if part_counts.len() < 2 {
            return None;
        }
        let original = self.blocks.get(block_index)?.clone();
        let available =
            self.rows[original.rows.clone()]
                .iter()
                .fold([0usize; 3], |mut counts, row| {
                    counts[0] += usize::from(row.a.is_some());
                    counts[1] += usize::from(row.b.is_some());
                    counts[2] += usize::from(row.c.is_some());
                    counts
                });
        let requested = part_counts.iter().fold([0usize; 3], |mut total, counts| {
            for source in 0..3 {
                total[source] += counts[source];
            }
            total
        });
        if available != requested {
            return None;
        }

        let mut cursor = original.rows.start;
        let mut ranges = Vec::with_capacity(part_counts.len());
        for counts in part_counts {
            if counts.iter().all(|count| *count == 0) {
                return None;
            }
            let start = cursor;
            let mut remaining = *counts;
            while remaining.iter().any(|count| *count > 0) {
                let row = self.rows.get(cursor)?;
                for (source, present) in [row.a.is_some(), row.b.is_some(), row.c.is_some()]
                    .into_iter()
                    .enumerate()
                {
                    if !present {
                        continue;
                    }
                    if remaining[source] == 0 {
                        return None;
                    }
                    remaining[source] -= 1;
                }
                cursor += 1;
            }
            if cursor == start {
                return None;
            }
            ranges.push(start..cursor);
        }
        if cursor != original.rows.end {
            return None;
        }

        let replacement = ranges.into_iter().map(|rows| MergeBlock {
            rows,
            ..original.clone()
        });
        self.blocks
            .splice(block_index..block_index + 1, replacement);
        self.refresh_block_ids();
        self.refresh_unresolved_blocks();
        Some(part_counts.len())
    }

    /// Join a contiguous range of plan blocks into one unresolved conflict.
    pub(crate) fn join_block_range(
        &mut self,
        first_block_index: usize,
        last_block_index: usize,
    ) -> Option<usize> {
        if first_block_index > last_block_index || last_block_index >= self.blocks.len() {
            return None;
        }
        let rows =
            self.blocks[first_block_index].rows.start..self.blocks[last_block_index].rows.end;
        let whitespace_conflict = self.rows[rows.clone()]
            .iter()
            .all(|row| row_whitespace_conflict(row, self.has_base()));
        let joined = MergeBlock {
            id: MergeBlockId {
                fingerprint: 0,
                occurrence: 0,
            },
            rows,
            classification: if self.has_base() {
                MergeBlockClassification::BCChanged
            } else {
                MergeBlockClassification::BChanged
            },
            is_delta: true,
            original_conflict: true,
            whitespace_conflict,
            automatic_selection: OrderedSelection::new(),
            selection: OrderedSelection::new(),
            manual_content: None,
        };
        let removed = last_block_index - first_block_index;
        self.blocks
            .splice(first_block_index..last_block_index + 1, [joined]);
        self.refresh_block_ids();
        self.refresh_unresolved_blocks();
        Some(removed)
    }

    /// Restore decisions by stable block identity.
    pub fn restore_decisions_from(&mut self, previous: &MergePlan) {
        let same_sequence = self.blocks.len() == previous.blocks.len()
            && self
                .blocks
                .iter()
                .zip(&previous.blocks)
                .all(|(current, previous)| current.id.fingerprint == previous.id.fingerprint);
        if same_sequence {
            for (block, previous) in self.blocks.iter_mut().zip(&previous.blocks) {
                block.selection = previous.selection.clone();
                block.manual_content = previous.manual_content.clone();
            }
            self.refresh_unresolved_blocks();
            return;
        }

        let mut previous_counts = FxHashMap::<u64, usize>::default();
        let mut current_counts = FxHashMap::<u64, usize>::default();
        for block in &previous.blocks {
            *previous_counts.entry(block.id.fingerprint).or_default() += 1;
        }
        for block in &self.blocks {
            *current_counts.entry(block.id.fingerprint).or_default() += 1;
        }
        let decisions: FxHashMap<u64, (&OrderedSelection, Option<&String>)> = previous
            .blocks
            .iter()
            .filter(|block| previous_counts.get(&block.id.fingerprint) == Some(&1))
            .map(|block| {
                (
                    block.id.fingerprint,
                    (&block.selection, block.manual_content.as_ref()),
                )
            })
            .collect();
        for block in &mut self.blocks {
            let fingerprint = block.id.fingerprint;
            if current_counts.get(&fingerprint) != Some(&1) {
                continue;
            }
            if let Some((selection, manual)) = decisions.get(&fingerprint) {
                block.selection = (*selection).clone();
                block.manual_content = manual.map(|content| (*content).clone());
            }
        }
        self.refresh_unresolved_blocks();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PairChunk {
    equal: usize,
    left: usize,
    right: usize,
}

fn pair_chunks(edits: &[Edit<'_>]) -> Vec<PairChunk> {
    let mut chunks = Vec::new();
    let mut index = 0usize;
    while index < edits.len() {
        let mut chunk = PairChunk::default();
        while index < edits.len() && edits[index].kind == EditKind::Equal {
            chunk.equal += 1;
            index += 1;
        }
        while index < edits.len() && edits[index].kind != EditKind::Equal {
            match edits[index].kind {
                EditKind::Delete => chunk.left += 1,
                EditKind::Insert => chunk.right += 1,
                EditKind::Equal => unreachable!(),
            }
            index += 1;
        }
        if chunk != PairChunk::default() {
            chunks.push(chunk);
        }
    }
    chunks
}

fn diff_chunks<'a>(
    left: &[&'a str],
    right: &[&'a str],
    algorithm: DiffAlgorithm,
) -> Vec<PairChunk> {
    // GNU diff peels byte-identical prefixes and suffixes before applying its
    // whitespace-insensitive equivalence classes. Preserve that detail: it
    // makes an exact repeated line win over an earlier whitespace-equivalent
    // candidate (one of KDiff3's upstream alignment regressions).
    let mut prefix = 0usize;
    while prefix < left.len() && prefix < right.len() && left[prefix] == right[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while prefix + suffix < left.len()
        && prefix + suffix < right.len()
        && left[left.len() - 1 - suffix] == right[right.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let left_mid = &left[prefix..left.len() - suffix];
    let right_mid = &right[prefix..right.len() - suffix];

    // KDiff3's whole-line alignment ignores whitespace, then its fine-diff
    // pass restores exact equality for merge classification.
    let left_normalized: Vec<String> = left_mid
        .iter()
        .map(|line| normalized_without_whitespace(line))
        .collect();
    let right_normalized: Vec<String> = right_mid
        .iter()
        .map(|line| normalized_without_whitespace(line))
        .collect();
    let left_normalized: Vec<&str> = left_normalized.iter().map(String::as_str).collect();
    let right_normalized: Vec<&str> = right_normalized.iter().map(String::as_str).collect();
    let edits = match algorithm {
        DiffAlgorithm::Myers => myers_edits(&left_normalized, &right_normalized),
        DiffAlgorithm::Histogram => histogram_edits(&left_normalized, &right_normalized),
    };
    let mut chunks = Vec::new();
    if prefix > 0 {
        chunks.push(PairChunk {
            equal: prefix,
            ..PairChunk::default()
        });
    }
    chunks.extend(pair_chunks(&edits));
    if suffix > 0 {
        chunks.push(PairChunk {
            equal: suffix,
            ..PairChunk::default()
        });
    }
    chunks
}

/// Split a source pair into the segments a manual alignment forces on it.
///
/// Each pinned range becomes its own segment, and the text between consecutive
/// pins becomes another. Diffing those segments independently is what makes the
/// pinned ranges line up: no match can ever cross a segment boundary. An empty
/// result means nothing constrains this pair, so the caller diffs it whole.
fn alignment_segments(
    entries: &[ManualAlignment],
    three_way: bool,
    left: MergeSource,
    left_len: usize,
    right: MergeSource,
    right_len: usize,
) -> Vec<(Range<usize>, Range<usize>)> {
    // Each valid pin contributes the gap before it and the pinned range itself,
    // plus one trailing segment for the whole list.
    let mut segments = Vec::with_capacity(entries.len().saturating_mul(2).saturating_add(1));
    let mut left_cursor = 0usize;
    let mut right_cursor = 0usize;
    for entry in entries {
        let pinned_left = entry.source_range(left, three_way);
        let pinned_right = entry.source_range(right, three_way);
        // A stale entry — recorded against text that has since changed, or one
        // this pair happens to order inconsistently — constrains nothing rather
        // than truncating the inputs.
        if pinned_left.end > left_len
            || pinned_right.end > right_len
            || pinned_left.start < left_cursor
            || pinned_right.start < right_cursor
        {
            continue;
        }
        push_segment(
            &mut segments,
            left_cursor..pinned_left.start,
            right_cursor..pinned_right.start,
        );
        left_cursor = pinned_left.end;
        right_cursor = pinned_right.end;
        push_segment(&mut segments, pinned_left, pinned_right);
    }
    if segments.is_empty() {
        return segments;
    }
    push_segment(
        &mut segments,
        left_cursor..left_len,
        right_cursor..right_len,
    );
    segments
}

fn push_segment(
    segments: &mut Vec<(Range<usize>, Range<usize>)>,
    left: Range<usize>,
    right: Range<usize>,
) {
    if !left.is_empty() || !right.is_empty() {
        segments.push((left, right));
    }
}

fn diff_chunks_segmented<'a>(
    left: &[&'a str],
    right: &[&'a str],
    algorithm: DiffAlgorithm,
    segments: &[(Range<usize>, Range<usize>)],
) -> Vec<PairChunk> {
    if segments.is_empty() {
        return diff_chunks(left, right, algorithm);
    }
    segments
        .iter()
        .flat_map(|(left_range, right_range)| {
            diff_chunks(
                &left[left_range.clone()],
                &right[right_range.clone()],
                algorithm,
            )
        })
        .collect()
}

/// Per-source segment index for every line, used to keep later passes from
/// undoing what the segmented diff established.
#[derive(Clone, Debug, Default)]
struct AlignmentBarriers {
    a: Vec<usize>,
    b: Vec<usize>,
    c: Vec<usize>,
}

impl AlignmentBarriers {
    fn new(entries: &[ManualAlignment], three_way: bool) -> Self {
        let mut barriers = Self::default();
        for entry in entries {
            for (source, bounds) in [
                (MergeSource::A, &mut barriers.a),
                (MergeSource::B, &mut barriers.b),
                (MergeSource::C, &mut barriers.c),
            ] {
                let range = entry.source_range(source, three_way);
                bounds.push(range.start);
                bounds.push(range.end);
            }
        }
        barriers
    }

    fn is_empty(&self) -> bool {
        self.a.is_empty() && self.b.is_empty() && self.c.is_empty()
    }

    fn segment(&self, source: MergeSource, line: usize) -> usize {
        let bounds = match source {
            MergeSource::A => &self.a,
            MergeSource::B => &self.b,
            MergeSource::C => &self.c,
        };
        bounds.partition_point(|bound| *bound <= line)
    }

    fn row_segment(&self, row: &AlignedRow) -> usize {
        [MergeSource::A, MergeSource::B, MergeSource::C]
            .into_iter()
            .filter_map(|source| row.line(source).map(|line| self.segment(source, line)))
            .min()
            .unwrap_or(0)
    }

    /// Segment index of every row, captured before a pass starts moving lines.
    fn row_segments(&self, rows: &[AlignedRow]) -> Vec<usize> {
        rows.iter().map(|row| self.row_segment(row)).collect()
    }
}

#[derive(Clone, Debug)]
struct RowNode {
    row: AlignedRow,
    previous: Option<usize>,
    next: Option<usize>,
    alive: bool,
}

#[derive(Clone, Debug, Default)]
struct RowList {
    nodes: Vec<RowNode>,
    first: Option<usize>,
    last: Option<usize>,
}

impl RowList {
    fn push_back(&mut self, row: AlignedRow) -> usize {
        self.insert_before(None, row)
    }

    fn insert_before(&mut self, before: Option<usize>, row: AlignedRow) -> usize {
        let previous = before
            .and_then(|id| self.nodes.get(id).and_then(|node| node.previous))
            .or_else(|| before.is_none().then_some(self.last).flatten());
        let id = self.nodes.len();
        self.nodes.push(RowNode {
            row,
            previous,
            next: before,
            alive: true,
        });
        if let Some(previous) = previous {
            self.nodes[previous].next = Some(id);
        } else {
            self.first = Some(id);
        }
        if let Some(before) = before {
            self.nodes[before].previous = Some(id);
        } else {
            self.last = Some(id);
        }
        id
    }

    fn next(&self, id: usize) -> Option<usize> {
        self.nodes.get(id).and_then(|node| node.next)
    }

    fn find_from(
        &self,
        mut cursor: Option<usize>,
        source: MergeSource,
        line: usize,
    ) -> Option<usize> {
        while let Some(id) = cursor {
            let node = &self.nodes[id];
            if node.alive && node.row.line(source) == Some(line) {
                return Some(id);
            }
            cursor = node.next;
        }
        None
    }

    fn comes_before(&self, left: usize, right: usize) -> bool {
        let mut cursor = Some(left);
        while let Some(id) = cursor {
            if id == right {
                return true;
            }
            cursor = self.next(id);
        }
        false
    }

    fn ids_between(&self, start: usize, end: usize) -> Vec<usize> {
        let mut ids = Vec::new();
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if id == end {
                break;
            }
            ids.push(id);
            cursor = self.next(id);
        }
        ids
    }

    fn into_rows(self) -> Vec<AlignedRow> {
        let mut rows = Vec::new();
        let mut cursor = self.first;
        while let Some(id) = cursor {
            let node = &self.nodes[id];
            if node.alive {
                rows.push(node.row.clone());
            }
            cursor = node.next;
        }
        rows
    }

    fn from_rows(rows: Vec<AlignedRow>) -> Self {
        let mut list = Self::default();
        for row in rows {
            list.push_back(row);
        }
        list
    }
}

fn build_ab(chunks: &[PairChunk]) -> RowList {
    let mut rows = RowList::default();
    let mut a = 0usize;
    let mut b = 0usize;
    for chunk in chunks {
        for _ in 0..chunk.equal {
            rows.push_back(AlignedRow {
                a: Some(a),
                b: Some(b),
                ..AlignedRow::default()
            });
            a += 1;
            b += 1;
        }
        let paired = chunk.left.min(chunk.right);
        for _ in 0..paired {
            rows.push_back(AlignedRow {
                a: Some(a),
                b: Some(b),
                ..AlignedRow::default()
            });
            a += 1;
            b += 1;
        }
        for _ in paired..chunk.left {
            rows.push_back(AlignedRow {
                a: Some(a),
                ..AlignedRow::default()
            });
            a += 1;
        }
        for _ in paired..chunk.right {
            rows.push_back(AlignedRow {
                b: Some(b),
                ..AlignedRow::default()
            });
            b += 1;
        }
    }
    rows
}

fn integrate_ac(rows: &mut RowList, chunks: &[PairChunk]) {
    let mut cursor = rows.first;
    let mut a = 0usize;
    let mut c = 0usize;
    for chunk in chunks {
        for _ in 0..chunk.equal {
            let found = rows
                .find_from(cursor, MergeSource::A, a)
                .expect("A line from A↔C diff must exist in A↔B alignment");
            rows.nodes[found].row.c = Some(c);
            cursor = rows.next(found);
            a += 1;
            c += 1;
        }

        let paired = chunk.left.min(chunk.right);
        for _ in 0..paired {
            rows.insert_before(
                cursor,
                AlignedRow {
                    c: Some(c),
                    ..AlignedRow::default()
                },
            );
            a += 1;
            c += 1;
        }
        a += chunk.left - paired;
        for _ in paired..chunk.right {
            rows.insert_before(
                cursor,
                AlignedRow {
                    c: Some(c),
                    ..AlignedRow::default()
                },
            );
            c += 1;
        }
    }
}

fn integrate_bc(
    rows: &mut RowList,
    chunks: &[PairChunk],
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
) {
    let mut cursor_b = rows.first;
    let mut cursor_c = rows.first;
    let mut b = 0usize;
    let mut c = 0usize;

    for original in chunks {
        let mut chunk = *original;
        while chunk.equal > 0 {
            let mut changed_rows = Vec::new();
            let row_b = rows
                .find_from(cursor_b, MergeSource::B, b)
                .expect("B line from B↔C diff must exist");
            let row_c = rows
                .find_from(cursor_c, MergeSource::C, c)
                .expect("C line from B↔C diff must exist");

            if row_b == row_c {
                // Already aligned.
            } else if rows.comes_before(row_c, row_b) && !rows.nodes[row_b].row.whitespace_equal_ab
            {
                let between = rows.ids_between(row_c, row_b);
                if between.iter().any(|id| rows.nodes[*id].row.b.is_some()) {
                    let last_equal_a = between
                        .iter()
                        .rev()
                        .copied()
                        .find(|id| rows.nodes[*id].row.whitespace_equal_ab);
                    let mut before_or_on_equal_a = last_equal_a.is_some();
                    for id in between {
                        let should_move = rows.nodes[id].row.b.is_some()
                            || (before_or_on_equal_a && rows.nodes[id].row.a.is_some());
                        if should_move {
                            let a = if before_or_on_equal_a {
                                rows.nodes[id].row.a.take()
                            } else {
                                None
                            };
                            let moved = AlignedRow {
                                a,
                                b: rows.nodes[id].row.b.take(),
                                ..AlignedRow::default()
                            };
                            let inserted = rows.insert_before(Some(row_c), moved);
                            changed_rows.extend([id, inserted]);
                        }
                        if Some(id) == last_equal_a {
                            before_or_on_equal_a = false;
                        }
                    }
                }
                rows.nodes[row_b].row.b = None;
                rows.nodes[row_c].row.b = Some(b);
                changed_rows.extend([row_b, row_c]);
            } else if rows.comes_before(row_b, row_c) && !rows.nodes[row_c].row.whitespace_equal_ac
            {
                let between = rows.ids_between(row_b, row_c);
                if between.iter().any(|id| rows.nodes[*id].row.c.is_some()) {
                    let last_equal_a = between
                        .iter()
                        .rev()
                        .copied()
                        .find(|id| rows.nodes[*id].row.whitespace_equal_ac);
                    let mut before_or_on_equal_a = last_equal_a.is_some();
                    for id in between {
                        let should_move = rows.nodes[id].row.c.is_some()
                            || (before_or_on_equal_a && rows.nodes[id].row.a.is_some());
                        if should_move {
                            let a = if before_or_on_equal_a {
                                rows.nodes[id].row.a.take()
                            } else {
                                None
                            };
                            let moved = AlignedRow {
                                a,
                                c: rows.nodes[id].row.c.take(),
                                ..AlignedRow::default()
                            };
                            let inserted = rows.insert_before(Some(row_b), moved);
                            changed_rows.extend([id, inserted]);
                        }
                        if Some(id) == last_equal_a {
                            before_or_on_equal_a = false;
                        }
                    }
                }
                rows.nodes[row_c].row.c = None;
                rows.nodes[row_b].row.c = Some(c);
                changed_rows.extend([row_c, row_b]);
            }

            // Only moved rows lose or gain cells. Recomputing the entire list
            // for every equal B↔C anchor makes this pass quadratic on large,
            // mostly-identical files.
            changed_rows.sort_unstable();
            changed_rows.dedup();
            for id in changed_rows {
                recompute_row_metadata(&mut rows.nodes[id].row, a_lines, b_lines, c_lines);
            }
            chunk.equal -= 1;
            b += 1;
            c += 1;
            cursor_b = rows.next(row_b);
            cursor_c = rows.next(row_c);
        }

        while chunk.left > 0 {
            let found = rows
                .find_from(cursor_b, MergeSource::B, b)
                .expect("B diff line must exist");
            if Some(found) != cursor_b && !rows.nodes[found].row.whitespace_equal_ab {
                rows.nodes[found].row.b = None;
                rows.insert_before(
                    cursor_b,
                    AlignedRow {
                        b: Some(b),
                        ..AlignedRow::default()
                    },
                );
            } else {
                cursor_b = Some(found);
            }
            chunk.left -= 1;
            b += 1;
            cursor_b = cursor_b.and_then(|id| rows.next(id));
            if chunk.right > 0 {
                chunk.right -= 1;
                c += 1;
            }
        }
        c += chunk.right;
    }
}

pub(crate) fn normalized_without_whitespace(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// `normalized_without_whitespace(left) == normalized_without_whitespace(right)`
/// without materializing either side. This runs three times per aligned row
/// and the rows are recomputed up to six times per plan build, so the two
/// `String`s it replaced dominated the allocation count of a merge.
pub(crate) fn equal_ignoring_whitespace(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let mut left = left.chars().filter(|character| !character.is_whitespace());
    let mut right = right.chars().filter(|character| !character.is_whitespace());
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(l), Some(r)) if l == r => {}
            _ => return false,
        }
    }
}

/// Number of lines `split_lines` would produce, without producing them.
pub(crate) fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    memchr::memchr_iter(b'\n', text.as_bytes()).count() + usize::from(!text.ends_with('\n'))
}

fn pair_metadata(
    left: Option<usize>,
    right: Option<usize>,
    left_lines: &[&str],
    right_lines: &[&str],
) -> (bool, bool) {
    let (Some(left), Some(right)) = (left, right) else {
        return (false, false);
    };
    let Some(left) = left_lines.get(left) else {
        return (false, false);
    };
    let Some(right) = right_lines.get(right) else {
        return (false, false);
    };
    let equal = left == right;
    (equal, equal || equal_ignoring_whitespace(left, right))
}

fn recompute_row_metadata(
    row: &mut AlignedRow,
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
) {
    (row.equal_ab, row.whitespace_equal_ab) = pair_metadata(row.a, row.b, a_lines, b_lines);
    (row.equal_ac, row.whitespace_equal_ac) = pair_metadata(row.a, row.c, a_lines, c_lines);
    (row.equal_bc, row.whitespace_equal_bc) = pair_metadata(row.b, row.c, b_lines, c_lines);
    row.whitespace_a = row
        .a
        .and_then(|index| a_lines.get(index))
        .is_none_or(|line| line.trim().is_empty());
    row.whitespace_b = row
        .b
        .and_then(|index| b_lines.get(index))
        .is_none_or(|line| line.trim().is_empty());
    row.whitespace_c = row
        .c
        .and_then(|index| c_lines.get(index))
        .is_none_or(|line| line.trim().is_empty());
}

fn line_equal(
    left: Option<usize>,
    right: Option<usize>,
    left_lines: &[&str],
    right_lines: &[&str],
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left_lines.get(left) == right_lines.get(right),
        _ => false,
    }
}

fn trim_rows(
    mut rows: Vec<AlignedRow>,
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
    barriers: &AlignmentBarriers,
) -> Vec<AlignedRow> {
    rows.retain(|row| row.a.is_some() || row.b.is_some() || row.c.is_some());
    for row in &mut rows {
        recompute_row_metadata(row, a_lines, b_lines, c_lines);
    }
    if rows.is_empty() {
        return rows;
    }

    // Every move below pulls a line back into an earlier row, which would undo
    // a manual alignment if it crossed one of its boundaries. Segment indices
    // are captured now, before any line moves, so a row keeps the identity it
    // had when the segmented diff placed it.
    let row_segments = barriers.row_segments(&rows);
    let allows_move = |source: MergeSource, line: Option<usize>, target: usize| -> bool {
        match line {
            Some(line) if !barriers.is_empty() => row_segments
                .get(target)
                .is_none_or(|segment| barriers.segment(source, line) == *segment),
            _ => true,
        }
    };

    let mut cursor_a = 0usize;
    let mut cursor_b = 0usize;
    let mut cursor_c = 0usize;
    let mut line_a = 0usize;
    let mut line_b = 0usize;
    let mut line_c = 0usize;

    for look in 0..rows.len() {
        // Manual alignment: every row the segmented diff produced lies wholly
        // inside one segment, and a line may only be pulled back into a row of
        // its own. Advance the cursors past rows from earlier segments so a
        // move that would have crossed a boundary retries at the first row
        // that can legally hold it, instead of stalling on a blocked one.
        if !barriers.is_empty() {
            let wanted = row_segments[look];
            for cursor in [&mut cursor_a, &mut cursor_b, &mut cursor_c] {
                while *cursor < look && row_segments[*cursor] < wanted {
                    *cursor += 1;
                }
            }
        }

        if look > line_a
            && rows[look].a.is_some()
            && rows[cursor_a].b.is_some()
            && rows[cursor_a].whitespace_equal_bc
            && line_equal(rows[look].a, rows[cursor_a].b, a_lines, b_lines)
            && allows_move(MergeSource::A, rows[look].a, cursor_a)
        {
            rows[cursor_a].a = rows[look].a.take();
            recompute_row_metadata(&mut rows[cursor_a], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_a += 1;
            line_a += 1;
        }

        if look > line_b
            && rows[look].b.is_some()
            && rows[cursor_b].a.is_some()
            && rows[cursor_b].whitespace_equal_ac
            && line_equal(rows[look].b, rows[cursor_b].a, b_lines, a_lines)
            && allows_move(MergeSource::B, rows[look].b, cursor_b)
        {
            rows[cursor_b].b = rows[look].b.take();
            recompute_row_metadata(&mut rows[cursor_b], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_b += 1;
            line_b += 1;
        }

        if look > line_c
            && rows[look].c.is_some()
            && rows[cursor_c].a.is_some()
            && rows[cursor_c].whitespace_equal_ab
            && line_equal(rows[look].c, rows[cursor_c].a, c_lines, a_lines)
            && allows_move(MergeSource::C, rows[look].c, cursor_c)
        {
            rows[cursor_c].c = rows[look].c.take();
            recompute_row_metadata(&mut rows[cursor_c], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_c += 1;
            line_c += 1;
        }

        if look > line_a
            && rows[look].a.is_some()
            && !rows[look].whitespace_equal_ab
            && !rows[look].whitespace_equal_ac
            && allows_move(MergeSource::A, rows[look].a, cursor_a)
        {
            rows[cursor_a].a = rows[look].a.take();
            recompute_row_metadata(&mut rows[cursor_a], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_a += 1;
            line_a += 1;
        }

        if look > line_b
            && rows[look].b.is_some()
            && !rows[look].whitespace_equal_ab
            && !rows[look].whitespace_equal_bc
            && allows_move(MergeSource::B, rows[look].b, cursor_b)
        {
            rows[cursor_b].b = rows[look].b.take();
            recompute_row_metadata(&mut rows[cursor_b], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_b += 1;
            line_b += 1;
        }

        if look > line_c
            && rows[look].c.is_some()
            && !rows[look].whitespace_equal_ac
            && !rows[look].whitespace_equal_bc
            && allows_move(MergeSource::C, rows[look].c, cursor_c)
        {
            rows[cursor_c].c = rows[look].c.take();
            recompute_row_metadata(&mut rows[cursor_c], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_c += 1;
            line_c += 1;
        }

        // A paired move lands both lines in one row, so both must stay inside
        // the manual segment that row belongs to.
        let target_ab = if line_a > line_b { cursor_a } else { cursor_b };
        let target_ac = if line_a > line_c { cursor_a } else { cursor_c };
        let target_bc = if line_b > line_c { cursor_b } else { cursor_c };

        if look > line_a
            && look > line_b
            && rows[look].a.is_some()
            && rows[look].whitespace_equal_ab
            && !rows[look].whitespace_equal_ac
            && allows_move(MergeSource::A, rows[look].a, target_ab)
            && allows_move(MergeSource::B, rows[look].b, target_ab)
        {
            let target = target_ab;
            let next_line = line_a.max(line_b) + 1;
            rows[target].a = rows[look].a.take();
            rows[target].b = rows[look].b.take();
            recompute_row_metadata(&mut rows[target], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_a = target + 1;
            cursor_b = target + 1;
            line_a = next_line;
            line_b = next_line;
        } else if look > line_a
            && look > line_c
            && rows[look].a.is_some()
            && rows[look].whitespace_equal_ac
            && !rows[look].whitespace_equal_ab
            && allows_move(MergeSource::A, rows[look].a, target_ac)
            && allows_move(MergeSource::C, rows[look].c, target_ac)
        {
            let target = target_ac;
            let next_line = line_a.max(line_c) + 1;
            rows[target].a = rows[look].a.take();
            rows[target].c = rows[look].c.take();
            recompute_row_metadata(&mut rows[target], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_a = target + 1;
            cursor_c = target + 1;
            line_a = next_line;
            line_c = next_line;
        } else if look > line_b
            && look > line_c
            && rows[look].b.is_some()
            && rows[look].whitespace_equal_bc
            && !rows[look].whitespace_equal_ac
            && allows_move(MergeSource::B, rows[look].b, target_bc)
            && allows_move(MergeSource::C, rows[look].c, target_bc)
        {
            let target = target_bc;
            let next_line = line_b.max(line_c) + 1;
            rows[target].b = rows[look].b.take();
            rows[target].c = rows[look].c.take();
            recompute_row_metadata(&mut rows[target], a_lines, b_lines, c_lines);
            recompute_row_metadata(&mut rows[look], a_lines, b_lines, c_lines);
            cursor_b = target + 1;
            cursor_c = target + 1;
            line_b = next_line;
            line_c = next_line;
        }

        if rows[look].a.is_some() {
            line_a = look + 1;
            cursor_a = look + 1;
        }
        if rows[look].b.is_some() {
            line_b = look + 1;
            cursor_b = look + 1;
        }
        if rows[look].c.is_some() {
            line_c = look + 1;
            cursor_c = look + 1;
        }
    }

    rows.retain(|row| row.a.is_some() || row.b.is_some() || row.c.is_some());
    for row in &mut rows {
        recompute_row_metadata(row, a_lines, b_lines, c_lines);
    }
    rows
}

#[derive(Clone, Debug)]
struct RowDecision {
    classification: MergeBlockClassification,
    conflict: bool,
    selection: OrderedSelection,
}

fn classify_two_input(row: &AlignedRow) -> RowDecision {
    match (row.a, row.b) {
        (Some(_), Some(_)) if row.equal_ab => RowDecision {
            classification: MergeBlockClassification::NoChange,
            conflict: false,
            selection: MergeSource::A.into(),
        },
        (Some(_), Some(_)) => RowDecision {
            classification: MergeBlockClassification::BChanged,
            conflict: true,
            selection: OrderedSelection::new(),
        },
        _ => RowDecision {
            classification: MergeBlockClassification::BDeleted,
            conflict: true,
            selection: OrderedSelection::new(),
        },
    }
}

fn classify_three_way(row: &AlignedRow) -> RowDecision {
    use MergeBlockClassification as Kind;
    use MergeSource::{A, B, C};

    match (row.a, row.b, row.c) {
        (Some(_), Some(_), Some(_)) if row.equal_ab && row.equal_ac => RowDecision {
            classification: Kind::NoChange,
            conflict: false,
            selection: A.into(),
        },
        (Some(_), Some(_), Some(_)) if row.equal_ab => RowDecision {
            classification: Kind::CChanged,
            conflict: false,
            selection: C.into(),
        },
        (Some(_), Some(_), Some(_)) if row.equal_ac => RowDecision {
            classification: Kind::BChanged,
            conflict: false,
            selection: B.into(),
        },
        (Some(_), Some(_), Some(_)) if row.equal_bc => RowDecision {
            classification: Kind::BCChangedAndEqual,
            conflict: false,
            selection: C.into(),
        },
        (Some(_), Some(_), Some(_)) => RowDecision {
            classification: Kind::BCChanged,
            conflict: true,
            selection: OrderedSelection::new(),
        },
        (Some(_), Some(_), None) if !row.equal_ab => RowDecision {
            classification: Kind::BChangedCDeleted,
            conflict: true,
            selection: OrderedSelection::new(),
        },
        (Some(_), Some(_), None) => RowDecision {
            classification: Kind::CDeleted,
            conflict: false,
            selection: C.into(),
        },
        (Some(_), None, Some(_)) if !row.equal_ac => RowDecision {
            classification: Kind::CChangedBDeleted,
            conflict: true,
            selection: OrderedSelection::new(),
        },
        (Some(_), None, Some(_)) => RowDecision {
            classification: Kind::BDeleted,
            conflict: false,
            selection: B.into(),
        },
        (None, Some(_), Some(_)) if !row.equal_bc => RowDecision {
            classification: Kind::BCAdded,
            conflict: true,
            selection: OrderedSelection::new(),
        },
        (None, Some(_), Some(_)) => RowDecision {
            classification: Kind::BCAddedAndEqual,
            conflict: false,
            selection: C.into(),
        },
        (None, None, Some(_)) => RowDecision {
            classification: Kind::CAdded,
            conflict: false,
            selection: C.into(),
        },
        (None, Some(_), None) => RowDecision {
            classification: Kind::BAdded,
            conflict: false,
            selection: B.into(),
        },
        (Some(_), None, None) => RowDecision {
            classification: Kind::BCDeleted,
            conflict: false,
            selection: C.into(),
        },
        (None, None, None) => RowDecision {
            classification: Kind::Default,
            conflict: false,
            selection: OrderedSelection::new(),
        },
    }
}

fn row_whitespace_conflict(row: &AlignedRow, three_way: bool) -> bool {
    if three_way {
        (row.whitespace_equal_ab && row.whitespace_equal_ac)
            || (row.whitespace_a && row.whitespace_b && row.whitespace_c)
    } else {
        row.whitespace_equal_ab || (row.whitespace_a && row.whitespace_b)
    }
}

fn same_block_kind(
    previous: &RowDecision,
    next: &RowDecision,
    previous_row: &AlignedRow,
    next_row: &AlignedRow,
) -> bool {
    if previous.conflict && next.conflict {
        return previous_row.whitespace_equal_ac == next_row.whitespace_equal_ac
            && previous_row.whitespace_equal_ab == next_row.whitespace_equal_ab;
    }

    let previous_delta = previous.selection.as_slice() != [MergeSource::A];
    let next_delta = next.selection.as_slice() != [MergeSource::A];
    if !previous.conflict && !next.conflict && previous_delta && next_delta {
        return previous.selection == next.selection
            && (previous.classification == next.classification
                || (previous.classification != MergeBlockClassification::BCAddedAndEqual
                    && next.classification != MergeBlockClassification::BCAddedAndEqual));
    }

    !previous_delta && !next_delta
}

fn block_fingerprint(
    rows: &[AlignedRow],
    classification: MergeBlockClassification,
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
) -> u64 {
    use std::hash::Hasher as _;
    let mut hash = fnv::FnvHasher::default();
    hash.write(format!("{classification:?}").as_bytes());
    for row in rows {
        for (tag, index, lines) in [
            (b'A', row.a, a_lines),
            (b'B', row.b, b_lines),
            (b'C', row.c, c_lines),
        ] {
            hash.write(&[tag]);
            if let Some(line) = index.and_then(|index| lines.get(index)) {
                hash.write(line.as_bytes());
            } else {
                hash.write(&[0xff]);
            }
            hash.write(&[0]);
        }
    }
    hash.finish()
}

fn build_blocks(
    rows: &[AlignedRow],
    three_way: bool,
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
) -> Vec<MergeBlock> {
    if rows.is_empty() {
        return Vec::new();
    }
    let decisions: Vec<RowDecision> = rows
        .iter()
        .map(|row| {
            if three_way {
                classify_three_way(row)
            } else {
                classify_two_input(row)
            }
        })
        .collect();

    let mut raw = Vec::<(Range<usize>, RowDecision)>::new();
    let mut start = 0usize;
    for index in 1..rows.len() {
        if !same_block_kind(
            &decisions[index - 1],
            &decisions[index],
            &rows[index - 1],
            &rows[index],
        ) {
            raw.push((start..index, decisions[start].clone()));
            start = index;
        }
    }
    raw.push((start..rows.len(), decisions[start].clone()));

    let mut occurrences = FxHashMap::<u64, u32>::default();
    raw.into_iter()
        .map(|(range, decision)| {
            let fingerprint = block_fingerprint(
                &rows[range.clone()],
                decision.classification,
                a_lines,
                b_lines,
                c_lines,
            );
            let occurrence = occurrences.entry(fingerprint).or_default();
            let id = MergeBlockId {
                fingerprint,
                occurrence: *occurrence,
            };
            *occurrence = occurrence.saturating_add(1);
            let whitespace_conflict = decision.conflict
                && rows[range.clone()]
                    .iter()
                    .all(|row| row_whitespace_conflict(row, three_way));
            let is_delta = decision.selection.as_slice() != [MergeSource::A];
            let automatic_selection = decision.selection;
            MergeBlock {
                id,
                rows: range,
                classification: decision.classification,
                is_delta,
                original_conflict: decision.conflict,
                whitespace_conflict,
                selection: automatic_selection.clone(),
                automatic_selection,
                manual_content: None,
            }
        })
        .collect()
}

fn detect_line_ending(base: Option<&str>, local: &str, remote: &str) -> &'static str {
    // One pass per text: `matches("\r\n")` plus `matches('\n')` scanned each
    // input twice, on files that can be megabytes.
    let (mut crlf, mut lf) = (0usize, 0usize);
    for text in [base.unwrap_or_default(), local, remote] {
        let (text_crlf, text_lf) = gitcomet_text_utils_count_line_feeds(text);
        crlf += text_crlf;
        lf += text_lf - text_crlf;
    }
    if crlf > lf { "\r\n" } else { "\n" }
}

/// `(CRLF count, total LF count)` in one scan.
fn gitcomet_text_utils_count_line_feeds(text: &str) -> (usize, usize) {
    crate::text_utils::count_line_feeds(text)
}

fn source_indices_are_complete_and_ordered(
    rows: &[AlignedRow],
    source: MergeSource,
    expected_len: usize,
) -> bool {
    rows.iter()
        .filter_map(|row| row.line(source))
        .eq(0..expected_len)
}

fn exact_base_anchor_pairs(
    rows: &[AlignedRow],
    contributor: MergeSource,
) -> impl Iterator<Item = (usize, usize)> + '_ {
    rows.iter().filter_map(move |row| {
        let base = row.a?;
        let contributor_line = row.line(contributor)?;
        let equal = match contributor {
            MergeSource::B => row.equal_ab,
            MergeSource::C => row.equal_ac,
            MergeSource::A => true,
        };
        equal.then_some((base, contributor_line))
    })
}

/// Whether every exact base anchor of `baseline` survives in `candidate`.
///
/// Both row lists come out of base-anchored passes, so their base lines are
/// ascending and the pairs can be merged in one walk; that replaced four
/// `BTreeSet`s (a node allocation per equal row) per plan build. Falls back
/// to the set comparison if either sequence is ever not ascending.
fn base_anchor_pairs_subset(
    baseline: &[AlignedRow],
    candidate: &[AlignedRow],
    contributor: MergeSource,
) -> bool {
    let ascending = |rows: &[AlignedRow]| {
        exact_base_anchor_pairs(rows, contributor)
            .map(|(base, _)| base)
            .is_sorted_by(|left, right| left < right)
    };
    if !ascending(baseline) || !ascending(candidate) {
        let candidate: BTreeSet<(usize, usize)> =
            exact_base_anchor_pairs(candidate, contributor).collect();
        return exact_base_anchor_pairs(baseline, contributor)
            .all(|pair| candidate.contains(&pair));
    }
    let mut candidate = exact_base_anchor_pairs(candidate, contributor).peekable();
    'outer: for wanted in exact_base_anchor_pairs(baseline, contributor) {
        while let Some(&have) = candidate.peek() {
            if have.0 < wanted.0 {
                candidate.next();
                continue;
            }
            if have == wanted {
                continue 'outer;
            }
            return false;
        }
        return false;
    }
    true
}

fn contributor_alignment_preserves_base_anchors(
    baseline: &[AlignedRow],
    candidate: &[AlignedRow],
    a_len: usize,
    b_len: usize,
    c_len: usize,
) -> bool {
    source_indices_are_complete_and_ordered(candidate, MergeSource::A, a_len)
        && source_indices_are_complete_and_ordered(candidate, MergeSource::B, b_len)
        && source_indices_are_complete_and_ordered(candidate, MergeSource::C, c_len)
        && base_anchor_pairs_subset(baseline, candidate, MergeSource::B)
        && base_anchor_pairs_subset(baseline, candidate, MergeSource::C)
}

/// Whether every manual pin still holds: the first line each entry pins shares
/// one row across all the sources that entry names.
///
/// The contributor pass relocates B and C cells between rows that the
/// base-anchored passes placed, so a match that is legal within one B↔C
/// segment can still drag a line across a pin — and `trim_rows` only ever
/// pulls lines *earlier*, so it cannot put one back afterwards. KDiff3 repairs
/// this by re-running `correctManualDiffAlignment` after each automatic pass;
/// we instead keep the pass's result only when it left the pins intact, which
/// can never be worse than the pre-pass alignment it falls back to.
///
/// Matches KDiff3's own notion of a satisfied pin: `correctManualDiffAlignment`
/// aligns each entry's `firstLine(wi)`, not the whole pinned range.
fn alignment_preserves_manual_pins(
    rows: &[AlignedRow],
    entries: &[ManualAlignment],
    three_way: bool,
) -> bool {
    entries.iter().all(|entry| {
        let mut anchor: Option<usize> = None;
        for source in [MergeSource::A, MergeSource::B, MergeSource::C] {
            let range = entry.source_range(source, three_way);
            // An empty pinned range pins the other sources against nothing, so
            // it has no line of its own to co-locate.
            if range.is_empty() {
                continue;
            }
            let Some(row_index) = rows
                .iter()
                .position(|row| row.line(source) == Some(range.start))
            else {
                return false;
            };
            match anchor {
                None => anchor = Some(row_index),
                Some(anchor) => {
                    if anchor != row_index {
                        return false;
                    }
                }
            }
        }
        true
    })
}

/// Build a shared merge plan for an optional base.
pub fn build_merge_plan_with_optional_base(
    base: Option<&str>,
    local: &str,
    remote: &str,
    options: &MergeOptions,
) -> MergePlan {
    build_merge_plan_with_alignments(base, local, remote, options, &ManualAlignmentList::new())
}

/// Build a plan whose alignment honors the caller's manual constraints.
///
/// The constraints partition each input, and every diff pass runs per partition
/// so no automatic match can cross a pinned boundary. This is KDiff3's manual
/// diff help: the escape hatch for a block the automatic alignment gets wrong.
pub fn build_merge_plan_with_alignments(
    base: Option<&str>,
    local: &str,
    remote: &str,
    options: &MergeOptions,
    alignments: &ManualAlignmentList,
) -> MergePlan {
    let (a_text, b_text, c_text, three_way) = match base {
        Some(base) => (base, local, remote, true),
        None => (local, remote, "", false),
    };
    let a_lines = split_lines(a_text);
    let b_lines = split_lines(b_text);
    let c_lines = split_lines(c_text);

    let entries = alignments.as_slice();
    let barriers = AlignmentBarriers::new(entries, three_way);
    let segments = |left: MergeSource, left_len: usize, right: MergeSource, right_len: usize| {
        alignment_segments(entries, three_way, left, left_len, right, right_len)
    };

    let mut list = build_ab(&diff_chunks_segmented(
        &a_lines,
        &b_lines,
        options.diff_algorithm,
        &segments(MergeSource::A, a_lines.len(), MergeSource::B, b_lines.len()),
    ));
    recompute_rows_in_list(&mut list, &a_lines, &b_lines, &c_lines);

    let rows = if three_way {
        integrate_ac(
            &mut list,
            &diff_chunks_segmented(
                &a_lines,
                &c_lines,
                options.diff_algorithm,
                &segments(MergeSource::A, a_lines.len(), MergeSource::C, c_lines.len()),
            ),
        );
        recompute_rows_in_list(&mut list, &a_lines, &b_lines, &c_lines);
        let first_trim = trim_rows(list.into_rows(), &a_lines, &b_lines, &c_lines, &barriers);
        let mut list = RowList::from_rows(first_trim);
        recompute_rows_in_list(&mut list, &a_lines, &b_lines, &c_lines);
        if options.align_contributors && a_text != b_text && a_text != c_text && b_text != c_text {
            let baseline = list.clone().into_rows();
            let chunks = diff_chunks_segmented(
                &b_lines,
                &c_lines,
                options.diff_algorithm,
                &segments(MergeSource::B, b_lines.len(), MergeSource::C, c_lines.len()),
            );
            integrate_bc(&mut list, &chunks, &a_lines, &b_lines, &c_lines);
            recompute_rows_in_list(&mut list, &a_lines, &b_lines, &c_lines);
            let candidate = trim_rows(list.into_rows(), &a_lines, &b_lines, &c_lines, &barriers);
            if contributor_alignment_preserves_base_anchors(
                &baseline,
                &candidate,
                a_lines.len(),
                b_lines.len(),
                c_lines.len(),
            ) && alignment_preserves_manual_pins(&candidate, entries, three_way)
            {
                candidate
            } else {
                baseline
            }
        } else {
            list.into_rows()
        }
    } else {
        list.into_rows()
    };

    let blocks = build_blocks(&rows, three_way, &a_lines, &b_lines, &c_lines);
    let unresolved_blocks = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| (!block.is_resolved()).then_some(index))
        .collect();

    MergePlan {
        base: base.map(Arc::<str>::from),
        local: Arc::<str>::from(local),
        remote: Arc::<str>::from(remote),
        rows,
        blocks,
        unresolved_blocks,
        line_ending: detect_line_ending(base, local, remote),
    }
}

fn recompute_rows_in_list(
    rows: &mut RowList,
    a_lines: &[&str],
    b_lines: &[&str],
    c_lines: &[&str],
) {
    for node in &mut rows.nodes {
        if node.alive {
            recompute_row_metadata(&mut node.row, a_lines, b_lines, c_lines);
        }
    }
}

/// Build a shared three-input merge plan.
pub fn build_merge_plan(
    base: &str,
    local: &str,
    remote: &str,
    options: &MergeOptions,
) -> MergePlan {
    build_merge_plan_with_optional_base(Some(base), local, remote, options)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod perf_regression_tests;
