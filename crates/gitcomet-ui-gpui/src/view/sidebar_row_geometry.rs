//! Row offsets for the branch sidebar list, whose rows come in two heights.
use super::branch_sidebar::{BranchSection, BranchSidebarRow};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SidebarRowClass {
    Short,
    Tall,
}

impl BranchSidebarRow {
    /// Depends only on the variant, so arriving results never resize a row.
    pub(super) fn height_class(&self) -> SidebarRowClass {
        match self {
            BranchSidebarRow::Branch {
                section: BranchSection::Local,
                ..
            }
            | BranchSidebarRow::WorktreeItem { .. } => SidebarRowClass::Tall,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                ..
            }
            | BranchSidebarRow::ContributionHeader { .. }
            | BranchSidebarRow::ContributionItem { .. }
            | BranchSidebarRow::SectionHeader { .. }
            | BranchSidebarRow::SectionSpacer
            | BranchSidebarRow::Placeholder { .. }
            | BranchSidebarRow::RemoteHeader { .. }
            | BranchSidebarRow::GroupHeader { .. }
            | BranchSidebarRow::WorktreesHeader { .. }
            | BranchSidebarRow::WorktreePlaceholder { .. }
            | BranchSidebarRow::SubmodulesHeader { .. }
            | BranchSidebarRow::SubmodulePlaceholder { .. }
            | BranchSidebarRow::SubmoduleItem { .. }
            | BranchSidebarRow::AnnexHeader { .. }
            | BranchSidebarRow::AnnexPlaceholder { .. }
            | BranchSidebarRow::AnnexRepositoryItem { .. }
            | BranchSidebarRow::StashHeader { .. }
            | BranchSidebarRow::StashPlaceholder { .. }
            | BranchSidebarRow::StashItem { .. } => SidebarRowClass::Short,
        }
    }
}

/// Tall rows before each index, plus one entry for the end of the list.
#[derive(Debug)]
pub(super) struct SidebarRowClasses {
    tall_before: Box<[u32]>,
}

impl Default for SidebarRowClasses {
    fn default() -> Self {
        Self {
            tall_before: Box::new([0]),
        }
    }
}

impl SidebarRowClasses {
    pub(super) fn new(rows: &[BranchSidebarRow]) -> Self {
        let mut tall = 0;
        let tall_before = std::iter::once(0)
            .chain(rows.iter().map(|row| {
                tall += u32::from(row.height_class() == SidebarRowClass::Tall);
                tall
            }))
            .collect();
        Self { tall_before }
    }

    fn len(&self) -> usize {
        self.tall_before.len() - 1
    }
}

/// Device-pixel-snapped heights. Each is snapped on its own; sums never are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SidebarRowMetrics {
    pub(super) short: f32,
    pub(super) tall: f32,
}

impl SidebarRowMetrics {
    pub(super) fn uniform(height: f32) -> Self {
        Self {
            short: height,
            tall: height,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SidebarRowGeometry {
    classes: Rc<SidebarRowClasses>,
    metrics: SidebarRowMetrics,
}

impl SidebarRowGeometry {
    pub(super) fn new(classes: Rc<SidebarRowClasses>, metrics: SidebarRowMetrics) -> Self {
        Self { classes, metrics }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn uniform_for_test(rows: usize, height: f32) -> Self {
        Self::new(
            Rc::new(SidebarRowClasses {
                tall_before: vec![0; rows + 1].into(),
            }),
            SidebarRowMetrics::uniform(height),
        )
    }

    #[cfg(test)]
    pub(super) fn for_test(tall: &[bool], metrics: SidebarRowMetrics) -> Self {
        let mut count = 0;
        let tall_before = std::iter::once(0)
            .chain(tall.iter().map(|tall| {
                count += u32::from(*tall);
                count
            }))
            .collect();
        Self::new(Rc::new(SidebarRowClasses { tall_before }), metrics)
    }

    pub(super) fn metrics(&self) -> SidebarRowMetrics {
        self.metrics
    }

    pub(super) fn len(&self) -> usize {
        self.classes.len()
    }

    /// Top of row `ix` in content space; `top(len())` is the content height.
    pub(super) fn top(&self, ix: usize) -> f32 {
        let ix = ix.min(self.len());
        // Today's `ix * height`, plus a term that is exactly zero while every
        // row shares one height.
        ix as f32 * self.metrics.short
            + self.classes.tall_before[ix] as f32 * (self.metrics.tall - self.metrics.short)
    }

    /// Height of row `ix`; past the end, the short height.
    pub(super) fn height(&self, ix: usize) -> f32 {
        if self.is_tall(ix) {
            self.metrics.tall
        } else {
            self.metrics.short
        }
    }

    pub(super) fn is_tall(&self, ix: usize) -> bool {
        matches!(self.classes.tall_before.get(ix..ix + 2), Some([before, after]) if after > before)
    }

    /// Short-row slots a tall row occupies when budgeting the sticky band.
    pub(super) fn tall_units(&self) -> usize {
        if self.metrics.short <= 0.0 {
            return 1;
        }
        (self.metrics.tall / self.metrics.short).ceil().max(1.0) as usize
    }

    pub(super) fn units(&self, ix: usize) -> usize {
        if self.is_tall(ix) {
            self.tall_units()
        } else {
            1
        }
    }

    pub(super) fn total(&self) -> f32 {
        self.top(self.len())
    }

    pub(super) fn max_scroll(&self, viewport: f32) -> f32 {
        (self.total() - viewport).max(0.0)
    }

    /// The row under `y`, clamped to the list.
    pub(super) fn index_at(&self, y: f32) -> usize {
        partition_point(self.len(), |ix| self.top(ix) <= y)
            .saturating_sub(1)
            .min(self.len().saturating_sub(1))
    }

    /// The first row whose top is at or below `y`; `len()` when none is.
    pub(super) fn first_at_or_after(&self, y: f32) -> usize {
        partition_point(self.len(), |ix| self.top(ix) < y)
    }
}

impl gpui::UniformListItemLayout for SidebarRowGeometry {
    fn item_top(&self, ix: usize) -> gpui::Pixels {
        gpui::px(self.top(ix))
    }

    fn item_height(&self, ix: usize) -> gpui::Pixels {
        gpui::px(self.height(ix))
    }

    fn item_at(&self, y: gpui::Pixels) -> usize {
        self.index_at(f32::from(y))
    }
}

/// `slice::partition_point` over `0..len` without a slice.
fn partition_point(len: usize, mut before: impl FnMut(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let mid = low + (high - low) / 2;
        if before(mid) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::branch_sidebar::BranchMenuTarget;

    fn branch(section: BranchSection) -> BranchSidebarRow {
        BranchSidebarRow::Branch {
            name: "b".into(),
            target: BranchMenuTarget::Local { name: "b".into() },
            section,
            depth: 0,
            muted: false,
            divergence_ahead: None,
            divergence_behind: None,
            is_head: false,
            is_upstream: false,
        }
    }

    fn mixed() -> SidebarRowGeometry {
        let rows = [
            BranchSidebarRow::SectionSpacer,
            branch(BranchSection::Local),
            branch(BranchSection::Remote),
            branch(BranchSection::Local),
            BranchSidebarRow::SectionSpacer,
        ];
        SidebarRowGeometry::new(
            Rc::new(SidebarRowClasses::new(&rows)),
            SidebarRowMetrics {
                short: 24.0,
                tall: 42.0,
            },
        )
    }

    #[test]
    fn offsets_accumulate_both_heights() {
        let g = mixed();
        let tops: Vec<_> = (0..=g.len()).map(|ix| g.top(ix)).collect();
        assert_eq!(tops, [0.0, 24.0, 66.0, 90.0, 132.0, 156.0]);
        assert_eq!(g.height(1), 42.0);
        assert_eq!(g.height(2), 24.0);
        assert_eq!(g.height(99), 24.0);
        assert_eq!(g.total(), 156.0);
        assert_eq!(g.max_scroll(100.0), 56.0);
    }

    #[test]
    fn lookups_match_a_linear_scan() {
        let g = mixed();
        for step in -4..=640 {
            let y = step as f32 * 0.25;
            let under = (0..g.len()).rev().find(|ix| g.top(*ix) <= y).unwrap_or(0);
            assert_eq!(g.index_at(y), under, "y = {y}");
            let after = (0..g.len()).find(|ix| g.top(*ix) >= y).unwrap_or(g.len());
            assert_eq!(g.first_at_or_after(y), after, "y = {y}");
        }
    }

    #[test]
    fn uniform_metrics_reproduce_single_height_offsets() {
        let height = 46.25_f32;
        let rows = vec![branch(BranchSection::Local); 25_000];
        let g = SidebarRowGeometry::new(
            Rc::new(SidebarRowClasses::new(&rows)),
            SidebarRowMetrics::uniform(height),
        );
        for ix in [0, 1, 7, 12_345, 25_000] {
            assert_eq!(g.top(ix), ix as f32 * height);
        }
    }

    #[test]
    fn height_class_ignores_row_state() {
        let mut row = branch(BranchSection::Local);
        let tall = row.height_class();
        if let BranchSidebarRow::Branch {
            muted,
            is_head,
            divergence_ahead,
            ..
        } = &mut row
        {
            *muted = true;
            *is_head = true;
            *divergence_ahead = std::num::NonZeroU32::new(3);
        }
        assert_eq!(row.height_class(), tall);
        assert_eq!(
            branch(BranchSection::Remote).height_class(),
            SidebarRowClass::Short
        );
    }
}
