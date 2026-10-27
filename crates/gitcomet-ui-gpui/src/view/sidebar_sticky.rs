//! Geometry and identity for the expanded sidebar. All coordinates here are
//! uniform-list slots, never the visual height of a section spacer.
use super::branch_sidebar::{self, BranchMenuTarget, BranchSection, BranchSidebarRow};
use super::sidebar_row_geometry::{SidebarRowClasses, SidebarRowGeometry};
use gpui::SharedString;
use rustc_hash::FxHashMap;
use std::{ops::Range, rc::Rc};

#[derive(Clone, Debug)]
pub(super) enum StickySlot {
    Row(usize),
    Overflow {
        row: usize,
        bottom: bool,
        roots: Range<usize>,
    },
}

impl StickySlot {
    pub(super) fn row(&self) -> usize {
        match self {
            Self::Row(row) | Self::Overflow { row, .. } => *row,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct StickyLayout {
    pub(super) rows: Rc<[usize]>,
    pub(super) slots: Vec<StickySlot>,
    /// Tall slots before each rank, plus one entry for the end.
    tall_before: Rc<[u32]>,
}

impl StickyLayout {
    fn new(slots: Vec<StickySlot>, g: &SidebarRowGeometry) -> Self {
        let rows = slots
            .iter()
            .filter_map(|slot| match slot {
                StickySlot::Row(row) => Some(*row),
                StickySlot::Overflow { .. } => None,
            })
            .collect::<Vec<_>>()
            .into();
        let mut tall = 0;
        let tall_before = std::iter::once(0)
            .chain(slots.iter().map(|slot| {
                // The overflow control is always a short row.
                tall += u32::from(matches!(slot, StickySlot::Row(row) if g.is_tall(*row)));
                tall
            }))
            .collect();
        Self {
            rows,
            slots,
            tall_before,
        }
    }

    /// Height of the slots at `ranks`.
    pub(super) fn stack(&self, ranks: Range<usize>, g: &SidebarRowGeometry) -> f32 {
        let tall_before = |rank: usize| self.tall_before.get(rank).copied().unwrap_or(0);
        let metrics = g.metrics();
        ranks.len() as f32 * metrics.short
            + (tall_before(ranks.end) - tall_before(ranks.start)) as f32
                * (metrics.tall - metrics.short)
    }

    /// Where slot `rank` sits in the viewport: its natural position, held
    /// between the slots stacked above and below it.
    pub(super) fn row_y(
        &self,
        rank: usize,
        scroll: f32,
        height: f32,
        g: &SidebarRowGeometry,
    ) -> f32 {
        let above = self.stack(0..rank, g);
        let below = height - self.stack(rank..self.slots.len(), g);
        (g.top(self.slots[rank].row()) - scroll)
            .max(above)
            .min(below.max(above))
    }

    #[cfg(test)]
    pub(super) fn from_rows(rows: &[usize], g: &SidebarRowGeometry) -> Self {
        Self::new(rows.iter().copied().map(StickySlot::Row).collect(), g)
    }
}

/// Heights of `rows` stacked, computed like `SidebarRowGeometry::top`.
fn stacked(rows: &[usize], g: &SidebarRowGeometry) -> f32 {
    let metrics = g.metrics();
    let tall = rows.iter().filter(|row| g.is_tall(**row)).count();
    rows.len() as f32 * metrics.short + tall as f32 * (metrics.tall - metrics.short)
}

fn units(rows: &[usize], g: &SidebarRowGeometry) -> usize {
    rows.iter().map(|row| g.units(*row)).sum()
}

/// Only inspect the viewport's neighboring pin roots and the small sticky
/// budget. Hundreds of thousands of offscreen pins cost two binary searches.
/// The budget counts short-row slots; a tall row takes `tall_units` of them.
pub(super) fn fit_pins(
    base: &[usize],
    pins: &[usize],
    selected: Option<usize>,
    scroll: f32,
    height: f32,
    g: &SidebarRowGeometry,
) -> StickyLayout {
    let short = g.metrics().short;
    if short <= 0.0 {
        return StickyLayout::default();
    }
    let base_top = (0..base.len())
        .take_while(|rank| g.top(base[*rank]) - scroll <= stacked(&base[..*rank], g))
        .count();
    let base_bottom = (0..base.len())
        .rev()
        .take_while(|rank| g.top(base[*rank]) - scroll >= height - stacked(&base[*rank..], g))
        .count();
    let top_stack = stacked(&base[..base_top], g);
    let bottom_stack = stacked(&base[base.len() - base_bottom..], g);
    let top = pins.partition_point(|row| g.top(*row) < scroll + top_stack);
    let bottom = pins
        .partition_point(|row| g.top(*row + 1) <= scroll + height - bottom_stack)
        .max(top);
    let budget = ((height / short / 3.0).floor() as usize)
        .saturating_sub(
            selected
                .filter(|row| base.contains(row))
                .map_or(0, |row| g.units(row)),
        )
        .min(((height / short).floor() as usize).saturating_sub(units(base, g) + g.tall_units()));
    // Offscreen pin slots, counted only as far as the budget needs.
    let count = pins[..top]
        .iter()
        .chain(&pins[bottom..])
        .scan(0, |sum, row| {
            *sum += g.units(*row);
            Some(*sum)
        })
        .find(|sum| *sum > budget)
        .unwrap_or_else(|| units(&pins[..top], g) + units(&pins[bottom..], g));
    let controls = usize::from(top > 0) + usize::from(bottom < pins.len());
    let overflowing = count > budget;
    let mut chosen = Vec::new();
    if count > 0 && (!overflowing || budget >= controls) {
        let available = if overflowing {
            budget - controls
        } else {
            budget
        };
        let mut used = 0;
        if let Some(selected) = selected
            && let Ok(ix) = pins.binary_search(&selected)
            && (ix < top || ix >= bottom)
            && g.units(selected) <= available
        {
            chosen.push(selected);
            used += g.units(selected);
        }
        let mut above = top;
        let mut below = bottom;
        while used < available && (above > 0 || below < pins.len()) {
            let choose_top = above > 0
                && (below == pins.len()
                    || scroll - g.top(pins[above - 1]) <= g.top(pins[below]) - scroll - height);
            let row = if choose_top {
                above -= 1;
                pins[above]
            } else {
                let row = pins[below];
                below += 1;
                row
            };
            if !chosen.contains(&row) {
                if used + g.units(row) > available {
                    break;
                }
                chosen.push(row);
                used += g.units(row);
            }
        }
    }
    let mut slots: Vec<_> = base
        .iter()
        .chain(&chosen)
        .copied()
        .map(StickySlot::Row)
        .collect();
    if overflowing && budget >= controls {
        for (range, bottom_edge) in [(0..top, false), (bottom..pins.len(), true)] {
            let kept = chosen
                .iter()
                .filter(|row| pins[range.clone()].binary_search(row).is_ok())
                .count();
            if range.len() > kept {
                let row = if bottom_edge {
                    pins[range.end - 1]
                } else {
                    pins[range.start]
                };
                slots.push(StickySlot::Overflow {
                    row,
                    bottom: bottom_edge,
                    roots: range,
                });
            }
        }
    }
    slots.sort_by_key(|slot| {
        (
            slot.row(),
            match slot {
                StickySlot::Overflow { bottom: false, .. } => 0,
                StickySlot::Row(_) => 1,
                _ => 2,
            },
        )
    });
    slots.dedup_by(|a, b| matches!((&a, &b), (StickySlot::Row(a), StickySlot::Row(b)) if a == b));
    StickyLayout::new(slots, g)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SidebarRowSurface {
    Tree,
    Sticky { compact: bool },
    Pins,
    Rail,
}

#[derive(Default)]
pub(super) struct SidebarStructure {
    pub(super) sections: Vec<usize>,
    pub(super) headers: FxHashMap<SharedString, usize>,
    pub(super) pin_roots: Vec<usize>,
    pub(super) classes: Rc<SidebarRowClasses>,
}

pub(super) fn header_key(row: &BranchSidebarRow) -> Option<&SharedString> {
    match row {
        BranchSidebarRow::SectionHeader { collapse_key, .. }
        | BranchSidebarRow::ContributionHeader { collapse_key, .. }
        | BranchSidebarRow::WorktreesHeader { collapse_key, .. }
        | BranchSidebarRow::SubmodulesHeader { collapse_key, .. }
        | BranchSidebarRow::AnnexHeader { collapse_key, .. }
        | BranchSidebarRow::StashHeader { collapse_key, .. }
        | BranchSidebarRow::RemoteHeader { collapse_key, .. }
        | BranchSidebarRow::GroupHeader { collapse_key, .. } => Some(collapse_key),
        _ => None,
    }
}

pub(super) fn row_key(row: &BranchSidebarRow) -> SharedString {
    match row {
        BranchSidebarRow::ContributionItem { key, .. } => key.clone(),
        BranchSidebarRow::Branch { target, .. } => format!("branch:{target:?}").into(),
        BranchSidebarRow::WorktreeItem { path, .. } => {
            format!("worktree:{}", path.display()).into()
        }
        BranchSidebarRow::SubmoduleItem { path, .. } => {
            format!("submodule:{}", path.display()).into()
        }
        BranchSidebarRow::AnnexRepositoryItem { uuid, .. } => format!("annex:{uuid}").into(),
        BranchSidebarRow::StashItem { id, .. } => format!("stash:{}", id.0).into(),
        _ => header_key(row)
            .cloned()
            .unwrap_or_else(|| format!("{row:?}").into()),
    }
}

impl SidebarStructure {
    #[cfg(test)]
    pub(super) fn new(rows: &[BranchSidebarRow]) -> Self {
        Self::with_pins(rows, 0)
    }

    pub(super) fn with_pins(rows: &[BranchSidebarRow], pins: usize) -> Self {
        let mut result = Self {
            classes: Rc::new(SidebarRowClasses::new(rows)),
            ..Self::default()
        };
        for (ix, row) in rows.iter().enumerate() {
            if ix < pins {
                if matches!(
                    row,
                    BranchSidebarRow::Branch { depth: 0, .. }
                        | BranchSidebarRow::GroupHeader { depth: 0, .. }
                ) {
                    result.pin_roots.push(ix);
                }
                continue;
            }
            if let Some(key) = header_key(row) {
                result.headers.insert(key.clone(), ix);
                if branch_sidebar::is_top_level_collapse_key(key) {
                    result.sections.push(ix);
                }
            }
        }
        result
    }

    /// Only called when the presentation or active branch identities change.
    /// A closed ancestor suppresses the entire path, including its remote.
    pub(super) fn active_path(
        &self,
        rows: &[BranchSidebarRow],
        target: &BranchMenuTarget,
        search: &super::sidebar_search::SidebarSearch,
    ) -> Vec<usize> {
        let (name, remote) = match target {
            BranchMenuTarget::Local { name } => {
                if !search.matches_ref(name) {
                    return Vec::new();
                }
                (name.as_str(), None)
            }
            BranchMenuTarget::Remote { remote, branch } => {
                if !search.matches_remote(remote, branch) {
                    return Vec::new();
                }
                (branch.as_str(), Some(remote.as_str()))
            }
        };
        let mut path = Vec::new();
        if let Some(remote) = remote {
            let key = branch_sidebar::remote_header_storage_key(remote);
            if let Some(&ix) = self.headers.get(key.as_str()) {
                if matches!(
                    rows[ix],
                    BranchSidebarRow::RemoteHeader {
                        collapsed: true,
                        ..
                    }
                ) {
                    return Vec::new();
                }
                path.push(ix);
            }
        }
        for end in name
            .match_indices('/')
            .map(|(ix, _)| ix)
            .chain(std::iter::once(name.len()))
        {
            let key = match remote {
                Some(remote) => branch_sidebar::remote_group_storage_key(remote, &name[..end]),
                None => branch_sidebar::local_group_storage_key(&name[..end]),
            };
            if let Some(&ix) = self.headers.get(key.as_str()) {
                if matches!(
                    rows[ix],
                    BranchSidebarRow::GroupHeader {
                        collapsed: true,
                        ..
                    }
                ) {
                    return Vec::new();
                }
                path.push(ix);
            }
        }
        path
    }
}

/// Reserve a tall body row. Compact ancestor groups before giving up the
/// selected branch, then fall back to section headers or ordinary scrolling.
pub(super) fn fitted_rows<'a>(
    all: &'a [usize],
    priority: &'a [usize],
    sections: &'a [usize],
    height: f32,
    g: &SidebarRowGeometry,
) -> &'a [usize] {
    let short = g.metrics().short;
    let fits = |rows: &[usize]| height >= (units(rows, g) + g.tall_units()) as f32 * short;
    if short <= 0.0 || !fits(sections) {
        &[]
    } else if fits(all) {
        all
    } else if fits(priority) {
        priority
    } else {
        sections
    }
}

/// The scroll offset that puts `row` just below the slots stacked above it,
/// or centered in the space the stack leaves.
pub(super) fn navigation_offset(
    row: usize,
    layout: &StickyLayout,
    height: f32,
    g: &SidebarRowGeometry,
    center: bool,
) -> f32 {
    let before = layout.slots.partition_point(|slot| slot.row() < row);
    let top = layout.stack(0..before, g);
    let target_y = if center {
        let after = layout.slots.partition_point(|slot| slot.row() <= row);
        let after = layout.stack(after..layout.slots.len(), g);
        top + (height - top - after - g.height(row)).max(0.0) / 2.0
    } else {
        top
    };
    (g.top(row) - target_y).clamp(0.0, g.max_scroll(height))
}

pub(super) fn same_row(a: &BranchSidebarRow, b: &BranchSidebarRow) -> bool {
    match (a, b) {
        (
            BranchSidebarRow::Branch { target: a, .. },
            BranchSidebarRow::Branch { target: b, .. },
        ) => a == b,
        (
            BranchSidebarRow::WorktreeItem { path: a, .. },
            BranchSidebarRow::WorktreeItem { path: b, .. },
        )
        | (
            BranchSidebarRow::SubmoduleItem { path: a, .. },
            BranchSidebarRow::SubmoduleItem { path: b, .. },
        ) => a == b,
        (BranchSidebarRow::StashItem { id: a, .. }, BranchSidebarRow::StashItem { id: b, .. }) => {
            a == b
        }
        (
            BranchSidebarRow::AnnexRepositoryItem { uuid: a, .. },
            BranchSidebarRow::AnnexRepositoryItem { uuid: b, .. },
        ) => a == b,
        _ => header_key(a).is_some() && header_key(a) == header_key(b),
    }
}

pub(super) fn owning_section(row: &BranchSidebarRow) -> Option<BranchSection> {
    match row {
        BranchSidebarRow::GroupHeader { section, .. } => Some(*section),
        BranchSidebarRow::RemoteHeader { .. } => Some(BranchSection::Remote),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_collapsed_pinned_groups_remain_sticky_and_count_toward_overflow() {
        let rows: Vec<_> = (0..20)
            .map(|ix| BranchSidebarRow::GroupHeader {
                label: format!("feat-{ix}").into(),
                path: format!("feat-{ix}").into(),
                remote: None,
                section: BranchSection::Local,
                depth: 0,
                collapsed: true,
                collapse_key: format!("group:local:feat-{ix}").into(),
            })
            .collect();
        let structure = SidebarStructure::with_pins(&rows, rows.len());
        assert_eq!(structure.pin_roots.len(), 20);
        let g = SidebarRowGeometry::uniform_for_test(rows.len(), 24.0);
        let layout = fit_pins(&[], &structure.pin_roots, None, 600.0, 480.0, &g);
        assert!(
            layout.slots.iter().any(
                |slot| matches!(slot, StickySlot::Overflow { roots, .. } if roots.len() == 20)
            )
        );
    }

    #[test]
    fn many_pins_fit_both_edges_without_covering_the_body() {
        let pins: Vec<_> = (0..100_000).collect();
        let g = SidebarRowGeometry::uniform_for_test(100_200, 24.0);
        for height in [24.0, 72.0, 180.0, 480.0, 1200.0] {
            for scroll in [0.0, 240.0, 5000.0, 2_400_000.0] {
                let sections = [100_000, 100_100];
                let base = fitted_rows(&sections, &sections, &sections, height, &g);
                let layout = fit_pins(base, &pins, Some(0), scroll, height, &g);
                assert!(layout.slots.len() < (height / 24.0) as usize);
                assert!(layout.slots.len() - base.len() <= (height / 72.0) as usize);
                let mut previous = -24.0;
                for rank in 0..layout.slots.len() {
                    let y = layout.row_y(rank, scroll, height, &g);
                    assert!(y >= previous + 24.0);
                    previous = y;
                }
                if height == 480.0 && scroll == 5000.0 {
                    assert!(layout.rows.contains(&0), "selected pin has priority");
                    for edge in [false, true] {
                        assert!(layout.slots.iter().any(|slot| matches!(slot, StickySlot::Overflow { bottom, .. } if *bottom == edge)));
                    }
                }
            }
        }
    }

    #[test]
    fn dual_edge_headers_stay_ordered_and_clicks_reveal_content() {
        let headers = [0, 2, 80, 122, 180, 199, 260];
        for row_height in [24.0, 32.0, 37.0, 46.25] {
            let g = SidebarRowGeometry::uniform_for_test(400, row_height);
            let layout = StickyLayout::from_rows(&headers, &g);
            for height in [12.0 * row_height, 31.0 * row_height] {
                for step in 0..1200 {
                    let scroll = step as f32 * 0.25 * row_height;
                    let mut previous = -row_height;
                    for rank in 0..headers.len() {
                        let y = layout.row_y(rank, scroll, height, &g);
                        assert!(y >= previous + row_height - 0.001);
                        assert!(y >= 0.0 && y + row_height <= height + 0.001);
                        previous = y;
                    }
                }
                for (rank, row) in headers.into_iter().enumerate() {
                    let scroll = navigation_offset(row, &layout, height, &g, false);
                    assert_eq!(row as f32 * row_height - scroll, rank as f32 * row_height);
                }
            }
        }
    }

    #[test]
    fn mixed_heights_stack_without_gaps_and_keep_a_tall_body_row() {
        use crate::view::sidebar_row_geometry::SidebarRowMetrics;
        let headers = [0, 2, 80, 122, 180, 199, 260];
        let pins: Vec<_> = (300..340).collect();
        let tall: Vec<_> = (0..400)
            .map(|ix| ix % 3 == 1 && !headers.contains(&ix))
            .collect();
        for (short, tall_height) in [(24.0, 42.0), (32.0, 48.0), (36.0, 78.0)] {
            let g = SidebarRowGeometry::for_test(
                &tall,
                SidebarRowMetrics {
                    short,
                    tall: tall_height,
                },
            );
            let layout = StickyLayout::from_rows(&headers, &g);
            for height in [12.0 * short, 31.0 * short] {
                for step in 0..1200 {
                    let scroll = step as f32 * 0.25 * short;
                    let mut bottom = 0.0_f32;
                    for rank in 0..headers.len() {
                        let y = layout.row_y(rank, scroll, height, &g);
                        assert!(y >= bottom - 0.001, "overlap at rank {rank}");
                        bottom = y + g.height(headers[rank]);
                        assert!(bottom <= height + 0.001);
                    }
                    let fitted = fit_pins(&[], &pins, Some(320), scroll, height, &g);
                    let band = fitted.stack(0..fitted.slots.len(), &g);
                    assert!(
                        band + tall_height <= height + 0.001,
                        "{band} + body > {height}"
                    );
                }
                for (rank, row) in headers.into_iter().enumerate() {
                    let scroll = navigation_offset(row, &layout, height, &g, false);
                    assert_eq!(g.top(row) - scroll, layout.stack(0..rank, &g));
                }
            }
        }
    }

    #[test]
    fn short_viewports_compact_paths_then_use_ordinary_scrolling() {
        let all = [0, 2, 3, 8, 10, 20, 30];
        let priority = [0, 3, 8, 10, 20, 30];
        let sections = [0, 8, 10, 20, 30];
        let g = SidebarRowGeometry::uniform_for_test(31, 24.0);
        assert_eq!(fitted_rows(&all, &priority, &sections, 192.0, &g), all);
        assert_eq!(fitted_rows(&all, &priority, &sections, 168.0, &g), priority);
        assert_eq!(fitted_rows(&all, &priority, &sections, 144.0, &g), sections);
        assert!(fitted_rows(&all, &priority, &sections, 143.0, &g).is_empty());
    }
}
