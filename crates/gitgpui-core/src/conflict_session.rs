use crate::domain::FileConflictKind;
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
}

impl AutosolveRule {
    pub fn description(&self) -> &'static str {
        match self {
            AutosolveRule::IdenticalSides => "both sides identical",
            AutosolveRule::OnlyOursChanged => "only ours changed from base",
            AutosolveRule::OnlyTheirsChanged => "only theirs changed from base",
        }
    }
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
            ConflictRegionResolution::PickBase => {
                self.base.as_deref().or(Some(""))
            }
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
        let is_binary =
            base.is_binary() || ours.is_binary() || theirs.is_binary();
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
                region.resolution = ConflictRegionResolution::AutoResolved {
                    rule,
                    content,
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
        assert!(ConflictRegionResolution::AutoResolved {
            rule: AutosolveRule::IdenticalSides,
            content: "x".into(),
        }
        .is_resolved());
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
            make_region(Some("base\n"), "same\n", "same\n"),      // identical → auto
            make_region(Some("base\n"), "ours\n", "theirs\n"),    // both changed → no auto
            make_region(Some("base\n"), "changed\n", "base\n"),   // only ours → auto
        ]);
        let resolved = session.auto_resolve_safe();
        assert_eq!(resolved, 2);
        assert_eq!(session.solved_count(), 2);
        assert_eq!(session.unsolved_count(), 1);
        assert!(!session.is_fully_resolved());

        // Region 0: auto-resolved
        assert!(matches!(
            session.regions[0].resolution,
            ConflictRegionResolution::AutoResolved { rule: AutosolveRule::IdenticalSides, .. }
        ));
        // Region 1: still unresolved
        assert!(matches!(
            session.regions[1].resolution,
            ConflictRegionResolution::Unresolved
        ));
        // Region 2: auto-resolved
        assert!(matches!(
            session.regions[2].resolution,
            ConflictRegionResolution::AutoResolved { rule: AutosolveRule::OnlyOursChanged, .. }
        ));
    }

    #[test]
    fn auto_resolve_skips_already_resolved() {
        let mut session = make_session(vec![
            make_region(Some("base\n"), "same\n", "same\n"),
        ]);
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
        let mut session = make_session(vec![
            make_region(Some("b"), "a", "c"),
        ]);
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
    }
}
