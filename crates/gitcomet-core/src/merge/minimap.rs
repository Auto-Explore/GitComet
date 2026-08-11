//! kdiff3-style minimap column classification.
//!
//! Port of `Overview::drawColumn` from kdiff3's `Overview.cpp`. kdiff3 colors
//! one pixel band per aligned line, choosing the color from that line's merge
//! details. Our aligned runs already carry the equivalent classification
//! ([`AlignedRunKind`]), so the port reduces to a mapping from run kind to
//! color role.
//!
//! kdiff3's pairwise overview modes (A-B / A-C / B-C) are not ported: the
//! column always shows the merge itself. Whitespace-only changes are not
//! dithered the way kdiff3 does either — that behavior is tied to kdiff3's
//! global "show white space" option, which has no equivalent here.

use super::{AlignedRun, AlignedRunKind};

/// Color role for one minimap row.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MinimapRowKind {
    /// Nothing to show: painted as plain background.
    #[default]
    Unchanged,
    /// Only the local side changed here.
    LocalChanged,
    /// Only the remote side changed here.
    RemoteChanged,
    /// A conflict the user has already resolved.
    ResolvedConflict,
    /// Both sides changed and the conflict is still open.
    Conflict,
}

impl MinimapRowKind {
    /// Merge priority when several rows collapse into one painted band.
    ///
    /// Mirrors kdiff3's `oldConflictY` guard, which keeps a conflict band from
    /// being overpainted by a later non-conflict line. A still-open conflict
    /// outranks a resolved one so a band never hides remaining work.
    fn priority(self) -> u8 {
        match self {
            Self::Unchanged => 0,
            Self::LocalChanged | Self::RemoteChanged => 1,
            Self::ResolvedConflict => 2,
            Self::Conflict => 3,
        }
    }

    /// Combine two row kinds sharing one band, keeping the more significant.
    ///
    /// Two different single-side changes in the same band read as a conflict,
    /// because the band can only carry one color.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::LocalChanged, Self::RemoteChanged)
            | (Self::RemoteChanged, Self::LocalChanged) => Self::Conflict,
            (a, b) if a.priority() >= b.priority() => a,
            (_, b) => b,
        }
    }

    /// The resolved counterpart of this kind, for rows inside a conflict the
    /// user has settled. Only conflict rows have one.
    pub fn resolved(self) -> Self {
        match self {
            Self::Conflict => Self::ResolvedConflict,
            other => other,
        }
    }
}

/// Classify one aligned run.
///
/// kdiff3 `eOMNormal`: single-side changes take that side's color, everything
/// where both sides moved takes the conflict color — including the "both
/// changed the same way" cases, which kdiff3 groups with the conflicts
/// (eBCChangedAndEqual, eBCDeleted, eBCAddedAndEqual).
pub fn minimap_row_kind(run: AlignedRunKind) -> MinimapRowKind {
    match run {
        AlignedRunKind::Unchanged => MinimapRowKind::Unchanged,
        AlignedRunKind::OursChanged => MinimapRowKind::LocalChanged,
        AlignedRunKind::TheirsChanged => MinimapRowKind::RemoteChanged,
        AlignedRunKind::BothSame | AlignedRunKind::Conflict => MinimapRowKind::Conflict,
    }
}

/// Expand aligned runs into one row kind per visual row.
///
/// Callers that only need banded output should prefer walking the runs
/// directly; this is the straightforward form used by tests and small inputs.
pub fn minimap_rows(runs: &[AlignedRun]) -> Vec<MinimapRowKind> {
    let mut out = Vec::with_capacity(runs.iter().map(AlignedRun::visual_rows).sum());
    for run in runs {
        let kind = minimap_row_kind(run.kind);
        out.extend(std::iter::repeat_n(kind, run.visual_rows()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::{DiffAlgorithm, align_three_way};

    fn kinds(base: &str, ours: &str, theirs: &str) -> Vec<MinimapRowKind> {
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        minimap_rows(&runs)
    }

    #[test]
    fn each_side_change_gets_its_own_color() {
        let rows = kinds("a\nb\nc\n", "a\nB\nc\n", "a\nb\nC\n");
        assert_eq!(
            rows,
            vec![
                MinimapRowKind::Unchanged,
                MinimapRowKind::LocalChanged,
                MinimapRowKind::RemoteChanged,
            ]
        );
    }

    #[test]
    fn diverging_edits_are_a_conflict() {
        let rows = kinds("a\nb\n", "a\nX\n", "a\nY\n");
        assert_eq!(rows[1], MinimapRowKind::Conflict);
    }

    #[test]
    fn identical_edits_are_a_conflict_like_kdiff3() {
        // kdiff3 groups eBCChangedAndEqual with the conflict colors even though
        // the merge resolves automatically.
        let rows = kinds("a\nb\n", "a\nX\n", "a\nX\n");
        assert_eq!(rows[1], MinimapRowKind::Conflict);
    }

    #[test]
    fn band_merge_keeps_the_most_significant_kind() {
        use MinimapRowKind::*;
        assert_eq!(Unchanged.merge(LocalChanged), LocalChanged);
        assert_eq!(LocalChanged.merge(Unchanged), LocalChanged);
        assert_eq!(LocalChanged.merge(Conflict), Conflict);
        assert_eq!(Conflict.merge(Unchanged), Conflict);
        assert_eq!(LocalChanged.merge(LocalChanged), LocalChanged);
        // Opposing single-side changes cannot share one color.
        assert_eq!(LocalChanged.merge(RemoteChanged), Conflict);
    }

    #[test]
    fn an_open_conflict_outranks_a_resolved_one_in_a_shared_band() {
        use MinimapRowKind::*;
        assert_eq!(ResolvedConflict.merge(Conflict), Conflict);
        assert_eq!(Conflict.merge(ResolvedConflict), Conflict);
        // ...but it still outranks the one-sided changes.
        assert_eq!(ResolvedConflict.merge(LocalChanged), ResolvedConflict);
        assert_eq!(RemoteChanged.merge(ResolvedConflict), ResolvedConflict);
    }

    #[test]
    fn only_conflicts_have_a_resolved_counterpart() {
        use MinimapRowKind::*;
        assert_eq!(Conflict.resolved(), ResolvedConflict);
        assert_eq!(LocalChanged.resolved(), LocalChanged);
        assert_eq!(Unchanged.resolved(), Unchanged);
    }

    #[test]
    fn rows_expand_to_one_entry_per_visual_row() {
        // A one-line base replaced by three local lines occupies three rows.
        let runs = align_three_way("a\n", "x\ny\nz\n", "a\n", DiffAlgorithm::Myers);
        let rows = minimap_rows(&runs);
        assert_eq!(
            rows.len(),
            runs.iter().map(AlignedRun::visual_rows).sum::<usize>()
        );
    }
}
