mod split_row_index;
mod word_highlight;

#[cfg(test)]
use split_row_index::{CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES, CONFLICT_SPLIT_PAGE_SIZE};
pub use split_row_index::{ConflictSplitRowIndex, TwoWaySplitProjection, TwoWaySplitVisibleRow};
#[cfg(any(test, feature = "benchmarks"))]
pub use word_highlight::compute_three_way_word_highlights;
pub use word_highlight::compute_word_highlights_for_row;
#[cfg(feature = "benchmarks")]
pub use word_highlight::{TwoWayWordHighlights, compute_two_way_word_highlights};

use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictChoice {
    Base,
    Ours,
    Theirs,
    Both,
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveTraceMode {
    Safe,
    #[cfg(test)]
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictNavDirection {
    Prev,
    Next,
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
        Self {
            storage: ConflictTextStorage::Owned(value),
        }
    }
}

impl From<&str> for ConflictText {
    fn from(value: &str) -> Self {
        value.to_string().into()
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
pub type TwoWayWordHighlightPair = (Vec<Range<usize>>, Vec<Range<usize>>);

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

/// Resolve conflict quick-pick keyboard shortcuts to a concrete choice.
pub fn conflict_quick_pick_choice_for_key(key: &str) -> Option<ConflictChoice> {
    match key {
        "a" => Some(ConflictChoice::Base),
        "b" => Some(ConflictChoice::Ours),
        "c" => Some(ConflictChoice::Theirs),
        "d" => Some(ConflictChoice::Both),
        _ => None,
    }
}

/// Resolve conflict navigation shortcuts (`F2`, `F3`, `F7`) to a direction.
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
        AutosolveTraceMode::Safe => format!(
            "Last autosolve (safe): resolved {resolved} {blocks_word}, unresolved {} -> {} (pass1 {}, split {}, pass1-after-split {}).",
            unresolved_before,
            unresolved_after,
            stats.pass1,
            stats.pass2_split,
            stats.pass1_after_split
        ),
        #[cfg(test)]
        AutosolveTraceMode::History => format!(
            "Last autosolve (history): resolved {resolved} {blocks_word}, unresolved {} -> {} (history {}).",
            unresolved_before, unresolved_after, stats.history
        ),
    }
}

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
                    choice: ConflictChoice::Ours,
                    resolved: false,
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
    match block.choice {
        ConflictChoice::Base => block.base.as_deref().is_some_and(|base| content == base),
        ConflictChoice::Ours => content == block.ours,
        ConflictChoice::Theirs => content == block.theirs,
        ConflictChoice::Both => content
            .strip_prefix(block.ours.as_str())
            .is_some_and(|rest| rest == block.theirs),
    }
}

fn extract_block_contents_from_output(
    segments: &[ConflictSegment],
    output_text: &str,
) -> Option<Vec<String>> {
    let mut cursor = 0usize;
    let mut block_contents = Vec::new();

    for (seg_ix, seg) in segments.iter().enumerate() {
        match seg {
            ConflictSegment::Text(text) => {
                let tail = output_text.get(cursor..)?;
                if !tail.starts_with(text.as_str()) {
                    return None;
                }
                cursor = cursor.saturating_add(text.len());
            }
            ConflictSegment::Block(_) => {
                let next_anchor = segments[seg_ix + 1..].iter().find_map(|next| match next {
                    ConflictSegment::Text(text) if !text.is_empty() => Some(text.as_str()),
                    _ => None,
                });
                let end = match next_anchor {
                    Some(anchor) => {
                        let rel = output_text.get(cursor..)?.find(anchor)?;
                        cursor.saturating_add(rel)
                    }
                    None => output_text.len(),
                };
                if end < cursor {
                    return None;
                }
                block_contents.push(output_text[cursor..end].to_string());
                cursor = end;
            }
        }
    }

    (cursor == output_text.len()).then_some(block_contents)
}

/// Derive per-region session resolution updates from the current resolved output.
///
/// This is used to persist manual resolver edits back into state without
/// requiring marker reparse in the reducer.
pub fn derive_region_resolution_updates_from_output(
    segments: &[ConflictSegment],
    block_region_indices: &[usize],
    output_text: &str,
) -> Option<
    Vec<(
        usize,
        gitcomet_core::conflict_session::ConflictRegionResolution,
    )>,
> {
    use gitcomet_core::conflict_session::ConflictRegionResolution as R;

    let block_contents = extract_block_contents_from_output(segments, output_text)?;
    let mut updates = Vec::with_capacity(block_contents.len());

    let mut block_ix = 0usize;
    for seg in segments {
        let ConflictSegment::Block(block) = seg else {
            continue;
        };
        let content = block_contents.get(block_ix)?;
        let region_ix = block_region_indices
            .get(block_ix)
            .copied()
            .unwrap_or(block_ix);

        let resolution = if !block.resolved && content_matches_block_choice(block, content) {
            R::Unresolved
        } else if let Some(choice) = choice_for_resolved_content(block, content) {
            match choice {
                ConflictChoice::Base => R::PickBase,
                ConflictChoice::Ours => R::PickOurs,
                ConflictChoice::Theirs => R::PickTheirs,
                ConflictChoice::Both => R::PickBoth,
            }
        } else {
            R::ManualEdit(content.clone())
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
            match block.choice {
                ConflictChoice::Base => R::PickBase,
                ConflictChoice::Ours => R::PickOurs,
                ConflictChoice::Theirs => R::PickTheirs,
                ConflictChoice::Both => R::PickBoth,
            }
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
            block.resolved = false;
            None
        }
        R::PickBase => {
            if block.base.is_some() {
                block.choice = ConflictChoice::Base;
                block.resolved = true;
            } else {
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
                                    choice: ConflictChoice::Ours,
                                    resolved: false,
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

pub fn generate_resolved_text(segments: &[ConflictSegment]) -> String {
    use gitcomet_core::conflict_output::GenerateResolvedTextOptions;

    generate_resolved_text_with_options(segments, GenerateResolvedTextOptions::default())
}

pub fn generate_resolved_text_with_options(
    segments: &[ConflictSegment],
    options: gitcomet_core::conflict_output::GenerateResolvedTextOptions<'_>,
) -> String {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, ConflictOutputChoice, ConflictOutputSegmentRef,
        generate_resolved_text as generate_core_resolved_text,
    };

    fn map_choice(choice: ConflictChoice) -> ConflictOutputChoice {
        match choice {
            ConflictChoice::Base => ConflictOutputChoice::Base,
            ConflictChoice::Ours => ConflictOutputChoice::Ours,
            ConflictChoice::Theirs => ConflictOutputChoice::Theirs,
            ConflictChoice::Both => ConflictOutputChoice::Both,
        }
    }

    let core_segments: Vec<ConflictOutputSegmentRef<'_>> = segments
        .iter()
        .map(|segment| match segment {
            ConflictSegment::Text(text) => ConflictOutputSegmentRef::Text(text),
            ConflictSegment::Block(block) => {
                ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                    base: block.base.as_deref(),
                    ours: &block.ours,
                    theirs: &block.theirs,
                    choice: map_choice(block.choice),
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
}

#[derive(Clone, Debug)]
struct ResolvedOutputFragment {
    source: ResolvedOutputFragmentSource,
    line_starts: std::sync::Arc<[usize]>,
    newline_count: usize,
    ends_with_newline: bool,
    line_count: usize,
    widest_line_ix: usize,
    widest_line_len: usize,
}

impl ResolvedOutputFragment {
    fn line_text<'a>(&self, segments: &'a [ConflictSegment], line_ix: usize) -> Option<&'a str> {
        let text = match self.source {
            ResolvedOutputFragmentSource::TextSegment { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Text(text)) => text.as_str(),
                    _ => return None,
                }
            }
            ResolvedOutputFragmentSource::BlockBase { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => block.base.as_deref().unwrap_or(""),
                    _ => return None,
                }
            }
            ResolvedOutputFragmentSource::BlockOurs { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => block.ours.as_str(),
                    _ => return None,
                }
            }
            ResolvedOutputFragmentSource::BlockTheirs { segment_ix } => {
                match segments.get(segment_ix) {
                    Some(ConflictSegment::Block(block)) => block.theirs.as_str(),
                    _ => return None,
                }
            }
        };
        (line_ix < self.line_count)
            .then(|| line_text_from_starts(text, self.line_starts.as_ref(), line_ix))
    }

    fn widest_line(&self) -> Option<(usize, usize)> {
        (self.line_count > 0).then_some((self.widest_line_ix, self.widest_line_len))
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
    conflict_line_ranges: Vec<std::ops::Range<usize>>,
    line_count: usize,
    widest_line_ix: usize,
    output_hash: u64,
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

        fn fragment_line_stats(
            text: &str,
        ) -> (std::sync::Arc<[usize]>, usize, bool, usize, usize, usize) {
            let bytes = text.as_bytes();
            let mut starts = Vec::new();
            starts.push(0usize);
            let mut prev_pos = 0usize;
            let mut widest_line_ix = 0usize;
            let mut widest_line_len = 0usize;
            let mut line_count = 0usize;

            for pos in memchr::memchr_iter(b'\n', bytes) {
                starts.push(pos + 1);
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

            let newline_count = starts.len().saturating_sub(1);
            let ends_with_newline = bytes.last().copied() == Some(b'\n');
            (
                starts.into(),
                newline_count,
                ends_with_newline,
                line_count,
                widest_line_ix,
                widest_line_len,
            )
        }

        fn structural_output_hash(
            fragments: &[ResolvedOutputFragment],
            spans: &[ResolvedOutputSpan],
            conflict_line_ranges: &[std::ops::Range<usize>],
            line_count: usize,
        ) -> u64 {
            use std::hash::{Hash, Hasher};

            let mut hasher = rustc_hash::FxHasher::default();
            line_count.hash(&mut hasher);
            fragments.len().hash(&mut hasher);
            spans.len().hash(&mut hasher);
            conflict_line_ranges.len().hash(&mut hasher);
            for fragment in fragments {
                fragment.source.hash(&mut hasher);
                fragment.line_starts.len().hash(&mut hasher);
                fragment.newline_count.hash(&mut hasher);
                fragment.ends_with_newline.hash(&mut hasher);
            }
            for span in spans {
                match span {
                    ResolvedOutputSpan::SourceLines {
                        visible_start,
                        len,
                        fragment_ix,
                        fragment_line_start,
                    } => {
                        0u8.hash(&mut hasher);
                        visible_start.hash(&mut hasher);
                        len.hash(&mut hasher);
                        fragment_ix.hash(&mut hasher);
                        fragment_line_start.hash(&mut hasher);
                    }
                    ResolvedOutputSpan::MergedLine {
                        visible_index,
                        text,
                    } => {
                        1u8.hash(&mut hasher);
                        visible_index.hash(&mut hasher);
                        text.hash(&mut hasher);
                    }
                }
            }
            for range in conflict_line_ranges {
                range.start.hash(&mut hasher);
                range.end.hash(&mut hasher);
            }
            hasher.finish()
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
        let mut conflict_ranges: Vec<Option<std::ops::Range<usize>>> = vec![None; conflict_total];
        let mut conflict_line_anchors = vec![0usize; conflict_total];
        let mut fragments = Vec::new();
        let mut spans = Vec::new();
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
                line_starts,
                newline_count,
                ends_with_newline,
                line_count,
                widest_line_ix,
                widest_line_len,
            ) = fragment_line_stats(text);
            let fragment_ix = fragments.len();
            fragments.push(ResolvedOutputFragment {
                source,
                line_starts,
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

                    let mut fragment_sources: Vec<(ResolvedOutputFragmentSource, &str)> =
                        Vec::new();
                    match block.choice {
                        ConflictChoice::Base => {
                            if let Some(base) = block.base.as_deref() {
                                fragment_sources.push((
                                    ResolvedOutputFragmentSource::BlockBase { segment_ix },
                                    base,
                                ));
                            }
                        }
                        ConflictChoice::Ours => {
                            fragment_sources.push((
                                ResolvedOutputFragmentSource::BlockOurs { segment_ix },
                                block.ours.as_str(),
                            ));
                        }
                        ConflictChoice::Theirs => {
                            fragment_sources.push((
                                ResolvedOutputFragmentSource::BlockTheirs { segment_ix },
                                block.theirs.as_str(),
                            ));
                        }
                        ConflictChoice::Both => {
                            fragment_sources.push((
                                ResolvedOutputFragmentSource::BlockOurs { segment_ix },
                                block.ours.as_str(),
                            ));
                            fragment_sources.push((
                                ResolvedOutputFragmentSource::BlockTheirs { segment_ix },
                                block.theirs.as_str(),
                            ));
                        }
                    }

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
        let output_hash = structural_output_hash(
            &fragments,
            &spans,
            conflict_line_ranges.as_slice(),
            line_count,
        );

        Self {
            fragments,
            spans,
            conflict_line_ranges,
            line_count,
            widest_line_ix: widest_visible_line.0,
            output_hash,
        }
    }

    pub fn len(&self) -> usize {
        self.line_count
    }

    pub fn output_hash(&self) -> u64 {
        self.output_hash
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
                .map(|fragment| fragment.line_starts.len() * std::mem::size_of::<usize>())
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
        let conflict_ranges =
            self.conflict_line_ranges.len() * std::mem::size_of::<std::ops::Range<usize>>();
        fragments + spans + conflict_ranges
    }

    pub fn conflict_line_range(&self, conflict_ix: usize) -> Option<std::ops::Range<usize>> {
        self.conflict_line_ranges.get(conflict_ix).cloned()
    }

    pub fn conflict_line_ranges(&self) -> &[std::ops::Range<usize>] {
        self.conflict_line_ranges.as_slice()
    }

    pub fn line_text<'a>(
        &'a self,
        segments: &'a [ConflictSegment],
        line_ix: usize,
    ) -> Option<std::borrow::Cow<'a, str>> {
        let span_ix = self
            .spans
            .partition_point(|span| span.visible_start() <= line_ix)
            .checked_sub(1)?;
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

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn block_max_line_count(block: &ConflictBlock) -> usize {
    text_line_count_usize(block.base.as_deref().unwrap_or_default())
        .max(text_line_count_usize(&block.ours))
        .max(text_line_count_usize(&block.theirs))
}

#[cfg(any(test, feature = "benchmarks"))]
fn should_use_large_conflict_block_preview(block: &ConflictBlock) -> bool {
    block_max_line_count(block) > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES
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
    let whole_block_diff_ran = text_line_count_usize(old_text)
        > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES
        || text_line_count_usize(new_text) > LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES;
    debug_assert!(
        !whole_block_diff_ran,
        "bootstrap should not call side_by_side_rows on a giant conflict block"
    );
    let block_rows = gitcomet_core::file_diff::side_by_side_rows(old_text, new_text);
    for row in block_rows {
        rows.push(gitcomet_core::file_diff::FileDiffRow {
            kind: row.kind,
            old_line: row
                .old_line
                .map(|l| l.saturating_add(old_line_offset).saturating_sub(1)),
            new_line: row
                .new_line
                .map(|l| l.saturating_add(new_line_offset).saturating_sub(1)),
            old: row.old,
            new: row.new,
            eof_newline: row.eof_newline,
        });
    }
    whole_block_diff_ran
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
            old: Some(Arc::clone(&summary)),
            new: Some(summary),
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
    let mut rows = Vec::new();
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
fn push_block_local_boundary_context_rows(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    segments: &[ConflictSegment],
    segment_ix: usize,
    text: &str,
    old_line_start: u32,
    new_line_start: u32,
    context_lines: usize,
) -> u32 {
    let line_count = text_line_count(text);
    if text.is_empty() || context_lines == 0 {
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
        text.lines().enumerate().take(leading_count),
        old_line_start,
        new_line_start,
    );

    // Trailing context: find the byte offset of the trailing_start-th line
    // by scanning backwards from the end, avoiding a full-text forward scan.
    let effective_trailing_start = leading_count.max(trailing_start);
    if trailing_count > 0 && effective_trailing_start < line_count_usize {
        let bytes = text.as_bytes();
        // Find byte offset of the effective_trailing_start-th line by
        // reverse-scanning for the (line_count - effective_trailing_start)
        // newlines from the end.
        let lines_from_end = line_count_usize - effective_trailing_start;
        let byte_offset = byte_offset_of_nth_line_from_end(bytes, lines_from_end);
        push_block_local_context_lines(
            rows,
            text[byte_offset..]
                .lines()
                .enumerate()
                .map(move |(ix, line)| (effective_trailing_start + ix, line)),
            old_line_start,
            new_line_start,
        );
    }
    line_count
}

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
            old: Some(Arc::clone(&content)),
            new: Some(content),
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
}

impl ThreeWayVisibleSpan {
    fn visible_start(&self) -> usize {
        match *self {
            Self::Lines { visible_start, .. } => visible_start,
            Self::CollapsedResolvedBlock { visible_index, .. } => visible_index,
        }
    }

    fn visible_len(&self) -> usize {
        match *self {
            Self::Lines { len, .. } => len,
            Self::CollapsedResolvedBlock { .. } => 1,
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
}

/// Build a span-based visible projection for three-way views.
///
/// All lines in every conflict block are included (no preview gaps).
/// Resolved blocks collapse to a single summary row when `hide_resolved` is true.
pub fn build_three_way_visible_projection(
    total_lines: usize,
    conflict_ranges: &[std::ops::Range<usize>],
    segments: &[ConflictSegment],
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

    let resolved_blocks: Vec<bool> = segments
        .iter()
        .filter_map(|s| match s {
            ConflictSegment::Block(b) => Some(b.resolved),
            _ => None,
        })
        .collect();

    let mut spans: Vec<ThreeWayVisibleSpan> = Vec::new();
    let mut visible_ix = 0usize;
    let mut line_ix = 0usize;
    let mut range_ix = 0usize;

    // Helper: flush a pending lines run.
    let mut pending_lines_start: Option<usize> = None;

    let flush_pending = |spans: &mut Vec<ThreeWayVisibleSpan>,
                         pending: &mut Option<usize>,
                         vis: &mut usize,
                         line: usize| {
        if let Some(start) = pending.take() {
            let len = line - start;
            if len > 0 {
                spans.push(ThreeWayVisibleSpan::Lines {
                    visible_start: *vis - len,
                    source_line_start: start,
                    len,
                });
            }
        }
    };

    while line_ix < total_lines {
        // Advance range_ix past completed ranges.
        while let Some(range) = conflict_ranges.get(range_ix) {
            if range.end <= line_ix {
                range_ix += 1;
                continue;
            }
            break;
        }

        if let Some(range) = conflict_ranges.get(range_ix)
            && range.contains(&line_ix)
            && hide_resolved
            && resolved_blocks.get(range_ix).copied().unwrap_or(false)
        {
            flush_pending(
                &mut spans,
                &mut pending_lines_start,
                &mut visible_ix,
                line_ix,
            );
            spans.push(ThreeWayVisibleSpan::CollapsedResolvedBlock {
                visible_index: visible_ix,
                conflict_ix: range_ix,
            });
            visible_ix += 1;
            line_ix = range.end;
            continue;
        }

        // Normal line.
        if pending_lines_start.is_none() {
            pending_lines_start = Some(line_ix);
        }
        visible_ix += 1;
        line_ix += 1;
    }

    flush_pending(
        &mut spans,
        &mut pending_lines_start,
        &mut visible_ix,
        line_ix,
    );

    ThreeWayVisibleProjection {
        spans,
        visible_len: visible_ix,
    }
}

/// Build the mapping from visible row indices to actual three-way data items.
///
/// When `hide_resolved` is false, every line maps directly.
/// When true, resolved conflict ranges are collapsed to a single summary row.
#[cfg(any(test, feature = "benchmarks"))]
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
    let mut line_ix = 0usize;
    let mut range_ix = 0usize;

    while line_ix < total_lines {
        while let Some(range) = conflict_ranges.get(range_ix) {
            if range.end <= line_ix {
                range_ix += 1;
                continue;
            }
            break;
        }

        if let Some(range) = conflict_ranges.get(range_ix)
            && range.contains(&line_ix)
            && resolved_blocks.get(range_ix).copied().unwrap_or(false)
        {
            // Emit one collapsed summary row and skip the rest of the range.
            visible.push(ThreeWayVisibleItem::CollapsedBlock(range_ix));
            line_ix = range.end;
            continue;
        }

        visible.push(ThreeWayVisibleItem::Line(line_ix));
        line_ix += 1;
    }
    visible
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
    })
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
                block.base = Some(ancestor_text[prev_end..next_start].to_string().into());
            }
        }
    }
}

/// Check whether the given text still contains git conflict markers.
/// Used as a safety gate before "Save & stage" to warn the user about unresolved conflicts.
pub fn text_contains_conflict_markers(text: &str) -> bool {
    gitcomet_core::services::validate_conflict_resolution_text(text).has_conflict_markers
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

/// Split resolved output into one logical row per newline for outline rendering.
///
/// Uses `split('\n')` so trailing newlines are preserved as a final empty row.
pub fn split_output_lines_for_outline(output: &str) -> Vec<String> {
    output.split('\n').map(|line| line.to_string()).collect()
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
