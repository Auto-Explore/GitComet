//! kdiff3-style overview (minimap) column classification.
//!
//! Port of `Overview::drawColumn` from kdiff3's `Overview.cpp`. kdiff3 colors
//! one pixel band per aligned line, choosing the color from that line's merge
//! details and the active overview mode. Our aligned runs already carry the
//! equivalent classification ([`AlignedRunKind`]), so the port reduces to a
//! mode-dependent mapping from run kind to color role.
//!
//! Whitespace-only changes are not dithered the way kdiff3 does: that behavior
//! is tied to kdiff3's global "show white space" option, which has no
//! equivalent here.

use super::{AlignedRun, AlignedRunKind};

/// Which comparison the overview column visualizes.
///
/// kdiff3 calls these Normal / A-B / A-C / B-C, where A is the base, B the
/// local side and C the remote side.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OverviewMode {
    /// The merge itself: each side's own changes in its own color, conflicts
    /// in the conflict color (kdiff3 `eOMNormal`).
    #[default]
    Merge,
    /// Every row where local differs from base (kdiff3 `eOMAvsB`).
    BaseVsLocal,
    /// Every row where remote differs from base (kdiff3 `eOMAvsC`).
    BaseVsRemote,
    /// Every row where local differs from remote (kdiff3 `eOMBvsC`).
    LocalVsRemote,
}

impl OverviewMode {
    /// Short column label, matching the A/B/C names the resolver headers use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::BaseVsLocal => "A-B",
            Self::BaseVsRemote => "A-C",
            Self::LocalVsRemote => "B-C",
        }
    }

    /// Modes in the order kdiff3 lists them.
    pub const ALL: [Self; 4] = [
        Self::Merge,
        Self::BaseVsLocal,
        Self::BaseVsRemote,
        Self::LocalVsRemote,
    ];
}

/// Color role for one overview row.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OverviewRowKind {
    /// Nothing to show: painted as plain background.
    #[default]
    Unchanged,
    /// Only the local side changed here.
    LocalChanged,
    /// Only the remote side changed here.
    RemoteChanged,
    /// Both sides changed, or the active pairwise comparison differs.
    Conflict,
}

impl OverviewRowKind {
    /// Merge priority when several rows collapse into one painted band.
    ///
    /// Mirrors kdiff3's `oldConflictY` guard, which keeps a conflict band from
    /// being overpainted by a later non-conflict line.
    fn priority(self) -> u8 {
        match self {
            Self::Unchanged => 0,
            Self::LocalChanged | Self::RemoteChanged => 1,
            Self::Conflict => 2,
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
}

/// Classify one aligned run for the given overview mode.
pub fn overview_row_kind(run: AlignedRunKind, mode: OverviewMode) -> OverviewRowKind {
    use AlignedRunKind as Run;
    use OverviewRowKind as Kind;

    match mode {
        // kdiff3 eOMNormal: single-side changes take that side's color,
        // everything where both sides moved takes the conflict color —
        // including the "both changed the same way" cases, which kdiff3 groups
        // with the conflicts (eBCChangedAndEqual, eBCDeleted, eBCAddedAndEqual).
        OverviewMode::Merge => match run {
            Run::Unchanged => Kind::Unchanged,
            Run::OursChanged => Kind::LocalChanged,
            Run::TheirsChanged => Kind::RemoteChanged,
            Run::BothSame | Run::Conflict => Kind::Conflict,
        },
        // kdiff3 eOMAvsB: everything except no-change and remote-only edits.
        OverviewMode::BaseVsLocal => match run {
            Run::Unchanged | Run::TheirsChanged => Kind::Unchanged,
            Run::OursChanged | Run::BothSame | Run::Conflict => Kind::Conflict,
        },
        // kdiff3 eOMAvsC: everything except no-change and local-only edits.
        OverviewMode::BaseVsRemote => match run {
            Run::Unchanged | Run::OursChanged => Kind::Unchanged,
            Run::TheirsChanged | Run::BothSame | Run::Conflict => Kind::Conflict,
        },
        // kdiff3 eOMBvsC: the sides agree in Unchanged and BothSame, so only
        // one-sided edits and real conflicts differ between them.
        OverviewMode::LocalVsRemote => match run {
            Run::Unchanged | Run::BothSame => Kind::Unchanged,
            Run::OursChanged | Run::TheirsChanged | Run::Conflict => Kind::Conflict,
        },
    }
}

/// Expand aligned runs into one row kind per visual row.
///
/// Callers that only need banded output should prefer walking the runs
/// directly; this is the straightforward form used by tests and small inputs.
pub fn overview_rows(runs: &[AlignedRun], mode: OverviewMode) -> Vec<OverviewRowKind> {
    let mut out = Vec::with_capacity(runs.iter().map(AlignedRun::visual_rows).sum());
    for run in runs {
        let kind = overview_row_kind(run.kind, mode);
        out.extend(std::iter::repeat_n(kind, run.visual_rows()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::{DiffAlgorithm, align_three_way};

    fn kinds(base: &str, ours: &str, theirs: &str, mode: OverviewMode) -> Vec<OverviewRowKind> {
        let runs = align_three_way(base, ours, theirs, DiffAlgorithm::Myers);
        overview_rows(&runs, mode)
    }

    #[test]
    fn merge_mode_colors_each_side_change_separately() {
        let rows = kinds("a\nb\nc\n", "a\nB\nc\n", "a\nb\nC\n", OverviewMode::Merge);
        assert_eq!(
            rows,
            vec![
                OverviewRowKind::Unchanged,
                OverviewRowKind::LocalChanged,
                OverviewRowKind::RemoteChanged,
            ]
        );
    }

    #[test]
    fn merge_mode_marks_diverging_edits_as_conflict() {
        let rows = kinds("a\nb\n", "a\nX\n", "a\nY\n", OverviewMode::Merge);
        assert_eq!(rows[1], OverviewRowKind::Conflict);
    }

    #[test]
    fn merge_mode_marks_identical_edits_as_conflict_like_kdiff3() {
        // kdiff3 groups eBCChangedAndEqual with the conflict colors even though
        // the merge resolves automatically.
        let rows = kinds("a\nb\n", "a\nX\n", "a\nX\n", OverviewMode::Merge);
        assert_eq!(rows[1], OverviewRowKind::Conflict);
    }

    #[test]
    fn pairwise_modes_hide_the_side_they_do_not_compare() {
        let base = "a\nb\nc\n";
        let ours = "a\nB\nc\n";
        let theirs = "a\nb\nC\n";

        let ab = kinds(base, ours, theirs, OverviewMode::BaseVsLocal);
        assert_eq!(ab[1], OverviewRowKind::Conflict);
        assert_eq!(ab[2], OverviewRowKind::Unchanged);

        let ac = kinds(base, ours, theirs, OverviewMode::BaseVsRemote);
        assert_eq!(ac[1], OverviewRowKind::Unchanged);
        assert_eq!(ac[2], OverviewRowKind::Conflict);
    }

    #[test]
    fn local_vs_remote_hides_edits_the_sides_agree_on() {
        // Both sides made the same edit: identical to each other, so B-C is clean.
        let rows = kinds("a\nb\n", "a\nX\n", "a\nX\n", OverviewMode::LocalVsRemote);
        assert!(rows.iter().all(|kind| *kind == OverviewRowKind::Unchanged));

        // One-sided edit: the sides differ, so B-C shows it.
        let rows = kinds("a\nb\n", "a\nX\n", "a\nb\n", OverviewMode::LocalVsRemote);
        assert_eq!(rows[1], OverviewRowKind::Conflict);
    }

    #[test]
    fn band_merge_keeps_the_most_significant_kind() {
        use OverviewRowKind::*;
        assert_eq!(Unchanged.merge(LocalChanged), LocalChanged);
        assert_eq!(LocalChanged.merge(Unchanged), LocalChanged);
        assert_eq!(LocalChanged.merge(Conflict), Conflict);
        assert_eq!(Conflict.merge(Unchanged), Conflict);
        assert_eq!(LocalChanged.merge(LocalChanged), LocalChanged);
        // Opposing single-side changes cannot share one color.
        assert_eq!(LocalChanged.merge(RemoteChanged), Conflict);
    }

    #[test]
    fn rows_expand_to_one_entry_per_visual_row() {
        // A one-line base replaced by three local lines occupies three rows.
        let runs = align_three_way("a\n", "x\ny\nz\n", "a\n", DiffAlgorithm::Myers);
        let rows = overview_rows(&runs, OverviewMode::Merge);
        assert_eq!(
            rows.len(),
            runs.iter().map(AlignedRun::visual_rows).sum::<usize>()
        );
    }
}
