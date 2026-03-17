use crate::domain::FileConflictKind;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

mod autosolve;
mod history;
mod marker_parse;
mod subchunk;

#[cfg(test)]
use crate::text_utils::{LineEndingDetectionMode, detect_line_ending_from_texts};
use autosolve::{
    compile_regex_patterns, regex_assisted_auto_resolve_pick_with_compiled, safe_auto_resolve,
};
#[cfg(test)]
use history::history_section_suffix;
#[cfg(test)]
use marker_parse::parse_conflict_regions_from_markers;
#[cfg(test)]
use regex::Regex;

pub use autosolve::{
    is_whitespace_only_diff, regex_assisted_auto_resolve_pick, safe_auto_resolve_pick,
    try_autosolve_merged_text,
};
pub use history::{HistoryAutosolveOptions, history_merge_region};
pub use marker_parse::{
    ParsedConflictBlock, ParsedConflictBlockRanges, ParsedConflictSegment,
    ParsedConflictSegmentRanges, parse_conflict_marker_ranges, parse_conflict_marker_segments,
};
pub use subchunk::{Subchunk, split_conflict_into_subchunks};

/// The payload content for one side of a conflict.
///
/// Supports text, raw bytes (for non-UTF8 files), or absent content
/// (e.g. when a file was deleted on one side of a merge).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictPayload {
    /// Valid UTF-8 text content.
    Text(Arc<str>),
    /// Non-UTF8 binary content.
    Binary(Arc<[u8]>),
    /// Side is absent (file deleted or not present on this branch).
    Absent,
}

/// Tuple form used by staged conflict-file loading: `(raw_bytes, utf8_text)`.
pub type ConflictStageParts = (Option<Arc<[u8]>>, Option<Arc<str>>);

#[derive(Clone, Debug, Eq, PartialEq)]
enum ConflictRegionTextStorage {
    Owned(String),
    SharedSlice { text: Arc<str>, range: Range<usize> },
}

#[derive(Clone, Debug)]
pub struct ConflictRegionText {
    storage: ConflictRegionTextStorage,
}

impl ConflictRegionText {
    pub fn shared(text: Arc<str>) -> Self {
        let len = text.len();
        Self {
            storage: ConflictRegionTextStorage::SharedSlice {
                text,
                range: 0..len,
            },
        }
    }

    pub fn shared_slice(text: Arc<str>, range: Range<usize>) -> Self {
        debug_assert!(
            text.get(range.clone()).is_some(),
            "shared conflict region text range should stay within bounds"
        );
        Self {
            storage: ConflictRegionTextStorage::SharedSlice { text, range },
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.storage {
            ConflictRegionTextStorage::Owned(text) => text.as_str(),
            ConflictRegionTextStorage::SharedSlice { text, range } => text
                .get(range.clone())
                .expect("shared conflict region text range should stay valid"),
        }
    }

    pub fn into_owned_string(self) -> String {
        match self.storage {
            ConflictRegionTextStorage::Owned(text) => text,
            ConflictRegionTextStorage::SharedSlice { text, range } => text
                .get(range)
                .expect("shared conflict region text range should stay valid")
                .to_string(),
        }
    }

    pub fn shares_backing_with(&self, other: &Arc<str>) -> bool {
        match &self.storage {
            ConflictRegionTextStorage::Owned(_) => false,
            ConflictRegionTextStorage::SharedSlice { text, .. } => Arc::ptr_eq(text, other),
        }
    }
}

impl std::fmt::Display for ConflictRegionText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::ops::Deref for ConflictRegionText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for ConflictRegionText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for ConflictRegionText {
    fn from(value: String) -> Self {
        Self {
            storage: ConflictRegionTextStorage::Owned(value),
        }
    }
}

impl From<&str> for ConflictRegionText {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

impl PartialEq for ConflictRegionText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for ConflictRegionText {}

impl PartialEq<&str> for ConflictRegionText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ConflictRegionText> for &str {
    fn eq(&self, other: &ConflictRegionText) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<String> for ConflictRegionText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<ConflictRegionText> for String {
    fn eq(&self, other: &ConflictRegionText) -> bool {
        self.as_str() == other.as_str()
    }
}

impl ConflictPayload {
    /// Returns the text content if this payload is `Text`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ConflictPayload::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the raw bytes for this payload.
    ///
    /// For UTF-8 text payloads this returns the encoded text bytes.
    /// For binary payloads this returns the original bytes.
    /// For absent payloads this returns `None`.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            ConflictPayload::Text(s) => Some(s.as_bytes()),
            ConflictPayload::Binary(bytes) => Some(bytes.as_ref()),
            ConflictPayload::Absent => None,
        }
    }

    /// Returns the payload size in bytes, or `None` when absent.
    pub fn byte_len(&self) -> Option<usize> {
        self.as_bytes().map(<[u8]>::len)
    }

    /// Returns `true` if this side has no content.
    pub fn is_absent(&self) -> bool {
        matches!(self, ConflictPayload::Absent)
    }

    /// Returns `true` if this is binary content.
    pub fn is_binary(&self) -> bool {
        matches!(self, ConflictPayload::Binary(_))
    }

    /// Try to create from raw bytes: if valid UTF-8, produce `Text`; otherwise `Binary`.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(s) => ConflictPayload::Text(s.into()),
            Err(e) => ConflictPayload::Binary(e.into_bytes().into()),
        }
    }

    /// Construct from the separate bytes/text fields used by `ConflictFileStages`.
    ///
    /// Prefers text when present; falls back to binary bytes; produces `Absent`
    /// when both are `None`.
    pub fn from_stage_parts(bytes: Option<Arc<[u8]>>, text: Option<Arc<str>>) -> Self {
        if let Some(t) = text {
            ConflictPayload::Text(t)
        } else if let Some(b) = bytes {
            ConflictPayload::Binary(b)
        } else {
            ConflictPayload::Absent
        }
    }

    /// Decompose into the separate bytes/text fields used by `ConflictFileStages`.
    ///
    /// Inverse of [`from_stage_parts`](Self::from_stage_parts).
    pub fn into_stage_parts(self) -> ConflictStageParts {
        match self {
            ConflictPayload::Text(text) => (None, Some(text)),
            ConflictPayload::Binary(bytes) => (Some(bytes), None),
            ConflictPayload::Absent => (None, None),
        }
    }
}

/// Confidence level assigned to an auto-resolve decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveConfidence {
    /// Deterministic and effectively risk-free in the current model.
    High,
    /// Conservative heuristic or normalization-based decision.
    Medium,
    /// Advanced heuristic decision that should be reviewed by users.
    Low,
}

impl AutosolveConfidence {
    pub fn label(&self) -> &'static str {
        match self {
            AutosolveConfidence::High => "high",
            AutosolveConfidence::Medium => "medium",
            AutosolveConfidence::Low => "low",
        }
    }
}

/// How a single conflict region has been resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictRegionResolution {
    /// Not yet resolved by the user.
    Unresolved,
    /// User picked the base version.
    PickBase,
    /// User picked "ours" (local/HEAD).
    PickOurs,
    /// User picked "theirs" (remote/incoming).
    PickTheirs,
    /// User picked both (ours then theirs).
    PickBoth,
    /// User manually edited the output for this region.
    ManualEdit(String),
    /// Automatically resolved by a safe rule.
    AutoResolved {
        rule: AutosolveRule,
        /// Confidence assigned to the applied auto-resolve rule.
        confidence: AutosolveConfidence,
        /// The text chosen by the auto-resolver.
        content: String,
    },
}

impl ConflictRegionResolution {
    /// Returns `true` if this region has been resolved (any way).
    pub fn is_resolved(&self) -> bool {
        !matches!(self, ConflictRegionResolution::Unresolved)
    }
}

/// Identifies which auto-resolve rule was applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolveRule {
    /// Both sides are identical (`ours == theirs`), so either is correct.
    IdenticalSides,
    /// Only "ours" changed from base; "theirs" equals base.
    OnlyOursChanged,
    /// Only "theirs" changed from base; "ours" equals base.
    OnlyTheirsChanged,
    /// Whitespace-only difference between sides (optional Pass 1 toggle).
    WhitespaceOnly,
    /// Regex-assisted mode: sides differ textually but normalize to equal.
    RegexEquivalentSides,
    /// Regex-assisted mode: ours normalizes to base; theirs differs.
    RegexOnlyTheirsChanged,
    /// Regex-assisted mode: theirs normalizes to base; ours differs.
    RegexOnlyOursChanged,
    /// Pass 2: block was split into line-level subchunks and all could be merged.
    SubchunkFullyMerged,
    /// History-aware mode: entries in a history/changelog section were merged.
    HistoryMerged,
}

impl AutosolveRule {
    pub fn description(&self) -> &'static str {
        match self {
            AutosolveRule::IdenticalSides => "both sides identical",
            AutosolveRule::OnlyOursChanged => "only ours changed from base",
            AutosolveRule::OnlyTheirsChanged => "only theirs changed from base",
            AutosolveRule::WhitespaceOnly => "whitespace-only difference",
            AutosolveRule::RegexEquivalentSides => "regex-normalized sides equivalent",
            AutosolveRule::RegexOnlyTheirsChanged => {
                "regex-normalized: only theirs changed from base"
            }
            AutosolveRule::RegexOnlyOursChanged => "regex-normalized: only ours changed from base",
            AutosolveRule::SubchunkFullyMerged => "line-level subchunk merge",
            AutosolveRule::HistoryMerged => "history/changelog section merge",
        }
    }

    /// Confidence classification for this rule.
    pub fn confidence(&self) -> AutosolveConfidence {
        match self {
            AutosolveRule::IdenticalSides
            | AutosolveRule::OnlyOursChanged
            | AutosolveRule::OnlyTheirsChanged => AutosolveConfidence::High,
            AutosolveRule::WhitespaceOnly
            | AutosolveRule::RegexEquivalentSides
            | AutosolveRule::RegexOnlyTheirsChanged
            | AutosolveRule::RegexOnlyOursChanged
            | AutosolveRule::SubchunkFullyMerged => AutosolveConfidence::Medium,
            AutosolveRule::HistoryMerged => AutosolveConfidence::Low,
        }
    }
}

/// Side chosen by an auto-resolve decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutosolvePickSide {
    Ours,
    Theirs,
}

/// One regex replacement rule used by advanced autosolve mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegexAutosolvePattern {
    pub pattern: String,
    pub replacement: String,
}

impl RegexAutosolvePattern {
    pub fn new(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            replacement: replacement.into(),
        }
    }
}

/// Options for Pass 3 regex-assisted autosolve.
///
/// This mode is explicitly opt-in and intended for conservative normalization
/// patterns (for example, whitespace-insensitive matching).
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RegexAutosolveOptions {
    pub patterns: Vec<RegexAutosolvePattern>,
}

impl RegexAutosolveOptions {
    /// A conservative preset that ignores all whitespace differences.
    pub fn whitespace_insensitive() -> Self {
        Self {
            patterns: vec![RegexAutosolvePattern::new(r"\s+", "")],
        }
    }

    pub fn with_pattern(
        mut self,
        pattern: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        self.patterns
            .push(RegexAutosolvePattern::new(pattern, replacement));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// A single conflict region within a file — represents one conflict block
/// delimited by markers (`<<<<<<<` / `=======` / `>>>>>>>`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegion {
    /// The base (common ancestor) content for this region.
    pub base: Option<ConflictRegionText>,
    /// The "ours" (local/HEAD) content.
    pub ours: ConflictRegionText,
    /// The "theirs" (remote/incoming) content.
    pub theirs: ConflictRegionText,
    /// Current resolution state.
    pub resolution: ConflictRegionResolution,
}

impl ConflictRegion {
    /// Returns the resolved text for this region based on its resolution state.
    /// Returns `None` if unresolved.
    pub fn resolved_text(&self) -> Option<&str> {
        match &self.resolution {
            ConflictRegionResolution::Unresolved => None,
            ConflictRegionResolution::PickBase => self.base.as_deref().or(Some("")),
            ConflictRegionResolution::PickOurs => Some(self.ours.as_str()),
            ConflictRegionResolution::PickTheirs => Some(self.theirs.as_str()),
            ConflictRegionResolution::PickBoth => None, // caller must concat ours+theirs
            ConflictRegionResolution::ManualEdit(text) => Some(text.as_str()),
            ConflictRegionResolution::AutoResolved { content, .. } => Some(content.as_str()),
        }
    }

    /// Produce the resolved text for "both" picks (ours followed by theirs).
    pub fn resolved_text_both(&self) -> String {
        let mut out = String::with_capacity(self.ours.len() + self.theirs.len());
        out.push_str(self.ours.as_str());
        out.push_str(self.theirs.as_str());
        out
    }
}

/// What resolver strategy to use for a given conflict kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictResolverStrategy {
    /// Full 3-way text resolver with marker parsing, A/B/C picks, manual edit.
    /// Used for `BothModified`, `BothAdded`.
    FullTextResolver,
    /// 2-way resolver with one side being empty/absent. Shows keep/delete actions.
    /// Used for `DeletedByUs`, `DeletedByThem`, `AddedByUs`, `AddedByThem`.
    TwoWayKeepDelete,
    /// Decision-only panel — accept deletion or restore from a side.
    /// Used for `BothDeleted`.
    DecisionOnly,
    /// Binary/non-UTF8 side-pick resolver.
    BinarySidePick,
}

impl ConflictResolverStrategy {
    /// Determine the resolver strategy for a given conflict kind and payload state.
    pub fn for_conflict(kind: FileConflictKind, is_binary: bool) -> Self {
        match kind {
            // Both-deleted conflicts are decision-only regardless of payload encoding.
            // There is no side content to pick, so binary side-pick would dead-end.
            FileConflictKind::BothDeleted => ConflictResolverStrategy::DecisionOnly,
            _ if is_binary => ConflictResolverStrategy::BinarySidePick,
            FileConflictKind::BothModified | FileConflictKind::BothAdded => {
                ConflictResolverStrategy::FullTextResolver
            }
            FileConflictKind::DeletedByUs
            | FileConflictKind::DeletedByThem
            | FileConflictKind::AddedByUs
            | FileConflictKind::AddedByThem => ConflictResolverStrategy::TwoWayKeepDelete,
        }
    }

    /// Human-readable label for this strategy.
    pub fn label(&self) -> &'static str {
        match self {
            ConflictResolverStrategy::FullTextResolver => "Text Merge",
            ConflictResolverStrategy::TwoWayKeepDelete => "Keep / Delete",
            ConflictResolverStrategy::BinarySidePick => "Side Pick (Binary)",
            ConflictResolverStrategy::DecisionOnly => "Decision",
        }
    }
}

/// The main conflict session model. Holds all state for resolving conflicts
/// in a single file during a merge/rebase/cherry-pick.
///
/// Decouples "how conflict is represented" from "how the UI renders it",
/// allowing one resolver shell for all conflict kinds.
#[derive(Clone, Debug)]
pub struct ConflictSession {
    /// Path of the conflicted file relative to workdir.
    pub path: PathBuf,
    /// The kind of conflict from git status.
    pub conflict_kind: FileConflictKind,
    /// Resolver strategy determined from kind + payload.
    pub strategy: ConflictResolverStrategy,
    /// Base (common ancestor) content — full file.
    pub base: ConflictPayload,
    /// "Ours" (local/HEAD) content — full file.
    pub ours: ConflictPayload,
    /// "Theirs" (remote/incoming) content — full file.
    pub theirs: ConflictPayload,
    /// Loaded current merged/worktree content, when the loader already has it.
    ///
    /// `None` means the current payload was not loaded alongside the session.
    /// `Some(ConflictPayload::Absent)` means it was loaded and is absent.
    pub current: Option<ConflictPayload>,
    /// Parsed conflict regions (populated for marker-based text conflicts).
    pub regions: Vec<ConflictRegion>,
}

impl ConflictSession {
    fn has_implicit_binary_conflict(&self) -> bool {
        self.strategy == ConflictResolverStrategy::BinarySidePick && self.regions.is_empty()
    }

    fn payload_as_side_text(payload: &ConflictPayload) -> Option<ConflictRegionText> {
        match payload {
            ConflictPayload::Text(text) => Some(ConflictRegionText::shared(text.clone())),
            ConflictPayload::Absent => Some(ConflictRegionText::from(String::new())),
            ConflictPayload::Binary(_) => None,
        }
    }

    fn payload_as_base_text(payload: &ConflictPayload) -> Option<Option<ConflictRegionText>> {
        match payload {
            ConflictPayload::Text(text) => Some(Some(ConflictRegionText::shared(text.clone()))),
            ConflictPayload::Absent => Some(None),
            ConflictPayload::Binary(_) => None,
        }
    }

    fn synthetic_region_for_strategy(
        strategy: ConflictResolverStrategy,
        base: &ConflictPayload,
        ours: &ConflictPayload,
        theirs: &ConflictPayload,
    ) -> Option<ConflictRegion> {
        match strategy {
            ConflictResolverStrategy::TwoWayKeepDelete | ConflictResolverStrategy::DecisionOnly => {
                let base = Self::payload_as_base_text(base)?;
                let ours = Self::payload_as_side_text(ours)?;
                let theirs = Self::payload_as_side_text(theirs)?;
                Some(ConflictRegion {
                    base,
                    ours,
                    theirs,
                    resolution: ConflictRegionResolution::Unresolved,
                })
            }
            ConflictResolverStrategy::FullTextResolver
            | ConflictResolverStrategy::BinarySidePick => None,
        }
    }

    fn new_with_optional_current(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
        current: Option<ConflictPayload>,
    ) -> Self {
        let is_binary = base.is_binary() || ours.is_binary() || theirs.is_binary();
        let strategy = ConflictResolverStrategy::for_conflict(conflict_kind, is_binary);
        let regions = Self::synthetic_region_for_strategy(strategy, &base, &ours, &theirs)
            .into_iter()
            .collect();
        Self {
            path,
            conflict_kind,
            strategy,
            base,
            ours,
            theirs,
            current,
            regions,
        }
    }

    /// Create a new session from the three file-level payloads.
    pub fn new(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
    ) -> Self {
        Self::new_with_optional_current(path, conflict_kind, base, ours, theirs, None)
    }

    /// Create a new session and retain the loaded current merged/worktree payload.
    pub fn new_with_current(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
        current: ConflictPayload,
    ) -> Self {
        Self::new_with_optional_current(path, conflict_kind, base, ours, theirs, Some(current))
    }

    /// Build a session from shared merged marker text without copying it again.
    pub fn from_merged_shared_text(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
        merged_text: Arc<str>,
    ) -> Self {
        let mut session = Self::new_with_current(
            path,
            conflict_kind,
            base,
            ours,
            theirs,
            ConflictPayload::Text(merged_text.clone()),
        );
        session.parse_regions_from_shared_text(merged_text);
        session
    }

    /// Build a session and parse conflict regions from merged marker text.
    ///
    /// This is a convenience for loading a conflicted worktree file where the
    /// merged content still contains conflict markers.
    // Public for test and benchmark setup only; not called from production code.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_merged_text(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
        merged_text: &str,
    ) -> Self {
        Self::from_merged_shared_text(
            path,
            conflict_kind,
            base,
            ours,
            theirs,
            Arc::<str>::from(merged_text),
        )
    }

    /// Parse marker-based conflict regions from merged text and replace the
    /// current region list.
    ///
    /// Recognizes both 2-way (`<<<<<<<` / `=======` / `>>>>>>>`) and
    /// diff3-style (`|||||||` base section) markers.
    ///
    /// Returns the number of parsed regions.
    // Public for tests only; thin wrapper around parse_regions_from_shared_text.
    #[cfg(test)]
    pub fn parse_regions_from_merged_text(&mut self, merged_text: &str) -> usize {
        self.parse_regions_from_shared_text(Arc::<str>::from(merged_text))
    }

    /// Parse marker-based conflict regions from shared merged text and replace
    /// the current region list without copying each block payload.
    pub fn parse_regions_from_shared_text(&mut self, merged_text: Arc<str>) -> usize {
        self.regions = marker_parse::parse_conflict_regions_from_shared_text(merged_text);
        if self.regions.is_empty()
            && let Some(region) = Self::synthetic_region_for_strategy(
                self.strategy,
                &self.base,
                &self.ours,
                &self.theirs,
            )
        {
            self.regions.push(region);
        }
        self.regions.len()
    }

    /// Returns the base side bytes (stage 1 payload), when present.
    pub fn base_bytes(&self) -> Option<&[u8]> {
        self.base.as_bytes()
    }

    /// Returns the ours side bytes (stage 2 payload), when present.
    pub fn ours_bytes(&self) -> Option<&[u8]> {
        self.ours.as_bytes()
    }

    /// Returns the theirs side bytes (stage 3 payload), when present.
    pub fn theirs_bytes(&self) -> Option<&[u8]> {
        self.theirs.as_bytes()
    }

    /// Returns the loaded current merged/worktree text, when available.
    pub fn current_text(&self) -> Option<&str> {
        self.current.as_ref().and_then(ConflictPayload::as_text)
    }

    /// Returns the loaded current merged/worktree bytes, when available.
    pub fn current_bytes(&self) -> Option<&[u8]> {
        self.current.as_ref().and_then(ConflictPayload::as_bytes)
    }

    /// Total number of conflict regions.
    pub fn total_regions(&self) -> usize {
        if self.has_implicit_binary_conflict() {
            1
        } else {
            self.regions.len()
        }
    }

    /// Number of resolved conflict regions.
    pub fn solved_count(&self) -> usize {
        if self.has_implicit_binary_conflict() {
            0
        } else {
            self.regions
                .iter()
                .filter(|r| r.resolution.is_resolved())
                .count()
        }
    }

    /// Number of unresolved conflict regions.
    pub fn unsolved_count(&self) -> usize {
        self.total_regions() - self.solved_count()
    }

    /// Returns `true` when all regions are resolved.
    pub fn is_fully_resolved(&self) -> bool {
        !self.has_implicit_binary_conflict()
            && self.regions.iter().all(|r| r.resolution.is_resolved())
    }

    /// Find the index of the next unresolved region after `current`.
    /// Wraps around to the beginning if needed.
    /// Returns `None` if all regions are resolved.
    pub fn next_unresolved_after(&self, current: usize) -> Option<usize> {
        let len = self.regions.len();
        if len == 0 {
            return None;
        }
        // Search forward from current+1, wrapping around.
        for offset in 1..=len {
            let idx = (current + offset) % len;
            if !self.regions[idx].resolution.is_resolved() {
                return Some(idx);
            }
        }
        None
    }

    /// Find the index of the previous unresolved region before `current`.
    /// Wraps around to the end if needed.
    pub fn prev_unresolved_before(&self, current: usize) -> Option<usize> {
        let len = self.regions.len();
        if len == 0 {
            return None;
        }
        for offset in 1..=len {
            let idx = (current + len - offset) % len;
            if !self.regions[idx].resolution.is_resolved() {
                return Some(idx);
            }
        }
        None
    }

    /// Apply auto-resolve Pass 1 (always-safe rules) to all unresolved regions.
    ///
    /// Safe rules:
    /// 1. `ours == theirs` — both sides made the same change.
    /// 2. `ours == base` and `theirs != base` — only theirs changed.
    /// 3. `theirs == base` and `ours != base` — only ours changed.
    /// 4. (if `whitespace_normalize`) whitespace-only difference → pick ours.
    ///
    /// Returns the number of regions auto-resolved.
    pub fn auto_resolve_safe(&mut self) -> usize {
        self.auto_resolve_safe_with_options(false)
    }

    /// Like [`auto_resolve_safe`] but with an optional whitespace-normalization toggle.
    pub fn auto_resolve_safe_with_options(&mut self, whitespace_normalize: bool) -> usize {
        let mut count = 0;
        for region in &mut self.regions {
            if region.resolution.is_resolved() {
                continue;
            }
            if let Some((rule, content)) = safe_auto_resolve(region, whitespace_normalize) {
                region.resolution = ConflictRegionResolution::AutoResolved {
                    confidence: rule.confidence(),
                    rule,
                    content,
                };
                count += 1;
            }
        }
        count
    }

    /// Apply auto-resolve Pass 3 (regex-assisted, opt-in) to unresolved regions.
    ///
    /// This mode allows conservative normalization rules to treat text as
    /// equivalent even when byte-for-byte content differs (for example,
    /// whitespace-only differences).
    ///
    /// Returns the number of regions auto-resolved.
    pub fn auto_resolve_regex(&mut self, options: &RegexAutosolveOptions) -> usize {
        let Some(compiled) = compile_regex_patterns(options) else {
            return 0;
        };

        let mut count = 0;
        for region in &mut self.regions {
            if region.resolution.is_resolved() {
                continue;
            }
            if let Some((rule, pick)) = regex_assisted_auto_resolve_pick_with_compiled(
                region.base.as_deref(),
                &region.ours,
                &region.theirs,
                &compiled,
            ) {
                let content = match pick {
                    AutosolvePickSide::Ours => region.ours.to_string(),
                    AutosolvePickSide::Theirs => region.theirs.to_string(),
                };
                region.resolution = ConflictRegionResolution::AutoResolved {
                    confidence: rule.confidence(),
                    rule,
                    content,
                };
                count += 1;
            }
        }
        count
    }

    /// Apply auto-resolve Pass 2 (heuristic subchunk splitting) to unresolved regions.
    ///
    /// For each unresolved region that has a base, splits the conflict into
    /// line-level subchunks. If ALL subchunks can be auto-merged (no remaining
    /// conflicts), the region is fully resolved with the merged text.
    ///
    /// Returns the number of regions auto-resolved.
    pub fn auto_resolve_pass2(&mut self) -> usize {
        let mut count = 0;
        for region in &mut self.regions {
            if region.resolution.is_resolved() {
                continue;
            }
            let Some(base) = region.base.as_deref() else {
                continue;
            };
            if let Some(subchunks) =
                split_conflict_into_subchunks(base, &region.ours, &region.theirs)
                    .filter(|sc| sc.iter().all(|c| matches!(c, Subchunk::Resolved(_))))
            {
                let merged: String = subchunks
                    .iter()
                    .map(|c| match c {
                        Subchunk::Resolved(text) => text.as_str(),
                        _ => unreachable!(),
                    })
                    .collect();
                region.resolution = ConflictRegionResolution::AutoResolved {
                    confidence: AutosolveRule::SubchunkFullyMerged.confidence(),
                    rule: AutosolveRule::SubchunkFullyMerged,
                    content: merged,
                };
                count += 1;
            }
        }
        count
    }

    /// Apply auto-resolve history mode to unresolved regions.
    ///
    /// Detects history/changelog sections within conflict blocks and merges
    /// their entries by deduplication (kdiff3-inspired). Only resolves
    /// regions that match the configured section/entry patterns.
    ///
    /// Returns the number of regions auto-resolved.
    pub fn auto_resolve_history(&mut self, options: &HistoryAutosolveOptions) -> usize {
        if !options.is_valid() {
            return 0;
        }

        let mut count = 0;
        for region in &mut self.regions {
            if region.resolution.is_resolved() {
                continue;
            }
            if let Some(merged) = history_merge_region(
                region.base.as_deref(),
                &region.ours,
                &region.theirs,
                options,
            ) {
                region.resolution = ConflictRegionResolution::AutoResolved {
                    confidence: AutosolveRule::HistoryMerged.confidence(),
                    rule: AutosolveRule::HistoryMerged,
                    content: merged,
                };
                count += 1;
            }
        }
        count
    }

    /// Check whether the resolved output still contains unresolved conflict markers.
    /// This is the safety gate before staging.
    pub fn has_unresolved_markers(&self) -> bool {
        self.unsolved_count() > 0
    }
}

#[cfg(test)]
mod tests;
