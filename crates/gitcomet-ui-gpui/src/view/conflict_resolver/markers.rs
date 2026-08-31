use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum ConflictTextStorage {
    Owned(String),
    SharedSlice { text: Arc<str>, range: Range<usize> },
}

#[derive(Clone, Debug)]
pub struct ConflictText {
    pub(super) storage: ConflictTextStorage,
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
    /// Standing alone it reads as an unresolved marker that both starts and ends
    /// on this row, which is what the fallback needs when the marker array has
    /// no entry for it. A block wider than one row names only its first row, so
    /// where the marker array *does* cover the row it owns the bracket ends —
    /// otherwise a multi-row block would close its bracket on the named row.
    #[inline(always)]
    pub(in crate::view) fn with_unresolved_placeholder(self) -> Self {
        Self(self.0 | Self::PLACEHOLDER_FLAG)
    }

    #[inline(always)]
    fn is_placeholder(self) -> bool {
        (self.0 & Self::PLACEHOLDER_FLAG) != 0
    }

    /// A placeholder row the marker array does not cover — the only case where
    /// the row's own text has to stand in for marker bracket ends.
    #[inline(always)]
    fn is_unmarked_placeholder(self) -> bool {
        self.is_placeholder() && (self.0 >> Self::CONFLICT_SHIFT) == 0
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
        self.is_unmarked_placeholder() || (self.0 & Self::IS_START_FLAG) != 0
    }

    #[inline(always)]
    pub(in crate::view) fn is_end(self) -> bool {
        self.is_unmarked_placeholder() || (self.0 & Self::IS_END_FLAG) != 0
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
        let mut hasher = FxHasher::default();
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

pub(in crate::view) const CONFLICT_SPLIT_WORD_HIGHLIGHT_CACHE_ROWS: usize = 4_096;

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

pub(in crate::view) fn append_text_segment(
    segments: &mut Vec<ConflictSegment>,
    text: impl Into<ConflictText>,
) {
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

pub(in crate::view) fn choice_for_resolved_content(
    block: &ConflictBlock,
    content: &str,
) -> Option<ConflictChoice> {
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
