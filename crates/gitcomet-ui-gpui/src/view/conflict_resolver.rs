mod split_row_index;
mod word_highlight;

use super::CachedDiffStyledText;
use split_row_index::SparseLineIndex;
#[cfg(test)]
use split_row_index::{CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES, CONFLICT_SPLIT_PAGE_SIZE};
pub use split_row_index::{ConflictSplitRowIndex, TwoWaySplitProjection, TwoWaySplitVisibleRow};
#[cfg(any(test, feature = "benchmarks"))]
pub use word_highlight::compute_three_way_word_highlights;
#[cfg(feature = "benchmarks")]
pub use word_highlight::{TwoWayWordHighlights, compute_two_way_word_highlights};
pub use word_highlight::{compute_word_highlights_for_row, compute_word_highlights_for_texts};

use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

pub use gitcomet_core::conflict_output::ConflictOutputChoice as ConflictChoice;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictResolverViewMode {
    ThreeWay,
    TwoWayDiff,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictRenderingMode {
    EagerSmallFile,
    StreamedLargeFile,
}

impl ConflictRenderingMode {
    pub fn is_streamed_large_file(self) -> bool {
        matches!(self, Self::StreamedLargeFile)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub enum ConflictPickSide {
    Ours,
    Theirs,
}

#[derive(Clone, Debug, Default)]
struct ConflictSplitStyledTextCacheRow {
    ours: Option<CachedDiffStyledText>,
    theirs: Option<CachedDiffStyledText>,
}

const CONFLICT_SPLIT_STYLE_DENSE_ROWS: usize = 16_384;
const CONFLICT_SPLIT_STYLE_PAGE_ROWS: usize = 256;
const CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES: usize = 16;

#[derive(Clone, Debug, Default)]
pub(in crate::view) struct ConflictSplitStyledTextCache {
    rows: Vec<ConflictSplitStyledTextCacheRow>,
    sparse_pages: FxHashMap<usize, Vec<ConflictSplitStyledTextCacheRow>>,
    sparse_page_order: VecDeque<usize>,
    entries: usize,
}

impl ConflictSplitStyledTextCache {
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn with_row_capacity(row_count: usize) -> Self {
        let mut cache = Self::default();
        cache.rows.resize_with(
            row_count.min(CONFLICT_SPLIT_STYLE_DENSE_ROWS),
            ConflictSplitStyledTextCacheRow::default,
        );
        cache
    }

    fn slot(
        row: &ConflictSplitStyledTextCacheRow,
        side: ConflictPickSide,
    ) -> &Option<CachedDiffStyledText> {
        match side {
            ConflictPickSide::Ours => &row.ours,
            ConflictPickSide::Theirs => &row.theirs,
        }
    }

    fn slot_mut(
        row: &mut ConflictSplitStyledTextCacheRow,
        side: ConflictPickSide,
    ) -> &mut Option<CachedDiffStyledText> {
        match side {
            ConflictPickSide::Ours => &mut row.ours,
            ConflictPickSide::Theirs => &mut row.theirs,
        }
    }

    fn sparse_page_key(row_ix: usize) -> usize {
        (row_ix - CONFLICT_SPLIT_STYLE_DENSE_ROWS) / CONFLICT_SPLIT_STYLE_PAGE_ROWS
    }

    fn sparse_page_offset(row_ix: usize) -> usize {
        (row_ix - CONFLICT_SPLIT_STYLE_DENSE_ROWS) % CONFLICT_SPLIT_STYLE_PAGE_ROWS
    }

    fn row_entry_count(row: &ConflictSplitStyledTextCacheRow) -> usize {
        usize::from(row.ours.is_some()) + usize::from(row.theirs.is_some())
    }

    fn ensure_row(&mut self, row_ix: usize) -> &mut ConflictSplitStyledTextCacheRow {
        if row_ix < CONFLICT_SPLIT_STYLE_DENSE_ROWS {
            if row_ix >= self.rows.len() {
                self.rows
                    .resize_with(row_ix + 1, ConflictSplitStyledTextCacheRow::default);
            }
            return &mut self.rows[row_ix];
        }

        let page_key = Self::sparse_page_key(row_ix);
        if !self.sparse_pages.contains_key(&page_key) {
            while self.sparse_pages.len() >= CONFLICT_SPLIT_STYLE_MAX_SPARSE_PAGES {
                let Some(evicted_key) = self.sparse_page_order.pop_front() else {
                    break;
                };
                if let Some(evicted) = self.sparse_pages.remove(&evicted_key) {
                    let evicted_entries = evicted.iter().map(Self::row_entry_count).sum::<usize>();
                    self.entries = self.entries.saturating_sub(evicted_entries);
                }
            }
            self.sparse_pages.insert(
                page_key,
                vec![ConflictSplitStyledTextCacheRow::default(); CONFLICT_SPLIT_STYLE_PAGE_ROWS],
            );
            self.sparse_page_order.push_back(page_key);
        }
        &mut self
            .sparse_pages
            .get_mut(&page_key)
            .expect("inserted conflict style cache page")[Self::sparse_page_offset(row_ix)]
    }

    pub(in crate::view) fn get(
        &self,
        key: &(usize, ConflictPickSide),
    ) -> Option<&CachedDiffStyledText> {
        let (row_ix, side) = *key;
        let row = if row_ix < CONFLICT_SPLIT_STYLE_DENSE_ROWS {
            self.rows.get(row_ix)?
        } else {
            self.sparse_pages
                .get(&Self::sparse_page_key(row_ix))?
                .get(Self::sparse_page_offset(row_ix))?
        };
        Self::slot(row, side).as_ref()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn contains_key(&self, key: &(usize, ConflictPickSide)) -> bool {
        self.get(key).is_some()
    }

    pub(in crate::view) fn insert(
        &mut self,
        key: (usize, ConflictPickSide),
        value: CachedDiffStyledText,
    ) -> Option<CachedDiffStyledText> {
        let (row_ix, side) = key;
        let slot = Self::slot_mut(self.ensure_row(row_ix), side);
        let previous = slot.replace(value);
        if previous.is_none() {
            self.entries = self.entries.saturating_add(1);
        }
        previous
    }

    pub(in crate::view) fn clear(&mut self) {
        self.rows.clear();
        self.sparse_pages.clear();
        self.sparse_page_order.clear();
        self.entries = 0;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn len(&self) -> usize {
        self.entries
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn is_empty(&self) -> bool {
        self.entries == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveTraceMode {
    /// High+Medium tiers applied automatically when the file opened (section 30).
    OnOpen,
    #[cfg(test)]
    History,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNavDirection {
    Prev,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum ConflictNavTargetId {
    PlanBlock(gitcomet_core::merge::MergeBlockId),
    Region(usize),
    DisplayBlock(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct ConflictNavTarget {
    pub(in crate::view) id: ConflictNavTargetId,
    pub(in crate::view) order: usize,
    pub(in crate::view) aligned_rows: Option<Range<usize>>,
    pub(in crate::view) region_index: Option<usize>,
    pub(in crate::view) display_conflict_index: Option<usize>,
    pub(in crate::view) is_delta: bool,
    pub(in crate::view) original_conflict: bool,
    pub(in crate::view) unresolved: bool,
}

impl ConflictNavTarget {
    pub(in crate::view) fn anchor(&self) -> ConflictNavAnchor {
        ConflictNavAnchor {
            id: self.id,
            order_hint: self.order,
            aligned_row_hint: self.aligned_rows.as_ref().map(|range| range.start),
        }
    }

    fn contains_aligned_row(&self, row: usize) -> bool {
        self.aligned_rows
            .as_ref()
            .is_some_and(|range| range.contains(&row) || (range.is_empty() && range.start == row))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct ConflictNavAnchor {
    pub(in crate::view) id: ConflictNavTargetId,
    pub(in crate::view) order_hint: usize,
    pub(in crate::view) aligned_row_hint: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum ConflictNavTargetFilter {
    Delta,
    OriginalConflict,
    Unresolved,
}

impl ConflictNavTargetFilter {
    fn matches(self, target: &ConflictNavTarget) -> bool {
        match self {
            Self::Delta => target.is_delta,
            Self::OriginalConflict => target.original_conflict,
            Self::Unresolved => target.unresolved,
        }
    }
}

pub(in crate::view) fn fresh_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
) -> Option<usize> {
    targets
        .iter()
        .position(|target| target.unresolved)
        .or_else(|| targets.iter().position(|target| target.original_conflict))
        .or_else(|| targets.iter().position(|target| target.is_delta))
}

pub(in crate::view) fn reconcile_conflict_nav_target_index(
    anchor: Option<ConflictNavAnchor>,
    previous_targets: &[ConflictNavTarget],
    targets: &[ConflictNavTarget],
) -> Option<usize> {
    let Some(anchor) = anchor else {
        return fresh_conflict_nav_target_index(targets);
    };
    if targets.is_empty() {
        return None;
    }

    // Stable plan identities and unchanged legacy identities win.
    if let Some(index) = targets.iter().position(|target| target.id == anchor.id) {
        return Some(index);
    }

    // Bridge plan-backed and region-backed projections in either direction.
    let previous_region = match anchor.id {
        ConflictNavTargetId::Region(region_index) => Some(region_index),
        ConflictNavTargetId::PlanBlock(_) | ConflictNavTargetId::DisplayBlock(_) => {
            previous_targets
                .iter()
                .find(|target| target.id == anchor.id)
                .and_then(|target| target.region_index)
        }
    };
    if let Some(region_index) = previous_region
        && let Some(index) = targets
            .iter()
            .position(|target| target.region_index == Some(region_index))
    {
        return Some(index);
    }

    // Structural edits can replace target identities while retaining row
    // geometry. Prefer the target that still owns the remembered row.
    if let Some(row) = anchor.aligned_row_hint
        && let Some(index) = targets
            .iter()
            .position(|target| target.contains_aligned_row(row))
    {
        return Some(index);
    }

    // Finally retain the nearest ordered position. Orders are monotonic but
    // need not be dense, so compare explicitly after clamping the hint.
    let max_order = targets.iter().map(|target| target.order).max().unwrap_or(0);
    let clamped_order = anchor.order_hint.min(max_order);
    targets
        .iter()
        .enumerate()
        .min_by_key(|(_, target)| (target.order.abs_diff(clamped_order), target.order))
        .map(|(index, _)| index)
        .or_else(|| fresh_conflict_nav_target_index(targets))
}

fn conflict_nav_anchor_order(targets: &[ConflictNavTarget], anchor: ConflictNavAnchor) -> usize {
    targets
        .iter()
        .find(|target| target.id == anchor.id)
        .map(|target| target.order)
        .unwrap_or(anchor.order_hint)
}

pub(in crate::view) fn previous_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    let current_order = conflict_nav_anchor_order(targets, anchor);
    targets
        .iter()
        .enumerate()
        .rev()
        .find(|(_, target)| target.order < current_order && filter.matches(target))
        .map(|(index, _)| index)
}

pub(in crate::view) fn next_conflict_nav_target_index(
    targets: &[ConflictNavTarget],
    anchor: Option<ConflictNavAnchor>,
    filter: ConflictNavTargetFilter,
) -> Option<usize> {
    let anchor = anchor?;
    let current_order = conflict_nav_anchor_order(targets, anchor);
    targets
        .iter()
        .enumerate()
        .find(|(_, target)| target.order > current_order && filter.matches(target))
        .map(|(index, _)| index)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ConflictTextStorage {
    Owned(String),
    SharedSlice { text: Arc<str>, range: Range<usize> },
}

#[derive(Clone, Debug)]
pub struct ConflictText {
    storage: ConflictTextStorage,
}

impl ConflictText {
    pub fn shared(text: Arc<str>) -> Self {
        let len = text.len();
        Self {
            storage: ConflictTextStorage::SharedSlice {
                text,
                range: 0..len,
            },
        }
    }

    pub fn shared_slice(text: Arc<str>, range: Range<usize>) -> Self {
        debug_assert!(
            text.get(range.clone()).is_some(),
            "shared conflict text range should stay within bounds"
        );
        Self {
            storage: ConflictTextStorage::SharedSlice { text, range },
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.storage {
            ConflictTextStorage::Owned(text) => text.as_str(),
            ConflictTextStorage::SharedSlice { text, range } => text
                .get(range.clone())
                .expect("shared conflict text range should stay valid"),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn len(&self) -> usize {
        self.as_str().len()
    }

    pub fn push_str(&mut self, suffix: &str) {
        if suffix.is_empty() {
            return;
        }

        match &mut self.storage {
            ConflictTextStorage::Owned(text) => text.push_str(suffix),
            ConflictTextStorage::SharedSlice { .. } => {
                let mut owned = self.as_str().to_string();
                owned.push_str(suffix);
                self.storage = ConflictTextStorage::Owned(owned);
            }
        }
    }

    pub fn into_owned_string(self) -> String {
        match self.storage {
            ConflictTextStorage::Owned(text) => text,
            ConflictTextStorage::SharedSlice { text, range } => text
                .get(range)
                .expect("shared conflict text range should stay valid")
                .to_string(),
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn shares_backing_with(&self, other: &Arc<str>) -> bool {
        match &self.storage {
            ConflictTextStorage::Owned(_) => false,
            ConflictTextStorage::SharedSlice { text, .. } => Arc::ptr_eq(text, other),
        }
    }
}

impl Default for ConflictText {
    fn default() -> Self {
        String::new().into()
    }
}

impl std::fmt::Display for ConflictText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::ops::Deref for ConflictText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for ConflictText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for ConflictText {
    fn from(value: String) -> Self {
        Self::shared(Arc::from(value))
    }
}

impl From<&str> for ConflictText {
    fn from(value: &str) -> Self {
        Self::shared(Arc::from(value))
    }
}

impl PartialEq for ConflictText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for ConflictText {}

impl PartialEq<&str> for ConflictText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ConflictText> for &str {
    fn eq(&self, other: &ConflictText) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<String> for ConflictText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<ConflictText> for String {
    fn eq(&self, other: &ConflictText) -> bool {
        self.as_str() == other.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictBlock {
    pub base: Option<ConflictText>,
    pub ours: ConflictText,
    pub theirs: ConflictText,
    pub choice: ConflictChoice,
    /// Whether this block has been explicitly resolved (by user pick or auto-resolve).
    /// Blocks start unresolved; becomes `true` when the user picks a side or auto-resolve runs.
    pub resolved: bool,
    /// Whether every aligned row in this block differs only in whitespace
    /// (kdiff3 `MergeBlock::bWhiteSpaceConflict`).
    ///
    /// Set from the merge plan's classification, which applies kdiff3's
    /// per-row rule; marker-only sessions have no aligned rows to classify and
    /// leave this `false`. Drives the `(Whitespace only)` placeholder variant.
    pub whitespace_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictSegment {
    Text(ConflictText),
    Block(ConflictBlock),
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictInlineRow {
    pub side: ConflictPickSide,
    pub kind: gitcomet_core::domain::DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

/// Source provenance for a resolved output line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ResolvedLineSource {
    /// Line matches source A (Base in three-way, Ours in two-way).
    A,
    /// Line matches source B (Ours in three-way, Theirs in two-way).
    B,
    /// Line matches source C (Theirs in three-way; not used in two-way).
    C,
    /// Line was manually edited or does not match any source.
    Manual,
}

impl ResolvedLineSource {
    #[cfg(test)]
    /// Compact single-character label for UI badges.
    pub fn badge_char(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::Manual => 'M',
        }
    }
}

/// Packed per-line gutter state for resolved-output preview rows.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct ResolvedOutputGutterRow(u32);

impl ResolvedOutputGutterRow {
    const SOURCE_MASK: u32 = 0b11;
    const SOURCE_A: u32 = 0;
    const SOURCE_B: u32 = 1;
    const SOURCE_C: u32 = 2;
    const SOURCE_MANUAL: u32 = 3;
    const IS_START_FLAG: u32 = 1 << 2;
    const IS_END_FLAG: u32 = 1 << 3;
    const UNRESOLVED_FLAG: u32 = 1 << 4;
    /// The row renders an unresolved-conflict placeholder.
    ///
    /// kdiff3 reads the `?`, the conflict color and the placeholder string off
    /// one field (`srcSelect`, `mergeresultwindow.cpp:1515`), so its gutter can
    /// never disagree with its text. Our marker array is built separately and
    /// can go stale across incremental edits, so this flag lets the row fall
    /// back to the text's own verdict.
    const PLACEHOLDER_FLAG: u32 = 1 << 5;
    const CONFLICT_SHIFT: u32 = 6;
    const CONFLICT_VALUE_MASK: u32 = u32::MAX >> Self::CONFLICT_SHIFT;

    pub(in crate::view) fn new(
        source: ResolvedLineSource,
        marker_conflict_ix: Option<usize>,
        is_start: bool,
        is_end: bool,
        unresolved: bool,
    ) -> Self {
        let mut bits = match source {
            ResolvedLineSource::A => Self::SOURCE_A,
            ResolvedLineSource::B => Self::SOURCE_B,
            ResolvedLineSource::C => Self::SOURCE_C,
            ResolvedLineSource::Manual => Self::SOURCE_MANUAL,
        };

        if let Some(conflict_ix) = marker_conflict_ix {
            if is_start {
                bits |= Self::IS_START_FLAG;
            }
            if is_end {
                bits |= Self::IS_END_FLAG;
            }
            if unresolved {
                bits |= Self::UNRESOLVED_FLAG;
            }
            let encoded_conflict = u32::try_from(conflict_ix)
                .ok()
                .and_then(|ix| ix.checked_add(1))
                .unwrap_or(Self::CONFLICT_VALUE_MASK)
                .min(Self::CONFLICT_VALUE_MASK);
            bits |= encoded_conflict << Self::CONFLICT_SHIFT;
        }

        Self(bits)
    }

    /// Mark the row as rendering an unresolved-conflict placeholder.
    ///
    /// A placeholder is a whole one-line block, so it reads as an unresolved
    /// marker that both starts and ends on this row even when the marker array
    /// has no entry for it.
    #[inline(always)]
    pub(in crate::view) fn with_unresolved_placeholder(self) -> Self {
        Self(self.0 | Self::PLACEHOLDER_FLAG)
    }

    #[inline(always)]
    fn is_placeholder(self) -> bool {
        (self.0 & Self::PLACEHOLDER_FLAG) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn source(self) -> ResolvedLineSource {
        match self.0 & Self::SOURCE_MASK {
            Self::SOURCE_A => ResolvedLineSource::A,
            Self::SOURCE_B => ResolvedLineSource::B,
            Self::SOURCE_C => ResolvedLineSource::C,
            _ => ResolvedLineSource::Manual,
        }
    }

    #[inline(always)]
    pub(in crate::view) fn badge_char(self) -> char {
        if self.has_marker() && self.unresolved() {
            return '?';
        }
        match self.0 & Self::SOURCE_MASK {
            Self::SOURCE_A => 'A',
            Self::SOURCE_B => 'B',
            Self::SOURCE_C => 'C',
            _ => 'M',
        }
    }

    #[inline(always)]
    pub(in crate::view) fn has_marker(self) -> bool {
        self.is_placeholder() || (self.0 >> Self::CONFLICT_SHIFT) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn marker_conflict_ix(self) -> Option<usize> {
        let encoded_conflict = self.0 >> Self::CONFLICT_SHIFT;
        (encoded_conflict != 0).then(|| (encoded_conflict - 1) as usize)
    }

    #[inline(always)]
    pub(in crate::view) fn is_start(self) -> bool {
        self.is_placeholder() || (self.0 & Self::IS_START_FLAG) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn is_end(self) -> bool {
        self.is_placeholder() || (self.0 & Self::IS_END_FLAG) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn unresolved(self) -> bool {
        self.is_placeholder() || (self.0 & Self::UNRESOLVED_FLAG) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn manual_without_marker(self) -> bool {
        !self.has_marker() && (self.0 & Self::SOURCE_MASK) == Self::SOURCE_MANUAL
    }
}

impl Default for ResolvedOutputGutterRow {
    fn default() -> Self {
        Self(Self::SOURCE_MANUAL)
    }
}

/// Per-line provenance metadata for the resolved output outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLineMeta {
    /// 0-based line index in the resolved output.
    pub output_line: u32,
    /// Which source this line came from (or Manual).
    pub source: ResolvedLineSource,
    /// If source is A/B/C, the 1-based line number in that source pane.
    pub input_line: Option<u32>,
}

/// Key identifying a specific source line for dedupe gating (plus-icon visibility).
///
/// Two source lines with the same key are considered "the same row" for purposes
/// of preventing duplicate insertion into the resolved output.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceLineKey {
    pub view_mode: ConflictResolverViewMode,
    pub side: ResolvedLineSource,
    /// 1-based line number in the source pane.
    pub line_no: u32,
    /// Hash of the line's text content for fast equality checks.
    pub content_hash: u64,
}

impl SourceLineKey {
    pub fn new(
        view_mode: ConflictResolverViewMode,
        side: ResolvedLineSource,
        line_no: u32,
        content: &str,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        content.hash(&mut hasher);
        Self {
            view_mode,
            side,
            line_no,
            content_hash: hasher.finish(),
        }
    }
}

/// Per-line word-highlight ranges. `None` means no highlights for that line.
pub type WordHighlights = FxHashMap<usize, Vec<Range<usize>>>;

/// Per-line pair of `(old, new)` word-highlight ranges for a two-way diff row.
pub type TwoWayWordHighlightPair = (
    crate::view::word_diff::WordDiffRanges,
    crate::view::word_diff::WordDiffRanges,
);

const CONFLICT_SPLIT_WORD_HIGHLIGHT_CACHE_ROWS: usize = 4_096;

/// Bounded render cache for giant two-way conflicts. The same row is rendered
/// independently by the left and right lists, so sharing the computed pair here
/// avoids running the word diff twice per frame without retaining the whole file.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct ConflictSplitWordHighlightCache {
    rows: FxHashMap<usize, Arc<TwoWayWordHighlightPair>>,
    insertion_order: VecDeque<usize>,
}

impl ConflictSplitWordHighlightCache {
    pub(in crate::view) fn get(&self, row_ix: usize) -> Option<Arc<TwoWayWordHighlightPair>> {
        self.rows.get(&row_ix).cloned()
    }

    pub(in crate::view) fn insert(
        &mut self,
        row_ix: usize,
        highlights: TwoWayWordHighlightPair,
    ) -> Arc<TwoWayWordHighlightPair> {
        if let Some(existing) = self.rows.get(&row_ix) {
            return Arc::clone(existing);
        }
        while self.rows.len() >= CONFLICT_SPLIT_WORD_HIGHLIGHT_CACHE_ROWS {
            let Some(evicted) = self.insertion_order.pop_front() else {
                break;
            };
            self.rows.remove(&evicted);
        }
        let highlights = Arc::new(highlights);
        self.rows.insert(row_ix, Arc::clone(&highlights));
        self.insertion_order.push_back(row_ix);
        highlights
    }

    pub(in crate::view) fn clear(&mut self) {
        self.rows.clear();
        self.insertion_order.clear();
    }
}

/// Shared context rows kept around each block-local two-way conflict diff.
///
/// This preserves a small amount of unchanged surrounding code in the large-file
/// sparse path without regressing back to whole-file row materialization.
pub(crate) const BLOCK_LOCAL_DIFF_CONTEXT_LINES: usize = 3;
/// Above this size, one conflict block is effectively the whole document.
///
/// Bootstrap should stay bounded instead of diffing the entire block eagerly.
pub(crate) const LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES: usize = 20_000;
/// Above this merged-output line count the resolved output stays in read-only
/// streamed mode instead of materializing the whole text into the editable
/// `TextInput` buffer — the perf guard for whole-file conflicts. Sits above
/// [`LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES`] (a large-but-editable output still
/// materializes) and below the whole-file streamed fixtures at `+ 1_000`.
pub(crate) const RESOLVED_OUTPUT_EDITABLE_MAX_LINES: usize =
    LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 500;
/// Head/tail preview rows kept for very large conflict blocks during bootstrap.
#[cfg(any(test, feature = "benchmarks"))]
pub(crate) const LARGE_CONFLICT_BLOCK_PREVIEW_LINES: usize = 128;
/// Word-diff highlighting is optional chrome, so skip giant blocks entirely.
#[cfg(any(test, feature = "benchmarks"))]
pub(crate) const LARGE_CONFLICT_BLOCK_WORD_HIGHLIGHT_MAX_LINES: usize = 4_000;

/// Ordered pick choices for a view mode. Both the letter (`a/b/c/d`) and the
/// `Ctrl+1/2/3` shortcuts index into this list, so the key→choice mapping lives
/// in one place per mode. Note `Both` sits last, so `Ctrl+1/2/3` reaches it only
/// in two-way mode (three-way exposes it via the `d` letter pick).
fn conflict_pick_choices(view_mode: ConflictResolverViewMode) -> &'static [ConflictChoice] {
    match view_mode {
        ConflictResolverViewMode::ThreeWay => &[
            ConflictChoice::Base,
            ConflictChoice::Ours,
            ConflictChoice::Theirs,
            ConflictChoice::Both,
        ],
        ConflictResolverViewMode::TwoWayDiff => &[
            ConflictChoice::Ours,
            ConflictChoice::Theirs,
            ConflictChoice::Both,
        ],
    }
}

/// Resolve conflict quick-pick keyboard shortcuts to a concrete choice.
pub fn conflict_quick_pick_choice_for_key(
    key: &str,
    view_mode: ConflictResolverViewMode,
) -> Option<ConflictChoice> {
    let index = match key {
        "a" => 0,
        "b" => 1,
        "c" => 2,
        "d" => 3,
        _ => return None,
    };
    conflict_pick_choices(view_mode).get(index).copied()
}

/// Resolve kdiff3-compatible `Ctrl+1/2/3` pick aliases (section 30 keyboard model).
///
/// Unlike the single-letter picks these also work while the output editor is
/// focused, since they cannot collide with text input.
pub fn conflict_ctrl_pick_choice_for_key(
    key: &str,
    view_mode: ConflictResolverViewMode,
) -> Option<ConflictChoice> {
    let index = match key {
        "1" => 0,
        "2" => 1,
        "3" => 2,
        _ => return None,
    };
    conflict_pick_choices(view_mode).get(index).copied()
}

/// Resolve conflict navigation shortcuts (`F2`, `F3`, `F7`) to a direction.
#[cfg(test)]
pub fn conflict_nav_direction_for_key(key: &str, shift: bool) -> Option<ConflictNavDirection> {
    match key {
        "f2" => Some(ConflictNavDirection::Prev),
        "f3" => Some(ConflictNavDirection::Next),
        "f7" if shift => Some(ConflictNavDirection::Prev),
        "f7" => Some(ConflictNavDirection::Next),
        _ => None,
    }
}

/// Build a user-facing summary for the most recent autosolve run.
///
/// The summary is shown in the resolver UI so autosolve behavior remains
/// auditable without opening command logs.
pub fn format_autosolve_trace_summary(
    mode: AutosolveTraceMode,
    unresolved_before: usize,
    unresolved_after: usize,
    stats: &gitcomet_state::msg::ConflictAutosolveStats,
) -> String {
    let resolved = unresolved_before.saturating_sub(unresolved_after);
    let blocks_word = if resolved == 1 { "block" } else { "blocks" };
    match mode {
        AutosolveTraceMode::OnOpen => format!(
            "Auto-solved on open: resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, regex {}).",
            unresolved_before, unresolved_after, stats.pass1, stats.pass2_split, stats.regex
        ),
        #[cfg(test)]
        AutosolveTraceMode::History => format!(
            "Last autosolve (history): resolved {resolved} {blocks_word}, unresolved {} -> {} (history {}).",
            unresolved_before, unresolved_after, stats.history
        ),
    }
}

/// KDiff3-style accounting for a merge session.
///
/// Plan-backed sessions count every block classified as a conflict or delta,
/// not just the subset that still needs a user decision. The whitespace count
/// is optional because marker-only fallback sessions do not have KDiff3's
/// exact aligned-row classification available.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictSummaryCounts {
    pub total: usize,
    pub auto_solved: usize,
    pub unsolved: usize,
    pub whitespace_conflicts: Option<usize>,
}

impl ConflictSummaryCounts {
    fn normalized(self) -> Self {
        let unsolved = self.unsolved.min(self.total);
        Self {
            auto_solved: self.auto_solved.min(self.total.saturating_sub(unsolved)),
            unsolved,
            whitespace_conflicts: self.whitespace_conflicts.map(|count| count.min(self.total)),
            ..self
        }
    }
}

/// Format the shared total / auto-solved / unsolved report used by the toast
/// and resolver status bar.
pub fn format_conflict_summary(counts: ConflictSummaryCounts) -> String {
    let counts = counts.normalized();
    format!(
        "Total {} / auto-solved {} / unsolved {}",
        counts.total, counts.auto_solved, counts.unsolved
    )
}

/// Build the one-shot toast message pushed when a conflict file's resolver
/// opens fresh.
pub fn format_open_summary_toast(counts: ConflictSummaryCounts) -> Option<String> {
    if counts.total == 0 {
        return None;
    }
    Some(format_conflict_summary(counts))
}

/// Count a session using KDiff3's conflict-reporting convention.
///
/// With a merge plan, `total` is every stable conflict-or-delta block,
/// `unsolved` is the currently unresolved subset, and `auto_solved` is their
/// difference. Marker-only sessions retain region-based fallback accounting.
pub fn conflict_session_summary_counts(
    session: &gitcomet_core::conflict_session::ConflictSession,
) -> ConflictSummaryCounts {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    if let Some(plan) = session.merge_plan.as_ref() {
        let mut total = 0usize;
        let mut unsolved = 0usize;
        let mut whitespace_conflicts = 0usize;
        for block in plan
            .blocks
            .iter()
            .filter(|block| block.original_conflict || block.is_delta)
        {
            total += 1;
            unsolved += usize::from(!block.is_resolved());
            whitespace_conflicts += usize::from(block.whitespace_conflict);
        }
        return ConflictSummaryCounts {
            total,
            auto_solved: total.saturating_sub(unsolved),
            unsolved,
            whitespace_conflicts: Some(whitespace_conflicts),
        };
    }

    let auto_solved = session
        .regions
        .iter()
        .filter(|region| {
            matches!(
                &region.resolution,
                ConflictRegionResolution::AutoResolved { .. }
            )
        })
        .count();
    ConflictSummaryCounts {
        total: session.total_regions(),
        auto_solved,
        unsolved: session.unsolved_count(),
        whitespace_conflicts: None,
    }
}

/// Summarize the on-open autosolve pass from session region resolutions.
///
/// On a fresh open every resolved region is an [`AutoResolved`] one (user
/// picks cannot exist yet), so the confidence-tier breakdown can be
/// reconstructed from the applied rules. Returns `None` when nothing was
/// auto-resolved.
///
/// [`AutoResolved`]: gitcomet_core::conflict_session::ConflictRegionResolution::AutoResolved
pub fn on_open_autosolve_summary(
    session: &gitcomet_core::conflict_session::ConflictSession,
) -> Option<String> {
    use gitcomet_core::conflict_session::{AutosolveRule, ConflictRegionResolution};

    let mut stats = gitcomet_state::msg::ConflictAutosolveStats::default();
    for region in &session.regions {
        let ConflictRegionResolution::AutoResolved { rule, .. } = &region.resolution else {
            continue;
        };
        match rule {
            AutosolveRule::IdenticalSides
            | AutosolveRule::OnlyOursChanged
            | AutosolveRule::OnlyTheirsChanged
            | AutosolveRule::WhitespaceOnly => stats.pass1 += 1,
            AutosolveRule::SubchunkFullyMerged => stats.pass2_split += 1,
            AutosolveRule::RegexEquivalentSides
            | AutosolveRule::RegexOnlyTheirsChanged
            | AutosolveRule::RegexOnlyOursChanged => stats.regex += 1,
            AutosolveRule::HistoryMerged => stats.history += 1,
        }
    }

    let resolved = stats.total_resolved();
    if resolved == 0 {
        return None;
    }
    let unresolved_after = session.unsolved_count();
    Some(format_autosolve_trace_summary(
        AutosolveTraceMode::OnOpen,
        unresolved_after + resolved,
        unresolved_after,
        &stats,
    ))
}

/// Count conflict blocks whose backing session regions were auto-resolved when
/// the resolver opened.
/// Build a per-conflict autosolve trace label for the active conflict.
///
/// Returns `None` when the active conflict does not map to an auto-resolved
/// session region.
pub fn active_conflict_autosolve_trace_label(
    session: &gitcomet_core::conflict_session::ConflictSession,
    conflict_region_indices: &[usize],
    active_conflict: usize,
) -> Option<String> {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    let region_index = *conflict_region_indices.get(active_conflict)?;
    let region = session.regions.get(region_index)?;
    if let ConflictRegionResolution::AutoResolved {
        rule, confidence, ..
    } = &region.resolution
    {
        Some(format!(
            "Auto: {} ({})",
            rule.description(),
            confidence.label()
        ))
    } else {
        None
    }
}

pub fn parse_conflict_markers(text: &str) -> Vec<ConflictSegment> {
    parse_conflict_markers_shared(Arc::<str>::from(text))
}

pub fn parse_conflict_markers_shared(text: Arc<str>) -> Vec<ConflictSegment> {
    gitcomet_core::conflict_session::parse_conflict_marker_ranges(text.as_ref())
        .into_iter()
        .map(|segment| match segment {
            gitcomet_core::conflict_session::ParsedConflictSegmentRanges::Text(range) => {
                ConflictSegment::Text(ConflictText::shared_slice(Arc::clone(&text), range))
            }
            gitcomet_core::conflict_session::ParsedConflictSegmentRanges::Conflict(block) => {
                ConflictSegment::Block(ConflictBlock {
                    base: block
                        .base
                        .map(|range| ConflictText::shared_slice(Arc::clone(&text), range)),
                    ours: ConflictText::shared_slice(Arc::clone(&text), block.ours),
                    theirs: ConflictText::shared_slice(Arc::clone(&text), block.theirs),
                    choice: ConflictChoice::empty(),
                    resolved: false,
                    whitespace_only: false,
                })
            }
        })
        .collect()
}

/// Parse marker segments only when the text plausibly contains conflict
/// markers, returning an empty segment list for clean inputs.
pub fn parse_conflict_markers_shared_nonempty(text: Arc<str>) -> Vec<ConflictSegment> {
    if memchr::memmem::find(text.as_bytes(), b"<<<<<<<").is_none() {
        return Vec::new();
    }

    let segments = parse_conflict_markers_shared(text);
    if conflict_count(&segments) == 0 {
        Vec::new()
    } else {
        segments
    }
}

fn append_text_segment(segments: &mut Vec<ConflictSegment>, text: impl Into<ConflictText>) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    if let Some(ConflictSegment::Text(prev)) = segments.last_mut() {
        prev.push_str(text.as_str());
        return;
    }
    segments.push(ConflictSegment::Text(text));
}

fn choice_for_resolved_content(block: &ConflictBlock, content: &str) -> Option<ConflictChoice> {
    if !block.choice.is_empty() && content_matches_block_choice(block, content) {
        return Some(block.choice);
    }
    if content == block.ours {
        return Some(ConflictChoice::Ours);
    }
    if content == block.theirs {
        return Some(ConflictChoice::Theirs);
    }
    if block.base.as_deref().is_some_and(|base| content == base) {
        return Some(ConflictChoice::Base);
    }
    content
        .strip_prefix(block.ours.as_str())
        .is_some_and(|rest| rest == block.theirs)
        .then_some(ConflictChoice::Both)
}

fn content_matches_block_choice(block: &ConflictBlock, content: &str) -> bool {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut selected = String::new();
    for source in block.choice.iter() {
        match source {
            ConflictOutputSource::Base => {
                let Some(base) = block.base.as_deref() else {
                    return false;
                };
                selected.push_str(base);
            }
            ConflictOutputSource::Ours => selected.push_str(&block.ours),
            ConflictOutputSource::Theirs => selected.push_str(&block.theirs),
        }
    }
    selected == content
}

fn resolution_for_choice(
    choice: ConflictChoice,
    has_base: bool,
) -> gitcomet_core::conflict_session::ConflictRegionResolution {
    use gitcomet_core::conflict_output::ConflictOutputSource;
    use gitcomet_core::conflict_session::ConflictRegionResolution;
    use gitcomet_core::merge::{MergeSource, OrderedSelection};

    let selection = OrderedSelection::from_sources(choice.iter().filter_map(|source| {
        match (has_base, source) {
            (true, ConflictOutputSource::Base) => Some(MergeSource::A),
            (true, ConflictOutputSource::Ours) => Some(MergeSource::B),
            (true, ConflictOutputSource::Theirs) => Some(MergeSource::C),
            (false, ConflictOutputSource::Base) => None,
            (false, ConflictOutputSource::Ours) => Some(MergeSource::A),
            (false, ConflictOutputSource::Theirs) => Some(MergeSource::B),
        }
    }));
    if selection.is_empty() {
        ConflictRegionResolution::Unresolved
    } else {
        ConflictRegionResolution::Sources(selection)
    }
}

pub(in crate::view) fn choice_for_selection(
    selection: &gitcomet_core::merge::OrderedSelection,
    has_base: bool,
) -> Option<ConflictChoice> {
    use gitcomet_core::conflict_output::{ConflictOutputChoice, ConflictOutputSource};
    use gitcomet_core::merge::MergeSource;

    let mut choice = ConflictOutputChoice::empty();
    for source in selection.iter() {
        let output_source = match (has_base, source) {
            (true, MergeSource::A) => ConflictOutputSource::Base,
            (true, MergeSource::B) => ConflictOutputSource::Ours,
            (true, MergeSource::C) => ConflictOutputSource::Theirs,
            (false, MergeSource::A) => ConflictOutputSource::Ours,
            (false, MergeSource::B) => ConflictOutputSource::Theirs,
            (false, MergeSource::C) => return None,
        };
        choice.append(output_source);
    }
    Some(choice)
}

/// Byte ownership for the editable output of each displayed conflict block.
///
/// The editor text is authoritative; these ranges only decide which bytes
/// belong to which conflict when deriving session resolutions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResolvedOutputBlockMap {
    ranges: Vec<Range<usize>>,
    text_len: usize,
}

impl ResolvedOutputBlockMap {
    pub(in crate::view) fn from_segments(segments: &[ConflictSegment]) -> Self {
        let mut ranges = Vec::with_capacity(conflict_count(segments));
        let mut cursor = 0usize;
        for segment in segments {
            match segment {
                ConflictSegment::Text(text) => {
                    cursor = cursor.saturating_add(text.len());
                }
                ConflictSegment::Block(block) => {
                    let start = cursor;
                    cursor = cursor.saturating_add(editable_conflict_block_len(block));
                    ranges.push(start..cursor);
                }
            }
        }
        Self {
            ranges,
            text_len: cursor,
        }
    }

    pub(in crate::view) fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    pub(in crate::view) fn is_valid_for(
        &self,
        segments: &[ConflictSegment],
        output_text: &str,
    ) -> bool {
        self.text_len == output_text.len()
            && self.ranges.len() == conflict_count(segments)
            && self.ranges.iter().all(|range| {
                range.start <= range.end
                    && range.end <= output_text.len()
                    && output_text.is_char_boundary(range.start)
                    && output_text.is_char_boundary(range.end)
            })
            && self
                .ranges
                .windows(2)
                .all(|ranges| ranges[0].end <= ranges[1].start)
    }

    pub(in crate::view) fn block_slice<'a>(
        &self,
        segments: &[ConflictSegment],
        output_text: &'a str,
        block_index: usize,
    ) -> Option<&'a str> {
        self.is_valid_for(segments, output_text).then_some(())?;
        output_text.get(self.ranges.get(block_index)?.clone())
    }

    fn owner_for_edit_start(&self, start: usize) -> Option<usize> {
        self.ranges
            .iter()
            .position(|range| range.start <= start && start < range.end)
            .or_else(|| {
                self.ranges
                    .iter()
                    .position(|range| range.is_empty() && range.start == start)
            })
    }

    pub(in crate::view) fn apply_edit_delta(
        &mut self,
        old_range: Range<usize>,
        new_range: Range<usize>,
    ) -> bool {
        if old_range.start > old_range.end
            || old_range.end > self.text_len
            || new_range.start != old_range.start
            || new_range.start > new_range.end
        {
            return false;
        }

        let owner = self.owner_for_edit_start(old_range.start);
        let shift = new_range.len() as isize - old_range.len() as isize;
        let shift_offset = |offset: usize| {
            if shift >= 0 {
                offset.saturating_add(shift as usize)
            } else {
                offset.saturating_sub((-shift) as usize)
            }
        };

        for (block_index, range) in self.ranges.iter_mut().enumerate() {
            let previous = range.clone();
            if owner == Some(block_index) {
                let start = if previous.start < old_range.start {
                    previous.start
                } else {
                    old_range.start
                };
                let end = if previous.end > old_range.end {
                    shift_offset(previous.end)
                } else {
                    new_range.end
                };
                *range = start..end.max(start);
                continue;
            }

            if previous.end <= old_range.start {
                continue;
            }
            if previous.start >= old_range.end {
                *range = shift_offset(previous.start)..shift_offset(previous.end);
                continue;
            }

            let keeps_left = previous.start < old_range.start;
            let keeps_right = previous.end > old_range.end;
            *range = match (keeps_left, keeps_right) {
                (true, true) => previous.start..shift_offset(previous.end),
                (true, false) => previous.start..old_range.start,
                (false, true) => new_range.end..shift_offset(previous.end),
                // A later block deleted by an edit owned by an earlier block
                // collapses after the replacement, never into the owner's text.
                (false, false) => new_range.end..new_range.end,
            };
        }

        self.text_len = shift_offset(self.text_len);
        self.ranges
            .windows(2)
            .all(|ranges| ranges[0].end <= ranges[1].start)
            && self
                .ranges
                .iter()
                .all(|range| range.start <= range.end && range.end <= self.text_len)
    }

    pub(in crate::view) fn apply_edit_deltas(
        &mut self,
        deltas: impl IntoIterator<Item = (Range<usize>, Range<usize>)>,
    ) -> bool {
        for (old_range, new_range) in deltas {
            if !self.apply_edit_delta(old_range, new_range) {
                return false;
            }
        }
        true
    }
}

/// Derive per-region session resolution updates from the current resolved output.
///
/// This is used to persist manual resolver edits back into state without
/// requiring marker reparse in the reducer.
pub fn derive_region_resolution_updates_from_output(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
    block_map: &ResolvedOutputBlockMap,
    output_text: &str,
) -> Option<
    Vec<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionResolution,
    )>,
> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    if !block_map.is_valid_for(segments, output_text) {
        return None;
    }
    let mut updates = Vec::with_capacity(block_map.ranges().len());

    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let content = block_map.block_slice(segments, output_text, block_ix)?;
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);

        let resolution = if !block.resolved
            && (content.contains(UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER)
                || (!block.choice.is_empty() && content_matches_block_choice(block, content)))
        {
            R::Unresolved
        } else if content.is_empty() && block.choice.is_empty() {
            // Empty bytes alone cannot prove that an empty source was chosen.
            // Only the explicit block choice below carries that intent.
            R::ManualEdit(String::new())
        } else if let Some(choice) = choice_for_resolved_content(block, content) {
            resolution_for_choice(choice, block.base.is_some())
        } else {
            R::ManualEdit(content.to_string())
        };
        updates.push((region_ix, resolution));
        block_ix += 1;
    }

    Some(updates)
}

/// Derive per-region session resolution updates directly from marker segments.
///
/// Streamed resolved-output mode is read-only until explicit materialization,
/// so the block choice state is the source of truth and no full output string
/// needs to be assembled.
pub fn derive_region_resolution_updates_from_segments(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
) -> Vec<(
    usize,
    gitcomet_core::conflict_session::ConflictRegionResolution,
)> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let mut updates = Vec::with_capacity(conflict_count(segments));
    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);
        let resolution = if !block.resolved {
            R::Unresolved
        } else {
            resolution_for_choice(block.choice, block.base.is_some())
        };
        updates.push((region_ix, resolution));
        block_ix += 1;
    }
    updates
}

/// Result of applying state-layer region resolutions to UI marker segments.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionRegionApplyResult {
    /// Number of source regions visited/applied.
    pub applied_regions: usize,
    /// Mapping from visible block index -> source `ConflictSession` region index.
    pub block_region_indices: Vec<usize>,
}

/// Result of applying ConflictRegion choices to a plan projection while
/// retaining exact semantic block identities for every visible marker.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanSessionRegionApplyResult {
    pub block_region_indices: Vec<usize>,
    pub block_plan_indices: Vec<usize>,
}

/// Apply marker-backed region choices to a projection whose marker blocks are
/// identified by merge-plan block index.
///
/// A plan-only automatic delta can become unresolved after a source toggle.
/// Such a block has no ConflictRegion, so it is retained untouched and given a
/// synthetic, out-of-range grouping key while its plan identity remains exact.
pub fn apply_plan_session_region_resolutions_with_index_map(
    segments: &mut Vec<ConflictSegment>,
    session: &gitcomet_core::conflict_session::ConflictSession,
    projected_plan_blocks: &[usize],
) -> Option<PlanSessionRegionApplyResult> {
    if conflict_count(segments) != projected_plan_blocks.len() {
        return None;
    }

    let mut marker_index = 0usize;
    let mut block_region_indices = Vec::new();
    let mut block_plan_indices = Vec::new();
    let mut synced = Vec::with_capacity(segments.len());
    for segment in segments.drain(..) {
        match segment {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                let block_index = projected_plan_blocks[marker_index];
                marker_index += 1;
                // Only the plan applies kdiff3's per-row whitespace rule, so
                // carry its verdict onto the display block here rather than
                // re-deriving a weaker one from the block text.
                block.whitespace_only = session
                    .merge_plan
                    .as_ref()
                    .and_then(|plan| plan.blocks.get(block_index))
                    .is_some_and(|plan_block| plan_block.whitespace_conflict);
                let region_index = session
                    .region_plan_blocks
                    .iter()
                    .position(|candidate| *candidate == block_index);
                if let Some(region_index) = region_index
                    && let Some(region) = session.regions.get(region_index)
                    && let Some(materialized) =
                        apply_region_resolution_to_block(&mut block, &region.resolution)
                {
                    append_text_segment(&mut synced, materialized);
                    continue;
                }

                synced.push(ConflictSegment::Block(block));
                block_plan_indices.push(block_index);
                block_region_indices.push(region_index.unwrap_or_else(|| {
                    // Real regions occupy 0..len. Plan-only keys start at len
                    // and stay unique by semantic block index.
                    session.regions.len().saturating_add(block_index)
                }));
            }
        }
    }
    *segments = synced;
    Some(PlanSessionRegionApplyResult {
        block_region_indices,
        block_plan_indices,
    })
}

/// Build a default visible block -> region index mapping by position.
pub fn sequential_conflict_region_indices(segments: &[ConflictSegment]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut conflict_ix = 0usize;
    for seg in segments {
        if matches!(seg, ConflictSegment::Block(_)) {
            out.push(conflict_ix);
            conflict_ix += 1;
        }
    }
    out
}

fn apply_region_resolution_to_block(
    block: &mut ConflictBlock,
    resolution: &gitcomet_core::conflict_session::ConflictRegionResolution,
) -> Option<String> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    match resolution {
        R::Unresolved => {
            block.choice = ConflictChoice::empty();
            block.resolved = false;
            None
        }
        R::PickBase => {
            if block.base.is_some() {
                block.choice = ConflictChoice::Base;
                block.resolved = true;
            } else {
                block.choice = ConflictChoice::empty();
                block.resolved = false;
            }
            None
        }
        R::PickOurs => {
            block.choice = ConflictChoice::Ours;
            block.resolved = true;
            None
        }
        R::PickTheirs => {
            block.choice = ConflictChoice::Theirs;
            block.resolved = true;
            None
        }
        R::PickBoth => {
            block.choice = ConflictChoice::Both;
            block.resolved = true;
            None
        }
        R::Sources(selection) => {
            if let Some(choice) = choice_for_selection(selection, block.base.is_some()) {
                block.choice = choice;
                block.resolved = !choice.is_empty();
            } else {
                block.choice = ConflictChoice::empty();
                block.resolved = false;
            }
            None
        }
        R::ManualEdit(text) => {
            if let Some(choice) = choice_for_resolved_content(block, text) {
                block.choice = choice;
                block.resolved = true;
                return None;
            }
            Some(text.clone())
        }
        R::AutoResolved { content, .. } => {
            if let Some(choice) = choice_for_resolved_content(block, content) {
                block.choice = choice;
                block.resolved = true;
                return None;
            }
            Some(content.clone())
        }
    }
}

/// Apply ordered per-block resolutions to parsed UI marker segments.
///
/// This is used by save/export paths that derive resolutions from the current
/// resolved-output buffer and need to keep manual edits as plain text while
/// preserving untouched unresolved blocks.
pub(in crate::view) fn apply_ordered_region_resolutions(
    segments: &mut Vec<ConflictSegment>,
    resolutions: &[gitcomet_core::conflict_session::ConflictRegionResolution],
) -> usize {
    if segments.is_empty() || resolutions.is_empty() {
        return 0;
    }

    let mut applied = 0usize;
    let mut block_ix = 0usize;
    let mut synced: Vec<ConflictSegment> = Vec::with_capacity(segments.len());

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                if let Some(resolution) = resolutions.get(block_ix) {
                    if let Some(materialized_text) =
                        apply_region_resolution_to_block(&mut block, resolution)
                    {
                        append_text_segment(&mut synced, materialized_text);
                    } else {
                        synced.push(ConflictSegment::Block(block));
                    }
                    applied += 1;
                } else {
                    synced.push(ConflictSegment::Block(block));
                }
                block_ix += 1;
            }
        }
    }

    *segments = synced;
    applied
}

/// Apply state-layer region resolutions to parsed UI marker segments.
///
/// This allows resolver rebuilds to preserve choices tracked in
/// `RepoState.conflict_state.conflict_session`, and materializes manual/auto-resolved
/// non-side-pick text into plain `Text` segments when needed.
///
/// Returns how many conflict regions were applied.
#[cfg(test)]
pub fn apply_session_region_resolutions(
    segments: &mut Vec<ConflictSegment>,
    regions: &[gitcomet_core::conflict_session::ConflictRegion],
) -> usize {
    apply_session_region_resolutions_with_index_map(segments, regions).applied_regions
}

/// Like [`apply_session_region_resolutions`] but also returns a visible block
/// index map back to the original `ConflictSession` region indices.
pub fn apply_session_region_resolutions_with_index_map(
    segments: &mut Vec<ConflictSegment>,
    regions: &[gitcomet_core::conflict_session::ConflictRegion],
) -> SessionRegionApplyResult {
    if segments.is_empty() {
        return SessionRegionApplyResult::default();
    }
    if regions.is_empty() {
        return SessionRegionApplyResult {
            applied_regions: 0,
            block_region_indices: sequential_conflict_region_indices(segments),
        };
    }

    let mut applied = 0usize;
    let mut conflict_ix = 0usize;
    let mut block_region_indices = Vec::new();
    let mut synced: Vec<ConflictSegment> = Vec::with_capacity(segments.len());

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Text(text) => append_text_segment(&mut synced, text),
            ConflictSegment::Block(mut block) => {
                if let Some(region) = regions.get(conflict_ix) {
                    if let Some(materialized_text) =
                        apply_region_resolution_to_block(&mut block, &region.resolution)
                    {
                        append_text_segment(&mut synced, materialized_text);
                    } else {
                        synced.push(ConflictSegment::Block(block));
                        block_region_indices.push(conflict_ix);
                    }
                    applied += 1;
                } else {
                    synced.push(ConflictSegment::Block(block));
                    block_region_indices.push(conflict_ix);
                }
                conflict_ix += 1;
            }
        }
    }

    *segments = synced;
    SessionRegionApplyResult {
        applied_regions: applied,
        block_region_indices,
    }
}

pub(in crate::view) fn conflict_nav_region_aligned_ranges(
    session: &gitcomet_core::conflict_session::ConflictSession,
    fallback_ranges: &[Range<usize>],
) -> Vec<Option<Range<usize>>> {
    if let Some(plan) = session.merge_plan.as_ref() {
        return (0..session.regions.len())
            .map(|region_index| {
                let block_index = *session.region_plan_blocks.get(region_index)?;
                plan.blocks.get(block_index).map(|block| block.rows.clone())
            })
            .collect();
    }

    (0..session.regions.len())
        .map(|region_index| fallback_ranges.get(region_index).cloned())
        .collect()
}

pub(in crate::view) fn build_conflict_nav_targets(
    session: Option<&gitcomet_core::conflict_session::ConflictSession>,
    region_aligned_ranges: &[Option<Range<usize>>],
    display_region_indices: &[usize],
    display_aligned_ranges: &[Option<Range<usize>>],
    segments: &[ConflictSegment],
) -> Vec<ConflictNavTarget> {
    let display_resolved: Vec<bool> = segments
        .iter()
        .filter_map(|segment| match segment {
            ConflictSegment::Block(block) => Some(block.resolved),
            ConflictSegment::Text(_) => None,
        })
        .collect();
    let display_for_region = |region_index: usize| {
        display_region_indices
            .iter()
            .position(|candidate| *candidate == region_index)
    };

    if let Some(session) = session {
        if let Some(plan) = session.merge_plan.as_ref() {
            let mut targets = Vec::new();
            for (block_index, block) in plan.blocks.iter().enumerate() {
                if !block.is_delta && !block.original_conflict {
                    continue;
                }
                let region_index = session
                    .region_plan_blocks
                    .iter()
                    .position(|candidate| *candidate == block_index);
                // `display_conflict_index` addresses a *rendered marker block*
                // — it becomes `active_conflict`, which the rest of the UI uses
                // as `conflict_ix`. Only the region mapping can produce it: a
                // plan block's position among the plan's blocks is a different
                // index space, and blocks that render no marker (automatically
                // selected deltas) are absent from the displayed space entirely.
                let display_conflict_index = region_index.and_then(display_for_region);
                // Prefer the displayed block's own verdict where the block is
                // rendered, since an in-progress edit can resolve a marker
                // ahead of the plan; otherwise the plan block is the authority,
                // which is what makes a plan-only delta navigable.
                let unresolved = display_conflict_index
                    .and_then(|index| display_resolved.get(index))
                    .map_or_else(|| !block.is_resolved(), |resolved| !resolved);
                targets.push(ConflictNavTarget {
                    id: ConflictNavTargetId::PlanBlock(block.id),
                    order: targets.len(),
                    aligned_rows: Some(block.rows.clone()),
                    region_index,
                    display_conflict_index,
                    is_delta: block.is_delta,
                    original_conflict: block.original_conflict,
                    unresolved,
                });
            }
            return targets;
        }

        return session
            .regions
            .iter()
            .enumerate()
            .map(|(region_index, region)| {
                let display_conflict_index = display_for_region(region_index);
                let unresolved = display_conflict_index
                    .and_then(|index| display_resolved.get(index))
                    .map_or_else(|| !region.resolution.is_resolved(), |resolved| !resolved);
                ConflictNavTarget {
                    id: ConflictNavTargetId::Region(region_index),
                    order: region_index,
                    aligned_rows: region_aligned_ranges.get(region_index).cloned().flatten(),
                    region_index: Some(region_index),
                    display_conflict_index,
                    is_delta: true,
                    original_conflict: true,
                    unresolved,
                }
            })
            .collect();
    }

    display_resolved
        .into_iter()
        .enumerate()
        .map(|(display_conflict_index, resolved)| ConflictNavTarget {
            id: ConflictNavTargetId::DisplayBlock(display_conflict_index),
            order: display_conflict_index,
            aligned_rows: display_aligned_ranges
                .get(display_conflict_index)
                .cloned()
                .flatten(),
            region_index: display_region_indices.get(display_conflict_index).copied(),
            display_conflict_index: Some(display_conflict_index),
            is_delta: true,
            original_conflict: true,
            unresolved: !resolved,
        })
        .collect()
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

/// Compute effective conflict counters for resolver UI state.
///
/// Marker segments are authoritative for text-based conflict flows. For
/// non-marker strategies (binary side-pick / keep-delete / decision-only),
/// callers can pass state-layer session counters as a fallback.
pub fn effective_conflict_counts(
    segments: &[ConflictSegment],
    session_counts: Option<(usize, usize)>,
) -> (usize, usize) {
    let total = conflict_count(segments);
    if total > 0 {
        return (total, resolved_conflict_count(segments));
    }
    if let Some((session_total, session_resolved)) = session_counts {
        return (session_total, session_resolved.min(session_total));
    }
    (0, 0)
}

/// Return conflict indices for currently unresolved blocks in queue order.
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
pub fn auto_resolve_segments(segments: &mut [ConflictSegment]) -> usize {
    auto_resolve_segments_with_options(segments, false)
}

/// Like [`auto_resolve_segments`] but with an optional whitespace-normalization toggle.
///
/// Segment-based autosolve is now test-only: the live on-open path uses the
/// session-based `apply_autosolve_to_session` in gitcomet-state, and the
/// manual re-trigger button was removed.
#[cfg(test)]
pub fn auto_resolve_segments_with_options(
    segments: &mut [ConflictSegment],
    whitespace_normalize: bool,
) -> usize {
    use gitcomet_core::conflict_session::{AutosolvePickSide, safe_auto_resolve_pick};

    let mut count = 0;
    for seg in segments.iter_mut() {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        if block.resolved {
            continue;
        }

        let Some((_, pick)) = safe_auto_resolve_pick(
            block.base.as_deref(),
            &block.ours,
            &block.theirs,
            whitespace_normalize,
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

/// Apply Pass 3 regex-assisted auto-resolve rules (opt-in) to unresolved blocks.
///
/// This mode uses regex normalization rules from core and only performs
/// side-picks (`Ours` / `Theirs`), never synthetic text rewrites.
#[cfg(test)]
pub fn auto_resolve_segments_regex(
    segments: &mut [ConflictSegment],
    options: &gitcomet_core::conflict_session::RegexAutosolveOptions,
) -> usize {
    use gitcomet_core::conflict_session::{AutosolvePickSide, regex_assisted_auto_resolve_pick};

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
#[cfg(test)]
pub fn auto_resolve_segments_history(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_history_with_region_indices(segments, options, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_history`] but keeps block->region mappings in sync.
#[cfg(test)]
pub fn auto_resolve_segments_history_with_region_indices(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
    block_region_indices: &mut Vec<usize>,
) -> usize {
    use gitcomet_core::conflict_session::history_merge_region;

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut new_block_region_indices = Vec::with_capacity(block_region_indices.len());
    let mut block_ix = 0usize;
    let mut count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(block) => {
                let region_ix = block_region_indices
                    .get(block_ix)
                    .copied()
                    .unwrap_or(block_ix);
                block_ix += 1;
                if !block.resolved
                    && let Some(merged) = history_merge_region(
                        block.base.as_deref(),
                        &block.ours,
                        &block.theirs,
                        options,
                    )
                {
                    // Merge adjacent Text segments for cleanliness.
                    if let Some(ConflictSegment::Text(prev)) = new_segments.last_mut() {
                        prev.push_str(&merged);
                    } else {
                        new_segments.push(ConflictSegment::Text(merged.into()));
                    }
                    count += 1;
                    continue;
                }
                new_segments.push(ConflictSegment::Block(block));
                new_block_region_indices.push(region_ix);
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    *block_region_indices = new_block_region_indices;
    count
}

/// Apply Pass 2 (heuristic subchunk splitting) to unresolved conflict blocks.
///
/// For each unresolved block that has a base, attempts to split it into
/// line-level subchunks via 3-way diff/merge. Non-conflicting subchunks
/// become `Text` segments; remaining conflicts become smaller `Block` segments.
///
/// Returns the number of original blocks that were split.
#[cfg(test)]
pub fn auto_resolve_segments_pass2(segments: &mut Vec<ConflictSegment>) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_pass2_with_region_indices(segments, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_pass2`] but keeps block->region mappings in sync.
#[cfg(test)]
pub fn auto_resolve_segments_pass2_with_region_indices(
    segments: &mut Vec<ConflictSegment>,
    block_region_indices: &mut Vec<usize>,
) -> usize {
    use gitcomet_core::conflict_session::{Subchunk, split_conflict_into_subchunks};

    let mut new_segments = Vec::with_capacity(segments.len());
    let mut new_block_region_indices = Vec::with_capacity(block_region_indices.len());
    let mut block_ix = 0usize;
    let mut split_count = 0;

    for seg in segments.drain(..) {
        match seg {
            ConflictSegment::Block(block) => {
                let region_ix = block_region_indices
                    .get(block_ix)
                    .copied()
                    .unwrap_or(block_ix);
                block_ix += 1;
                if !block.resolved
                    && let Some(base) = block.base.as_deref()
                    && let Some(subchunks) =
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
                                    new_segments.push(ConflictSegment::Text(text.into()));
                                }
                            }
                            Subchunk::Conflict { base, ours, theirs } => {
                                new_segments.push(ConflictSegment::Block(ConflictBlock {
                                    base: Some(base.into()),
                                    ours: ours.into(),
                                    theirs: theirs.into(),
                                    choice: ConflictChoice::empty(),
                                    resolved: false,
                                    // Every row of a whitespace-only block is
                                    // whitespace-only, so each subchunk of it
                                    // is too. kdiff3 clears the flag the same
                                    // way when a split lands a real change in
                                    // a block (MergeEditLine.h join/append).
                                    whitespace_only: block.whitespace_only,
                                }));
                                new_block_region_indices.push(region_ix);
                            }
                        }
                    }
                    // If all subchunks resolved, no Block segments remain
                    // from this split (all became Text above).
                    continue;
                }
                new_segments.push(ConflictSegment::Block(block));
                new_block_region_indices.push(region_ix);
            }
            other => new_segments.push(other),
        }
    }

    *segments = new_segments;
    *block_region_indices = new_block_region_indices;
    split_count
}

/// KDiff3-compatible text shown for a merge block with no selected sources.
pub(in crate::view) const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER: &str = "<Merge Conflict>";

/// Same, for a block whose sides differ only in whitespace.
pub(in crate::view) const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER: &str =
    "<Merge Conflict (Whitespace only)>";

const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_LF: &str = "<Merge Conflict>\n";
const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CRLF: &str = "<Merge Conflict>\r\n";
const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CR: &str = "<Merge Conflict>\r";

const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_LF: &str = "<Merge Conflict (Whitespace only)>\n";
const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CRLF: &str =
    "<Merge Conflict (Whitespace only)>\r\n";
const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CR: &str = "<Merge Conflict (Whitespace only)>\r";

/// Whether one output line is an unresolved-conflict placeholder row.
///
/// The resolved output is a text document, so a placeholder row is identified
/// by its own content — the same fact the reader sees. This keeps the gutter
/// marker in step with the text however the marker array was built, mirroring
/// how kdiff3 derives both from a single `srcSelect`.
pub(in crate::view) fn line_is_unresolved_conflict_placeholder(line: &str) -> bool {
    let line = line.trim_end_matches(['\r', '\n']);
    line == UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER
        || line == UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER
}

fn uses_unresolved_merge_conflict_placeholder(block: &ConflictBlock) -> bool {
    !block.resolved && block.choice.is_empty()
}

fn unresolved_merge_conflict_placeholder_text(block: &ConflictBlock) -> &'static str {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, detect_conflict_block_line_ending,
    };

    let line_ending = detect_conflict_block_line_ending(ConflictOutputBlockRef {
        base: block.base.as_deref(),
        ours: &block.ours,
        theirs: &block.theirs,
        choice: block.choice,
        resolved: block.resolved,
    });
    // kdiff3 mergeresultwindow.cpp: a block whose sides differ only in
    // whitespace names itself, so the trivial ones can be told apart from real
    // clashes without opening them.
    if block.whitespace_only {
        return match line_ending {
            "\r\n" => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CRLF,
            "\r" => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CR,
            _ => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_LF,
        };
    }
    match line_ending {
        "\r\n" => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CRLF,
        "\r" => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CR,
        _ => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_LF,
    }
}

fn editable_conflict_block_len(block: &ConflictBlock) -> usize {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    if uses_unresolved_merge_conflict_placeholder(block) {
        return unresolved_merge_conflict_placeholder_text(block).len();
    }
    block.choice.iter().fold(0usize, |len, source| {
        len.saturating_add(match source {
            ConflictOutputSource::Base => block.base.as_ref().map_or(0, ConflictText::len),
            ConflictOutputSource::Ours => block.ours.len(),
            ConflictOutputSource::Theirs => block.theirs.len(),
        })
    })
}

/// Generate the editable merge-output projection.
///
/// A truly unresolved block has no selected sources and occupies one explicit
/// KDiff3-style placeholder row. Resolved blocks retain their ordered source
/// selection, while marker-preserving save/export paths use
/// [`generate_resolved_text_with_options`] directly.
pub fn generate_resolved_text(segments: &[ConflictSegment]) -> String {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut output = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => output.push_str(text),
            ConflictSegment::Block(block) if uses_unresolved_merge_conflict_placeholder(block) => {
                output.push_str(unresolved_merge_conflict_placeholder_text(block));
            }
            ConflictSegment::Block(block) => {
                for source in block.choice.iter() {
                    match source {
                        ConflictOutputSource::Base => {
                            if let Some(base) = block.base.as_deref() {
                                output.push_str(base);
                            }
                        }
                        ConflictOutputSource::Ours => output.push_str(&block.ours),
                        ConflictOutputSource::Theirs => output.push_str(&block.theirs),
                    }
                }
            }
        }
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedOutputText {
    Shared(Arc<str>),
    Owned(String),
}

impl ResolvedOutputText {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shared(text) => text.as_ref(),
            Self::Owned(text) => text.as_str(),
        }
    }

    pub fn line_count(&self) -> usize {
        text_line_count_usize(self.as_str())
    }

    pub fn into_shared_string(self) -> gpui::SharedString {
        match self {
            Self::Shared(text) => text.into(),
            Self::Owned(text) => text.into(),
        }
    }
}

pub fn bootstrap_resolved_output_text(
    segments: &[ConflictSegment],
    current_text: Option<&Arc<str>>,
    ours_text: Option<&Arc<str>>,
    theirs_text: Option<&Arc<str>>,
) -> ResolvedOutputText {
    if segments.is_empty() {
        return current_text
            .or(ours_text)
            .or(theirs_text)
            .cloned()
            .map(ResolvedOutputText::Shared)
            .unwrap_or_else(|| ResolvedOutputText::Owned(String::new()));
    }

    ResolvedOutputText::Owned(generate_resolved_text(segments))
}

pub fn generate_resolved_text_with_options(
    segments: &[ConflictSegment],
    options: gitcomet_core::conflict_output::GenerateResolvedTextOptions<'_>,
) -> String {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, ConflictOutputSegmentRef,
        generate_resolved_text as generate_core_resolved_text,
    };

    let core_segments: Vec<ConflictOutputSegmentRef<'_>> = segments
        .iter()
        .map(|segment| match segment {
            ConflictSegment::Text(text) => ConflictOutputSegmentRef::Text(text),
            ConflictSegment::Block(block) => {
                ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                    base: block.base.as_deref(),
                    ours: &block.ours,
                    theirs: &block.theirs,
                    choice: block.choice,
                    resolved: block.resolved,
                })
            }
        })
        .collect();

    generate_core_resolved_text(&core_segments, options)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ResolvedOutputFragmentSource {
    TextSegment { segment_ix: usize },
    BlockBase { segment_ix: usize },
    BlockOurs { segment_ix: usize },
    BlockTheirs { segment_ix: usize },
    UnresolvedPlaceholder { text: &'static str },
}

fn resolved_output_block_source_fragment(
    segment_ix: usize,
    block: &ConflictBlock,
    source: gitcomet_core::conflict_output::ConflictOutputSource,
) -> Option<(ResolvedOutputFragmentSource, &str)> {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    match source {
        ConflictOutputSource::Base => block
            .base
            .as_deref()
            .map(|base| (ResolvedOutputFragmentSource::BlockBase { segment_ix }, base)),
        ConflictOutputSource::Ours => Some((
            ResolvedOutputFragmentSource::BlockOurs { segment_ix },
            block.ours.as_str(),
        )),
        ConflictOutputSource::Theirs => Some((
            ResolvedOutputFragmentSource::BlockTheirs { segment_ix },
            block.theirs.as_str(),
        )),
    }
}

const RESOLVED_OUTPUT_SPARSE_LINE_INDEX_MIN_LINES: usize = LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES;
const RESOLVED_OUTPUT_SPAN_CHECKPOINT_STRIDE: usize = 256;

#[derive(Clone, Debug)]
enum ResolvedOutputFragmentLineIndex {
    SingleLine,
    Dense(Arc<[usize]>),
    Sparse(SparseLineIndex),
}

impl ResolvedOutputFragmentLineIndex {
    fn line_text<'a>(&self, text: &'a str, line_ix: usize) -> Option<&'a str> {
        match self {
            Self::SingleLine => (line_ix == 0).then_some(text.strip_suffix('\n').unwrap_or(text)),
            Self::Dense(line_starts) => {
                Some(line_text_from_starts(text, line_starts.as_ref(), line_ix))
            }
            Self::Sparse(line_index) => line_index.line_text(text, line_ix),
        }
    }

    fn for_each_line_text<'a>(
        &self,
        text: &'a str,
        range: Range<usize>,
        mut visit: impl FnMut(usize, &'a str),
    ) {
        if range.start >= range.end {
            return;
        }

        match self {
            Self::SingleLine => {
                if range.start == 0 {
                    visit(0, text.strip_suffix('\n').unwrap_or(text));
                }
            }
            Self::Dense(line_starts) => {
                let line_starts = line_starts.as_ref();
                for line_ix in range {
                    visit(line_ix, line_text_from_starts(text, line_starts, line_ix));
                }
            }
            Self::Sparse(line_index) => {
                for line_ix in range {
                    if let Some(line) = line_index.line_text(text, line_ix) {
                        visit(line_ix, line);
                    }
                }
            }
        }
    }

    #[cfg(all(test, feature = "benchmarks"))]
    fn metadata_byte_size(&self) -> usize {
        match self {
            Self::SingleLine => 0,
            Self::Dense(line_starts) => line_starts.len() * std::mem::size_of::<usize>(),
            Self::Sparse(line_index) => line_index.metadata_byte_size(),
        }
    }
}

#[derive(Clone, Debug)]
struct ResolvedOutputFragment {
    source: ResolvedOutputFragmentSource,
    line_index: ResolvedOutputFragmentLineIndex,
    newline_count: usize,
    ends_with_newline: bool,
    line_count: usize,
    widest_line_ix: usize,
    widest_line_len: usize,
}

impl ResolvedOutputFragment {
    fn source_text<'a>(&self, segments: &'a [ConflictSegment]) -> Option<&'a str> {
        match self.source {
            ResolvedOutputFragmentSource::TextSegment { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Text(text)) => Some(text.as_str()),
                    _ => None,
                }
            }
            ResolvedOutputFragmentSource::BlockBase { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => {
                        Some(block.base.as_deref().unwrap_or(""))
                    }
                    _ => None,
                }
            }
            ResolvedOutputFragmentSource::BlockOurs { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => Some(block.ours.as_str()),
                    _ => None,
                }
            }
            ResolvedOutputFragmentSource::BlockTheirs { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => Some(block.theirs.as_str()),
                    _ => None,
                }
            }
            ResolvedOutputFragmentSource::UnresolvedPlaceholder { text } => Some(text),
        }
    }

    fn line_text<'a>(&self, segments: &'a [ConflictSegment], line_ix: usize) -> Option<&'a str> {
        let text = self.source_text(segments)?;
        if line_ix < self.line_count {
            self.line_index.line_text(text, line_ix)
        } else {
            None
        }
    }

    fn for_each_line_text<'a>(
        &self,
        segments: &'a [ConflictSegment],
        range: Range<usize>,
        visit: impl FnMut(usize, &'a str),
    ) {
        let Some(text) = self.source_text(segments) else {
            return;
        };
        let start = range.start.min(self.line_count);
        let end = range.end.min(self.line_count);
        if start >= end {
            return;
        }
        self.line_index.for_each_line_text(text, start..end, visit);
    }

    fn widest_line(&self) -> Option<(usize, usize)> {
        (self.line_count > 0).then_some((self.widest_line_ix, self.widest_line_len))
    }

    #[cfg(all(test, feature = "benchmarks"))]
    fn metadata_byte_size(&self) -> usize {
        self.line_index.metadata_byte_size()
    }
}

#[derive(Clone, Debug)]
enum ResolvedOutputSpan {
    SourceLines {
        visible_start: usize,
        len: usize,
        fragment_ix: usize,
        fragment_line_start: usize,
    },
    MergedLine {
        visible_index: usize,
        text: String,
    },
}

impl ResolvedOutputSpan {
    fn visible_start(&self) -> usize {
        match self {
            Self::SourceLines { visible_start, .. } => *visible_start,
            Self::MergedLine { visible_index, .. } => *visible_index,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::SourceLines { len, .. } => *len,
            Self::MergedLine { .. } => 1,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedOutputProjection {
    fragments: Vec<ResolvedOutputFragment>,
    spans: Vec<ResolvedOutputSpan>,
    span_checkpoints: Vec<usize>,
    conflict_line_ranges: Vec<std::ops::Range<usize>>,
    line_count: usize,
    widest_line_ix: usize,
}

impl ResolvedOutputProjection {
    pub fn from_segments(segments: &[ConflictSegment]) -> Self {
        #[derive(Clone, Debug)]
        enum PendingLine {
            Empty,
            Source {
                fragment_ix: usize,
                line_ix: usize,
                conflict_ix: Option<usize>,
            },
            Composed {
                text: String,
                conflict_ix: Option<usize>,
            },
        }

        impl PendingLine {
            fn conflict_ix(&self) -> Option<usize> {
                match self {
                    Self::Empty => None,
                    Self::Source { conflict_ix, .. } | Self::Composed { conflict_ix, .. } => {
                        *conflict_ix
                    }
                }
            }
        }

        fn dense_line_starts_and_widest_line(text: &str) -> (Arc<[usize]>, usize, usize, usize) {
            let bytes = text.as_bytes();
            let mut starts = Vec::with_capacity(bytes.len().saturating_div(64).saturating_add(1));
            starts.push(0usize);
            let mut line_count = 0usize;
            let mut line_start = 0usize;
            let mut widest_line_ix = 0usize;
            let mut widest_line_len = 0usize;

            for pos in memchr::memchr_iter(b'\n', bytes) {
                let line_len = pos.saturating_sub(line_start);
                if line_len > widest_line_len {
                    widest_line_len = line_len;
                    widest_line_ix = line_count;
                }
                line_count = line_count.saturating_add(1);
                line_start = pos.saturating_add(1);
                starts.push(line_start);
            }

            if line_start < bytes.len() {
                let line_len = bytes.len().saturating_sub(line_start);
                if line_len > widest_line_len {
                    widest_line_len = line_len;
                    widest_line_ix = line_count;
                }
                line_count = line_count.saturating_add(1);
            }

            (starts.into(), line_count, widest_line_ix, widest_line_len)
        }

        fn fragment_line_stats(
            text: &str,
        ) -> (
            ResolvedOutputFragmentLineIndex,
            usize,
            bool,
            usize,
            usize,
            usize,
        ) {
            let bytes = text.as_bytes();
            let ends_with_newline = bytes.last().copied() == Some(b'\n');
            let (line_index, line_count, widest_line_ix, widest_line_len) = if ends_with_newline
                && bytes
                    .iter()
                    .take(bytes.len().saturating_sub(1))
                    .all(|&b| b != b'\n')
            {
                (
                    ResolvedOutputFragmentLineIndex::SingleLine,
                    1,
                    0,
                    bytes.len() - 1,
                )
            } else if !ends_with_newline && bytes.iter().all(|&b| b != b'\n') {
                (
                    ResolvedOutputFragmentLineIndex::SingleLine,
                    1,
                    0,
                    bytes.len(),
                )
            } else {
                let (dense_line_starts, line_count, widest_line_ix, widest_line_len) =
                    dense_line_starts_and_widest_line(text);
                if line_count >= RESOLVED_OUTPUT_SPARSE_LINE_INDEX_MIN_LINES {
                    let line_index = SparseLineIndex::for_text(text);
                    let (widest_line_ix, widest_line_len) =
                        line_index.widest_line().unwrap_or((0, 0));
                    (
                        ResolvedOutputFragmentLineIndex::Sparse(line_index),
                        line_count,
                        widest_line_ix,
                        widest_line_len,
                    )
                } else {
                    (
                        ResolvedOutputFragmentLineIndex::Dense(dense_line_starts),
                        line_count,
                        widest_line_ix,
                        widest_line_len,
                    )
                }
            };
            let newline_count = if ends_with_newline {
                line_count
            } else {
                line_count.saturating_sub(1)
            };
            (
                line_index,
                newline_count,
                ends_with_newline,
                line_count,
                widest_line_ix,
                widest_line_len,
            )
        }

        fn push_source_span(
            spans: &mut Vec<ResolvedOutputSpan>,
            visible_start: usize,
            fragment_ix: usize,
            fragment_line_start: usize,
            len: usize,
        ) {
            if len == 0 {
                return;
            }
            if let Some(ResolvedOutputSpan::SourceLines {
                visible_start: prev_visible_start,
                len: prev_len,
                fragment_ix: prev_fragment_ix,
                fragment_line_start: prev_fragment_line_start,
            }) = spans.last_mut()
                && *prev_fragment_ix == fragment_ix
                && prev_visible_start.saturating_add(*prev_len) == visible_start
                && prev_fragment_line_start.saturating_add(*prev_len) == fragment_line_start
            {
                *prev_len = prev_len.saturating_add(len);
                return;
            }
            spans.push(ResolvedOutputSpan::SourceLines {
                visible_start,
                len,
                fragment_ix,
                fragment_line_start,
            });
        }

        fn push_merged_line(
            spans: &mut Vec<ResolvedOutputSpan>,
            visible_index: usize,
            text: String,
        ) {
            spans.push(ResolvedOutputSpan::MergedLine {
                visible_index,
                text,
            });
        }

        fn merge_conflict_ix(current: Option<usize>, next: Option<usize>) -> Option<usize> {
            match (current, next) {
                (None, other) | (other, None) => other,
                (Some(left), Some(right)) => {
                    debug_assert_eq!(
                        left, right,
                        "resolved output line should not span multiple conflict blocks"
                    );
                    Some(left)
                }
            }
        }

        fn build_span_checkpoints(spans: &[ResolvedOutputSpan], line_count: usize) -> Vec<usize> {
            if spans.is_empty() || line_count == 0 {
                return Vec::new();
            }

            let checkpoint_count = line_count
                .saturating_add(RESOLVED_OUTPUT_SPAN_CHECKPOINT_STRIDE - 1)
                / RESOLVED_OUTPUT_SPAN_CHECKPOINT_STRIDE;
            let mut checkpoints = Vec::with_capacity(checkpoint_count);
            let mut span_ix = 0usize;

            for checkpoint_ix in 0..checkpoint_count {
                let visible_line = checkpoint_ix * RESOLVED_OUTPUT_SPAN_CHECKPOINT_STRIDE;
                while span_ix + 1 < spans.len()
                    && spans[span_ix]
                        .visible_start()
                        .saturating_add(spans[span_ix].len())
                        <= visible_line
                {
                    span_ix = span_ix.saturating_add(1);
                }
                checkpoints.push(span_ix);
            }

            checkpoints
        }

        fn extend_conflict_line_range(
            ranges: &mut [Option<std::ops::Range<usize>>],
            conflict_ix: Option<usize>,
            line_ix: usize,
        ) {
            let Some(conflict_ix) = conflict_ix else {
                return;
            };
            let Some(slot) = ranges.get_mut(conflict_ix) else {
                return;
            };
            match slot {
                Some(range) => {
                    range.start = range.start.min(line_ix);
                    range.end = range.end.max(line_ix.saturating_add(1));
                }
                None => {
                    *slot = Some(line_ix..line_ix.saturating_add(1));
                }
            }
        }

        fn finalize_pending_line(
            pending: &mut PendingLine,
            fragments: &[ResolvedOutputFragment],
            segments: &[ConflictSegment],
            spans: &mut Vec<ResolvedOutputSpan>,
            visible_line: &mut usize,
            conflict_ranges: &mut [Option<std::ops::Range<usize>>],
            widest_visible_line: &mut (usize, usize),
        ) {
            let line_conflict = pending.conflict_ix();
            let line_len = match pending {
                PendingLine::Empty => 0,
                PendingLine::Source {
                    fragment_ix,
                    line_ix,
                    ..
                } => fragments
                    .get(*fragment_ix)
                    .and_then(|fragment| fragment.line_text(segments, *line_ix))
                    .map_or(0, str::len),
                PendingLine::Composed { text, .. } => text.len(),
            };
            if line_len > widest_visible_line.1 {
                *widest_visible_line = (*visible_line, line_len);
            }
            match pending {
                PendingLine::Empty => {
                    push_merged_line(spans, *visible_line, String::new());
                }
                PendingLine::Source {
                    fragment_ix,
                    line_ix,
                    ..
                } => {
                    push_source_span(spans, *visible_line, *fragment_ix, *line_ix, 1);
                }
                PendingLine::Composed { text, .. } => {
                    push_merged_line(spans, *visible_line, std::mem::take(text));
                }
            }
            extend_conflict_line_range(conflict_ranges, line_conflict, *visible_line);
            *visible_line = visible_line.saturating_add(1);
            *pending = PendingLine::Empty;
        }

        fn update_widest_from_source_span(
            widest_visible_line: &mut (usize, usize),
            fragments: &[ResolvedOutputFragment],
            visible_start: usize,
            fragment_ix: usize,
            fragment_line_start: usize,
            len: usize,
        ) {
            let Some(fragment) = fragments.get(fragment_ix) else {
                return;
            };
            let Some((widest_line_ix, widest_line_len)) = fragment.widest_line() else {
                return;
            };
            let fragment_line_end = fragment_line_start.saturating_add(len);
            if widest_line_ix < fragment_line_start || widest_line_ix >= fragment_line_end {
                return;
            }

            let visible_ix =
                visible_start.saturating_add(widest_line_ix.saturating_sub(fragment_line_start));
            if widest_line_len > widest_visible_line.1 {
                *widest_visible_line = (visible_ix, widest_line_len);
            }
        }

        fn append_source_piece_to_pending(
            pending: &mut PendingLine,
            fragments: &[ResolvedOutputFragment],
            segments: &[ConflictSegment],
            fragment_ix: usize,
            line_ix: usize,
            conflict_ix: Option<usize>,
        ) {
            let piece_text = fragments
                .get(fragment_ix)
                .and_then(|fragment| fragment.line_text(segments, line_ix))
                .unwrap_or("");
            match pending {
                PendingLine::Empty => {
                    if piece_text.is_empty() {
                        return;
                    }
                    *pending = PendingLine::Source {
                        fragment_ix,
                        line_ix,
                        conflict_ix,
                    };
                }
                PendingLine::Source {
                    fragment_ix: existing_fragment_ix,
                    line_ix: existing_line_ix,
                    conflict_ix: existing_conflict_ix,
                } => {
                    let existing_text = fragments
                        .get(*existing_fragment_ix)
                        .and_then(|fragment| fragment.line_text(segments, *existing_line_ix))
                        .unwrap_or("");
                    let mut composed =
                        String::with_capacity(existing_text.len().saturating_add(piece_text.len()));
                    composed.push_str(existing_text);
                    composed.push_str(piece_text);
                    *pending = PendingLine::Composed {
                        text: composed,
                        conflict_ix: merge_conflict_ix(*existing_conflict_ix, conflict_ix),
                    };
                }
                PendingLine::Composed {
                    text,
                    conflict_ix: existing_conflict_ix,
                } => {
                    text.push_str(piece_text);
                    *existing_conflict_ix = merge_conflict_ix(*existing_conflict_ix, conflict_ix);
                }
            }
        }

        let conflict_total = conflict_count(segments);
        let projected_fragment_count = segments
            .iter()
            .map(|segment| match segment {
                ConflictSegment::Text(text) => usize::from(!text.is_empty()),
                ConflictSegment::Block(block)
                    if uses_unresolved_merge_conflict_placeholder(block) =>
                {
                    1
                }
                ConflictSegment::Block(block) => block
                    .choice
                    .iter()
                    .filter(|source| match source {
                        gitcomet_core::conflict_output::ConflictOutputSource::Base => {
                            block.base.as_ref().is_some_and(|base| !base.is_empty())
                        }
                        gitcomet_core::conflict_output::ConflictOutputSource::Ours => {
                            !block.ours.is_empty()
                        }
                        gitcomet_core::conflict_output::ConflictOutputSource::Theirs => {
                            !block.theirs.is_empty()
                        }
                    })
                    .count(),
            })
            .sum();
        let mut conflict_ranges: Vec<Option<std::ops::Range<usize>>> = vec![None; conflict_total];
        let mut conflict_line_anchors = vec![0usize; conflict_total];
        let mut fragments = Vec::with_capacity(projected_fragment_count);
        let mut spans = Vec::with_capacity(projected_fragment_count.saturating_add(conflict_total));
        let mut pending = PendingLine::Empty;
        let mut visible_line = 0usize;
        let mut block_ix = 0usize;
        let mut widest_visible_line = (0usize, 0usize);

        fn push_fragment(
            fragments: &mut Vec<ResolvedOutputFragment>,
            source: ResolvedOutputFragmentSource,
            text: &str,
        ) -> Option<usize> {
            if text.is_empty() {
                return None;
            }
            let (
                line_index,
                newline_count,
                ends_with_newline,
                line_count,
                widest_line_ix,
                widest_line_len,
            ) = fragment_line_stats(text);
            let fragment_ix = fragments.len();
            fragments.push(ResolvedOutputFragment {
                source,
                line_index,
                newline_count,
                ends_with_newline,
                line_count,
                widest_line_ix,
                widest_line_len,
            });
            Some(fragment_ix)
        }

        for (segment_ix, segment) in segments.iter().enumerate() {
            match segment {
                ConflictSegment::Text(text) => {
                    let Some(fragment_ix) = push_fragment(
                        &mut fragments,
                        ResolvedOutputFragmentSource::TextSegment { segment_ix },
                        text.as_str(),
                    ) else {
                        continue;
                    };
                    let fragment = &fragments[fragment_ix];
                    if fragment.newline_count == 0 {
                        append_source_piece_to_pending(
                            &mut pending,
                            &fragments,
                            segments,
                            fragment_ix,
                            0,
                            None,
                        );
                        continue;
                    }

                    if !matches!(pending, PendingLine::Empty) {
                        append_source_piece_to_pending(
                            &mut pending,
                            &fragments,
                            segments,
                            fragment_ix,
                            0,
                            None,
                        );
                        finalize_pending_line(
                            &mut pending,
                            &fragments,
                            segments,
                            &mut spans,
                            &mut visible_line,
                            &mut conflict_ranges,
                            &mut widest_visible_line,
                        );
                        if fragment.newline_count > 1 {
                            push_source_span(
                                &mut spans,
                                visible_line,
                                fragment_ix,
                                1,
                                fragment.newline_count - 1,
                            );
                            update_widest_from_source_span(
                                &mut widest_visible_line,
                                &fragments,
                                visible_line,
                                fragment_ix,
                                1,
                                fragment.newline_count - 1,
                            );
                            visible_line = visible_line.saturating_add(fragment.newline_count - 1);
                        }
                    } else {
                        push_source_span(
                            &mut spans,
                            visible_line,
                            fragment_ix,
                            0,
                            fragment.newline_count,
                        );
                        update_widest_from_source_span(
                            &mut widest_visible_line,
                            &fragments,
                            visible_line,
                            fragment_ix,
                            0,
                            fragment.newline_count,
                        );
                        visible_line = visible_line.saturating_add(fragment.newline_count);
                    }

                    if !fragment.ends_with_newline {
                        pending = PendingLine::Source {
                            fragment_ix,
                            line_ix: fragment.newline_count,
                            conflict_ix: None,
                        };
                    }
                }
                ConflictSegment::Block(block) => {
                    let conflict_ix = block_ix;
                    block_ix = block_ix.saturating_add(1);
                    if let Some(anchor) = conflict_line_anchors.get_mut(conflict_ix) {
                        *anchor = visible_line;
                    }

                    let fragment_sources: Vec<_> =
                        if uses_unresolved_merge_conflict_placeholder(block) {
                            let text = unresolved_merge_conflict_placeholder_text(block);
                            vec![(
                                ResolvedOutputFragmentSource::UnresolvedPlaceholder { text },
                                text,
                            )]
                        } else {
                            block
                                .choice
                                .iter()
                                .filter_map(|source| {
                                    resolved_output_block_source_fragment(segment_ix, block, source)
                                })
                                .collect()
                        };

                    for (source, text) in fragment_sources {
                        let Some(fragment_ix) = push_fragment(&mut fragments, source, text) else {
                            continue;
                        };
                        let fragment = &fragments[fragment_ix];
                        if fragment.newline_count == 0 {
                            append_source_piece_to_pending(
                                &mut pending,
                                &fragments,
                                segments,
                                fragment_ix,
                                0,
                                Some(conflict_ix),
                            );
                            continue;
                        }

                        if !matches!(pending, PendingLine::Empty) {
                            append_source_piece_to_pending(
                                &mut pending,
                                &fragments,
                                segments,
                                fragment_ix,
                                0,
                                Some(conflict_ix),
                            );
                            finalize_pending_line(
                                &mut pending,
                                &fragments,
                                segments,
                                &mut spans,
                                &mut visible_line,
                                &mut conflict_ranges,
                                &mut widest_visible_line,
                            );
                            if fragment.newline_count > 1 {
                                let middle_len = fragment.newline_count - 1;
                                push_source_span(
                                    &mut spans,
                                    visible_line,
                                    fragment_ix,
                                    1,
                                    middle_len,
                                );
                                update_widest_from_source_span(
                                    &mut widest_visible_line,
                                    &fragments,
                                    visible_line,
                                    fragment_ix,
                                    1,
                                    middle_len,
                                );
                                for offset in 0..middle_len {
                                    extend_conflict_line_range(
                                        &mut conflict_ranges,
                                        Some(conflict_ix),
                                        visible_line.saturating_add(offset),
                                    );
                                }
                                visible_line = visible_line.saturating_add(middle_len);
                            }
                        } else {
                            push_source_span(
                                &mut spans,
                                visible_line,
                                fragment_ix,
                                0,
                                fragment.newline_count,
                            );
                            update_widest_from_source_span(
                                &mut widest_visible_line,
                                &fragments,
                                visible_line,
                                fragment_ix,
                                0,
                                fragment.newline_count,
                            );
                            for offset in 0..fragment.newline_count {
                                extend_conflict_line_range(
                                    &mut conflict_ranges,
                                    Some(conflict_ix),
                                    visible_line.saturating_add(offset),
                                );
                            }
                            visible_line = visible_line.saturating_add(fragment.newline_count);
                        }

                        if !fragment.ends_with_newline {
                            pending = PendingLine::Source {
                                fragment_ix,
                                line_ix: fragment.newline_count,
                                conflict_ix: Some(conflict_ix),
                            };
                        }
                    }
                }
            }
        }

        finalize_pending_line(
            &mut pending,
            &fragments,
            segments,
            &mut spans,
            &mut visible_line,
            &mut conflict_ranges,
            &mut widest_visible_line,
        );

        let conflict_line_ranges: Vec<std::ops::Range<usize>> = conflict_ranges
            .into_iter()
            .enumerate()
            .map(|(conflict_ix, range)| {
                range.unwrap_or_else(|| {
                    let anchor = conflict_line_anchors
                        .get(conflict_ix)
                        .copied()
                        .unwrap_or_default()
                        .min(visible_line);
                    anchor..anchor
                })
            })
            .collect();
        let line_count = visible_line.max(1);
        let span_checkpoints = build_span_checkpoints(&spans, line_count);

        Self {
            fragments,
            spans,
            span_checkpoints,
            conflict_line_ranges,
            line_count,
            widest_line_ix: widest_visible_line.0,
        }
    }

    pub fn len(&self) -> usize {
        self.line_count
    }

    pub fn widest_line_ix(&self) -> usize {
        self.widest_line_ix
    }

    /// Approximate heap bytes used by projection metadata, excluding the
    /// underlying segment texts which are shared with the resolver state.
    #[cfg(all(test, feature = "benchmarks"))]
    pub fn metadata_byte_size(&self) -> usize {
        let fragments = self.fragments.len() * std::mem::size_of::<ResolvedOutputFragment>()
            + self
                .fragments
                .iter()
                .map(ResolvedOutputFragment::metadata_byte_size)
                .sum::<usize>();
        let spans = self.spans.len() * std::mem::size_of::<ResolvedOutputSpan>()
            + self
                .spans
                .iter()
                .map(|span| match span {
                    ResolvedOutputSpan::SourceLines { .. } => 0,
                    ResolvedOutputSpan::MergedLine { text, .. } => text.capacity(),
                })
                .sum::<usize>();
        let span_checkpoints = self.span_checkpoints.len() * std::mem::size_of::<usize>();
        let conflict_ranges =
            self.conflict_line_ranges.len() * std::mem::size_of::<std::ops::Range<usize>>();
        fragments + spans + span_checkpoints + conflict_ranges
    }

    fn span_ix_for_visible_line(&self, line_ix: usize) -> Option<usize> {
        if self.spans.is_empty() || line_ix >= self.line_count {
            return None;
        }

        let checkpoint_ix = line_ix / RESOLVED_OUTPUT_SPAN_CHECKPOINT_STRIDE;
        let mut span_ix = self
            .span_checkpoints
            .get(checkpoint_ix)
            .copied()
            .unwrap_or_default();

        while let Some(span) = self.spans.get(span_ix) {
            let span_start = span.visible_start();
            let span_end = span_start.saturating_add(span.len());
            if line_ix < span_end {
                return (line_ix >= span_start).then_some(span_ix);
            }
            span_ix = span_ix.saturating_add(1);
        }

        None
    }

    pub fn conflict_line_range(&self, conflict_ix: usize) -> Option<std::ops::Range<usize>> {
        self.conflict_line_ranges.get(conflict_ix).cloned()
    }

    pub fn conflict_line_ranges(&self) -> &[std::ops::Range<usize>] {
        self.conflict_line_ranges.as_slice()
    }

    pub fn for_each_line_text_in_range<'a>(
        &'a self,
        segments: &'a [ConflictSegment],
        range: Range<usize>,
        mut visit: impl FnMut(usize, &'a str),
    ) {
        if range.start >= range.end || range.start >= self.line_count || self.spans.is_empty() {
            return;
        }

        let end = range.end.min(self.line_count);
        let mut line_ix = range.start;
        let mut span_ix = match self.span_ix_for_visible_line(line_ix) {
            Some(span_ix) => span_ix,
            None => return,
        };

        while line_ix < end {
            let Some(span) = self.spans.get(span_ix) else {
                break;
            };
            let span_start = span.visible_start();
            let span_end = span_start.saturating_add(span.len());
            if line_ix < span_start {
                line_ix = span_start;
                if line_ix >= end {
                    break;
                }
            }

            let visit_end = end.min(span_end);
            match span {
                ResolvedOutputSpan::SourceLines {
                    visible_start,
                    fragment_ix,
                    fragment_line_start,
                    ..
                } => {
                    let Some(fragment) = self.fragments.get(*fragment_ix) else {
                        break;
                    };
                    let fragment_range_start =
                        fragment_line_start.saturating_add(line_ix.saturating_sub(*visible_start));
                    let fragment_range_end =
                        fragment_range_start.saturating_add(visit_end.saturating_sub(line_ix));
                    fragment.for_each_line_text(
                        segments,
                        fragment_range_start..fragment_range_end,
                        |fragment_line_ix, line| {
                            let visible_ix = visible_start.saturating_add(
                                fragment_line_ix.saturating_sub(*fragment_line_start),
                            );
                            visit(visible_ix, line);
                        },
                    );
                }
                ResolvedOutputSpan::MergedLine {
                    visible_index,
                    text,
                } => {
                    if line_ix == *visible_index {
                        visit(*visible_index, text.as_str());
                    }
                }
            }

            line_ix = visit_end;
            span_ix = span_ix.saturating_add(1);
        }
    }

    pub fn line_text<'a>(
        &'a self,
        segments: &'a [ConflictSegment],
        line_ix: usize,
    ) -> Option<std::borrow::Cow<'a, str>> {
        let span_ix = self.span_ix_for_visible_line(line_ix)?;
        let span = self.spans.get(span_ix)?;
        if line_ix >= span.visible_start().saturating_add(span.len()) {
            return None;
        }
        match span {
            ResolvedOutputSpan::SourceLines {
                visible_start,
                fragment_ix,
                fragment_line_start,
                ..
            } => {
                let fragment = self.fragments.get(*fragment_ix)?;
                let line_ix_in_fragment =
                    fragment_line_start.saturating_add(line_ix.saturating_sub(*visible_start));
                fragment
                    .line_text(segments, line_ix_in_fragment)
                    .map(std::borrow::Cow::Borrowed)
            }
            ResolvedOutputSpan::MergedLine { text, .. } => {
                Some(std::borrow::Cow::Borrowed(text.as_str()))
            }
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_inline_rows(rows: &[gitcomet_core::file_diff::FileDiffRow]) -> Vec<ConflictInlineRow> {
    use gitcomet_core::domain::DiffLineKind as K;
    use gitcomet_core::file_diff::FileDiffRowKind as RK;

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

pub(super) fn block_max_line_count(block: &ConflictBlock) -> usize {
    text_line_count_usize(block.base.as_deref().unwrap_or_default())
        .max(text_line_count_usize(&block.ours))
        .max(text_line_count_usize(&block.theirs))
}

/// Conservative line budget for materializing an unresolved output.
///
/// An unresolved block currently occupies one `<Merge Conflict>` row, but the
/// first source pick can expand it to any one of its A/B/C inputs. Account for
/// that expansion before moving a streamed document into the editable buffer.
/// Summing fragment line counts can over-count at fragment boundaries, which is
/// intentional: this is a safety limit rather than an exact output projection.
pub(super) fn single_source_output_line_upper_bound(segments: &[ConflictSegment]) -> usize {
    segments.iter().fold(0usize, |total, segment| {
        let fragment_lines = match segment {
            ConflictSegment::Text(text) => text_line_count_usize(text),
            ConflictSegment::Block(block) => block_max_line_count(block).max(1),
        };
        total.saturating_add(fragment_lines)
    })
}

#[cfg(any(test, feature = "benchmarks"))]
fn should_use_large_conflict_block_preview(block: &ConflictBlock) -> bool {
    block_max_line_count(block) > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES
}

/// Whether computing the three-way alignment is practical for these sides
/// (section 30 aligned row space).
///
/// The alignment diff is O(size × dissimilarity): a whole-file conflict on a
/// large file makes Myers effectively quadratic. Small files always align;
/// large ones only when each side still shares a reasonable fraction of its
/// lines with base.
pub fn three_way_alignment_is_practical(base: &str, ours: &str, theirs: &str) -> bool {
    gitcomet_core::merge::interactive_merge_plan_is_practical(
        Some(base),
        ours,
        theirs,
        gitcomet_core::merge::InteractiveMergePlanBudget::default(),
    )
}

/// Whether computing the direct two-way alignment is practical (section 30 aligned
/// row space, no-base fallback). Same rationale as
/// [`three_way_alignment_is_practical`], with ours standing in for the base
/// as the similarity anchor.
pub fn two_way_alignment_is_practical(ours: &str, theirs: &str) -> bool {
    gitcomet_core::merge::interactive_merge_plan_is_practical(
        None,
        ours,
        theirs,
        gitcomet_core::merge::InteractiveMergePlanBudget::default(),
    )
}

pub fn select_conflict_rendering_mode(
    segments: &[ConflictSegment],
    combined_line_count: usize,
) -> ConflictRenderingMode {
    let _ = combined_line_count;
    if !segments.is_empty() {
        ConflictRenderingMode::StreamedLargeFile
    } else {
        ConflictRenderingMode::EagerSmallFile
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn preview_line_starts(text: &str) -> Vec<usize> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    starts.push(0);
    for (ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(ix.saturating_add(1));
        }
    }
    starts
}

#[cfg(any(test, feature = "benchmarks"))]
fn line_slice_text<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_count: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> &'a str {
    if text.is_empty() || line_count == 0 {
        return "";
    }

    let start = start_line_ix.min(line_count);
    let end = end_line_ix.min(line_count);
    if start >= end {
        return "";
    }

    let text_len = text.len();
    let start_byte = line_starts
        .get(start)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let end_byte = if end >= line_count {
        text_len
    } else {
        line_starts
            .get(end)
            .copied()
            .unwrap_or(text_len)
            .min(text_len)
    };
    if start_byte >= end_byte {
        return "";
    }
    text.get(start_byte..end_byte).unwrap_or("")
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_renumbered_block_diff_rows(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    old_text: &str,
    new_text: &str,
    old_line_offset: u32,
    new_line_offset: u32,
) -> bool {
    let old_line_count = text_line_count_usize(old_text);
    let new_line_count = text_line_count_usize(new_text);
    let whole_block_diff_ran = old_line_count > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES
        || new_line_count > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES;
    debug_assert!(
        !whole_block_diff_ran,
        "bootstrap should not call side_by_side_rows on a giant conflict block"
    );
    if push_tiny_block_diff_rows(rows, old_text, new_text, old_line_offset, new_line_offset) {
        return false;
    }
    gitcomet_core::file_diff::append_side_by_side_rows_with_offsets(
        rows,
        old_text,
        new_text,
        old_line_offset,
        new_line_offset,
    );
    whole_block_diff_ran
}

#[cfg(any(test, feature = "benchmarks"))]
fn collect_tiny_block_lines(text: &str) -> Option<([&str; 2], usize)> {
    let mut lines = ["", ""];
    let mut count = 0usize;
    for line in text.lines() {
        if count == lines.len() {
            return None;
        }
        lines[count] = line;
        count += 1;
    }
    Some((lines, count))
}

#[cfg(any(test, feature = "benchmarks"))]
fn tiny_block_line_number(start: u32, offset: usize) -> u32 {
    start.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX))
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_tiny_block_context_row(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    old_line: u32,
    new_line: u32,
    text: &str,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    rows.push(FileDiffRow {
        kind: FileDiffRowKind::Context,
        old_line: Some(old_line),
        new_line: Some(new_line),
        old: Some(text.into()),
        new: Some(text.into()),
        eof_newline: None,
    });
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_tiny_block_modify_row(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    old_line: u32,
    new_line: u32,
    old_text: &str,
    new_text: &str,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    rows.push(FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: Some(old_line),
        new_line: Some(new_line),
        old: Some(old_text.into()),
        new: Some(new_text.into()),
        eof_newline: None,
    });
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_tiny_block_remove_row(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    old_line: u32,
    text: &str,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    rows.push(FileDiffRow {
        kind: FileDiffRowKind::Remove,
        old_line: Some(old_line),
        new_line: None,
        old: Some(text.into()),
        new: None,
        eof_newline: None,
    });
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_tiny_block_add_row(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    new_line: u32,
    text: &str,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    rows.push(FileDiffRow {
        kind: FileDiffRowKind::Add,
        old_line: None,
        new_line: Some(new_line),
        old: None,
        new: Some(text.into()),
        eof_newline: None,
    });
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_tiny_block_diff_rows(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    old_text: &str,
    new_text: &str,
    old_line_offset: u32,
    new_line_offset: u32,
) -> bool {
    if (!old_text.is_empty() && !old_text.ends_with('\n'))
        || (!new_text.is_empty() && !new_text.ends_with('\n'))
    {
        return false;
    }

    let Some((old_lines, old_len)) = collect_tiny_block_lines(old_text) else {
        return false;
    };
    let Some((new_lines, new_len)) = collect_tiny_block_lines(new_text) else {
        return false;
    };
    if old_len > 1 && new_len > 1 {
        return false;
    }

    let mut prefix = 0usize;
    while prefix < old_len && prefix < new_len && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while prefix + suffix < old_len
        && prefix + suffix < new_len
        && old_lines[old_len - 1 - suffix] == new_lines[new_len - 1 - suffix]
    {
        suffix += 1;
    }

    for (offset, line) in old_lines.iter().enumerate().take(prefix) {
        push_tiny_block_context_row(
            rows,
            tiny_block_line_number(old_line_offset, offset),
            tiny_block_line_number(new_line_offset, offset),
            line,
        );
    }

    let old_mid_start = prefix;
    let new_mid_start = prefix;
    let old_mid_len = old_len.saturating_sub(prefix + suffix);
    let new_mid_len = new_len.saturating_sub(prefix + suffix);
    let paired_len = old_mid_len.min(new_mid_len);

    for offset in 0..paired_len {
        let old_ix = old_mid_start + offset;
        let new_ix = new_mid_start + offset;
        push_tiny_block_modify_row(
            rows,
            tiny_block_line_number(old_line_offset, old_ix),
            tiny_block_line_number(new_line_offset, new_ix),
            old_lines[old_ix],
            new_lines[new_ix],
        );
    }

    for offset in paired_len..old_mid_len {
        let old_ix = old_mid_start + offset;
        push_tiny_block_remove_row(
            rows,
            tiny_block_line_number(old_line_offset, old_ix),
            old_lines[old_ix],
        );
    }

    for offset in paired_len..new_mid_len {
        let new_ix = new_mid_start + offset;
        push_tiny_block_add_row(
            rows,
            tiny_block_line_number(new_line_offset, new_ix),
            new_lines[new_ix],
        );
    }

    for offset in 0..suffix {
        let old_ix = old_len - suffix + offset;
        let new_ix = new_len - suffix + offset;
        push_tiny_block_context_row(
            rows,
            tiny_block_line_number(old_line_offset, old_ix),
            tiny_block_line_number(new_line_offset, new_ix),
            old_lines[old_ix],
        );
    }

    true
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_large_conflict_block_preview_rows(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    block: &ConflictBlock,
    ours_offset: u32,
    theirs_offset: u32,
) {
    let ours_count = text_line_count_usize(&block.ours);
    let theirs_count = text_line_count_usize(&block.theirs);
    let ours_line_starts = preview_line_starts(&block.ours);
    let theirs_line_starts = preview_line_starts(&block.theirs);

    let head_ours_end = ours_count.min(LARGE_CONFLICT_BLOCK_PREVIEW_LINES);
    let head_theirs_end = theirs_count.min(LARGE_CONFLICT_BLOCK_PREVIEW_LINES);
    let _ = push_renumbered_block_diff_rows(
        rows,
        line_slice_text(&block.ours, &ours_line_starts, ours_count, 0, head_ours_end),
        line_slice_text(
            &block.theirs,
            &theirs_line_starts,
            theirs_count,
            0,
            head_theirs_end,
        ),
        ours_offset,
        theirs_offset,
    );

    let tail_ours_start = ours_count.saturating_sub(LARGE_CONFLICT_BLOCK_PREVIEW_LINES);
    let tail_theirs_start = theirs_count.saturating_sub(LARGE_CONFLICT_BLOCK_PREVIEW_LINES);
    let omitted_ours = tail_ours_start.saturating_sub(head_ours_end);
    let omitted_theirs = tail_theirs_start.saturating_sub(head_theirs_end);
    let can_show_tail = omitted_ours > 0 && omitted_theirs > 0;

    if omitted_ours > 0 || omitted_theirs > 0 {
        let summary: Arc<str> = format!(
            "... large conflict block preview omitted {omitted_ours} ours lines and {omitted_theirs} theirs lines ..."
        )
        .into();
        rows.push(gitcomet_core::file_diff::FileDiffRow {
            kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
            old_line: (omitted_ours > 0).then(|| {
                ours_offset.saturating_add(u32::try_from(head_ours_end).unwrap_or(u32::MAX))
            }),
            new_line: (omitted_theirs > 0).then(|| {
                theirs_offset.saturating_add(u32::try_from(head_theirs_end).unwrap_or(u32::MAX))
            }),
            old: Some(Arc::clone(&summary).into()),
            new: Some(summary.into()),
            eof_newline: None,
        });
    }

    if can_show_tail {
        let _ = push_renumbered_block_diff_rows(
            rows,
            line_slice_text(
                &block.ours,
                &ours_line_starts,
                ours_count,
                tail_ours_start,
                ours_count,
            ),
            line_slice_text(
                &block.theirs,
                &theirs_line_starts,
                theirs_count,
                tail_theirs_start,
                theirs_count,
            ),
            ours_offset.saturating_add(u32::try_from(tail_ours_start).unwrap_or(u32::MAX)),
            theirs_offset.saturating_add(u32::try_from(tail_theirs_start).unwrap_or(u32::MAX)),
        );
    }
}

/// Build two-way diff rows using block-local diffs instead of a full-file Myers diff.
///
/// For each `Block` segment, a block-local `side_by_side_rows` is run on just
/// the block's ours vs theirs text, and the resulting rows are re-numbered to
/// global line positions. Surrounding `Text` segments contribute only a small
/// boundary context window, so unchanged file regions are not materialized in
/// full.
///
/// The output is proportional to total conflict-block size plus a fixed amount
/// of context per block, making it suitable for very large files where running
/// Myers on the entire ours/theirs content would be prohibitively expensive.
#[cfg(any(test, feature = "benchmarks"))]
pub fn block_local_two_way_diff_rows(
    segments: &[ConflictSegment],
) -> Vec<gitcomet_core::file_diff::FileDiffRow> {
    block_local_two_way_diff_rows_with_stats(segments).0
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BlockLocalTwoWayDiffStats {
    pub(crate) whole_block_diff_ran: bool,
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn block_local_two_way_diff_rows_with_stats(
    segments: &[ConflictSegment],
) -> (
    Vec<gitcomet_core::file_diff::FileDiffRow>,
    BlockLocalTwoWayDiffStats,
) {
    block_local_two_way_diff_rows_with_context_and_stats(segments, BLOCK_LOCAL_DIFF_CONTEXT_LINES)
}

#[cfg(test)]
fn block_local_two_way_diff_rows_with_context(
    segments: &[ConflictSegment],
    context_lines: usize,
) -> Vec<gitcomet_core::file_diff::FileDiffRow> {
    block_local_two_way_diff_rows_with_context_and_stats(segments, context_lines).0
}

#[cfg(any(test, feature = "benchmarks"))]
fn block_local_two_way_diff_rows_with_context_and_stats(
    segments: &[ConflictSegment],
    context_lines: usize,
) -> (
    Vec<gitcomet_core::file_diff::FileDiffRow>,
    BlockLocalTwoWayDiffStats,
) {
    let mut rows = Vec::with_capacity(estimate_block_local_two_way_row_capacity(
        segments,
        context_lines,
    ));
    let mut stats = BlockLocalTwoWayDiffStats::default();
    let mut ours_line = 1u32;
    let mut theirs_line = 1u32;

    for (segment_ix, segment) in segments.iter().enumerate() {
        match segment {
            ConflictSegment::Text(text) => {
                let count = push_block_local_boundary_context_rows(
                    &mut rows,
                    segments,
                    segment_ix,
                    text,
                    ours_line,
                    theirs_line,
                    context_lines,
                );
                ours_line = ours_line.saturating_add(count);
                theirs_line = theirs_line.saturating_add(count);
            }
            ConflictSegment::Block(block) => {
                let ours_offset = ours_line;
                let theirs_offset = theirs_line;
                if should_use_large_conflict_block_preview(block) {
                    push_large_conflict_block_preview_rows(
                        &mut rows,
                        block,
                        ours_offset,
                        theirs_offset,
                    );
                } else {
                    stats.whole_block_diff_ran |= push_renumbered_block_diff_rows(
                        &mut rows,
                        &block.ours,
                        &block.theirs,
                        ours_offset,
                        theirs_offset,
                    );
                }
                let ours_count = text_line_count(&block.ours);
                let theirs_count = text_line_count(&block.theirs);
                ours_line = ours_line.saturating_add(ours_count);
                theirs_line = theirs_line.saturating_add(theirs_count);
            }
        }
    }
    (rows, stats)
}

#[cfg(any(test, feature = "benchmarks"))]
fn estimate_block_local_two_way_row_capacity(
    segments: &[ConflictSegment],
    context_lines: usize,
) -> usize {
    segments
        .len()
        .saturating_mul(context_lines.saturating_add(2))
        .max(1)
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_block_local_boundary_context_rows(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    segments: &[ConflictSegment],
    segment_ix: usize,
    text: &ConflictText,
    old_line_start: u32,
    new_line_start: u32,
    context_lines: usize,
) -> u32 {
    let text_str = text.as_str();
    let line_count = text_line_count(text_str);
    if text_str.is_empty() || context_lines == 0 {
        return line_count;
    }

    let has_prev_block = segment_ix > 0
        && matches!(
            segments.get(segment_ix - 1),
            Some(ConflictSegment::Block(_))
        );
    let has_next_block = matches!(
        segments.get(segment_ix + 1),
        Some(ConflictSegment::Block(_))
    );
    if !has_prev_block && !has_next_block {
        return line_count;
    }

    let line_count_usize = usize::try_from(line_count).unwrap_or(usize::MAX);

    let leading_count = if has_prev_block {
        context_lines.min(line_count_usize)
    } else {
        0
    };
    let trailing_count = if has_next_block {
        context_lines.min(line_count_usize)
    } else {
        0
    };
    let trailing_start = line_count_usize.saturating_sub(trailing_count);

    // Leading context: scan forward for the first `leading_count` lines.
    push_block_local_context_lines(
        rows,
        text_str.lines().enumerate().take(leading_count),
        old_line_start,
        new_line_start,
    );

    // Trailing context: find the byte offset of the trailing_start-th line
    // by scanning backwards from the end, avoiding a full-text forward scan.
    let effective_trailing_start = leading_count.max(trailing_start);
    if trailing_count > 0 && effective_trailing_start < line_count_usize {
        let bytes = text_str.as_bytes();
        // Find byte offset of the effective_trailing_start-th line by
        // reverse-scanning for the (line_count - effective_trailing_start)
        // newlines from the end.
        let lines_from_end = line_count_usize - effective_trailing_start;
        let byte_offset = byte_offset_of_nth_line_from_end(bytes, lines_from_end);
        push_block_local_context_lines(
            rows,
            text_str[byte_offset..]
                .lines()
                .enumerate()
                .map(move |(ix, line)| (effective_trailing_start + ix, line)),
            old_line_start,
            new_line_start,
        );
    }

    line_count
}

#[cfg(any(test, feature = "benchmarks"))]
/// Find the byte offset of the `n`-th line from the end of the text.
/// Returns the byte offset where the `n`-th-from-end line starts.
#[cfg(any(test, feature = "benchmarks"))]
fn byte_offset_of_nth_line_from_end(bytes: &[u8], n: usize) -> usize {
    if n == 0 {
        return bytes.len();
    }
    // Count newlines from the end. We need to find `n` line-start positions.
    // A line starts either at the beginning of the text or after a newline.
    // If the text ends with \n, the last newline does NOT start a new line
    // (text_line_count_usize treats trailing \n as the last line's terminator).
    let mut remaining = n;
    let mut pos = bytes.len();
    // Skip trailing newline if present (it terminates the last counted line,
    // not a new line after it).
    if pos > 0 && bytes[pos - 1] == b'\n' {
        pos -= 1;
    }
    while remaining > 0 && pos > 0 {
        if let Some(nl) = memchr::memrchr(b'\n', &bytes[..pos]) {
            pos = nl;
            remaining -= 1;
        } else {
            // No more newlines; the first line starts at offset 0.
            return 0;
        }
    }
    if remaining > 0 {
        0
    } else {
        // `pos` points at the newline; the line starts after it.
        pos + 1
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn push_block_local_context_lines<'a>(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    lines: impl Iterator<Item = (usize, &'a str)>,
    old_line_start: u32,
    new_line_start: u32,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    for (line_ix, text) in lines {
        let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
        let content: Arc<str> = text.into();
        rows.push(FileDiffRow {
            kind: FileDiffRowKind::Context,
            old_line: Some(old_line_start.saturating_add(line_offset)),
            new_line: Some(new_line_start.saturating_add(line_offset)),
            old: Some(Arc::clone(&content).into()),
            new: Some(content.into()),
            eof_newline: None,
        });
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn text_line_count(text: &str) -> u32 {
    u32::try_from(text_line_count_usize(text)).unwrap_or(u32::MAX)
}

#[cfg(any(test, feature = "benchmarks"))]
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

#[cfg(any(test, feature = "benchmarks"))]
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

fn text_line_count_usize(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let newline_count = memchr::memchr_iter(b'\n', bytes).count();
    if bytes.last() == Some(&b'\n') {
        newline_count
    } else {
        newline_count + 1
    }
}

fn indexed_line_count(text: &str, line_starts: &[usize]) -> usize {
    if text.is_empty() {
        0
    } else {
        line_starts.len()
    }
}

pub(super) fn indexed_line_text<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_ix: usize,
) -> Option<&'a str> {
    if text.is_empty() {
        return None;
    }
    let text_len = text.len();
    let start = line_starts.get(line_ix).copied().unwrap_or(text_len);
    if start >= text_len {
        return None;
    }
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    Some(text.get(start..end).unwrap_or(""))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TextLineStats {
    pub(super) line_count: usize,
    pub(super) widest_line_ix: usize,
    pub(super) widest_line_len: usize,
}

impl TextLineStats {
    pub(super) fn widest_line(self) -> Option<(usize, usize)> {
        (self.line_count > 0).then_some((self.widest_line_ix, self.widest_line_len))
    }
}

pub(super) fn scan_text_line_stats(text: &str) -> TextLineStats {
    if text.is_empty() {
        return TextLineStats::default();
    }

    let bytes = text.as_bytes();
    let mut line_count = 0usize;
    let mut prev_pos = 0usize;
    let mut widest_line_ix = 0usize;
    let mut widest_line_len = 0usize;

    for pos in memchr::memchr_iter(b'\n', bytes) {
        let line_len = pos - prev_pos;
        if line_len > widest_line_len {
            widest_line_len = line_len;
            widest_line_ix = line_count;
        }
        line_count += 1;
        prev_pos = pos + 1;
    }

    // Handle last line (no trailing newline).
    if prev_pos < bytes.len() {
        let line_len = bytes.len() - prev_pos;
        if line_len > widest_line_len {
            widest_line_len = line_len;
            widest_line_ix = line_count;
        }
        line_count += 1;
    }

    TextLineStats {
        line_count,
        widest_line_ix,
        widest_line_len,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ThreeWayConflictMaps {
    /// Per-side conflict ranges indexed by [base, ours, theirs].
    pub conflict_ranges: [Vec<std::ops::Range<usize>>; 3],
    /// Per-side per-line conflict maps (populated only in eager mode).
    pub line_conflict_maps: [Vec<Option<usize>>; 3],
    pub conflict_has_base: Vec<bool>,
    pub conflict_resolved: Vec<bool>,
}

/// Project marker-region ranges into the shared aligned source-row space.
///
/// This is the legacy/current-only fallback used when a merge plan is not
/// available. Callers may retain the result before resolved regions are
/// materialized into plain text so every original region remains navigable.
pub(in crate::view) fn project_conflict_ranges_to_aligned_rows(
    segments: &[ConflictSegment],
    aligned: &ThreeWayAlignedMap,
    side_line_counts: [usize; 3],
) -> Vec<Range<usize>> {
    let maps = build_three_way_conflict_maps_without_line_maps(
        segments,
        side_line_counts[0],
        side_line_counts[1],
        side_line_counts[2],
    );
    let block_count = maps.conflict_ranges[1].len();
    let mut aligned_ranges: Vec<Range<usize>> = Vec::with_capacity(block_count);
    for block_ix in 0..block_count {
        let mut start = usize::MAX;
        let mut end = 0usize;
        for side in 0..3 {
            let side_range = &maps.conflict_ranges[side][block_ix];
            if side_range.is_empty() {
                continue;
            }
            let mapped = aligned.aligned_range_for_side_range(side, side_range.clone());
            start = start.min(mapped.start);
            end = end.max(mapped.end);
        }
        if start == usize::MAX {
            start = end;
        }
        if let Some(previous) = aligned_ranges.last() {
            start = start.max(previous.end);
            end = end.max(start);
        }
        aligned_ranges.push(start..end);
    }
    aligned_ranges
}

/// Resolve visible marker blocks back to their exact aligned merge-plan rows.
///
/// Marker text is an output projection. Text between unresolved blocks can
/// come from only one source, so advancing every source offset by that text's
/// line count can merge adjacent conflict highlights or move later highlights
/// past their real rows. Full text sessions retain the authoritative mapping
/// from marker regions to merge-plan blocks; use it whenever it is available.
pub(super) fn merge_plan_aligned_conflict_ranges(
    session: &gitcomet_core::conflict_session::ConflictSession,
    visible_region_indices: &[usize],
    visible_plan_block_indices: &[usize],
) -> Option<Vec<Range<usize>>> {
    let plan = session.merge_plan.as_ref()?;
    if !visible_plan_block_indices.is_empty() {
        return visible_plan_block_indices
            .iter()
            .map(|block_index| {
                plan.blocks
                    .get(*block_index)
                    .map(|block| block.rows.clone())
            })
            .collect();
    }
    visible_region_indices
        .iter()
        .map(|region_index| {
            let block_index = *session.region_plan_blocks.get(*region_index)?;
            plan.blocks.get(block_index).map(|block| block.rows.clone())
        })
        .collect()
}

/// Binary search on sorted, non-overlapping ranges to find which conflict a line belongs to.
///
/// Returns `Some(conflict_index)` if the line falls within a range, `None` otherwise.
/// Ranges must be sorted by start and non-overlapping for correct results.
pub fn conflict_index_for_line(ranges: &[std::ops::Range<usize>], line: usize) -> Option<usize> {
    ranges
        .binary_search_by(|range| {
            if line < range.start {
                std::cmp::Ordering::Greater
            } else if line >= range.end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .ok()
}

/// Build per-column line-to-conflict maps for three-way conflict rendering.
///
/// The returned `conflict_ranges` follow the legacy behavior and are expressed
/// in the ours-column line space. The line maps provide O(1) conflict lookup
/// for each column at render/navigation time.
fn build_three_way_conflict_maps_impl(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
    include_line_conflict_maps: bool,
) -> ThreeWayConflictMaps {
    if segments.is_empty() {
        return ThreeWayConflictMaps {
            conflict_ranges: Default::default(),
            line_conflict_maps: if include_line_conflict_maps {
                [
                    vec![None; base_line_count],
                    vec![None; ours_line_count],
                    vec![None; theirs_line_count],
                ]
            } else {
                Default::default()
            },
            conflict_has_base: Vec::new(),
            conflict_resolved: Vec::new(),
        };
    }

    let block_count = segments
        .iter()
        .filter(|segment| matches!(segment, ConflictSegment::Block(_)))
        .count();
    let mut maps = ThreeWayConflictMaps {
        conflict_ranges: [
            Vec::with_capacity(block_count),
            Vec::with_capacity(block_count),
            Vec::with_capacity(block_count),
        ],
        line_conflict_maps: if include_line_conflict_maps {
            [
                vec![None; base_line_count],
                vec![None; ours_line_count],
                vec![None; theirs_line_count],
            ]
        } else {
            Default::default()
        },
        conflict_has_base: Vec::with_capacity(block_count),
        conflict_resolved: Vec::with_capacity(block_count),
    };

    fn mark_range(map: &mut [Option<usize>], start: usize, end: usize, conflict_ix: usize) {
        if map.is_empty() {
            return;
        }
        let from = start.min(map.len());
        let to = end.min(map.len());
        for slot in &mut map[from..to] {
            *slot = Some(conflict_ix);
        }
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    let mut conflict_ix = 0usize;
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                let line_count = text_line_count_usize(text);
                base_offset = base_offset.saturating_add(line_count);
                ours_offset = ours_offset.saturating_add(line_count);
                theirs_offset = theirs_offset.saturating_add(line_count);
            }
            ConflictSegment::Block(block) => {
                let base_count = text_line_count_usize(block.base.as_deref().unwrap_or_default());
                let ours_count = text_line_count_usize(&block.ours);
                let theirs_count = text_line_count_usize(&block.theirs);

                let base_end = base_offset.saturating_add(base_count);
                let ours_end = ours_offset.saturating_add(ours_count);
                let theirs_end = theirs_offset.saturating_add(theirs_count);

                maps.conflict_ranges[0].push(base_offset..base_end);
                maps.conflict_ranges[1].push(ours_offset..ours_end);
                maps.conflict_ranges[2].push(theirs_offset..theirs_end);
                maps.conflict_has_base.push(block.base.is_some());
                maps.conflict_resolved.push(block.resolved);

                mark_range(
                    &mut maps.line_conflict_maps[0],
                    base_offset,
                    base_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.line_conflict_maps[1],
                    ours_offset,
                    ours_end,
                    conflict_ix,
                );
                mark_range(
                    &mut maps.line_conflict_maps[2],
                    theirs_offset,
                    theirs_end,
                    conflict_ix,
                );

                base_offset = base_end;
                ours_offset = ours_end;
                theirs_offset = theirs_end;
                conflict_ix = conflict_ix.saturating_add(1);
            }
        }
    }

    maps
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_conflict_maps(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
) -> ThreeWayConflictMaps {
    build_three_way_conflict_maps_impl(
        segments,
        base_line_count,
        ours_line_count,
        theirs_line_count,
        true,
    )
}

/// Build compact three-way conflict metadata without eager per-line side maps.
pub fn build_three_way_conflict_maps_without_line_maps(
    segments: &[ConflictSegment],
    base_line_count: usize,
    ours_line_count: usize,
    theirs_line_count: usize,
) -> ThreeWayConflictMaps {
    build_three_way_conflict_maps_impl(
        segments,
        base_line_count,
        ours_line_count,
        theirs_line_count,
        false,
    )
}

/// Build conflict-index maps for two-way split and inline rows.
///
/// Each output entry is `Some(conflict_index)` when the row belongs to a marker
/// conflict block, or `None` for non-conflict context rows.
#[cfg(any(test, feature = "benchmarks"))]
pub fn map_two_way_rows_to_conflicts(
    segments: &[ConflictSegment],
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
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
#[cfg(any(test, feature = "benchmarks"))]
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
#[cfg(test)]
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

/// Build unresolved-only visible navigation entries for two-way views.
///
/// Returns visible list indices (not source row indices) in unresolved queue
/// order so callers can feed them directly into shared diff navigation helpers.
#[cfg(test)]
pub fn unresolved_visible_nav_entries_for_two_way(
    segments: &[ConflictSegment],
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
) -> Vec<usize> {
    unresolved_conflict_indices(segments)
        .into_iter()
        .filter_map(|conflict_ix| {
            visible_index_for_two_way_conflict(row_conflict_map, visible_row_indices, conflict_ix)
        })
        .collect()
}

/// Map a two-way visible index back to its conflict index.
#[cfg(test)]
pub fn two_way_conflict_index_for_visible_row(
    row_conflict_map: &[Option<usize>],
    visible_row_indices: &[usize],
    visible_ix: usize,
) -> Option<usize> {
    let row_ix = *visible_row_indices.get(visible_ix)?;
    row_conflict_map.get(row_ix).copied().flatten()
}

/// Represents a visible row in the three-way view when hide-resolved is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleItem {
    /// A normal line at the given index in the three-way data.
    Line(usize),
    /// A collapsed summary row for a resolved conflict block (by conflict index).
    CollapsedBlock(usize),
    /// A folded run of unchanged context lines (section 30 collapsed context mode).
    CollapsedContext {
        source_line_start: usize,
        len: usize,
        /// Stable fold identity (the fold's start line before any reveals),
        /// used to key partial-reveal state.
        fold_id: usize,
    },
}

/// kdiff3-style aligned row space over base/ours/theirs (section 30).
///
/// Maps between visual rows (shared by all columns) and per-side line
/// indices. Sides shorter than a run are padded: their rows map to `None`.
/// The default value is an unbounded identity map (row == line on every
/// side), which is also the fallback when alignment is unavailable
/// (missing/binary sides, giant files).
#[derive(Clone, Debug, Default)]
pub struct ThreeWayAlignedMap {
    runs: Vec<AlignedMapRun>,
    aligned_len: usize,
}

#[derive(Clone, Copy, Debug)]
struct AlignedMapRun {
    aligned_start: usize,
    rows: usize,
    starts: [usize; 3],
    lens: [usize; 3],
    kind: gitcomet_core::merge::AlignedRunKind,
}

impl ThreeWayAlignedMap {
    /// Build from the merge engine's alignment runs.
    pub fn from_alignment(alignment: &[gitcomet_core::merge::AlignedRun]) -> Self {
        let mut runs = Vec::with_capacity(alignment.len());
        let mut aligned_start = 0usize;
        for run in alignment {
            let rows = run.visual_rows();
            runs.push(AlignedMapRun {
                aligned_start,
                rows,
                starts: [run.base.start, run.ours.start, run.theirs.start],
                lens: [run.base.len(), run.ours.len(), run.theirs.len()],
                kind: run.kind,
            });
            aligned_start += rows;
        }
        Self {
            runs,
            aligned_len: aligned_start,
        }
    }

    /// The identity map behaves as if every side had `row == line`.
    pub fn is_identity(&self) -> bool {
        self.runs.is_empty()
    }

    /// Total aligned rows. Zero for the identity map (callers keep their own
    /// length in that case).
    pub fn aligned_len(&self) -> usize {
        self.aligned_len
    }

    fn run_for_row(&self, row: usize) -> Option<&AlignedMapRun> {
        if row >= self.aligned_len {
            return None;
        }
        let ix = self
            .runs
            .partition_point(|run| run.aligned_start + run.rows <= row);
        self.runs.get(ix)
    }

    /// The side line rendered at `row`, or `None` for padding rows (and rows
    /// past the aligned end).
    pub fn side_line_for_row(&self, side: usize, row: usize) -> Option<usize> {
        if self.is_identity() {
            return Some(row);
        }
        let run = self.run_for_row(row)?;
        let offset = row - run.aligned_start;
        (offset < run.lens[side]).then(|| run.starts[side] + offset)
    }

    /// The row at which a side line renders. Lines past the side's end clamp
    /// to the end of the aligned space.
    pub fn row_for_side_line(&self, side: usize, line: usize) -> usize {
        if self.is_identity() {
            return line;
        }
        let ix = self
            .runs
            .partition_point(|run| run.starts[side] + run.lens[side] <= line);
        match self.runs.get(ix) {
            Some(run) => run.aligned_start + line.saturating_sub(run.starts[side]),
            None => self.aligned_len,
        }
    }

    /// section 30 split: the side line index a split boundary at aligned `row` maps
    /// to — i.e. the first side line at or after `row` (padding rows round up
    /// to the next real line; rows past the aligned end clamp to the side
    /// length). Use with `row` and `row_end + 1` to bracket a selection.
    pub fn side_line_lower_bound(&self, side: usize, row: usize) -> usize {
        if self.is_identity() {
            return row;
        }
        match self.run_for_row(row) {
            Some(run) => {
                let offset = row - run.aligned_start;
                run.starts[side] + offset.min(run.lens[side])
            }
            None => self
                .runs
                .last()
                .map(|run| run.starts[side] + run.lens[side])
                .unwrap_or(0),
        }
    }

    /// Map a per-side line range to the aligned row range covering it.
    pub fn aligned_range_for_side_range(
        &self,
        side: usize,
        range: std::ops::Range<usize>,
    ) -> std::ops::Range<usize> {
        if self.is_identity() {
            return range;
        }
        if range.is_empty() {
            let boundary = self.row_for_side_line(side, range.start);
            return boundary..boundary;
        }
        let start = self.row_for_side_line(side, range.start);
        let end = self.row_for_side_line(side, range.end.saturating_sub(1)) + 1;
        start..end.max(start)
    }
}

/// section 30 R11: cap on aligned rows that receive word-level highlights, bounding
/// the per-row word-diff work on files with huge change counts.
pub const ALIGNED_WORD_HIGHLIGHT_MAX_ROWS: usize = 4_000;

fn merge_word_highlight_ranges(
    highlights: &mut WordHighlights,
    line_ix: usize,
    ranges: Vec<Range<usize>>,
) {
    if ranges.is_empty() {
        return;
    }
    let entry = highlights.entry(line_ix).or_default();
    entry.extend(ranges);
    entry.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(entry.len());
    for r in entry.drain(..) {
        if let Some(last) = merged.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        merged.push(r);
    }
    *entry = merged;
}

/// section 30 R11 (kdiff3 change colours): word highlights over the aligned row
/// space. For each aligned row where a side's line differs from the base line
/// paired at the same row, word-diff the pair and record ranges keyed by each
/// side's own line index (the renderer's cache key space). Padding rows
/// (added/removed lines) get no word ranges — the per-side row tint already
/// marks them whole. Requires a real base; both-added (two-way) maps use the
/// two-way highlight path instead.
pub fn compute_aligned_three_way_word_highlights(
    aligned: &ThreeWayAlignedMap,
    base_text: &str,
    base_line_starts: &[usize],
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let mut wh_base = WordHighlights::default();
    let mut wh_ours = WordHighlights::default();
    let mut wh_theirs = WordHighlights::default();
    if aligned.is_identity() || base_text.is_empty() {
        return (wh_base, wh_ours, wh_theirs);
    }

    let mut budget = ALIGNED_WORD_HIGHLIGHT_MAX_ROWS;
    for row in 0..aligned.aligned_len() {
        if budget == 0 {
            break;
        }
        let Some(base_ix) = aligned.side_line_for_row(0, row) else {
            continue;
        };
        let Some(base_line) = indexed_line_text(base_text, base_line_starts, base_ix) else {
            continue;
        };
        let mut row_diffed = false;
        for (side, side_text, side_starts, side_highlights) in [
            (1usize, ours_text, ours_line_starts, &mut wh_ours),
            (2usize, theirs_text, theirs_line_starts, &mut wh_theirs),
        ] {
            let Some(side_ix) = aligned.side_line_for_row(side, row) else {
                continue;
            };
            let Some(side_line) = indexed_line_text(side_text, side_starts, side_ix) else {
                continue;
            };
            if side_line == base_line {
                continue;
            }
            let (base_ranges, side_ranges) =
                crate::view::word_diff::capped_word_diff_ranges(base_line, side_line);
            merge_word_highlight_ranges(&mut wh_base, base_ix, base_ranges);
            merge_word_highlight_ranges(side_highlights, side_ix, side_ranges);
            row_diffed = true;
        }
        if row_diffed {
            budget -= 1;
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

/// section 30 R11: aligned two-way (ours↔theirs) word highlights, precomputed
/// once per conflict-source rebuild and shared by both diff columns (Ours and
/// Theirs). Keyed by aligned row — the renderer's row space. Only rows where
/// both sides have a line and the two lines differ byte-wise get an entry; the
/// render-time whitespace mode still decides whether to *apply* them
/// (whitespace-equal rows render as context), so this stays independent of that
/// toggle. Replaces the previous per-render, per-column inline word diff.
pub fn compute_aligned_two_way_word_highlights(
    aligned: &ThreeWayAlignedMap,
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> FxHashMap<usize, TwoWayWordHighlightPair> {
    let mut highlights = FxHashMap::default();
    if aligned.is_identity() {
        return highlights;
    }

    let mut budget = ALIGNED_WORD_HIGHLIGHT_MAX_ROWS;
    for row in 0..aligned.aligned_len() {
        if budget == 0 {
            break;
        }
        let (Some(ours_ix), Some(theirs_ix)) = (
            aligned.side_line_for_row(1, row),
            aligned.side_line_for_row(2, row),
        ) else {
            continue;
        };
        let (Some(ours_line), Some(theirs_line)) = (
            indexed_line_text(ours_text, ours_line_starts, ours_ix),
            indexed_line_text(theirs_text, theirs_line_starts, theirs_ix),
        ) else {
            continue;
        };
        if ours_line == theirs_line {
            continue;
        }
        if let Some(pair) = compute_word_highlights_for_texts(ours_line, theirs_line) {
            highlights.insert(row, pair);
            budget -= 1;
        }
    }

    highlights
}

/// Span-based replacement for `Vec<ThreeWayVisibleItem>` that uses O(spans) memory
/// instead of O(visible lines). Each span covers a contiguous run of source lines
/// or a single synthetic row (collapsed block / preview gap).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreeWayVisibleSpan {
    /// A contiguous run of source lines mapped 1:1 to visible indices.
    Lines {
        visible_start: usize,
        source_line_start: usize,
        len: usize,
    },
    /// A single collapsed-block row at the given visible index.
    CollapsedResolvedBlock {
        visible_index: usize,
        conflict_ix: usize,
    },
    /// A single fold row hiding `len` unchanged context lines.
    CollapsedContext {
        visible_index: usize,
        source_line_start: usize,
        len: usize,
        /// Stable fold identity (start line before any reveals).
        fold_id: usize,
    },
}

impl ThreeWayVisibleSpan {
    fn visible_start(&self) -> usize {
        match *self {
            Self::Lines { visible_start, .. } => visible_start,
            Self::CollapsedResolvedBlock { visible_index, .. } => visible_index,
            Self::CollapsedContext { visible_index, .. } => visible_index,
        }
    }

    fn visible_len(&self) -> usize {
        match *self {
            Self::Lines { len, .. } => len,
            Self::CollapsedResolvedBlock { .. } | Self::CollapsedContext { .. } => 1,
        }
    }
}

/// Compact visible-index projection for three-way views.
///
/// Replaces `Vec<ThreeWayVisibleItem>` for giant mode. Stores spans instead of
/// per-row entries, keeping memory proportional to the number of conflict blocks
/// rather than the number of file lines.
#[derive(Clone, Debug, Default)]
pub struct ThreeWayVisibleProjection {
    spans: Vec<ThreeWayVisibleSpan>,
    visible_len: usize,
}

enum ThreeWayVisibleRun {
    Lines { start: usize, end: usize },
    Collapsed { conflict_ix: usize },
}

fn for_each_three_way_visible_run(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
    mut visit: impl FnMut(ThreeWayVisibleRun),
) {
    if total_lines == 0 {
        return;
    }

    if !hide_resolved {
        visit(ThreeWayVisibleRun::Lines {
            start: 0,
            end: total_lines,
        });
        return;
    }

    let mut line_ix = 0usize;

    for (range_ix, range) in conflict_ranges.iter().enumerate() {
        if line_ix >= total_lines {
            break;
        }

        let range_start = range.start.min(total_lines);
        let range_end = range.end.min(total_lines);

        if line_ix < range_start {
            visit(ThreeWayVisibleRun::Lines {
                start: line_ix,
                end: range_start,
            });
            line_ix = range_start;
        }

        let resolved = conflict_resolved.get(range_ix).copied().unwrap_or(false);
        if resolved && range_start < range_end && line_ix < range_end {
            visit(ThreeWayVisibleRun::Collapsed {
                conflict_ix: range_ix,
            });
            line_ix = range_end;
            continue;
        }

        if line_ix < range_end {
            visit(ThreeWayVisibleRun::Lines {
                start: line_ix,
                end: range_end,
            });
            line_ix = range_end;
        }
    }

    if line_ix < total_lines {
        visit(ThreeWayVisibleRun::Lines {
            start: line_ix,
            end: total_lines,
        });
    }
}

impl ThreeWayVisibleProjection {
    /// Total number of visible rows.
    pub fn len(&self) -> usize {
        self.visible_len
    }

    /// Look up the visible item at the given visible index. O(log spans).
    pub fn get(&self, visible_ix: usize) -> Option<ThreeWayVisibleItem> {
        if visible_ix >= self.visible_len {
            return None;
        }
        let span_ix = self
            .spans
            .partition_point(|s| s.visible_start() + s.visible_len() <= visible_ix);
        let span = self.spans.get(span_ix)?;
        match *span {
            ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => {
                let offset = visible_ix.checked_sub(visible_start)?;
                if offset >= len {
                    return None;
                }
                Some(ThreeWayVisibleItem::Line(source_line_start + offset))
            }
            ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index,
                conflict_ix,
            } => {
                if visible_ix != visible_index {
                    return None;
                }
                Some(ThreeWayVisibleItem::CollapsedBlock(conflict_ix))
            }
            ThreeWayVisibleSpan::CollapsedContext {
                visible_index,
                source_line_start,
                len,
                fold_id,
            } => {
                if visible_ix != visible_index {
                    return None;
                }
                Some(ThreeWayVisibleItem::CollapsedContext {
                    source_line_start,
                    len,
                    fold_id,
                })
            }
        }
    }

    /// Find the visible index for the first line of a conflict range, or its
    /// collapsed entry. Returns `None` if the range is not visible.
    /// O(log spans).
    pub fn visible_index_for_conflict(
        &self,
        conflict_ranges: &[std::ops::Range<usize>],
        range_ix: usize,
    ) -> Option<usize> {
        let range = conflict_ranges.get(range_ix)?;
        for span in &self.spans {
            match *span {
                ThreeWayVisibleSpan::Lines {
                    visible_start,
                    source_line_start,
                    len,
                } => {
                    let source_end = source_line_start + len;
                    if range.start >= source_line_start && range.start < source_end {
                        return Some(visible_start + (range.start - source_line_start));
                    }
                }
                ThreeWayVisibleSpan::CollapsedResolvedBlock {
                    visible_index,
                    conflict_ix,
                } if conflict_ix == range_ix => {
                    return Some(visible_index);
                }
                _ => {}
            }
        }
        None
    }

    /// Access the underlying spans for direct iteration (avoids per-item O(log n) lookup).
    pub fn spans(&self) -> &[ThreeWayVisibleSpan] {
        &self.spans
    }

    /// Find the visible index showing the given source line. Lines hidden
    /// inside a collapsed context fold map to the fold's row.
    pub fn visible_index_for_source_line(&self, line: usize) -> Option<usize> {
        for span in &self.spans {
            match *span {
                ThreeWayVisibleSpan::Lines {
                    visible_start,
                    source_line_start,
                    len,
                } => {
                    if line >= source_line_start && line < source_line_start + len {
                        return Some(visible_start + (line - source_line_start));
                    }
                }
                ThreeWayVisibleSpan::CollapsedContext {
                    visible_index,
                    source_line_start,
                    len,
                    ..
                } => {
                    if line >= source_line_start && line < source_line_start + len {
                        return Some(visible_index);
                    }
                }
                ThreeWayVisibleSpan::CollapsedResolvedBlock { .. } => {}
            }
        }
        None
    }
}

/// Blank rows appended below the last line of the source diff lists so the
/// tail of the file can be scrolled up into a comfortable reading position.
pub const CONFLICT_BOTTOM_OVERSCROLL_ROWS: usize = 10;

/// Number of bands the overview column is quantized into.
///
/// kdiff3 paints one band per line; bounding the band count keeps paint cost
/// independent of file size while staying far finer than any column height.
pub const OVERVIEW_BAND_COUNT: usize = 2048;

/// Build the overview column's bands for the current three-way projection.
///
/// The result is in *visible* row space so the painted column lines up with
/// what the panes actually show: rows hidden by hide-resolved or collapsed
/// context are folded into their summary row's band, exactly as they are in
/// the lists. Returns an empty vector when the map carries no classification
/// (the identity fallback used for unaligned/giant files), which callers treat
/// as "no overview available".
///
/// `trailing_rows` are the blank overscroll rows the lists append below the
/// last line. They carry no changes but do take up scroll range, so the bands
/// have to cover them for the viewport frame to line up with the panes.
pub fn build_overview_bands(
    aligned: &ThreeWayAlignedMap,
    projection: &ThreeWayVisibleProjection,
    mode: gitcomet_core::merge::OverviewMode,
    trailing_rows: usize,
) -> Vec<gitcomet_core::merge::OverviewRowKind> {
    use gitcomet_core::merge::OverviewRowKind;

    if aligned.is_identity() || projection.len() == 0 {
        return Vec::new();
    }
    let visible_len = projection.len() + trailing_rows;

    let band_count = OVERVIEW_BAND_COUNT.min(visible_len);
    let mut bands = vec![OverviewRowKind::Unchanged; band_count];
    let mut paint = |visible: std::ops::Range<usize>, kind: OverviewRowKind| {
        if kind == OverviewRowKind::Unchanged || visible.is_empty() {
            return;
        }
        let first = visible.start.min(visible_len - 1) * band_count / visible_len;
        let last = (visible.end - 1).min(visible_len - 1) * band_count / visible_len;
        for band in &mut bands[first..=last.min(band_count - 1)] {
            *band = band.merge(kind);
        }
    };

    // Walk the visible spans and, for each, the aligned runs it covers. A
    // collapsed span shows several aligned rows on one visible row, so every
    // run it hides merges into that row's band.
    let spans = projection.spans();
    let runs = &aligned.runs;
    let aligned_len = aligned.aligned_len();
    let span_source_start = |span: &ThreeWayVisibleSpan| match *span {
        ThreeWayVisibleSpan::Lines {
            source_line_start, ..
        }
        | ThreeWayVisibleSpan::CollapsedContext {
            source_line_start, ..
        } => Some(source_line_start),
        // A collapsed conflict block carries no source range of its own.
        ThreeWayVisibleSpan::CollapsedResolvedBlock { .. } => None,
    };

    let mut covered = 0usize;
    for (span_ix, span) in spans.iter().enumerate() {
        let (source, visible_start, collapsed) = match *span {
            ThreeWayVisibleSpan::Lines {
                visible_start,
                source_line_start,
                len,
            } => (
                source_line_start..source_line_start + len,
                visible_start,
                false,
            ),
            ThreeWayVisibleSpan::CollapsedContext {
                visible_index,
                source_line_start,
                len,
                ..
            } => (
                source_line_start..source_line_start + len,
                visible_index,
                true,
            ),
            ThreeWayVisibleSpan::CollapsedResolvedBlock { visible_index, .. } => {
                // The hidden rows are everything between the previous span and
                // the next one that names a source line.
                let next = spans[span_ix + 1..]
                    .iter()
                    .find_map(span_source_start)
                    .unwrap_or(aligned_len);
                (covered..next.max(covered), visible_index, true)
            }
        };
        covered = covered.max(source.end);
        if source.is_empty() {
            continue;
        }

        let mut run_ix = runs.partition_point(|run| run.aligned_start + run.rows <= source.start);
        while let Some(run) = runs
            .get(run_ix)
            .filter(|run| run.aligned_start < source.end)
        {
            run_ix += 1;
            let kind = gitcomet_core::merge::overview_row_kind(run.kind, mode);
            if kind == OverviewRowKind::Unchanged {
                continue;
            }
            if collapsed {
                paint(visible_start..visible_start + 1, kind);
                continue;
            }
            let start = run.aligned_start.max(source.start);
            let end = (run.aligned_start + run.rows).min(source.end);
            paint(
                visible_start + (start - source.start)..visible_start + (end - source.start),
                kind,
            );
        }
    }

    bands
}

#[cfg(any(test, feature = "benchmarks"))]
fn resolved_conflict_flags_from_segments(segments: &[ConflictSegment]) -> Vec<bool> {
    segments
        .iter()
        .filter_map(|segment| match segment {
            ConflictSegment::Block(block) => Some(block.resolved),
            ConflictSegment::Text(_) => None,
        })
        .collect()
}

/// Build a span-based visible projection for three-way views.
///
/// All lines in every conflict block are included (no preview gaps).
/// Resolved blocks collapse to a single summary row when `hide_resolved` is true.
/// Context lines kept visible on each side of a conflict when collapsed
/// context mode is active (section 30).
pub(crate) const CONFLICT_COLLAPSED_CONTEXT_LINES: usize = 3;

/// Runs shorter than this stay expanded — a fold row would not be
/// meaningfully shorter than the lines it hides.
const MIN_CONTEXT_FOLD_LINES: usize = 2;

/// Lines revealed per click of a fold's reveal arrows (matches the diff
/// view's collapsed-hunk reveal step).
pub(crate) const CONFLICT_FOLD_REVEAL_STEP: usize = 20;

/// Per-fold partial-reveal state, keyed by the fold's stable identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct ConflictFoldReveal {
    /// Lines revealed from the top edge of the fold.
    pub top: usize,
    /// Lines revealed from the bottom edge of the fold.
    pub bottom: usize,
    /// The user expanded the whole fold.
    pub expand_all: bool,
}

/// Visibility options for the three-way projection.
#[derive(Clone, Copy, Default)]
pub(in crate::view) struct ThreeWayVisibleOptions<'a> {
    pub hide_resolved: bool,
    /// section 30 collapsed context mode: fold unchanged runs beyond
    /// [`CONFLICT_COLLAPSED_CONTEXT_LINES`] around each conflict.
    pub collapse_context: bool,
    /// Per-fold reveal state, keyed by fold identity (pre-reveal start line).
    pub context_fold_reveals: Option<&'a std::collections::HashMap<usize, ConflictFoldReveal>>,
}

/// Build the three-way visible projection with hide-resolved and collapsed
/// context folding applied.
pub(in crate::view) fn build_three_way_visible_projection_with_options(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    options: ThreeWayVisibleOptions<'_>,
) -> ThreeWayVisibleProjection {
    if !options.collapse_context || conflict_ranges.is_empty() {
        return build_three_way_visible_projection_with_resolved_flags(
            total_lines,
            conflict_ranges,
            conflict_resolved,
            options.hide_resolved,
        );
    }
    if total_lines == 0 {
        return ThreeWayVisibleProjection::default();
    }

    let fold_reveal = |fold_id: usize| {
        options
            .context_fold_reveals
            .and_then(|reveals| reveals.get(&fold_id).copied())
            .unwrap_or_default()
    };

    let mut spans: Vec<ThreeWayVisibleSpan> = Vec::new();
    let mut visible_ix = 0usize;
    let push_lines =
        |spans: &mut Vec<ThreeWayVisibleSpan>, visible_ix: &mut usize, start: usize, len: usize| {
            if len == 0 {
                return;
            }
            spans.push(ThreeWayVisibleSpan::Lines {
                visible_start: *visible_ix,
                source_line_start: start,
                len,
            });
            *visible_ix += len;
        };
    // Emit an unchanged gap, keeping `leading_keep` lines adjacent to the
    // previous conflict and `trailing_keep` lines before the next one;
    // anything beyond that folds unless the user expanded it.
    let push_gap = |spans: &mut Vec<ThreeWayVisibleSpan>,
                    visible_ix: &mut usize,
                    start: usize,
                    end: usize,
                    leading_keep: usize,
                    trailing_keep: usize| {
        let len = end.saturating_sub(start);
        if len == 0 {
            return;
        }
        let keep = leading_keep.saturating_add(trailing_keep);
        let fold_len = len.saturating_sub(keep);
        let fold_start = start + leading_keep;
        // The fold identity is its pre-reveal start line, so partial reveals
        // keep addressing the same fold.
        let fold_id = fold_start;
        let reveal = fold_reveal(fold_id);
        let revealed_top = reveal.top.min(fold_len);
        let revealed_bottom = reveal.bottom.min(fold_len.saturating_sub(revealed_top));
        let remaining = fold_len - revealed_top - revealed_bottom;
        if reveal.expand_all
            || fold_len < MIN_CONTEXT_FOLD_LINES
            || remaining < MIN_CONTEXT_FOLD_LINES
        {
            push_lines(spans, visible_ix, start, len);
            return;
        }
        push_lines(spans, visible_ix, start, leading_keep + revealed_top);
        spans.push(ThreeWayVisibleSpan::CollapsedContext {
            visible_index: *visible_ix,
            source_line_start: fold_start + revealed_top,
            len: remaining,
            fold_id,
        });
        *visible_ix += 1;
        push_lines(
            spans,
            visible_ix,
            fold_start + revealed_top + remaining,
            revealed_bottom + trailing_keep,
        );
    };

    let ctx = CONFLICT_COLLAPSED_CONTEXT_LINES;
    let mut line_ix = 0usize;
    for (range_ix, range) in conflict_ranges.iter().enumerate() {
        if line_ix >= total_lines {
            break;
        }
        let range_start = range.start.min(total_lines).max(line_ix);
        let range_end = range.end.min(total_lines).max(range_start);

        let leading_keep = if range_ix == 0 { 0 } else { ctx };
        push_gap(
            &mut spans,
            &mut visible_ix,
            line_ix,
            range_start,
            leading_keep,
            ctx,
        );

        let resolved = conflict_resolved.get(range_ix).copied().unwrap_or(false);
        if options.hide_resolved && resolved && range_start < range_end {
            spans.push(ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index: visible_ix,
                conflict_ix: range_ix,
            });
            visible_ix += 1;
        } else {
            push_lines(
                &mut spans,
                &mut visible_ix,
                range_start,
                range_end - range_start,
            );
        }
        line_ix = range_end;
    }
    if line_ix < total_lines {
        push_gap(&mut spans, &mut visible_ix, line_ix, total_lines, ctx, 0);
    }

    ThreeWayVisibleProjection {
        spans,
        visible_len: visible_ix,
    }
}

pub(in crate::view) fn build_three_way_visible_projection_with_resolved_flags(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
) -> ThreeWayVisibleProjection {
    if total_lines == 0 {
        return ThreeWayVisibleProjection::default();
    }

    if !hide_resolved {
        return ThreeWayVisibleProjection {
            spans: vec![ThreeWayVisibleSpan::Lines {
                visible_start: 0,
                source_line_start: 0,
                len: total_lines,
            }],
            visible_len: total_lines,
        };
    }

    let mut spans: Vec<ThreeWayVisibleSpan> =
        Vec::with_capacity(conflict_ranges.len().saturating_mul(2).saturating_add(1));
    let mut visible_ix = 0usize;
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                let len = end.saturating_sub(start);
                if len == 0 {
                    return;
                }
                spans.push(ThreeWayVisibleSpan::Lines {
                    visible_start: visible_ix,
                    source_line_start: start,
                    len,
                });
                visible_ix += len;
            }
            ThreeWayVisibleRun::Collapsed { conflict_ix } => {
                spans.push(ThreeWayVisibleSpan::CollapsedResolvedBlock {
                    visible_index: visible_ix,
                    conflict_ix,
                });
                visible_ix += 1;
            }
        },
    );

    ThreeWayVisibleProjection {
        spans,
        visible_len: visible_ix,
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_visible_projection(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> ThreeWayVisibleProjection {
    let conflict_resolved = resolved_conflict_flags_from_segments(segments);
    build_three_way_visible_projection_with_resolved_flags(
        total_lines,
        conflict_ranges,
        &conflict_resolved,
        hide_resolved,
    )
}

/// Build the mapping from visible row indices to actual three-way data items.
///
/// When `hide_resolved` is false, every line maps directly.
/// When true, resolved conflict ranges are collapsed to a single summary row.
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn build_three_way_visible_map_with_resolved_flags(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    conflict_resolved: &[bool],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    if total_lines == 0 {
        return Vec::new();
    }

    if !hide_resolved {
        return (0..total_lines).map(ThreeWayVisibleItem::Line).collect();
    }

    let mut visible_len = 0usize;
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                visible_len += end.saturating_sub(start);
            }
            ThreeWayVisibleRun::Collapsed { .. } => {
                visible_len += 1;
            }
        },
    );

    let mut visible = Vec::with_capacity(visible_len);
    for_each_three_way_visible_run(
        total_lines,
        conflict_ranges,
        conflict_resolved,
        true,
        |run| match run {
            ThreeWayVisibleRun::Lines { start, end } => {
                for line_ix in start..end {
                    visible.push(ThreeWayVisibleItem::Line(line_ix));
                }
            }
            ThreeWayVisibleRun::Collapsed { conflict_ix } => {
                visible.push(ThreeWayVisibleItem::CollapsedBlock(conflict_ix));
            }
        },
    );
    visible
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn build_three_way_visible_map(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
    hide_resolved: bool,
) -> Vec<ThreeWayVisibleItem> {
    let conflict_resolved = resolved_conflict_flags_from_segments(segments);
    build_three_way_visible_map_with_resolved_flags(
        total_lines,
        conflict_ranges,
        &conflict_resolved,
        hide_resolved,
    )
}

/// Find the visible index for the first line of a conflict range, or the
/// collapsed block entry. Returns `None` if the range is not visible.
#[cfg(test)]
pub fn visible_index_for_conflict(
    visible_map: &[ThreeWayVisibleItem],
    conflict_ranges: &[std::ops::Range<usize>],
    range_ix: usize,
) -> Option<usize> {
    let range = conflict_ranges.get(range_ix)?;
    visible_map.iter().position(|item| match item {
        ThreeWayVisibleItem::Line(ix) => range.contains(ix),
        ThreeWayVisibleItem::CollapsedBlock(ci) => *ci == range_ix,
        ThreeWayVisibleItem::CollapsedContext { .. } => false,
    })
}

/// When conflict markers use 2-way style (no `|||||||` base section), `block.base`
/// will be `None` even though the git ancestor content (index stage :1:) is available.
/// This function populates `block.base` by using the Text segments as anchors to
/// locate the corresponding base content in the ancestor file.
fn populate_block_bases_from_ancestor_impl(
    segments: &mut [ConflictSegment],
    ancestor_text: &str,
    shared_ancestor_text: Option<&Arc<str>>,
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
    let mut text_byte_ranges: Vec<std::ops::Range<usize>> =
        Vec::with_capacity(segments.len().saturating_add(1) / 2);
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
                block.base = Some(if let Some(shared_ancestor_text) = shared_ancestor_text {
                    ConflictText::shared_slice(
                        Arc::clone(shared_ancestor_text),
                        prev_end..next_start,
                    )
                } else {
                    ancestor_text[prev_end..next_start].to_string().into()
                });
            }
        }
    }
}

#[cfg(test)]
pub fn populate_block_bases_from_ancestor(segments: &mut [ConflictSegment], ancestor_text: &str) {
    populate_block_bases_from_ancestor_impl(segments, ancestor_text, None);
}

pub fn populate_block_bases_from_shared_ancestor(
    segments: &mut [ConflictSegment],
    ancestor_text: Arc<str>,
) {
    populate_block_bases_from_ancestor_impl(segments, ancestor_text.as_ref(), Some(&ancestor_text));
}

/// Check whether the given text still contains a complete git conflict-marker
/// block. Marker-looking content on its own (for example a Markdown `=======`
/// Setext underline) is not enough to block Save.
pub fn text_contains_conflict_markers(text: &str) -> bool {
    #[derive(Clone, Copy)]
    enum MarkerState {
        Outside,
        Ours,
        Theirs,
    }

    let mut state = MarkerState::Outside;
    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = MarkerState::Ours;
            continue;
        }
        state = match (state, line) {
            (MarkerState::Ours, line) if line.starts_with("=======") => MarkerState::Theirs,
            (MarkerState::Theirs, line) if line.starts_with(">>>>>>>") => return true,
            (current, _) => current,
        };
    }
    false
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictStageSafetyCheck {
    pub has_conflict_markers: bool,
    pub unresolved_blocks: usize,
}

impl ConflictStageSafetyCheck {
    pub fn blocks_save(self) -> bool {
        self.has_conflict_markers || self.unresolved_blocks > 0
    }
}

/// Compute stage-safety status for the current conflict resolver output/state.
///
/// This gate is stricter than marker-only checks: unresolved conflict blocks
/// still block the save even if the current output text no longer contains
/// marker lines.
pub fn conflict_stage_safety_check(
    output_text: &str,
    segments: &[ConflictSegment],
    block_map: &ResolvedOutputBlockMap,
) -> ConflictStageSafetyCheck {
    use gitcomet_core::conflict_session::ConflictRegionResolution;

    // The editor is intentionally not synchronized into session state on
    // every keystroke. Derive the effective resolutions from its current
    // contents so a manual replacement can enable Save, which then performs
    // the actual synchronization.
    let unresolved_blocks =
        derive_region_resolution_updates_from_output(segments, &[], block_map, output_text)
            .map(|updates| {
                updates
                    .iter()
                    .filter(|(_, resolution)| {
                        matches!(resolution, ConflictRegionResolution::Unresolved)
                    })
                    .count()
            })
            .unwrap_or_else(|| {
                // Ownership validation failed. Treat every displayed block as
                // unresolved so Save fails closed instead of guessing from
                // repeated context anchors.
                conflict_count(segments)
            });
    ConflictStageSafetyCheck {
        has_conflict_markers: text_contains_conflict_markers(output_text),
        unresolved_blocks,
    }
}

/// Count logical resolved-output rows while preserving a trailing empty row
/// after a final newline.
pub fn resolved_output_outline_line_count(output: &str) -> usize {
    memchr::memchr_iter(b'\n', output.as_bytes())
        .count()
        .saturating_add(1)
}

/// Split resolved output into one logical row per newline for outline rendering.
///
/// Uses `split('\n')` so trailing newlines are preserved as a final empty row.
#[cfg(any(test, feature = "benchmarks"))]
pub fn split_output_lines_for_outline(output: &str) -> Vec<String> {
    let mut lines = Vec::with_capacity(resolved_output_outline_line_count(output));
    lines.extend(output.split('\n').map(str::to_string));
    lines
}

#[cfg(test)]
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

// ---------------------------------------------------------------------------
// Provenance mapping: classify resolved output lines as A/B/C/Manual
// ---------------------------------------------------------------------------

/// Source lines from the three input panes, used for provenance matching.
///
/// In three-way mode: A = Base, B = Ours, C = Theirs.
/// In two-way mode: A = Ours (old), B = Theirs (new), C is empty.
#[cfg(any(test, feature = "benchmarks"))]
pub struct SourceLines<'a> {
    pub a: &'a [gpui::SharedString],
    pub b: &'a [gpui::SharedString],
    pub c: &'a [gpui::SharedString],
}

#[cfg(any(test, feature = "benchmarks"))]
fn build_source_line_lookup<'a>(
    sources: &'a SourceLines<'a>,
) -> rustc_hash::FxHashMap<&'a str, (ResolvedLineSource, u32)> {
    let mut lookup = rustc_hash::FxHashMap::default();

    // Insert in reverse order so duplicates keep the first line number within a side.
    // Later sides overwrite earlier ones to enforce priority A > B > C.
    for (ix, line) in sources.c.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::C,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }
    for (ix, line) in sources.b.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::B,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }
    for (ix, line) in sources.a.iter().enumerate().rev() {
        lookup.insert(
            line.as_ref(),
            (
                ResolvedLineSource::A,
                u32::try_from(ix + 1).unwrap_or(u32::MAX),
            ),
        );
    }

    lookup
}

fn compute_resolved_line_provenance_from_iter<'a>(
    output_lines: impl Iterator<Item = &'a str>,
    lookup: &rustc_hash::FxHashMap<&str, (ResolvedLineSource, u32)>,
) -> Vec<ResolvedLineMeta> {
    let mut result = Vec::new();
    for (out_ix, out_line) in output_lines.enumerate() {
        let (source, input_line) = match lookup.get(out_line).copied() {
            Some((src, line_no)) => (src, Some(line_no)),
            None => (ResolvedLineSource::Manual, None),
        };
        result.push(ResolvedLineMeta {
            output_line: out_ix as u32,
            source,
            input_line,
        });
    }
    result
}

/// Compute per-line provenance metadata for the resolved output.
///
/// Each output line is compared (exact text equality) against every source line
/// in A, B, C. The first match found (priority: A, B, C) wins; if none match
/// the line is labeled `Manual`.
#[cfg(any(test, feature = "benchmarks"))]
pub fn compute_resolved_line_provenance(
    output_lines: &[String],
    sources: &SourceLines<'_>,
) -> Vec<ResolvedLineMeta> {
    let lookup = build_source_line_lookup(sources);
    compute_resolved_line_provenance_from_iter(output_lines.iter().map(String::as_str), &lookup)
}

fn insert_indexed_source_lines<'a>(
    lookup: &mut rustc_hash::FxHashMap<&'a str, (ResolvedLineSource, u32)>,
    source: ResolvedLineSource,
    text: &'a str,
    line_starts: &[usize],
) {
    let line_count = indexed_line_count(text, line_starts);
    for line_ix in (0..line_count).rev() {
        if let Some(line) = indexed_line_text(text, line_starts, line_ix) {
            lookup.insert(
                line,
                (
                    source,
                    u32::try_from(line_ix.saturating_add(1)).unwrap_or(u32::MAX),
                ),
            );
        }
    }
}

pub fn compute_resolved_line_provenance_from_text_with_indexed_sources(
    output_text: &str,
    a_text: &str,
    a_line_starts: &[usize],
    b_text: &str,
    b_line_starts: &[usize],
    c_text: &str,
    c_line_starts: &[usize],
) -> Vec<ResolvedLineMeta> {
    let mut lookup = rustc_hash::FxHashMap::default();
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::C, c_text, c_line_starts);
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::B, b_text, b_line_starts);
    insert_indexed_source_lines(&mut lookup, ResolvedLineSource::A, a_text, a_line_starts);
    compute_resolved_line_provenance_from_iter(output_text.split('\n'), &lookup)
}

pub fn compute_resolved_line_provenance_from_text_two_way_indexed_sources(
    output_text: &str,
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
) -> Vec<ResolvedLineMeta> {
    let mut lookup = rustc_hash::FxHashMap::default();
    insert_indexed_source_lines(
        &mut lookup,
        ResolvedLineSource::B,
        theirs_text,
        theirs_line_starts,
    );
    insert_indexed_source_lines(
        &mut lookup,
        ResolvedLineSource::A,
        ours_text,
        ours_line_starts,
    );
    compute_resolved_line_provenance_from_iter(output_text.split('\n'), &lookup)
}

// ---------------------------------------------------------------------------
// Dedupe key index: tracks which source lines are present in resolved output
// ---------------------------------------------------------------------------

/// Build the set of `SourceLineKey`s currently represented in the resolved output.
///
/// Used to gate the plus-icon: a source row's plus-icon is hidden when its key
/// is already in this set (preventing duplicate insertion).
#[cfg(test)]
pub fn build_resolved_output_line_sources_index(
    meta: &[ResolvedLineMeta],
    output_lines: &[String],
    view_mode: ConflictResolverViewMode,
) -> rustc_hash::FxHashSet<SourceLineKey> {
    let mut index = rustc_hash::FxHashSet::with_capacity_and_hasher(meta.len(), Default::default());
    for m in meta {
        if m.source == ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = m.input_line else {
            continue;
        };
        let content = output_lines
            .get(m.output_line as usize)
            .map(|s| s.as_str())
            .unwrap_or("");
        index.insert(SourceLineKey::new(view_mode, m.source, line_no, content));
    }
    index
}

pub fn build_resolved_output_line_sources_index_from_text(
    meta: &[ResolvedLineMeta],
    output_text: &str,
    view_mode: ConflictResolverViewMode,
) -> rustc_hash::FxHashSet<SourceLineKey> {
    let mut index = rustc_hash::FxHashSet::with_capacity_and_hasher(meta.len(), Default::default());
    for (ix, line) in output_text.split('\n').enumerate() {
        let Some(m) = meta.get(ix) else {
            break;
        };
        if m.source == ResolvedLineSource::Manual {
            continue;
        }
        let Some(line_no) = m.input_line else {
            continue;
        };
        index.insert(SourceLineKey::new(view_mode, m.source, line_no, line));
    }
    index
}

/// Check whether a given source line is already present in the resolved output.
///
/// Returns `true` if the source line's key is in the dedupe index — meaning
/// the plus-icon for that row should be hidden.
#[cfg(test)]
pub fn is_source_line_in_output(
    index: &rustc_hash::FxHashSet<SourceLineKey>,
    view_mode: ConflictResolverViewMode,
    side: ResolvedLineSource,
    line_no: u32,
    content: &str,
) -> bool {
    let key = SourceLineKey::new(view_mode, side, line_no, content);
    index.contains(&key)
}

/// Extract a single line from text using pre-computed line starts.
fn line_text_from_starts<'a>(text: &'a str, line_starts: &[usize], line_ix: usize) -> &'a str {
    let text_len = text.len();
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let end = line_starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if start >= end {
        return "";
    }
    let slice = text.get(start..end).unwrap_or("");
    slice.strip_suffix('\n').unwrap_or(slice)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests;
