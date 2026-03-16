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
    #[allow(dead_code)] // constructed only in tests, but matched in production
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

    #[cfg_attr(not(test), allow(dead_code))]
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
pub type WordHighlights = std::collections::HashMap<usize, Vec<std::ops::Range<usize>>>;

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
pub(crate) const LARGE_CONFLICT_BLOCK_PREVIEW_LINES: usize = 128;
/// Word-diff highlighting is optional chrome, so skip giant blocks entirely.
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn auto_resolve_segments_history(
    segments: &mut Vec<ConflictSegment>,
    options: &gitcomet_core::conflict_session::HistoryAutosolveOptions,
) -> usize {
    let mut block_region_indices = sequential_conflict_region_indices(segments);
    auto_resolve_segments_history_with_region_indices(segments, options, &mut block_region_indices)
}

/// Like [`auto_resolve_segments_history`] but keeps block->region mappings in sync.
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
#[cfg_attr(not(test), allow(dead_code))]
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
        (line_ix < self.line_starts.len())
            .then(|| line_text_from_starts(text, self.line_starts.as_ref(), line_ix))
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

        fn fragment_line_stats(text: &str) -> (std::sync::Arc<[usize]>, usize, bool) {
            let mut starts = Vec::new();
            starts.push(0usize);
            for (ix, byte) in text.as_bytes().iter().enumerate() {
                if *byte == b'\n' {
                    starts.push(ix.saturating_add(1));
                }
            }
            let newline_count = starts.len().saturating_sub(1);
            let ends_with_newline = text.as_bytes().last().copied() == Some(b'\n');
            (starts.into(), newline_count, ends_with_newline)
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
            spans: &mut Vec<ResolvedOutputSpan>,
            visible_line: &mut usize,
            conflict_ranges: &mut [Option<std::ops::Range<usize>>],
        ) {
            let line_conflict = pending.conflict_ix();
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

        fn push_fragment(
            fragments: &mut Vec<ResolvedOutputFragment>,
            source: ResolvedOutputFragmentSource,
            text: &str,
        ) -> Option<usize> {
            if text.is_empty() {
                return None;
            }
            let (line_starts, newline_count, ends_with_newline) = fragment_line_stats(text);
            let fragment_ix = fragments.len();
            fragments.push(ResolvedOutputFragment {
                source,
                line_starts,
                newline_count,
                ends_with_newline,
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
                            &mut spans,
                            &mut visible_line,
                            &mut conflict_ranges,
                        );
                        if fragment.newline_count > 1 {
                            push_source_span(
                                &mut spans,
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
                                &mut spans,
                                &mut visible_line,
                                &mut conflict_ranges,
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
            &mut spans,
            &mut visible_line,
            &mut conflict_ranges,
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
            output_hash,
        }
    }

    pub fn len(&self) -> usize {
        self.line_count
    }

    pub fn output_hash(&self) -> u64 {
        self.output_hash
    }

    /// Approximate heap bytes used by projection metadata, excluding the
    /// underlying segment texts which are shared with the resolver state.
    #[cfg_attr(not(test), allow(dead_code))]
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

fn block_max_line_count(block: &ConflictBlock) -> usize {
    text_line_count_usize(block.base.as_deref().unwrap_or_default())
        .max(text_line_count_usize(&block.ours))
        .max(text_line_count_usize(&block.theirs))
}

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

fn should_skip_large_block_word_highlights(block: &ConflictBlock) -> bool {
    block_max_line_count(block) > LARGE_CONFLICT_BLOCK_WORD_HIGHLIGHT_MAX_LINES
}

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
        let summary = format!(
            "... large conflict block preview omitted {omitted_ours} ours lines and {omitted_theirs} theirs lines ..."
        );
        rows.push(gitcomet_core::file_diff::FileDiffRow {
            kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
            old_line: (omitted_ours > 0).then(|| {
                ours_offset.saturating_add(u32::try_from(head_ours_end).unwrap_or(u32::MAX))
            }),
            new_line: (omitted_theirs > 0).then(|| {
                theirs_offset.saturating_add(u32::try_from(head_theirs_end).unwrap_or(u32::MAX))
            }),
            old: Some(summary.clone()),
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
pub fn block_local_two_way_diff_rows(
    segments: &[ConflictSegment],
) -> Vec<gitcomet_core::file_diff::FileDiffRow> {
    block_local_two_way_diff_rows_with_stats(segments).0
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BlockLocalTwoWayDiffStats {
    pub(crate) whole_block_diff_ran: bool,
}

pub(crate) fn block_local_two_way_diff_rows_with_stats(
    segments: &[ConflictSegment],
) -> (
    Vec<gitcomet_core::file_diff::FileDiffRow>,
    BlockLocalTwoWayDiffStats,
) {
    block_local_two_way_diff_rows_with_context_and_stats(segments, BLOCK_LOCAL_DIFF_CONTEXT_LINES)
}

#[cfg_attr(not(test), allow(dead_code))]
fn block_local_two_way_diff_rows_with_context(
    segments: &[ConflictSegment],
    context_lines: usize,
) -> Vec<gitcomet_core::file_diff::FileDiffRow> {
    block_local_two_way_diff_rows_with_context_and_stats(segments, context_lines).0
}

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

    push_block_local_context_lines(
        rows,
        text.lines().enumerate().take(leading_count),
        old_line_start,
        new_line_start,
    );
    push_block_local_context_lines(
        rows,
        text.lines()
            .enumerate()
            .skip(leading_count.max(trailing_start)),
        old_line_start,
        new_line_start,
    );
    line_count
}

fn push_block_local_context_lines<'a>(
    rows: &mut Vec<gitcomet_core::file_diff::FileDiffRow>,
    lines: impl Iterator<Item = (usize, &'a str)>,
    old_line_start: u32,
    new_line_start: u32,
) {
    use gitcomet_core::file_diff::{FileDiffRow, FileDiffRowKind};

    for (line_ix, text) in lines {
        let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
        let content = text.to_string();
        rows.push(FileDiffRow {
            kind: FileDiffRowKind::Context,
            old_line: Some(old_line_start.saturating_add(line_offset)),
            new_line: Some(new_line_start.saturating_add(line_offset)),
            old: Some(content.clone()),
            new: Some(content),
            eof_newline: None,
        });
    }
}

fn text_line_count(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
}

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
        0
    } else {
        text.lines().count()
    }
}

fn indexed_line_count(text: &str, line_starts: &[usize]) -> usize {
    if text.is_empty() {
        0
    } else {
        line_starts.len()
    }
}

fn indexed_line_text<'a>(text: &'a str, line_starts: &[usize], line_ix: usize) -> Option<&'a str> {
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

    /// Approximate heap bytes used by the projection metadata (spans vec).
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn metadata_byte_size(&self) -> usize {
        self.spans.len() * std::mem::size_of::<ThreeWayVisibleSpan>()
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

pub fn compute_three_way_word_highlights(
    base_text: &str,
    base_line_starts: &[usize],
    ours_text: &str,
    ours_line_starts: &[usize],
    theirs_text: &str,
    theirs_line_starts: &[usize],
    marker_segments: &[ConflictSegment],
) -> (WordHighlights, WordHighlights, WordHighlights) {
    let mut wh_base: WordHighlights = WordHighlights::new();
    let mut wh_ours: WordHighlights = WordHighlights::new();
    let mut wh_theirs: WordHighlights = WordHighlights::new();

    fn merge_line_ranges(
        highlights: &mut WordHighlights,
        line_ix: usize,
        ranges: Vec<std::ops::Range<usize>>,
    ) {
        if ranges.is_empty() {
            return;
        }
        highlights
            .entry(line_ix)
            .and_modify(|existing| {
                *existing = merge_ranges(existing, &ranges);
            })
            .or_insert(ranges);
    }

    fn line_index(start: usize, line_no: Option<u32>) -> Option<usize> {
        let local = usize::try_from(line_no?).ok()?.checked_sub(1)?;
        start.checked_add(local)
    }

    fn full_line_range(
        text: &str,
        line_starts: &[usize],
        line_ix: usize,
    ) -> Vec<std::ops::Range<usize>> {
        let Some(line) = indexed_line_text(text, line_starts, line_ix) else {
            return Vec::new();
        };
        if line.is_empty() {
            return Vec::new();
        }
        std::iter::once(0..line.len()).collect()
    }

    struct HighlightSide<'a> {
        global_start: usize,
        text: &'a str,
        line_starts: &'a [usize],
    }

    fn apply_aligned_word_highlights(
        old_text: &str,
        new_text: &str,
        old_side: HighlightSide<'_>,
        new_side: HighlightSide<'_>,
        old_highlights: &mut WordHighlights,
        new_highlights: &mut WordHighlights,
    ) {
        use gitcomet_core::file_diff::FileDiffRowKind;

        let rows = gitcomet_core::file_diff::side_by_side_rows(old_text, new_text);
        for row in rows {
            match row.kind {
                FileDiffRowKind::Modify => {
                    let old = row.old.as_deref().unwrap_or("");
                    let new = row.new.as_deref().unwrap_or("");
                    let (old_ranges, new_ranges) =
                        super::word_diff::capped_word_diff_ranges(old, new);

                    if let Some(ix) = line_index(old_side.global_start, row.old_line) {
                        merge_line_ranges(old_highlights, ix, old_ranges);
                    }
                    if let Some(ix) = line_index(new_side.global_start, row.new_line) {
                        merge_line_ranges(new_highlights, ix, new_ranges);
                    }
                }
                FileDiffRowKind::Remove => {
                    if let Some(ix) = line_index(old_side.global_start, row.old_line) {
                        merge_line_ranges(
                            old_highlights,
                            ix,
                            full_line_range(old_side.text, old_side.line_starts, ix),
                        );
                    }
                }
                FileDiffRowKind::Add => {
                    if let Some(ix) = line_index(new_side.global_start, row.new_line) {
                        merge_line_ranges(
                            new_highlights,
                            ix,
                            full_line_range(new_side.text, new_side.line_starts, ix),
                        );
                    }
                }
                FileDiffRowKind::Context => {}
            }
        }
    }

    let mut base_offset = 0usize;
    let mut ours_offset = 0usize;
    let mut theirs_offset = 0usize;
    for seg in marker_segments {
        match seg {
            ConflictSegment::Text(text) => {
                let n = usize::try_from(text_line_count(text)).unwrap_or(0);
                base_offset = base_offset.saturating_add(n);
                ours_offset = ours_offset.saturating_add(n);
                theirs_offset = theirs_offset.saturating_add(n);
            }
            ConflictSegment::Block(block) => {
                let base_count =
                    usize::try_from(text_line_count(block.base.as_deref().unwrap_or_default()))
                        .unwrap_or(0);
                let ours_count = usize::try_from(text_line_count(&block.ours)).unwrap_or(0);
                let theirs_count = usize::try_from(text_line_count(&block.theirs)).unwrap_or(0);
                if should_skip_large_block_word_highlights(block) {
                    base_offset = base_offset.saturating_add(base_count);
                    ours_offset = ours_offset.saturating_add(ours_count);
                    theirs_offset = theirs_offset.saturating_add(theirs_count);
                    continue;
                }

                if let Some(base) = block.base.as_deref() {
                    apply_aligned_word_highlights(
                        base,
                        &block.ours,
                        HighlightSide {
                            global_start: base_offset,
                            text: base_text,
                            line_starts: base_line_starts,
                        },
                        HighlightSide {
                            global_start: ours_offset,
                            text: ours_text,
                            line_starts: ours_line_starts,
                        },
                        &mut wh_base,
                        &mut wh_ours,
                    );
                    apply_aligned_word_highlights(
                        base,
                        &block.theirs,
                        HighlightSide {
                            global_start: base_offset,
                            text: base_text,
                            line_starts: base_line_starts,
                        },
                        HighlightSide {
                            global_start: theirs_offset,
                            text: theirs_text,
                            line_starts: theirs_line_starts,
                        },
                        &mut wh_base,
                        &mut wh_theirs,
                    );
                }
                // Local/Remote highlighting must align by diff rows, not absolute same-row index.
                apply_aligned_word_highlights(
                    &block.ours,
                    &block.theirs,
                    HighlightSide {
                        global_start: ours_offset,
                        text: ours_text,
                        line_starts: ours_line_starts,
                    },
                    HighlightSide {
                        global_start: theirs_offset,
                        text: theirs_text,
                        line_starts: theirs_line_starts,
                    },
                    &mut wh_ours,
                    &mut wh_theirs,
                );
                base_offset = base_offset.saturating_add(base_count);
                ours_offset = ours_offset.saturating_add(ours_count);
                theirs_offset = theirs_offset.saturating_add(theirs_count);
            }
        }
    }

    (wh_base, wh_ours, wh_theirs)
}

fn merge_ranges(
    a: &[std::ops::Range<usize>],
    b: &[std::ops::Range<usize>],
) -> Vec<std::ops::Range<usize>> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    let mut combined: Vec<std::ops::Range<usize>> = Vec::with_capacity(a.len() + b.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    combined.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<std::ops::Range<usize>> = Vec::with_capacity(combined.len());
    for r in combined {
        if let Some(last) = out.last_mut().filter(|l| r.start <= l.end) {
            last.end = last.end.max(r.end);
            continue;
        }
        out.push(r);
    }
    out
}

/// Per-line pair of (old, new) word-highlight ranges for two-way diff.
pub type TwoWayWordHighlights =
    Vec<Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)>>;

pub fn compute_two_way_word_highlights(
    diff_rows: &[gitcomet_core::file_diff::FileDiffRow],
) -> TwoWayWordHighlights {
    diff_rows
        .iter()
        .map(|row| {
            if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
                return None;
            }
            let old = row.old.as_deref().unwrap_or("");
            let new = row.new.as_deref().unwrap_or("");
            let (old_ranges, new_ranges) = super::word_diff::capped_word_diff_ranges(old, new);
            if old_ranges.is_empty() && new_ranges.is_empty() {
                None
            } else {
                Some((old_ranges, new_ranges))
            }
        })
        .collect()
}

/// Compute word-level highlights for a single `FileDiffRow` on the fly.
///
/// Used in giant/streamed mode where word highlights are not pre-computed for
/// all rows. Only produces highlights for `Modify` rows (both sides present,
/// text differs).
pub fn compute_word_highlights_for_row(
    row: &gitcomet_core::file_diff::FileDiffRow,
) -> Option<(Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>)> {
    if row.kind != gitcomet_core::file_diff::FileDiffRowKind::Modify {
        return None;
    }
    let old = row.old.as_deref().unwrap_or("");
    let new = row.new.as_deref().unwrap_or("");
    let (old_ranges, new_ranges) = super::word_diff::capped_word_diff_ranges(old, new);
    if old_ranges.is_empty() && new_ranges.is_empty() {
        None
    } else {
        Some((old_ranges, new_ranges))
    }
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

#[cfg_attr(not(test), allow(dead_code))]
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
pub struct SourceLines<'a> {
    pub a: &'a [gpui::SharedString],
    pub b: &'a [gpui::SharedString],
    pub c: &'a [gpui::SharedString],
}

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
#[cfg_attr(not(test), allow(dead_code))]
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

// ---------------------------------------------------------------------------
// Phase 3: Paged two-way split rows for giant conflict files
// ---------------------------------------------------------------------------

const CONFLICT_SPLIT_PAGE_SIZE: usize = 256;
const CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES: usize = 8;
const CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE: usize = CONFLICT_SPLIT_PAGE_SIZE;

/// Sparse line-start checkpoints for lazy row materialization.
///
/// Startup only needs line counts and occasional random access into the
/// visible window, so storing every line start for giant blocks is wasted
/// work.  Instead we keep one byte offset every N lines and rescan the small
/// local window from the nearest checkpoint when a page is requested.
#[derive(Clone, Debug, Default)]
struct SparseLineIndex {
    line_count: usize,
    checkpoints: Vec<usize>,
}

impl SparseLineIndex {
    fn for_text(text: &str) -> Self {
        if text.is_empty() {
            return Self::default();
        }

        let mut checkpoints = Vec::with_capacity(text.len().saturating_div(4096).saturating_add(1));
        checkpoints.push(0usize);
        let bytes = text.as_bytes();
        let mut line_count = 1usize;
        for (ix, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' && ix.saturating_add(1) < bytes.len() {
                if line_count % CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE == 0 {
                    checkpoints.push(ix.saturating_add(1));
                }
                line_count = line_count.saturating_add(1);
            }
        }

        Self {
            line_count,
            checkpoints,
        }
    }

    fn line_count(&self) -> usize {
        self.line_count
    }

    fn line_ranges(
        &self,
        text: &str,
        start_line_ix: usize,
        max_lines: usize,
    ) -> Vec<std::ops::Range<usize>> {
        if start_line_ix >= self.line_count || max_lines == 0 {
            return Vec::new();
        }

        let target_len = (self.line_count - start_line_ix).min(max_lines);
        let checkpoint_line = (start_line_ix / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE)
            * CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let checkpoint_ix = checkpoint_line / CONFLICT_SPLIT_LINE_CHECKPOINT_STRIDE;
        let Some(mut byte_ix) = self.checkpoints.get(checkpoint_ix).copied() else {
            return Vec::new();
        };

        let bytes = text.as_bytes();
        let mut current_line = checkpoint_line;
        let mut ranges = Vec::with_capacity(target_len);
        while current_line < self.line_count && byte_ix <= bytes.len() && ranges.len() < target_len
        {
            let line_start = byte_ix;
            while byte_ix < bytes.len() && bytes[byte_ix] != b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            let line_end = byte_ix;
            if byte_ix < bytes.len() && bytes[byte_ix] == b'\n' {
                byte_ix = byte_ix.saturating_add(1);
            }
            if current_line >= start_line_ix {
                ranges.push(line_start..line_end);
            }
            current_line = current_line.saturating_add(1);
        }
        ranges
    }

    fn line_text<'a>(&self, text: &'a str, line_ix: usize) -> Option<&'a str> {
        let range = self.line_ranges(text, line_ix, 1).into_iter().next()?;
        text.get(range)
    }
}

/// Pre-computed segment layout entry for lazy two-way split row generation.
#[derive(Clone, Debug)]
enum SplitLayoutKind {
    /// Boundary context lines from a `Text` segment.
    Context {
        line_index: SparseLineIndex,
        /// Number of leading context rows included from the start of the text.
        leading_row_count: usize,
        /// Source line index where the trailing context window begins.
        trailing_row_start: usize,
        /// 1-based starting ours line number.
        ours_start_line: u32,
        /// 1-based starting theirs line number.
        theirs_start_line: u32,
    },
    /// Plain split rows from a conflict block.
    Block {
        ours_line_index: SparseLineIndex,
        theirs_line_index: SparseLineIndex,
        ours_start_line: u32,
        theirs_start_line: u32,
    },
}

#[derive(Clone, Debug)]
struct SplitLayoutEntry {
    /// First row index in the flat row space.
    row_start: usize,
    /// Number of rows this entry contributes.
    row_count: usize,
    /// Index into the original `marker_segments` slice.
    segment_ix: usize,
    /// Conflict index (for block entries only).
    conflict_ix: Option<usize>,
    kind: SplitLayoutKind,
}

#[allow(dead_code)]
const CONFLICT_ANCHOR_MIN_GAP_LINES: u32 = 64;
#[allow(dead_code)]
const CONFLICT_ANCHOR_CHUNK_LINE_WIDTH: usize = 3;

/// Maximum gap size (per side) for which a bounded local diff is computed
/// to improve alignment quality within anchor gaps.  Gaps larger than this
/// fall back to simple positional pairing.
#[allow(dead_code)]
const CONFLICT_GAP_DIFF_MAX_LINES: usize = 512;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
struct LineFingerprint {
    hash: u64,
    len: u32,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
struct UniqueLineLocation {
    line_ix: u32,
    count: u32,
}

/// Local diff alignment for one gap between consecutive anchors.
///
/// Each entry maps a gap-local row to `(Option<ours_offset>, Option<theirs_offset>)`
/// where offsets are 0-based within the gap.  Row count may exceed
/// `max(ours_gap, theirs_gap)` because the diff can split Delete+Insert runs
/// into separate rows.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct GapDiffAlignment {
    /// Per-row alignment: (ours 0-based offset in gap, theirs 0-based offset in gap).
    rows: Vec<(Option<u16>, Option<u16>)>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ConflictAnchorIndex {
    pub ours_to_theirs: Vec<(u32, u32)>,
    anchor_row_starts: Vec<usize>,
    row_count: usize,
    /// Per-gap local diff alignments.  Index `i` corresponds to gap `i`
    /// (gap 0 = before first anchor, gap N = after last anchor).
    /// `None` entries fall back to positional pairing.
    gap_alignments: Vec<Option<GapDiffAlignment>>,
}

#[allow(dead_code)]
impl ConflictAnchorIndex {
    /// Public entry point for benchmark/test use only.
    #[cfg(test)]
    pub fn build_for_benchmark(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_count: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_count: usize,
    ) -> Self {
        Self::build(
            ours_text,
            ours_line_starts,
            ours_count,
            theirs_text,
            theirs_line_starts,
            theirs_count,
        )
    }

    fn build(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_count: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_count: usize,
    ) -> Self {
        let ours_to_theirs = Self::build_anchor_pairs(
            ours_text,
            ours_line_starts,
            ours_count,
            theirs_text,
            theirs_line_starts,
            theirs_count,
        );
        let gap_count = ours_to_theirs.len().saturating_add(1);
        let mut anchor_row_starts = Vec::with_capacity(ours_to_theirs.len());
        let mut gap_alignments: Vec<Option<GapDiffAlignment>> = Vec::with_capacity(gap_count);
        let mut row_count = 0usize;
        let mut prev_ours = 0usize;
        let mut prev_theirs = 0usize;

        for &(ours_line_ix, theirs_line_ix) in &ours_to_theirs {
            let ours_line_ix = usize::try_from(ours_line_ix)
                .unwrap_or(usize::MAX)
                .min(ours_count);
            let theirs_line_ix = usize::try_from(theirs_line_ix)
                .unwrap_or(usize::MAX)
                .min(theirs_count);
            let ours_gap = ours_line_ix.saturating_sub(prev_ours);
            let theirs_gap = theirs_line_ix.saturating_sub(prev_theirs);

            let gap_rows = Self::compute_gap(
                ours_text,
                ours_line_starts,
                prev_ours,
                ours_gap,
                theirs_text,
                theirs_line_starts,
                prev_theirs,
                theirs_gap,
                &mut gap_alignments,
            );
            row_count = row_count.saturating_add(gap_rows);
            anchor_row_starts.push(row_count);
            row_count = row_count.saturating_add(1);
            prev_ours = ours_line_ix.saturating_add(1);
            prev_theirs = theirs_line_ix.saturating_add(1);
        }

        // Trailing gap after last anchor.
        let trailing_ours = ours_count.saturating_sub(prev_ours);
        let trailing_theirs = theirs_count.saturating_sub(prev_theirs);
        let trailing_gap_rows = Self::compute_gap(
            ours_text,
            ours_line_starts,
            prev_ours,
            trailing_ours,
            theirs_text,
            theirs_line_starts,
            prev_theirs,
            trailing_theirs,
            &mut gap_alignments,
        );
        row_count = row_count.saturating_add(trailing_gap_rows);

        Self {
            ours_to_theirs,
            anchor_row_starts,
            row_count,
            gap_alignments,
        }
    }

    /// Compute alignment for a single gap, push to `gap_alignments`, return row count.
    fn compute_gap(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_start: usize,
        ours_gap: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_start: usize,
        theirs_gap: usize,
        gap_alignments: &mut Vec<Option<GapDiffAlignment>>,
    ) -> usize {
        // Both sides must be non-empty and within the size threshold for a diff
        // to improve over positional pairing.
        if ours_gap > 0
            && theirs_gap > 0
            && ours_gap <= CONFLICT_GAP_DIFF_MAX_LINES
            && theirs_gap <= CONFLICT_GAP_DIFF_MAX_LINES
        {
            let ours_slice = gap_text_slice(ours_text, ours_line_starts, ours_start, ours_gap);
            let theirs_slice =
                gap_text_slice(theirs_text, theirs_line_starts, theirs_start, theirs_gap);
            let diff_rows = gitcomet_core::file_diff::side_by_side_rows(ours_slice, theirs_slice);
            // Only use the diff alignment if it found at least one matching
            // line (Context row).  When all lines differ, positional pairing
            // is simpler and equally valid.
            let has_context = diff_rows
                .iter()
                .any(|r| r.kind == gitcomet_core::file_diff::FileDiffRowKind::Context);
            if has_context {
                let alignment = GapDiffAlignment {
                    rows: diff_rows
                        .iter()
                        .map(|r| {
                            (
                                r.old_line.map(|n| n.saturating_sub(1) as u16),
                                r.new_line.map(|n| n.saturating_sub(1) as u16),
                            )
                        })
                        .collect(),
                };
                let row_count = alignment.rows.len();
                gap_alignments.push(Some(alignment));
                row_count
            } else {
                gap_alignments.push(None);
                ours_gap.max(theirs_gap)
            }
        } else {
            gap_alignments.push(None);
            ours_gap.max(theirs_gap)
        }
    }

    fn row_count(&self) -> usize {
        self.row_count
    }

    fn mapped_lines(
        &self,
        row_ix: usize,
        ours_count: usize,
        theirs_count: usize,
    ) -> Option<(Option<usize>, Option<usize>)> {
        if row_ix >= self.row_count {
            return None;
        }

        let anchor_pos = self
            .anchor_row_starts
            .partition_point(|&start| start <= row_ix);
        if let Some(anchor_ix) = anchor_pos.checked_sub(1)
            && self.anchor_row_starts.get(anchor_ix).copied() == Some(row_ix)
        {
            let &(ours_line_ix, theirs_line_ix) = self.ours_to_theirs.get(anchor_ix)?;
            return Some((
                Some(
                    usize::try_from(ours_line_ix)
                        .unwrap_or(usize::MAX)
                        .min(ours_count),
                ),
                Some(
                    usize::try_from(theirs_line_ix)
                        .unwrap_or(usize::MAX)
                        .min(theirs_count),
                ),
            ));
        }

        let (prev_ours, prev_theirs, gap_row_start, gap_ix) =
            if let Some(prev_anchor_ix) = anchor_pos.checked_sub(1) {
                let &(prev_ours, prev_theirs) = self.ours_to_theirs.get(prev_anchor_ix)?;
                (
                    usize::try_from(prev_ours)
                        .unwrap_or(usize::MAX)
                        .min(ours_count)
                        .saturating_add(1),
                    usize::try_from(prev_theirs)
                        .unwrap_or(usize::MAX)
                        .min(theirs_count)
                        .saturating_add(1),
                    self.anchor_row_starts[prev_anchor_ix].saturating_add(1),
                    // Gap after anchor `prev_anchor_ix` is gap index `prev_anchor_ix + 1`.
                    prev_anchor_ix.saturating_add(1),
                )
            } else {
                (0, 0, 0, 0)
            };
        let local_row_ix = row_ix.saturating_sub(gap_row_start);

        // If we have a diff-based alignment for this gap, use it.
        if let Some(Some(alignment)) = self.gap_alignments.get(gap_ix) {
            let &(ours_offset, theirs_offset) = alignment.rows.get(local_row_ix)?;
            return Some((
                ours_offset.map(|o| prev_ours.saturating_add(usize::from(o))),
                theirs_offset.map(|t| prev_theirs.saturating_add(usize::from(t))),
            ));
        }

        // Fallback: positional pairing.
        let (next_ours, next_theirs) = self
            .ours_to_theirs
            .get(anchor_pos)
            .map(|&(o, t)| {
                (
                    usize::try_from(o).unwrap_or(usize::MAX).min(ours_count),
                    usize::try_from(t).unwrap_or(usize::MAX).min(theirs_count),
                )
            })
            .unwrap_or((ours_count, theirs_count));

        let ours_gap = next_ours.saturating_sub(prev_ours);
        let theirs_gap = next_theirs.saturating_sub(prev_theirs);
        let gap_rows = ours_gap.max(theirs_gap);
        if local_row_ix >= gap_rows {
            return None;
        }

        Some((
            (local_row_ix < ours_gap).then_some(prev_ours.saturating_add(local_row_ix)),
            (local_row_ix < theirs_gap).then_some(prev_theirs.saturating_add(local_row_ix)),
        ))
    }

    fn build_anchor_pairs(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_count: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_count: usize,
    ) -> Vec<(u32, u32)> {
        if ours_count == 0 || theirs_count == 0 {
            return Vec::new();
        }

        let mut candidates = Self::unique_line_anchor_candidates(
            ours_text,
            ours_line_starts,
            ours_count,
            theirs_text,
            theirs_line_starts,
            theirs_count,
        );
        Self::extend_unique_chunk_anchor_candidates(
            ours_text,
            ours_line_starts,
            ours_count,
            theirs_text,
            theirs_line_starts,
            theirs_count,
            CONFLICT_ANCHOR_CHUNK_LINE_WIDTH,
            &mut candidates,
        );
        candidates
            .sort_unstable_by_key(|&(ours_line_ix, theirs_line_ix)| (ours_line_ix, theirs_line_ix));
        candidates.dedup();
        let anchors = Self::longest_increasing_anchor_pairs(&candidates);
        Self::sparsify_anchor_pairs(&anchors)
    }

    fn unique_line_anchor_candidates(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_count: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_count: usize,
    ) -> Vec<(u32, u32)> {
        let ours_unique = Self::unique_line_locations(ours_text, ours_line_starts, ours_count);
        let theirs_unique =
            Self::unique_line_locations(theirs_text, theirs_line_starts, theirs_count);
        let mut candidates = Vec::new();

        for (fingerprint, ours_loc) in &ours_unique {
            if ours_loc.count != 1 {
                continue;
            }
            let Some(theirs_loc) = theirs_unique.get(fingerprint) else {
                continue;
            };
            if theirs_loc.count != 1 {
                continue;
            }

            let ours_line = line_text_from_starts(
                ours_text,
                ours_line_starts,
                usize::try_from(ours_loc.line_ix).unwrap_or(usize::MAX),
            );
            let theirs_line = line_text_from_starts(
                theirs_text,
                theirs_line_starts,
                usize::try_from(theirs_loc.line_ix).unwrap_or(usize::MAX),
            );
            if ours_line == theirs_line {
                candidates.push((ours_loc.line_ix, theirs_loc.line_ix));
            }
        }

        candidates
    }

    fn extend_unique_chunk_anchor_candidates(
        ours_text: &str,
        ours_line_starts: &[usize],
        ours_count: usize,
        theirs_text: &str,
        theirs_line_starts: &[usize],
        theirs_count: usize,
        chunk_width: usize,
        candidates: &mut Vec<(u32, u32)>,
    ) {
        if chunk_width <= 1 || ours_count < chunk_width || theirs_count < chunk_width {
            return;
        }

        let ours_unique =
            Self::unique_chunk_locations(ours_text, ours_line_starts, ours_count, chunk_width);
        let theirs_unique = Self::unique_chunk_locations(
            theirs_text,
            theirs_line_starts,
            theirs_count,
            chunk_width,
        );

        for (fingerprint, ours_loc) in &ours_unique {
            if ours_loc.count != 1 {
                continue;
            }
            let Some(theirs_loc) = theirs_unique.get(fingerprint) else {
                continue;
            };
            if theirs_loc.count != 1 {
                continue;
            }

            let ours_start = usize::try_from(ours_loc.line_ix).unwrap_or(usize::MAX);
            let theirs_start = usize::try_from(theirs_loc.line_ix).unwrap_or(usize::MAX);
            let ours_chunk = line_slice_text(
                ours_text,
                ours_line_starts,
                ours_count,
                ours_start,
                ours_start.saturating_add(chunk_width),
            );
            let theirs_chunk = line_slice_text(
                theirs_text,
                theirs_line_starts,
                theirs_count,
                theirs_start,
                theirs_start.saturating_add(chunk_width),
            );
            if ours_chunk == theirs_chunk {
                candidates.push((ours_loc.line_ix, theirs_loc.line_ix));
            }
        }
    }

    fn unique_line_locations(
        text: &str,
        line_starts: &[usize],
        line_count: usize,
    ) -> rustc_hash::FxHashMap<LineFingerprint, UniqueLineLocation> {
        let mut locations = rustc_hash::FxHashMap::default();
        for line_ix in 0..line_count {
            let fingerprint =
                Self::line_fingerprint(line_text_from_starts(text, line_starts, line_ix));
            match locations.entry(fingerprint) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(UniqueLineLocation {
                        line_ix: u32::try_from(line_ix).unwrap_or(u32::MAX),
                        count: 1,
                    });
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().count = entry.get().count.saturating_add(1);
                }
            }
        }
        locations
    }

    fn unique_chunk_locations(
        text: &str,
        line_starts: &[usize],
        line_count: usize,
        chunk_width: usize,
    ) -> rustc_hash::FxHashMap<LineFingerprint, UniqueLineLocation> {
        let mut locations = rustc_hash::FxHashMap::default();
        if chunk_width == 0 || line_count < chunk_width {
            return locations;
        }

        let last_start = line_count.saturating_sub(chunk_width);
        for line_ix in 0..=last_start {
            let fingerprint =
                Self::chunk_fingerprint(text, line_starts, line_count, line_ix, chunk_width);
            match locations.entry(fingerprint) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(UniqueLineLocation {
                        line_ix: u32::try_from(line_ix).unwrap_or(u32::MAX),
                        count: 1,
                    });
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().count = entry.get().count.saturating_add(1);
                }
            }
        }
        locations
    }

    fn line_fingerprint(line: &str) -> LineFingerprint {
        use std::hash::{Hash, Hasher};

        let mut hasher = rustc_hash::FxHasher::default();
        line.hash(&mut hasher);
        LineFingerprint {
            hash: hasher.finish(),
            len: u32::try_from(line.len()).unwrap_or(u32::MAX),
        }
    }

    fn chunk_fingerprint(
        text: &str,
        line_starts: &[usize],
        line_count: usize,
        start_line_ix: usize,
        chunk_width: usize,
    ) -> LineFingerprint {
        Self::line_fingerprint(line_slice_text(
            text,
            line_starts,
            line_count,
            start_line_ix,
            start_line_ix.saturating_add(chunk_width),
        ))
    }

    fn longest_increasing_anchor_pairs(candidates: &[(u32, u32)]) -> Vec<(u32, u32)> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut tails: Vec<usize> = Vec::new();
        let mut prev: Vec<Option<usize>> = vec![None; candidates.len()];

        for (ix, &(_, theirs_line_ix)) in candidates.iter().enumerate() {
            let pos =
                tails.partition_point(|&candidate_ix| candidates[candidate_ix].1 < theirs_line_ix);
            if pos > 0 {
                prev[ix] = tails.get(pos.saturating_sub(1)).copied();
            }
            if pos == tails.len() {
                tails.push(ix);
            } else {
                tails[pos] = ix;
            }
        }

        let mut anchors = Vec::with_capacity(tails.len());
        let mut cursor = tails.last().copied();
        while let Some(ix) = cursor {
            anchors.push(candidates[ix]);
            cursor = prev[ix];
        }
        anchors.reverse();
        anchors
    }

    fn sparsify_anchor_pairs(anchors: &[(u32, u32)]) -> Vec<(u32, u32)> {
        if anchors.len() <= 2
            || anchors.len() <= usize::try_from(CONFLICT_ANCHOR_MIN_GAP_LINES).unwrap_or(usize::MAX)
        {
            return anchors.to_vec();
        }

        let last_ix = anchors.len().saturating_sub(1);
        let mut out = Vec::with_capacity(anchors.len());
        let mut last_kept: Option<(u32, u32)> = None;

        for (ix, &anchor) in anchors.iter().enumerate() {
            let keep = match last_kept {
                None => true,
                Some((last_ours, last_theirs)) => {
                    ix == last_ix
                        || anchor.0.saturating_sub(last_ours) >= CONFLICT_ANCHOR_MIN_GAP_LINES
                        || anchor.1.saturating_sub(last_theirs) >= CONFLICT_ANCHOR_MIN_GAP_LINES
                }
            };
            if keep {
                out.push(anchor);
                last_kept = Some(anchor);
            }
        }

        out
    }

    /// Approximate heap bytes used by the anchor index metadata.
    #[cfg(test)]
    pub fn metadata_byte_size(&self) -> usize {
        let anchors = self.ours_to_theirs.len() * std::mem::size_of::<(u32, u32)>();
        let row_starts = self.anchor_row_starts.len() * std::mem::size_of::<usize>();
        let gap_vec = self.gap_alignments.len() * std::mem::size_of::<Option<GapDiffAlignment>>();
        let gap_rows: usize = self
            .gap_alignments
            .iter()
            .filter_map(|g| g.as_ref())
            .map(|g| g.rows.len() * std::mem::size_of::<(Option<u16>, Option<u16>)>())
            .sum();
        anchors + row_starts + gap_vec + gap_rows
    }
}

/// Extract a contiguous range of lines as a `&str` from `text` using `line_starts`.
#[allow(dead_code)]
fn gap_text_slice<'a>(
    text: &'a str,
    line_starts: &[usize],
    start_line: usize,
    line_count: usize,
) -> &'a str {
    if line_count == 0 || text.is_empty() {
        return "";
    }
    let text_len = text.len();
    let start = line_starts
        .get(start_line)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let end = line_starts
        .get(start_line.saturating_add(line_count))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    text.get(start..end).unwrap_or("")
}

#[derive(Debug, Default)]
struct ConflictSplitPageCache {
    pages:
        std::collections::HashMap<usize, std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]>>,
    lru: std::collections::VecDeque<usize>,
}

impl ConflictSplitPageCache {
    fn touch(&mut self, page_ix: usize) {
        if let Some(pos) = self.lru.iter().position(|&cached_ix| cached_ix == page_ix) {
            self.lru.remove(pos);
        }
        self.lru.push_back(page_ix);
    }

    fn get(
        &mut self,
        page_ix: usize,
    ) -> Option<std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        let page = self.pages.get(&page_ix).cloned()?;
        self.touch(page_ix);
        Some(page)
    }

    fn insert(
        &mut self,
        page_ix: usize,
        page: std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]>,
    ) -> std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]> {
        self.pages.insert(page_ix, std::sync::Arc::clone(&page));
        self.touch(page_ix);
        while self.pages.len() > CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES {
            if let Some(evicted) = self.lru.pop_front() {
                self.pages.remove(&evicted);
            }
        }
        page
    }

    fn clear(&mut self) {
        self.pages.clear();
        self.lru.clear();
    }
}

/// Pre-computed index for lazy two-way split row access in giant mode.
///
/// Instead of eagerly building all `FileDiffRow` objects for every conflict block,
/// this stores compact per-segment metadata and generates rows on demand.
#[derive(Clone, Debug)]
pub struct ConflictSplitRowIndex {
    entries: Vec<SplitLayoutEntry>,
    total_rows: usize,
    page_size: usize,
    pages: std::sync::Arc<std::sync::Mutex<ConflictSplitPageCache>>,
}

impl Default for ConflictSplitRowIndex {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            total_rows: 0,
            page_size: CONFLICT_SPLIT_PAGE_SIZE,
            pages: std::sync::Arc::new(std::sync::Mutex::new(ConflictSplitPageCache::default())),
        }
    }
}

impl ConflictSplitRowIndex {
    /// Build the layout from conflict segments.
    pub fn new(segments: &[ConflictSegment], context_lines: usize) -> Self {
        let mut entries = Vec::new();
        let mut total_rows = 0usize;
        let mut ours_line = 1u32;
        let mut theirs_line = 1u32;
        let mut conflict_ix = 0usize;

        for (segment_ix, segment) in segments.iter().enumerate() {
            match segment {
                ConflictSegment::Text(text) => {
                    let line_index = SparseLineIndex::for_text(text);
                    let line_count_usize = line_index.line_count();
                    let line_count = u32::try_from(line_count_usize).unwrap_or(u32::MAX);

                    let has_prev_block = segment_ix > 0
                        && matches!(
                            segments.get(segment_ix - 1),
                            Some(ConflictSegment::Block(_))
                        );
                    let has_next_block = matches!(
                        segments.get(segment_ix + 1),
                        Some(ConflictSegment::Block(_))
                    );

                    let leading = if has_prev_block {
                        context_lines.min(line_count_usize)
                    } else {
                        0
                    };
                    let trailing = if has_next_block {
                        context_lines.min(line_count_usize)
                    } else {
                        0
                    };
                    let trailing_row_start = leading.max(line_count_usize.saturating_sub(trailing));
                    let row_count =
                        leading.saturating_add(line_count_usize.saturating_sub(trailing_row_start));

                    if row_count > 0 {
                        entries.push(SplitLayoutEntry {
                            row_start: total_rows,
                            row_count,
                            segment_ix,
                            conflict_ix: None,
                            kind: SplitLayoutKind::Context {
                                line_index,
                                leading_row_count: leading,
                                trailing_row_start,
                                ours_start_line: ours_line,
                                theirs_start_line: theirs_line,
                            },
                        });
                        total_rows += row_count;
                    }

                    ours_line = ours_line.saturating_add(line_count);
                    theirs_line = theirs_line.saturating_add(line_count);
                }
                ConflictSegment::Block(block) => {
                    let ours_line_index = SparseLineIndex::for_text(&block.ours);
                    let theirs_line_index = SparseLineIndex::for_text(&block.theirs);
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    let row_count = ours_count.max(theirs_count);

                    entries.push(SplitLayoutEntry {
                        row_start: total_rows,
                        row_count,
                        segment_ix,
                        conflict_ix: Some(conflict_ix),
                        kind: SplitLayoutKind::Block {
                            ours_line_index,
                            theirs_line_index,
                            ours_start_line: ours_line,
                            theirs_start_line: theirs_line,
                        },
                    });
                    total_rows += row_count;

                    let ours_count_u32 = u32::try_from(ours_count).unwrap_or(u32::MAX);
                    let theirs_count_u32 = u32::try_from(theirs_count).unwrap_or(u32::MAX);
                    ours_line = ours_line.saturating_add(ours_count_u32);
                    theirs_line = theirs_line.saturating_add(theirs_count_u32);
                    conflict_ix += 1;
                }
            }
        }

        Self {
            entries,
            total_rows,
            page_size: CONFLICT_SPLIT_PAGE_SIZE,
            pages: std::sync::Arc::new(std::sync::Mutex::new(ConflictSplitPageCache::default())),
        }
    }

    /// Total number of rows across all segments (before visibility filtering).
    pub fn total_rows(&self) -> usize {
        self.total_rows
    }

    fn page_bounds(&self, page_ix: usize) -> Option<(usize, usize)> {
        let start = page_ix.saturating_mul(self.page_size);
        (start < self.total_rows).then(|| {
            let end = start.saturating_add(self.page_size).min(self.total_rows);
            (start, end)
        })
    }

    /// Find the layout entry that contains `row_ix`.
    fn entry_for_row(&self, row_ix: usize) -> Option<(usize, &SplitLayoutEntry)> {
        if row_ix >= self.total_rows {
            return None;
        }
        // Binary search: find the last entry where row_start <= row_ix.
        let pos = self
            .entries
            .partition_point(|e| e.row_start <= row_ix)
            .saturating_sub(1);
        let entry = self.entries.get(pos)?;
        if row_ix >= entry.row_start && row_ix < entry.row_start + entry.row_count {
            Some((pos, entry))
        } else {
            None
        }
    }

    fn build_page(
        &self,
        segments: &[ConflictSegment],
        page_ix: usize,
    ) -> Option<std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        let (start, end) = self.page_bounds(page_ix)?;
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        let mut row_ix = start;
        while row_ix < end {
            let (_, entry) = self.entry_for_row(row_ix)?;
            let entry_row_end = (entry.row_start + entry.row_count).min(end);
            let local_start = row_ix.saturating_sub(entry.row_start);
            let local_end = entry_row_end.saturating_sub(entry.row_start);
            let segment = segments.get(entry.segment_ix)?;

            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ours_start_line,
                        theirs_start_line,
                    },
                    ConflictSegment::Text(text),
                ) => {
                    let leading_end = local_end.min(*leading_row_count);
                    if local_start < leading_end {
                        let line_ranges =
                            line_index.line_ranges(text, local_start, leading_end - local_start);
                        for (offset, range) in line_ranges.into_iter().enumerate() {
                            let line_ix = local_start.saturating_add(offset);
                            let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
                            let content = text.get(range).unwrap_or("");
                            rows.push(gitcomet_core::file_diff::FileDiffRow {
                                kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                                old_line: Some(ours_start_line.saturating_add(line_offset)),
                                new_line: Some(theirs_start_line.saturating_add(line_offset)),
                                old: Some(content.to_string()),
                                new: Some(content.to_string()),
                                eof_newline: None,
                            });
                        }
                    }

                    let trailing_local_start = local_start.max(*leading_row_count);
                    if trailing_local_start < local_end {
                        let trailing_line_start = trailing_row_start.saturating_add(
                            trailing_local_start.saturating_sub(*leading_row_count),
                        );
                        let line_ranges = line_index.line_ranges(
                            text,
                            trailing_line_start,
                            local_end - trailing_local_start,
                        );
                        for (offset, range) in line_ranges.into_iter().enumerate() {
                            let line_ix = trailing_line_start.saturating_add(offset);
                            let line_offset = u32::try_from(line_ix).unwrap_or(u32::MAX);
                            let content = text.get(range).unwrap_or("");
                            rows.push(gitcomet_core::file_diff::FileDiffRow {
                                kind: gitcomet_core::file_diff::FileDiffRowKind::Context,
                                old_line: Some(ours_start_line.saturating_add(line_offset)),
                                new_line: Some(theirs_start_line.saturating_add(line_offset)),
                                old: Some(content.to_string()),
                                new: Some(content.to_string()),
                                eof_newline: None,
                            });
                        }
                    }
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ours_start_line,
                        theirs_start_line,
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let row_count = local_end.saturating_sub(local_start);
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    let ours_ranges = if local_start < ours_count {
                        ours_line_index.line_ranges(
                            &block.ours,
                            local_start,
                            row_count.min(ours_count - local_start),
                        )
                    } else {
                        Vec::new()
                    };
                    let theirs_ranges = if local_start < theirs_count {
                        theirs_line_index.line_ranges(
                            &block.theirs,
                            local_start,
                            row_count.min(theirs_count - local_start),
                        )
                    } else {
                        Vec::new()
                    };

                    for offset in 0..row_count {
                        let source_line_ix = local_start.saturating_add(offset);
                        let old_line = (source_line_ix < ours_count).then(|| {
                            ours_start_line
                                .saturating_add(u32::try_from(source_line_ix).unwrap_or(u32::MAX))
                        });
                        let new_line = (source_line_ix < theirs_count).then(|| {
                            theirs_start_line
                                .saturating_add(u32::try_from(source_line_ix).unwrap_or(u32::MAX))
                        });
                        let old_text = ours_ranges
                            .get(offset)
                            .and_then(|range: &std::ops::Range<usize>| {
                                block.ours.get(range.clone())
                            })
                            .map(ToString::to_string);
                        let new_text = theirs_ranges
                            .get(offset)
                            .and_then(|range: &std::ops::Range<usize>| {
                                block.theirs.get(range.clone())
                            })
                            .map(ToString::to_string);

                        let kind = match (old_text.as_deref(), new_text.as_deref()) {
                            (Some(old), Some(new)) if old == new => {
                                gitcomet_core::file_diff::FileDiffRowKind::Context
                            }
                            (Some(_), Some(_)) => gitcomet_core::file_diff::FileDiffRowKind::Modify,
                            (Some(_), None) => gitcomet_core::file_diff::FileDiffRowKind::Remove,
                            (None, Some(_)) => gitcomet_core::file_diff::FileDiffRowKind::Add,
                            (None, None) => continue,
                        };

                        rows.push(gitcomet_core::file_diff::FileDiffRow {
                            kind,
                            old_line,
                            new_line,
                            old: old_text,
                            new: new_text,
                            eof_newline: None,
                        });
                    }
                }
                _ => return None,
            }

            row_ix = entry_row_end;
        }
        Some(std::sync::Arc::from(rows))
    }

    fn load_page(
        &self,
        segments: &[ConflictSegment],
        page_ix: usize,
    ) -> Option<std::sync::Arc<[gitcomet_core::file_diff::FileDiffRow]>> {
        if let Ok(mut pages) = self.pages.lock()
            && let Some(page) = pages.get(page_ix)
        {
            return Some(page);
        }

        let page = self.build_page(segments, page_ix)?;
        if let Ok(mut pages) = self.pages.lock() {
            return Some(pages.insert(page_ix, page));
        }
        Some(page)
    }

    /// Generate a single `FileDiffRow` on demand from segment text.
    pub fn row_at(
        &self,
        segments: &[ConflictSegment],
        row_ix: usize,
    ) -> Option<gitcomet_core::file_diff::FileDiffRow> {
        if row_ix >= self.total_rows {
            return None;
        }
        let page_ix = row_ix / self.page_size;
        let row_offset = row_ix % self.page_size;
        let page = self.load_page(segments, page_ix)?;
        page.get(row_offset).cloned()
    }

    pub(in crate::view) fn clear_cached_pages(&self) {
        if let Ok(mut pages) = self.pages.lock() {
            pages.clear();
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn cached_page_count(&self) -> usize {
        self.pages
            .lock()
            .map(|pages| pages.pages.len())
            .unwrap_or(0)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::view) fn cached_page_indices(&self) -> Vec<usize> {
        let mut pages = self
            .pages
            .lock()
            .map(|pages| pages.pages.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        pages.sort_unstable();
        pages
    }

    /// Approximate heap bytes used by the index metadata,
    /// excluding the bounded page cache.
    #[cfg(test)]
    pub fn metadata_byte_size(&self) -> usize {
        let entry_overhead = self.entries.len() * std::mem::size_of::<SplitLayoutEntry>();
        let entry_vecs: usize = self
            .entries
            .iter()
            .map(|e| match &e.kind {
                SplitLayoutKind::Context { line_index, .. } => {
                    line_index.checkpoints.len() * std::mem::size_of::<usize>()
                }
                SplitLayoutKind::Block {
                    ours_line_index,
                    theirs_line_index,
                    ..
                } => {
                    ours_line_index.checkpoints.len() * std::mem::size_of::<usize>()
                        + theirs_line_index.checkpoints.len() * std::mem::size_of::<usize>()
                }
            })
            .sum();
        entry_overhead + entry_vecs
    }

    /// Look up the conflict index for a given source row.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn conflict_ix_for_row(&self, row_ix: usize) -> Option<usize> {
        let (_, entry) = self.entry_for_row(row_ix)?;
        entry.conflict_ix
    }

    /// Find the first source row index belonging to a conflict block.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn first_row_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        self.entries
            .iter()
            .find(|e| e.conflict_ix == Some(conflict_ix))
            .map(|e| e.row_start)
    }

    /// Find all source row indices whose text matches a predicate.
    ///
    /// Searches old (ours) and new (theirs) text for each row without
    /// allocating `FileDiffRow` objects, making this much cheaper than
    /// iterating `row_at()` for every row in a giant file.
    pub fn search_matching_rows(
        &self,
        segments: &[ConflictSegment],
        predicate: impl Fn(&str) -> bool,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        for entry in &self.entries {
            let Some(segment) = segments.get(entry.segment_ix) else {
                continue;
            };
            match (&entry.kind, segment) {
                (
                    SplitLayoutKind::Context {
                        line_index,
                        leading_row_count,
                        trailing_row_start,
                        ..
                    },
                    ConflictSegment::Text(text),
                ) => {
                    for offset in 0..entry.row_count {
                        let line_ix = if offset < *leading_row_count {
                            offset
                        } else {
                            trailing_row_start
                                .saturating_add(offset.saturating_sub(*leading_row_count))
                        };
                        let Some(line) = line_index.line_text(text, line_ix) else {
                            continue;
                        };
                        if predicate(line) {
                            out.push(entry.row_start + offset);
                        }
                    }
                }
                (
                    SplitLayoutKind::Block {
                        ours_line_index,
                        theirs_line_index,
                        ..
                    },
                    ConflictSegment::Block(block),
                ) => {
                    let ours_count = ours_line_index.line_count();
                    let theirs_count = theirs_line_index.line_count();
                    for offset in 0..entry.row_count {
                        let ours_line_ix = (offset < ours_count).then_some(offset);
                        let theirs_line_ix = (offset < theirs_count).then_some(offset);
                        let ours_match = ours_line_ix.is_some_and(|line_ix| {
                            ours_line_index
                                .line_text(&block.ours, line_ix)
                                .is_some_and(|line| predicate(line))
                        });
                        let theirs_match = theirs_line_ix.is_some_and(|line_ix| {
                            theirs_line_index
                                .line_text(&block.theirs, line_ix)
                                .is_some_and(|line| predicate(line))
                        });
                        if ours_match || theirs_match {
                            out.push(entry.row_start + offset);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
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

// ---------------------------------------------------------------------------
// Two-way split visible projection (analogous to ThreeWayVisibleProjection)
// ---------------------------------------------------------------------------

/// A contiguous span of visible split rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TwoWaySplitSpan {
    /// First visible index for this span.
    pub visible_start: usize,
    /// First source row index.
    pub source_row_start: usize,
    /// Number of rows in this span.
    pub len: usize,
    /// Conflict index if all rows in this span belong to one block.
    pub conflict_ix: Option<usize>,
}

/// Span-based visible projection for the two-way split view in giant mode.
#[derive(Clone, Debug)]
pub struct TwoWaySplitProjection {
    spans: Vec<TwoWaySplitSpan>,
    visible_len: usize,
}

impl Default for TwoWaySplitProjection {
    fn default() -> Self {
        Self {
            spans: Vec::new(),
            visible_len: 0,
        }
    }
}

impl TwoWaySplitProjection {
    /// Build a projection from the split row index, filtering out resolved blocks.
    pub fn new(
        index: &ConflictSplitRowIndex,
        segments: &[ConflictSegment],
        hide_resolved: bool,
    ) -> Self {
        let resolved_blocks: Vec<bool> = segments
            .iter()
            .filter_map(|s| match s {
                ConflictSegment::Block(b) => Some(b.resolved),
                _ => None,
            })
            .collect();

        let mut spans = Vec::new();
        let mut visible_len = 0usize;

        for entry in &index.entries {
            if hide_resolved {
                if let Some(ci) = entry.conflict_ix {
                    if resolved_blocks.get(ci).copied().unwrap_or(false) {
                        continue;
                    }
                }
            }
            spans.push(TwoWaySplitSpan {
                visible_start: visible_len,
                source_row_start: entry.row_start,
                len: entry.row_count,
                conflict_ix: entry.conflict_ix,
            });
            visible_len += entry.row_count;
        }

        Self { spans, visible_len }
    }

    /// Total number of visible rows.
    pub fn visible_len(&self) -> usize {
        self.visible_len
    }

    /// Map a visible index to a source row index and conflict index.
    pub fn get(&self, visible_ix: usize) -> Option<(usize, Option<usize>)> {
        if visible_ix >= self.visible_len {
            return None;
        }
        let pos = self
            .spans
            .partition_point(|s| s.visible_start <= visible_ix)
            .saturating_sub(1);
        let span = self.spans.get(pos)?;
        let offset = visible_ix.checked_sub(span.visible_start)?;
        if offset >= span.len {
            return None;
        }
        Some((span.source_row_start + offset, span.conflict_ix))
    }

    /// Find the first visible index for a given conflict.
    pub fn visible_index_for_conflict(&self, conflict_ix: usize) -> Option<usize> {
        self.spans
            .iter()
            .find(|s| s.conflict_ix == Some(conflict_ix))
            .map(|s| s.visible_start)
    }

    /// Map a source row index back to a visible index.
    pub fn source_to_visible(&self, source_row_ix: usize) -> Option<usize> {
        let pos = self
            .spans
            .partition_point(|s| s.source_row_start <= source_row_ix)
            .saturating_sub(1);
        let span = self.spans.get(pos)?;
        let offset = source_row_ix.checked_sub(span.source_row_start)?;
        if offset >= span.len {
            return None;
        }
        Some(span.visible_start + offset)
    }

    /// Approximate heap bytes used by the projection metadata (spans vec).
    #[cfg(test)]
    pub fn metadata_byte_size(&self) -> usize {
        self.spans.len() * std::mem::size_of::<TwoWaySplitSpan>()
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests;
