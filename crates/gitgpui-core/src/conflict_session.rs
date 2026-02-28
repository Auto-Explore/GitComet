use crate::domain::FileConflictKind;
use crate::file_diff::{Edit, EditKind, myers_edits, split_lines};
use regex::Regex;
use std::path::PathBuf;

/// The payload content for one side of a conflict.
///
/// Supports text, raw bytes (for non-UTF8 files), or absent content
/// (e.g. when a file was deleted on one side of a merge).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConflictPayload {
    /// Valid UTF-8 text content.
    Text(String),
    /// Non-UTF8 binary content.
    Binary(Vec<u8>),
    /// Side is absent (file deleted or not present on this branch).
    Absent,
}

impl ConflictPayload {
    /// Returns the text content if this payload is `Text`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ConflictPayload::Text(s) => Some(s),
            _ => None,
        }
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
            Ok(s) => ConflictPayload::Text(s),
            Err(e) => ConflictPayload::Binary(e.into_bytes()),
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
    /// Regex-assisted mode: sides differ textually but normalize to equal.
    RegexEquivalentSides,
    /// Regex-assisted mode: ours normalizes to base; theirs differs.
    RegexOnlyTheirsChanged,
    /// Regex-assisted mode: theirs normalizes to base; ours differs.
    RegexOnlyOursChanged,
    /// Pass 2: block was split into line-level subchunks and all could be merged.
    SubchunkFullyMerged,
}

impl AutosolveRule {
    pub fn description(&self) -> &'static str {
        match self {
            AutosolveRule::IdenticalSides => "both sides identical",
            AutosolveRule::OnlyOursChanged => "only ours changed from base",
            AutosolveRule::OnlyTheirsChanged => "only theirs changed from base",
            AutosolveRule::RegexEquivalentSides => "regex-normalized sides equivalent",
            AutosolveRule::RegexOnlyTheirsChanged => {
                "regex-normalized: only theirs changed from base"
            }
            AutosolveRule::RegexOnlyOursChanged => "regex-normalized: only ours changed from base",
            AutosolveRule::SubchunkFullyMerged => "line-level subchunk merge",
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

#[derive(Clone)]
struct CompiledRegexAutosolvePattern {
    regex: Regex,
    replacement: String,
}

/// A single conflict region within a file — represents one conflict block
/// delimited by markers (`<<<<<<<` / `=======` / `>>>>>>>`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegion {
    /// The base (common ancestor) content for this region.
    pub base: Option<String>,
    /// The "ours" (local/HEAD) content.
    pub ours: String,
    /// The "theirs" (remote/incoming) content.
    pub theirs: String,
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
            ConflictRegionResolution::PickOurs => Some(&self.ours),
            ConflictRegionResolution::PickTheirs => Some(&self.theirs),
            ConflictRegionResolution::PickBoth => None, // caller must concat ours+theirs
            ConflictRegionResolution::ManualEdit(text) => Some(text),
            ConflictRegionResolution::AutoResolved { content, .. } => Some(content),
        }
    }

    /// Produce the resolved text for "both" picks (ours followed by theirs).
    pub fn resolved_text_both(&self) -> String {
        let mut out = String::with_capacity(self.ours.len() + self.theirs.len());
        out.push_str(&self.ours);
        out.push_str(&self.theirs);
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
        if is_binary {
            return ConflictResolverStrategy::BinarySidePick;
        }
        match kind {
            FileConflictKind::BothModified | FileConflictKind::BothAdded => {
                ConflictResolverStrategy::FullTextResolver
            }
            FileConflictKind::DeletedByUs
            | FileConflictKind::DeletedByThem
            | FileConflictKind::AddedByUs
            | FileConflictKind::AddedByThem => ConflictResolverStrategy::TwoWayKeepDelete,
            FileConflictKind::BothDeleted => ConflictResolverStrategy::DecisionOnly,
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
    /// Parsed conflict regions (populated for marker-based text conflicts).
    pub regions: Vec<ConflictRegion>,
}

impl ConflictSession {
    /// Create a new session from the three file-level payloads.
    pub fn new(
        path: PathBuf,
        conflict_kind: FileConflictKind,
        base: ConflictPayload,
        ours: ConflictPayload,
        theirs: ConflictPayload,
    ) -> Self {
        let is_binary = base.is_binary() || ours.is_binary() || theirs.is_binary();
        let strategy = ConflictResolverStrategy::for_conflict(conflict_kind, is_binary);
        Self {
            path,
            conflict_kind,
            strategy,
            base,
            ours,
            theirs,
            regions: Vec::new(),
        }
    }

    /// Total number of conflict regions.
    pub fn total_regions(&self) -> usize {
        self.regions.len()
    }

    /// Number of resolved conflict regions.
    pub fn solved_count(&self) -> usize {
        self.regions
            .iter()
            .filter(|r| r.resolution.is_resolved())
            .count()
    }

    /// Number of unresolved conflict regions.
    pub fn unsolved_count(&self) -> usize {
        self.total_regions() - self.solved_count()
    }

    /// Returns `true` when all regions are resolved.
    pub fn is_fully_resolved(&self) -> bool {
        self.regions.iter().all(|r| r.resolution.is_resolved())
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
    ///
    /// Returns the number of regions auto-resolved.
    pub fn auto_resolve_safe(&mut self) -> usize {
        let mut count = 0;
        for region in &mut self.regions {
            if region.resolution.is_resolved() {
                continue;
            }
            if let Some((rule, content)) = safe_auto_resolve(region) {
                region.resolution = ConflictRegionResolution::AutoResolved { rule, content };
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
                    AutosolvePickSide::Ours => region.ours.clone(),
                    AutosolvePickSide::Theirs => region.theirs.clone(),
                };
                region.resolution = ConflictRegionResolution::AutoResolved { rule, content };
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
            {
                if subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))) {
                    let merged: String = subchunks
                        .iter()
                        .map(|c| match c {
                            Subchunk::Resolved(text) => text.as_str(),
                            _ => unreachable!(),
                        })
                        .collect();
                    region.resolution = ConflictRegionResolution::AutoResolved {
                        rule: AutosolveRule::SubchunkFullyMerged,
                        content: merged,
                    };
                    count += 1;
                }
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

/// Attempt to auto-resolve a single conflict region using Pass 1 safe rules.
///
/// Returns `Some((rule, resolved_content))` if a safe resolution is found.
fn safe_auto_resolve(region: &ConflictRegion) -> Option<(AutosolveRule, String)> {
    // Rule 1: both sides identical.
    if region.ours == region.theirs {
        return Some((AutosolveRule::IdenticalSides, region.ours.clone()));
    }

    // Rules 2 & 3 require a base.
    let base = region.base.as_deref()?;

    // Rule 2: only theirs changed (ours == base).
    if region.ours == base && region.theirs != base {
        return Some((AutosolveRule::OnlyTheirsChanged, region.theirs.clone()));
    }

    // Rule 3: only ours changed (theirs == base).
    if region.theirs == base && region.ours != base {
        return Some((AutosolveRule::OnlyOursChanged, region.ours.clone()));
    }

    None
}

fn compile_regex_patterns(
    options: &RegexAutosolveOptions,
) -> Option<Vec<CompiledRegexAutosolvePattern>> {
    if options.is_empty() {
        return None;
    }
    let mut compiled = Vec::with_capacity(options.patterns.len());
    for pattern in &options.patterns {
        let regex = Regex::new(&pattern.pattern).ok()?;
        compiled.push(CompiledRegexAutosolvePattern {
            regex,
            replacement: pattern.replacement.clone(),
        });
    }
    Some(compiled)
}

fn normalize_with_patterns(text: &str, patterns: &[CompiledRegexAutosolvePattern]) -> String {
    let mut out = text.to_string();
    for rule in patterns {
        out = rule
            .regex
            .replace_all(&out, rule.replacement.as_str())
            .into_owned();
    }
    out
}

/// Pass 3 regex-assisted decision helper.
///
/// Returns which side to pick when regex-normalized comparison indicates a
/// conservative auto-resolution opportunity.
pub fn regex_assisted_auto_resolve_pick(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &RegexAutosolveOptions,
) -> Option<(AutosolveRule, AutosolvePickSide)> {
    let compiled = compile_regex_patterns(options)?;
    regex_assisted_auto_resolve_pick_with_compiled(base, ours, theirs, &compiled)
}

fn regex_assisted_auto_resolve_pick_with_compiled(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    compiled: &[CompiledRegexAutosolvePattern],
) -> Option<(AutosolveRule, AutosolvePickSide)> {
    // Skip cases already covered by Pass 1 safe rules.
    if ours == theirs {
        return None;
    }
    if let Some(base_raw) = base
        && ((ours == base_raw && theirs != base_raw) || (theirs == base_raw && ours != base_raw))
    {
        return None;
    }

    let norm_ours = normalize_with_patterns(ours, compiled);
    let norm_theirs = normalize_with_patterns(theirs, compiled);

    if norm_ours == norm_theirs {
        return Some((AutosolveRule::RegexEquivalentSides, AutosolvePickSide::Ours));
    }

    let base = base?;
    let norm_base = normalize_with_patterns(base, compiled);

    if norm_ours == norm_base && norm_theirs != norm_base {
        return Some((
            AutosolveRule::RegexOnlyTheirsChanged,
            AutosolvePickSide::Theirs,
        ));
    }
    if norm_theirs == norm_base && norm_ours != norm_base {
        return Some((AutosolveRule::RegexOnlyOursChanged, AutosolvePickSide::Ours));
    }

    None
}

// ---------------------------------------------------------------------------
// Pass 2: heuristic subchunk splitting (meld-inspired)
// ---------------------------------------------------------------------------

/// A subchunk produced by splitting a conflict block into line-level pieces.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Subchunk {
    /// Lines that could be auto-resolved (identical across sides, or only one
    /// side changed from base).
    Resolved(String),
    /// Lines where both sides changed differently — still needs resolution.
    Conflict {
        base: String,
        ours: String,
        theirs: String,
    },
}

/// A contiguous range of base lines that were changed (deleted/replaced/inserted)
/// on one side of a 2-way diff.
struct LineHunk {
    /// Start index in base lines (inclusive).
    base_start: usize,
    /// End index in base lines (exclusive). Equals `base_start` for pure insertions.
    base_end: usize,
    /// The replacement lines on this side.
    new_lines: Vec<String>,
}

/// Maximum number of lines per side before we skip subchunk splitting.
const SUBCHUNK_MAX_LINES: usize = 500;

/// Split a conflict region into line-level subchunks using 3-way diff/merge.
///
/// Returns `Some(subchunks)` if the block can be meaningfully decomposed into
/// a mix of resolved and conflicting pieces. Returns `None` if:
/// - Pass 1 would handle this (identical sides, only one side changed)
/// - Input is too large
/// - Splitting doesn't improve over the original block (all conflict, no context)
pub fn split_conflict_into_subchunks(
    base: &str,
    ours: &str,
    theirs: &str,
) -> Option<Vec<Subchunk>> {
    // Pass 1 would handle these — don't split.
    if ours == theirs || ours == base || theirs == base {
        return None;
    }

    let base_lines = split_lines(base);
    let ours_lines = split_lines(ours);
    let theirs_lines = split_lines(theirs);

    if base_lines.len() > SUBCHUNK_MAX_LINES
        || ours_lines.len() > SUBCHUNK_MAX_LINES
        || theirs_lines.len() > SUBCHUNK_MAX_LINES
    {
        return None;
    }

    let subchunks =
        if base_lines.len() == ours_lines.len() && base_lines.len() == theirs_lines.len() {
            // Same number of lines: simple per-line 3-way comparison.
            per_line_merge(&base_lines, &ours_lines, &theirs_lines)
        } else {
            // Different line counts: use diff-based hunk merge.
            let edits_ours = myers_edits(&base_lines, &ours_lines);
            let edits_theirs = myers_edits(&base_lines, &theirs_lines);
            let hunks_ours = edits_to_line_hunks(&edits_ours);
            let hunks_theirs = edits_to_line_hunks(&edits_theirs);
            merge_line_hunks(&base_lines, &hunks_ours, &hunks_theirs)
        };

    // Only worth returning if at least some content is resolved.
    let has_resolved = subchunks.iter().any(|c| matches!(c, Subchunk::Resolved(_)));
    if has_resolved { Some(subchunks) } else { None }
}

/// Convert a Myers edit script into line-level hunks relative to the base.
fn edits_to_line_hunks(edits: &[Edit<'_>]) -> Vec<LineHunk> {
    let mut hunks = Vec::new();
    let mut base_ix = 0usize;
    let mut i = 0;

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
                    new_lines.push(edits[i].new.unwrap_or("").to_string());
                }
                EditKind::Equal => unreachable!(),
            }
            i += 1;
        }

        hunks.push(LineHunk {
            base_start: hunk_base_start,
            base_end: base_ix,
            new_lines,
        });
    }

    hunks
}

/// Per-line 3-way merge for three sequences of equal length.
///
/// Walks line-by-line, classifying each line:
/// - all three equal → context (resolved)
/// - only ours changed → resolved (pick ours)
/// - only theirs changed → resolved (pick theirs)
/// - both changed same way → resolved (pick either)
/// - both changed differently → conflict
///
/// Groups consecutive lines with the same classification into subchunks.
fn per_line_merge(
    base_lines: &[&str],
    ours_lines: &[&str],
    theirs_lines: &[&str],
) -> Vec<Subchunk> {
    debug_assert_eq!(base_lines.len(), ours_lines.len());
    debug_assert_eq!(base_lines.len(), theirs_lines.len());

    let len = base_lines.len();
    let mut subchunks = Vec::new();
    let mut i = 0;

    while i < len {
        let same_bo = base_lines[i] == ours_lines[i];
        let same_bt = base_lines[i] == theirs_lines[i];
        let same_ot = ours_lines[i] == theirs_lines[i];

        if same_bo && same_bt {
            // All three equal → context.
            let start = i;
            while i < len && base_lines[i] == ours_lines[i] && base_lines[i] == theirs_lines[i] {
                i += 1;
            }
            subchunks.push(Subchunk::Resolved(lines_to_text(&base_lines[start..i])));
        } else if !same_bo && same_bt {
            // Only ours changed from base.
            let start = i;
            while i < len && base_lines[i] != ours_lines[i] && base_lines[i] == theirs_lines[i] {
                i += 1;
            }
            subchunks.push(Subchunk::Resolved(lines_to_text(&ours_lines[start..i])));
        } else if same_bo && !same_bt {
            // Only theirs changed from base.
            let start = i;
            while i < len && base_lines[i] == ours_lines[i] && base_lines[i] != theirs_lines[i] {
                i += 1;
            }
            subchunks.push(Subchunk::Resolved(lines_to_text(&theirs_lines[start..i])));
        } else if same_ot {
            // Both changed, same way.
            let start = i;
            while i < len && base_lines[i] != ours_lines[i] && ours_lines[i] == theirs_lines[i] {
                i += 1;
            }
            subchunks.push(Subchunk::Resolved(lines_to_text(&ours_lines[start..i])));
        } else {
            // Both changed differently → conflict.
            let start = i;
            while i < len
                && base_lines[i] != ours_lines[i]
                && base_lines[i] != theirs_lines[i]
                && ours_lines[i] != theirs_lines[i]
            {
                i += 1;
            }
            subchunks.push(Subchunk::Conflict {
                base: lines_to_text(&base_lines[start..i]),
                ours: lines_to_text(&ours_lines[start..i]),
                theirs: lines_to_text(&theirs_lines[start..i]),
            });
        }
    }

    subchunks
}

/// Merge two sets of line hunks (from base→ours and base→theirs diffs)
/// into a list of subchunks.
///
/// Non-overlapping single-side changes become `Resolved`. Overlapping changes
/// from both sides become `Conflict` (unless the replacement is identical or
/// the region can be further decomposed via per-line comparison).
/// Unchanged base regions become `Resolved` context.
fn merge_line_hunks(
    base_lines: &[&str],
    ours_hunks: &[LineHunk],
    theirs_hunks: &[LineHunk],
) -> Vec<Subchunk> {
    let mut result = Vec::new();
    let mut base_pos = 0usize;
    let mut oi = 0usize;
    let mut ti = 0usize;

    loop {
        let oh_start = ours_hunks
            .get(oi)
            .map(|h| h.base_start)
            .unwrap_or(usize::MAX);
        let th_start = theirs_hunks
            .get(ti)
            .map(|h| h.base_start)
            .unwrap_or(usize::MAX);

        if oh_start == usize::MAX && th_start == usize::MAX {
            // No more hunks — emit remaining base as context.
            if base_pos < base_lines.len() {
                result.push(Subchunk::Resolved(lines_to_text(&base_lines[base_pos..])));
            }
            break;
        }

        let change_start = oh_start.min(th_start);

        // Emit context (unchanged base lines) before the next change.
        if change_start > base_pos && base_pos < base_lines.len() {
            let ctx_end = change_start.min(base_lines.len());
            result.push(Subchunk::Resolved(lines_to_text(
                &base_lines[base_pos..ctx_end],
            )));
            base_pos = ctx_end;
        }

        // Expand the change region to include all overlapping hunks from both sides.
        // First consume hunks that start exactly at change_start (the trigger),
        // then expand with strictly overlapping hunks (base_start < region_end).
        // This prevents adjacent but non-overlapping hunks from being merged.
        let mut region_end = base_pos;
        let oi_start = oi;
        let ti_start = ti;

        // Consume initial hunks at change_start.
        while let Some(oh) = ours_hunks.get(oi) {
            if oh.base_start == change_start {
                region_end = region_end.max(oh.base_end);
                oi += 1;
            } else {
                break;
            }
        }
        while let Some(th) = theirs_hunks.get(ti) {
            if th.base_start == change_start {
                region_end = region_end.max(th.base_end);
                ti += 1;
            } else {
                break;
            }
        }

        // Expand with hunks that strictly overlap (start before region_end).
        loop {
            let mut extended = false;

            while let Some(oh) = ours_hunks.get(oi) {
                if oh.base_start < region_end {
                    region_end = region_end.max(oh.base_end);
                    oi += 1;
                    extended = true;
                } else {
                    break;
                }
            }

            while let Some(th) = theirs_hunks.get(ti) {
                if th.base_start < region_end {
                    region_end = region_end.max(th.base_end);
                    ti += 1;
                    extended = true;
                } else {
                    break;
                }
            }

            if !extended {
                break;
            }
        }

        let oi_end = oi;
        let ti_end = ti;
        let ours_involved = oi_end > oi_start;
        let theirs_involved = ti_end > ti_start;
        let region_base_end = region_end.min(base_lines.len());

        if ours_involved && theirs_involved {
            let base_text = lines_to_text(&base_lines[base_pos..region_base_end]);
            let ours_text = side_content(
                base_lines,
                base_pos,
                region_end,
                &ours_hunks[oi_start..oi_end],
            );
            let theirs_text = side_content(
                base_lines,
                base_pos,
                region_end,
                &theirs_hunks[ti_start..ti_end],
            );

            if ours_text == theirs_text {
                result.push(Subchunk::Resolved(ours_text));
            } else {
                // Try per-line decomposition of the overlapping region.
                let sub_base = split_lines(&base_text);
                let sub_ours = split_lines(&ours_text);
                let sub_theirs = split_lines(&theirs_text);

                if sub_base.len() == sub_ours.len() && sub_base.len() == sub_theirs.len() {
                    result.extend(per_line_merge(&sub_base, &sub_ours, &sub_theirs));
                } else {
                    result.push(Subchunk::Conflict {
                        base: base_text,
                        ours: ours_text,
                        theirs: theirs_text,
                    });
                }
            }
        } else if ours_involved {
            let ours_text = side_content(
                base_lines,
                base_pos,
                region_end,
                &ours_hunks[oi_start..oi_end],
            );
            result.push(Subchunk::Resolved(ours_text));
        } else if theirs_involved {
            let theirs_text = side_content(
                base_lines,
                base_pos,
                region_end,
                &theirs_hunks[ti_start..ti_end],
            );
            result.push(Subchunk::Resolved(theirs_text));
        }

        base_pos = region_end;
    }

    result
}

/// Reconstruct one side's content for a base line range, applying the given hunks.
///
/// Between hunks, base lines are kept unchanged. Hunk ranges provide
/// replacement content.
fn side_content(
    base_lines: &[&str],
    range_start: usize,
    range_end: usize,
    hunks: &[LineHunk],
) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut pos = range_start;

    for hunk in hunks {
        // Unchanged base lines before this hunk.
        let base_limit = hunk.base_start.min(range_end).min(base_lines.len());
        for p in pos..base_limit {
            lines.push(base_lines[p]);
        }
        // Hunk replacement content.
        for line in &hunk.new_lines {
            lines.push(line.as_str());
        }
        pos = hunk.base_end;
    }

    // Remaining base lines after last hunk.
    let tail_limit = range_end.min(base_lines.len());
    for p in pos..tail_limit {
        lines.push(base_lines[p]);
    }

    lines_to_text(&lines)
}

/// Join a slice of line strings into text with newline separators.
/// Each line gets a trailing newline (matching conflict block content convention).
fn lines_to_text(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let total: usize = lines.iter().map(|l| l.len() + 1).sum();
    let mut s = String::with_capacity(total);
    for line in lines {
        s.push_str(line);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_region(base: Option<&str>, ours: &str, theirs: &str) -> ConflictRegion {
        ConflictRegion {
            base: base.map(|s| s.to_string()),
            ours: ours.to_string(),
            theirs: theirs.to_string(),
            resolution: ConflictRegionResolution::Unresolved,
        }
    }

    fn make_session(regions: Vec<ConflictRegion>) -> ConflictSession {
        ConflictSession {
            path: PathBuf::from("test.txt"),
            conflict_kind: FileConflictKind::BothModified,
            strategy: ConflictResolverStrategy::FullTextResolver,
            base: ConflictPayload::Text("base\n".into()),
            ours: ConflictPayload::Text("ours\n".into()),
            theirs: ConflictPayload::Text("theirs\n".into()),
            regions,
        }
    }

    // -- ConflictPayload tests --

    #[test]
    fn payload_from_bytes_utf8() {
        let p = ConflictPayload::from_bytes(b"hello".to_vec());
        assert_eq!(p.as_text(), Some("hello"));
        assert!(!p.is_binary());
        assert!(!p.is_absent());
    }

    #[test]
    fn payload_from_bytes_binary() {
        let p = ConflictPayload::from_bytes(vec![0xFF, 0xFE, 0x00]);
        assert!(p.is_binary());
        assert!(p.as_text().is_none());
    }

    #[test]
    fn payload_absent() {
        let p = ConflictPayload::Absent;
        assert!(p.is_absent());
        assert!(p.as_text().is_none());
        assert!(!p.is_binary());
    }

    // -- ConflictRegionResolution tests --

    #[test]
    fn unresolved_is_not_resolved() {
        assert!(!ConflictRegionResolution::Unresolved.is_resolved());
    }

    #[test]
    fn all_pick_variants_are_resolved() {
        assert!(ConflictRegionResolution::PickBase.is_resolved());
        assert!(ConflictRegionResolution::PickOurs.is_resolved());
        assert!(ConflictRegionResolution::PickTheirs.is_resolved());
        assert!(ConflictRegionResolution::PickBoth.is_resolved());
        assert!(ConflictRegionResolution::ManualEdit("x".into()).is_resolved());
        assert!(
            ConflictRegionResolution::AutoResolved {
                rule: AutosolveRule::IdenticalSides,
                content: "x".into(),
            }
            .is_resolved()
        );
    }

    // -- ConflictRegion tests --

    #[test]
    fn resolved_text_for_picks() {
        let mut r = make_region(Some("base\n"), "ours\n", "theirs\n");

        r.resolution = ConflictRegionResolution::PickBase;
        assert_eq!(r.resolved_text(), Some("base\n"));

        r.resolution = ConflictRegionResolution::PickOurs;
        assert_eq!(r.resolved_text(), Some("ours\n"));

        r.resolution = ConflictRegionResolution::PickTheirs;
        assert_eq!(r.resolved_text(), Some("theirs\n"));

        r.resolution = ConflictRegionResolution::ManualEdit("custom\n".into());
        assert_eq!(r.resolved_text(), Some("custom\n"));
    }

    #[test]
    fn resolved_text_both_concatenates() {
        let r = make_region(Some("base\n"), "ours\n", "theirs\n");
        assert_eq!(r.resolved_text_both(), "ours\ntheirs\n");
    }

    #[test]
    fn resolved_text_unresolved_returns_none() {
        let r = make_region(Some("base\n"), "ours\n", "theirs\n");
        assert!(r.resolved_text().is_none());
    }

    // -- ConflictResolverStrategy tests --

    #[test]
    fn strategy_for_both_modified() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::BothModified, false),
            ConflictResolverStrategy::FullTextResolver,
        );
    }

    #[test]
    fn strategy_for_both_added() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::BothAdded, false),
            ConflictResolverStrategy::FullTextResolver,
        );
    }

    #[test]
    fn strategy_for_deleted_by_us() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByUs, false),
            ConflictResolverStrategy::TwoWayKeepDelete,
        );
    }

    #[test]
    fn strategy_for_deleted_by_them() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByThem, false),
            ConflictResolverStrategy::TwoWayKeepDelete,
        );
    }

    #[test]
    fn strategy_for_added_by_us() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::AddedByUs, false),
            ConflictResolverStrategy::TwoWayKeepDelete,
        );
    }

    #[test]
    fn strategy_for_added_by_them() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::AddedByThem, false),
            ConflictResolverStrategy::TwoWayKeepDelete,
        );
    }

    #[test]
    fn strategy_for_both_deleted() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::BothDeleted, false),
            ConflictResolverStrategy::DecisionOnly,
        );
    }

    #[test]
    fn strategy_binary_overrides_kind() {
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::BothModified, true),
            ConflictResolverStrategy::BinarySidePick,
        );
        assert_eq!(
            ConflictResolverStrategy::for_conflict(FileConflictKind::DeletedByUs, true),
            ConflictResolverStrategy::BinarySidePick,
        );
    }

    // -- ConflictSession counter & navigation tests --

    #[test]
    fn counters_all_unresolved() {
        let session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
            make_region(Some("b"), "p", "q"),
        ]);
        assert_eq!(session.total_regions(), 3);
        assert_eq!(session.solved_count(), 0);
        assert_eq!(session.unsolved_count(), 3);
        assert!(!session.is_fully_resolved());
    }

    #[test]
    fn counters_mixed_resolved() {
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
            make_region(Some("b"), "p", "q"),
        ]);
        session.regions[1].resolution = ConflictRegionResolution::PickOurs;
        assert_eq!(session.solved_count(), 1);
        assert_eq!(session.unsolved_count(), 2);
        assert!(!session.is_fully_resolved());
    }

    #[test]
    fn counters_all_resolved() {
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
        ]);
        session.regions[0].resolution = ConflictRegionResolution::PickOurs;
        session.regions[1].resolution = ConflictRegionResolution::PickTheirs;
        assert_eq!(session.solved_count(), 2);
        assert_eq!(session.unsolved_count(), 0);
        assert!(session.is_fully_resolved());
    }

    #[test]
    fn next_unresolved_wraps_around() {
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
            make_region(Some("b"), "p", "q"),
        ]);
        // Resolve regions 0 and 1, leave 2 unresolved.
        session.regions[0].resolution = ConflictRegionResolution::PickOurs;
        session.regions[1].resolution = ConflictRegionResolution::PickOurs;

        // From position 0, next unresolved should be 2.
        assert_eq!(session.next_unresolved_after(0), Some(2));
        // From position 2, should wrap to find none (2 is the current, only it's unresolved).
        // Actually from 2 it wraps: tries 0 (resolved), 1 (resolved), 2 (unresolved) -> Some(2).
        assert_eq!(session.next_unresolved_after(2), Some(2));
    }

    #[test]
    fn next_unresolved_returns_none_when_all_resolved() {
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
        ]);
        session.regions[0].resolution = ConflictRegionResolution::PickOurs;
        session.regions[1].resolution = ConflictRegionResolution::PickTheirs;
        assert_eq!(session.next_unresolved_after(0), None);
    }

    #[test]
    fn prev_unresolved_wraps_around() {
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
            make_region(Some("b"), "x", "y"),
            make_region(Some("b"), "p", "q"),
        ]);
        session.regions[1].resolution = ConflictRegionResolution::PickOurs;
        session.regions[2].resolution = ConflictRegionResolution::PickOurs;

        // From position 1, previous unresolved wraps to 0.
        assert_eq!(session.prev_unresolved_before(1), Some(0));
        // From position 0, should wrap: tries 2 (resolved), 1 (resolved), 0 (unresolved) -> Some(0).
        assert_eq!(session.prev_unresolved_before(0), Some(0));
    }

    #[test]
    fn navigation_empty_regions() {
        let session = make_session(vec![]);
        assert_eq!(session.next_unresolved_after(0), None);
        assert_eq!(session.prev_unresolved_before(0), None);
    }

    // -- Auto-resolve tests --

    #[test]
    fn auto_resolve_identical_sides() {
        let region = make_region(Some("base\n"), "same\n", "same\n");
        let result = safe_auto_resolve(&region);
        assert!(result.is_some());
        let (rule, content) = result.unwrap();
        assert_eq!(rule, AutosolveRule::IdenticalSides);
        assert_eq!(content, "same\n");

        // Verify it works via session.
        let mut session = make_session(vec![region.clone()]);
        assert_eq!(session.auto_resolve_safe(), 1);
        assert!(session.is_fully_resolved());
    }

    #[test]
    fn auto_resolve_only_ours_changed() {
        let region = make_region(Some("base\n"), "changed\n", "base\n");
        let result = safe_auto_resolve(&region);
        assert!(result.is_some());
        let (rule, content) = result.unwrap();
        assert_eq!(rule, AutosolveRule::OnlyOursChanged);
        assert_eq!(content, "changed\n");
    }

    #[test]
    fn auto_resolve_only_theirs_changed() {
        let region = make_region(Some("base\n"), "base\n", "changed\n");
        let result = safe_auto_resolve(&region);
        assert!(result.is_some());
        let (rule, content) = result.unwrap();
        assert_eq!(rule, AutosolveRule::OnlyTheirsChanged);
        assert_eq!(content, "changed\n");
    }

    #[test]
    fn auto_resolve_both_changed_differently_returns_none() {
        let region = make_region(Some("base\n"), "ours\n", "theirs\n");
        assert!(safe_auto_resolve(&region).is_none());
    }

    #[test]
    fn auto_resolve_no_base_both_different_returns_none() {
        let region = make_region(None, "ours\n", "theirs\n");
        assert!(safe_auto_resolve(&region).is_none());
    }

    #[test]
    fn auto_resolve_no_base_identical_sides() {
        let region = make_region(None, "same\n", "same\n");
        let result = safe_auto_resolve(&region);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, AutosolveRule::IdenticalSides);
    }

    #[test]
    fn auto_resolve_session_multiple_regions() {
        let mut session = make_session(vec![
            make_region(Some("base\n"), "same\n", "same\n"), // identical → auto
            make_region(Some("base\n"), "ours\n", "theirs\n"), // both changed → no auto
            make_region(Some("base\n"), "changed\n", "base\n"), // only ours → auto
        ]);
        let resolved = session.auto_resolve_safe();
        assert_eq!(resolved, 2);
        assert_eq!(session.solved_count(), 2);
        assert_eq!(session.unsolved_count(), 1);
        assert!(!session.is_fully_resolved());

        // Region 0: auto-resolved
        assert!(matches!(
            session.regions[0].resolution,
            ConflictRegionResolution::AutoResolved {
                rule: AutosolveRule::IdenticalSides,
                ..
            }
        ));
        // Region 1: still unresolved
        assert!(matches!(
            session.regions[1].resolution,
            ConflictRegionResolution::Unresolved
        ));
        // Region 2: auto-resolved
        assert!(matches!(
            session.regions[2].resolution,
            ConflictRegionResolution::AutoResolved {
                rule: AutosolveRule::OnlyOursChanged,
                ..
            }
        ));
    }

    #[test]
    fn auto_resolve_skips_already_resolved() {
        let mut session = make_session(vec![make_region(Some("base\n"), "same\n", "same\n")]);
        // Manually resolve first.
        session.regions[0].resolution = ConflictRegionResolution::PickOurs;
        // Auto-resolve should skip it.
        let resolved = session.auto_resolve_safe();
        assert_eq!(resolved, 0);
        // Still manually resolved, not overwritten.
        assert!(matches!(
            session.regions[0].resolution,
            ConflictRegionResolution::PickOurs
        ));
    }

    #[test]
    fn regex_auto_resolve_equivalent_sides() {
        let options = RegexAutosolveOptions::whitespace_insensitive();
        let decision = regex_assisted_auto_resolve_pick(
            Some("let answer = 42;\n"),
            "let  answer = 42;\n",
            "let answer\t=\t42;\n",
            &options,
        );
        assert_eq!(
            decision,
            Some((AutosolveRule::RegexEquivalentSides, AutosolvePickSide::Ours))
        );
    }

    #[test]
    fn regex_auto_resolve_only_theirs_changed_from_normalized_base() {
        let options = RegexAutosolveOptions::whitespace_insensitive();
        let decision = regex_assisted_auto_resolve_pick(
            Some("let answer = 42;\n"),
            "let answer=42;\n",
            "let answer = 43;\n",
            &options,
        );
        assert_eq!(
            decision,
            Some((
                AutosolveRule::RegexOnlyTheirsChanged,
                AutosolvePickSide::Theirs
            ))
        );
    }

    #[test]
    fn regex_auto_resolve_only_ours_changed_from_normalized_base() {
        let options = RegexAutosolveOptions::whitespace_insensitive();
        let decision = regex_assisted_auto_resolve_pick(
            Some("let answer = 42;\n"),
            "let answer = 43;\n",
            "let\tanswer=42;\n",
            &options,
        );
        assert_eq!(
            decision,
            Some((AutosolveRule::RegexOnlyOursChanged, AutosolvePickSide::Ours))
        );
    }

    #[test]
    fn regex_auto_resolve_invalid_pattern_is_ignored() {
        let options = RegexAutosolveOptions::default().with_pattern("(", "");
        let decision =
            regex_assisted_auto_resolve_pick(Some("base\n"), "ours\n", "theirs\n", &options);
        assert!(decision.is_none());
    }

    #[test]
    fn session_auto_resolve_regex_applies_to_unresolved_regions() {
        let mut session = make_session(vec![
            make_region(
                Some("let answer = 42;\n"),
                "let  answer = 42;\n",
                "let answer\t=\t42;\n",
            ),
            make_region(Some("base\n"), "ours\n", "theirs\n"),
        ]);
        let options = RegexAutosolveOptions::whitespace_insensitive();

        assert_eq!(session.auto_resolve_regex(&options), 1);
        assert_eq!(session.solved_count(), 1);
        assert_eq!(session.unsolved_count(), 1);
        match &session.regions[0].resolution {
            ConflictRegionResolution::AutoResolved { rule, content } => {
                assert_eq!(*rule, AutosolveRule::RegexEquivalentSides);
                assert_eq!(content, "let  answer = 42;\n");
            }
            other => panic!("expected regex auto-resolved region, got {:?}", other),
        }
        assert!(matches!(
            session.regions[1].resolution,
            ConflictRegionResolution::Unresolved
        ));
    }

    // -- ConflictSession::new tests --

    #[test]
    fn session_new_text_conflict() {
        let session = ConflictSession::new(
            PathBuf::from("file.txt"),
            FileConflictKind::BothModified,
            ConflictPayload::Text("base".into()),
            ConflictPayload::Text("ours".into()),
            ConflictPayload::Text("theirs".into()),
        );
        assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
        assert_eq!(session.total_regions(), 0); // No regions parsed yet
    }

    #[test]
    fn session_new_binary_conflict() {
        let session = ConflictSession::new(
            PathBuf::from("image.png"),
            FileConflictKind::BothModified,
            ConflictPayload::Binary(vec![0xFF]),
            ConflictPayload::Text("ours".into()),
            ConflictPayload::Text("theirs".into()),
        );
        assert_eq!(session.strategy, ConflictResolverStrategy::BinarySidePick);
    }

    #[test]
    fn session_new_deleted_by_us() {
        let session = ConflictSession::new(
            PathBuf::from("file.txt"),
            FileConflictKind::DeletedByUs,
            ConflictPayload::Text("base".into()),
            ConflictPayload::Absent,
            ConflictPayload::Text("theirs".into()),
        );
        assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    }

    #[test]
    fn session_new_both_deleted() {
        let session = ConflictSession::new(
            PathBuf::from("file.txt"),
            FileConflictKind::BothDeleted,
            ConflictPayload::Text("base".into()),
            ConflictPayload::Absent,
            ConflictPayload::Absent,
        );
        assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    }

    #[test]
    fn has_unresolved_markers_reflects_unsolved() {
        let mut session = make_session(vec![make_region(Some("b"), "a", "c")]);
        assert!(session.has_unresolved_markers());
        session.regions[0].resolution = ConflictRegionResolution::PickOurs;
        assert!(!session.has_unresolved_markers());
    }

    // -- AutosolveRule description test --

    #[test]
    fn autosolve_rule_descriptions() {
        assert!(!AutosolveRule::IdenticalSides.description().is_empty());
        assert!(!AutosolveRule::OnlyOursChanged.description().is_empty());
        assert!(!AutosolveRule::OnlyTheirsChanged.description().is_empty());
        assert!(!AutosolveRule::RegexEquivalentSides.description().is_empty());
        assert!(
            !AutosolveRule::RegexOnlyTheirsChanged
                .description()
                .is_empty()
        );
        assert!(!AutosolveRule::RegexOnlyOursChanged.description().is_empty());
        assert!(!AutosolveRule::SubchunkFullyMerged.description().is_empty());
    }

    // -- Pass 2: subchunk splitting tests --

    #[test]
    fn subchunk_split_identical_sides_returns_none() {
        // Pass 1 handles this — don't split.
        assert!(split_conflict_into_subchunks("base\n", "same\n", "same\n").is_none());
    }

    #[test]
    fn subchunk_split_ours_equals_base_returns_none() {
        // Pass 1 handles this.
        assert!(split_conflict_into_subchunks("base\n", "base\n", "changed\n").is_none());
    }

    #[test]
    fn subchunk_split_theirs_equals_base_returns_none() {
        // Pass 1 handles this.
        assert!(split_conflict_into_subchunks("base\n", "changed\n", "base\n").is_none());
    }

    #[test]
    fn subchunk_split_single_line_conflict_returns_none() {
        // Both sides changed the only line — no way to split meaningfully.
        assert!(split_conflict_into_subchunks("original\n", "ours\n", "theirs\n").is_none());
    }

    #[test]
    fn subchunk_split_mixed_lines() {
        // Base has 3 lines. Ours changes line 1, theirs changes line 3.
        // Line 2 is the same across all three → context.
        let base = "aaa\nbbb\nccc\n";
        let ours = "AAA\nbbb\nccc\n";
        let theirs = "aaa\nbbb\nCCC\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs);
        assert!(subchunks.is_some(), "should split into subchunks");
        let subchunks = subchunks.unwrap();

        // All subchunks should be resolved because changes don't overlap.
        assert!(
            subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))),
            "non-overlapping changes should all auto-merge"
        );

        // Concatenated resolved text should be the merged result.
        let merged: String = subchunks
            .iter()
            .map(|c| match c {
                Subchunk::Resolved(t) => t.as_str(),
                _ => panic!("unexpected conflict"),
            })
            .collect();
        assert_eq!(merged, "AAA\nbbb\nCCC\n");
    }

    #[test]
    fn subchunk_split_with_remaining_conflict() {
        // Both sides change the same line (line 2), different changes on line 1.
        let base = "aaa\nbbb\nccc\n";
        let ours = "AAA\nBBB\nccc\n";
        let theirs = "XXX\nYYY\nccc\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs);
        assert!(subchunks.is_some(), "should split");
        let subchunks = subchunks.unwrap();

        let has_resolved = subchunks.iter().any(|c| matches!(c, Subchunk::Resolved(_)));
        let has_conflict = subchunks
            .iter()
            .any(|c| matches!(c, Subchunk::Conflict { .. }));
        assert!(has_resolved, "should have resolved parts (line 3)");
        assert!(has_conflict, "should have conflicting parts (lines 1-2)");
    }

    #[test]
    fn subchunk_split_only_one_side_adds_lines() {
        // Ours adds a line, theirs doesn't change anything.
        // But theirs != base overall, so this is a genuine 3-way conflict.
        let base = "aaa\nccc\n";
        let ours = "aaa\nbbb\nccc\n";
        let theirs = "aaa\nCCC\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs);
        assert!(subchunks.is_some());
        let subchunks = subchunks.unwrap();

        // Should have context "aaa\n" resolved, then a conflict for the rest.
        let first = &subchunks[0];
        assert!(
            matches!(first, Subchunk::Resolved(t) if t == "aaa\n"),
            "first subchunk should be resolved context"
        );
    }

    #[test]
    fn subchunk_split_both_change_same_line_identically() {
        // Both sides change line 2 the same way.
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nBBB\nccc\n";
        let theirs = "aaa\nBBB\nccc\n";

        // This would be caught by Pass 1 (ours == theirs), returns None.
        assert!(split_conflict_into_subchunks(base, ours, theirs).is_none());
    }

    #[test]
    fn subchunk_split_nonoverlapping_changes_fully_merge() {
        // Ours changes line 1, theirs changes line 3. Line 2 is context.
        let base = "line1\nline2\nline3\n";
        let ours = "LINE1\nline2\nline3\n";
        let theirs = "line1\nline2\nLINE3\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs).unwrap();

        // Should be fully resolved.
        assert!(subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))));

        let merged: String = subchunks
            .iter()
            .map(|c| match c {
                Subchunk::Resolved(t) => t.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(merged, "LINE1\nline2\nLINE3\n");
    }

    #[test]
    fn subchunk_split_overlapping_different_changes_conflict() {
        // Both sides change the same line differently.
        let base = "ctx\noriginal\nctx2\n";
        let ours = "ctx\nours_version\nctx2\n";
        let theirs = "ctx\ntheirs_version\nctx2\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs).unwrap();

        // Should have context + conflict + context.
        assert_eq!(subchunks.len(), 3);
        assert!(matches!(&subchunks[0], Subchunk::Resolved(t) if t == "ctx\n"));
        assert!(
            matches!(&subchunks[1], Subchunk::Conflict { base, ours, theirs }
                if base == "original\n" && ours == "ours_version\n" && theirs == "theirs_version\n"
            )
        );
        assert!(matches!(&subchunks[2], Subchunk::Resolved(t) if t == "ctx2\n"));
    }

    #[test]
    fn subchunk_session_pass2_fully_merges() {
        let mut session = make_session(vec![ConflictRegion {
            base: Some("line1\nline2\nline3\n".into()),
            ours: "LINE1\nline2\nline3\n".into(),
            theirs: "line1\nline2\nLINE3\n".into(),
            resolution: ConflictRegionResolution::Unresolved,
        }]);

        // Pass 1 can't resolve this (both sides changed differently from base).
        assert_eq!(session.auto_resolve_safe(), 0);

        // Pass 2 should fully merge it (non-overlapping changes).
        assert_eq!(session.auto_resolve_pass2(), 1);
        assert!(session.is_fully_resolved());

        match &session.regions[0].resolution {
            ConflictRegionResolution::AutoResolved { rule, content } => {
                assert_eq!(*rule, AutosolveRule::SubchunkFullyMerged);
                assert_eq!(content, "LINE1\nline2\nLINE3\n");
            }
            other => panic!("expected AutoResolved, got {:?}", other),
        }
    }

    #[test]
    fn subchunk_session_pass2_skips_partial_conflicts() {
        let mut session = make_session(vec![ConflictRegion {
            base: Some("ctx\noriginal\nctx2\n".into()),
            ours: "ctx\nours_version\nctx2\n".into(),
            theirs: "ctx\ntheirs_version\nctx2\n".into(),
            resolution: ConflictRegionResolution::Unresolved,
        }]);

        // Pass 2 can't fully merge (overlap on line 2), so region stays unresolved.
        assert_eq!(session.auto_resolve_pass2(), 0);
        assert!(!session.is_fully_resolved());
    }

    #[test]
    fn subchunk_split_empty_base() {
        // Empty base, both sides have content.
        let base = "";
        let ours = "aaa\n";
        let theirs = "bbb\n";

        // Both sides differ from base and from each other.
        let result = split_conflict_into_subchunks(base, ours, theirs);
        // Can't meaningfully split an empty base with different insertions.
        assert!(result.is_none());
    }

    #[test]
    fn subchunk_split_with_deletions() {
        // Ours deletes line 2, theirs changes line 3.
        let base = "aaa\nbbb\nccc\n";
        let ours = "aaa\nccc\n";
        let theirs = "aaa\nbbb\nCCC\n";

        let subchunks = split_conflict_into_subchunks(base, ours, theirs);
        assert!(subchunks.is_some());
        let subchunks = subchunks.unwrap();

        // Should be fully resolved: non-overlapping changes.
        assert!(subchunks.iter().all(|c| matches!(c, Subchunk::Resolved(_))));

        let merged: String = subchunks
            .iter()
            .map(|c| match c {
                Subchunk::Resolved(t) => t.as_str(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(merged, "aaa\nCCC\n");
    }
}
