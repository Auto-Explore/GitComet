//! Geometry and identity for the expanded sidebar. All coordinates here are
//! uniform-list slots, never the visual height of a section spacer.
use super::branch_sidebar::{self, BranchMenuTarget, BranchSection, BranchSidebarRow};
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
}

/// Only inspect the viewport's neighboring pin roots and the small sticky
/// budget. Hundreds of thousands of offscreen pins cost two binary searches.
pub(super) fn fit_pins(
    base: &[usize],
    pins: &[usize],
    selected: Option<usize>,
    scroll: f32,
    height: f32,
    row_height: f32,
) -> StickyLayout {
    if row_height <= 0.0 {
        return StickyLayout::default();
    }
    let base_top = base
        .iter()
        .enumerate()
        .take_while(|(rank, row)| **row as f32 * row_height - scroll <= *rank as f32 * row_height)
        .count();
    let base_bottom = base
        .iter()
        .enumerate()
        .rev()
        .take_while(|(rank, row)| {
            **row as f32 * row_height - scroll >= height - (base.len() - *rank) as f32 * row_height
        })
        .count();
    let top = pins
        .partition_point(|row| *row as f32 * row_height < scroll + base_top as f32 * row_height);
    let bottom = pins
        .partition_point(|row| {
            (*row + 1) as f32 * row_height <= scroll + height - base_bottom as f32 * row_height
        })
        .max(top);
    let budget = ((height / row_height / 3.0).floor() as usize)
        .saturating_sub(usize::from(selected.is_some_and(|row| base.contains(&row))))
        .min(((height / row_height).floor() as usize).saturating_sub(base.len() + 1));
    let count = top + pins.len() - bottom;
    let controls = usize::from(top > 0) + usize::from(bottom < pins.len());
    let overflowing = count > budget;
    let mut chosen = Vec::new();
    if count > 0 && (!overflowing || budget >= controls) {
        let available = if overflowing {
            budget - controls
        } else {
            budget
        };
        if let Some(selected) = selected
            && let Ok(ix) = pins.binary_search(&selected)
            && (ix < top || ix >= bottom)
            && available > 0
        {
            chosen.push(selected);
        }
        let mut above = top;
        let mut below = bottom;
        while chosen.len() < available && (above > 0 || below < pins.len()) {
            let choose_top = above > 0
                && (below == pins.len()
                    || scroll - pins[above - 1] as f32 * row_height
                        <= pins[below] as f32 * row_height - scroll - height);
            let row = if choose_top {
                above -= 1;
                pins[above]
            } else {
                let row = pins[below];
                below += 1;
                row
            };
            if !chosen.contains(&row) {
                chosen.push(row);
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
    let rows = slots
        .iter()
        .filter_map(|slot| {
            if let StickySlot::Row(row) = slot {
                Some(*row)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .into();
    StickyLayout { rows, slots }
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
        let mut result = Self::default();
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

/// Reserve a body row. Compact ancestor groups before giving up the selected
/// branch, then fall back to section headers or ordinary scrolling.
pub(super) fn fitted_rows<'a>(
    all: &'a [usize],
    priority: &'a [usize],
    sections: &'a [usize],
    height: f32,
    row_height: f32,
) -> &'a [usize] {
    if row_height <= 0.0 || height < (sections.len() + 1) as f32 * row_height {
        &[]
    } else if height >= (all.len() + 1) as f32 * row_height {
        all
    } else if height >= (priority.len() + 1) as f32 * row_height {
        priority
    } else {
        sections
    }
}

pub(super) fn row_y(
    row: usize,
    rank: usize,
    count: usize,
    scroll: f32,
    height: f32,
    row_height: f32,
) -> f32 {
    (row as f32 * row_height - scroll).clamp(
        rank as f32 * row_height,
        height - (count - rank) as f32 * row_height,
    )
}

pub(super) fn navigation_offset(
    row: usize,
    headers: &[usize],
    height: f32,
    row_height: f32,
    total: usize,
    center: bool,
) -> f32 {
    let before = headers.partition_point(|ix| *ix < row);
    let top = before as f32 * row_height;
    let target_y = if center {
        let after = headers.iter().filter(|ix| **ix > row).count() as f32 * row_height;
        top + (height - top - after - row_height).max(0.0) / 2.0
    } else {
        top
    };
    (row as f32 * row_height - target_y).clamp(0.0, (total as f32 * row_height - height).max(0.0))
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
        let layout = fit_pins(&[], &structure.pin_roots, None, 600.0, 480.0, 24.0);
        assert!(
            layout.slots.iter().any(
                |slot| matches!(slot, StickySlot::Overflow { roots, .. } if roots.len() == 20)
            )
        );
    }

    #[test]
    fn many_pins_fit_both_edges_without_covering_the_body() {
        let pins: Vec<_> = (0..100_000).collect();
        for height in [24.0, 72.0, 180.0, 480.0, 1200.0] {
            for scroll in [0.0, 240.0, 5000.0, 2_400_000.0] {
                let sections = [100_000, 100_100];
                let base = fitted_rows(&sections, &sections, &sections, height, 24.0);
                let layout = fit_pins(base, &pins, Some(0), scroll, height, 24.0);
                assert!(layout.slots.len() < (height / 24.0) as usize);
                assert!(layout.slots.len() - base.len() <= (height / 72.0) as usize);
                let mut previous = -24.0;
                for (rank, slot) in layout.slots.iter().enumerate() {
                    let y = row_y(slot.row(), rank, layout.slots.len(), scroll, height, 24.0);
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
            for height in [12.0 * row_height, 31.0 * row_height] {
                for step in 0..1200 {
                    let scroll = step as f32 * 0.25 * row_height;
                    let mut previous = -row_height;
                    for (rank, row) in headers.into_iter().enumerate() {
                        let y = row_y(row, rank, headers.len(), scroll, height, row_height);
                        assert!(y >= previous + row_height - 0.001);
                        assert!(y >= 0.0 && y + row_height <= height + 0.001);
                        previous = y;
                    }
                }
                for (rank, row) in headers.into_iter().enumerate() {
                    let scroll = navigation_offset(row, &headers, height, row_height, 400, false);
                    assert_eq!(row as f32 * row_height - scroll, rank as f32 * row_height);
                }
            }
        }
    }

    #[test]
    fn short_viewports_compact_paths_then_use_ordinary_scrolling() {
        let all = [0, 2, 3, 8, 10, 20, 30];
        let priority = [0, 3, 8, 10, 20, 30];
        let sections = [0, 8, 10, 20, 30];
        assert_eq!(fitted_rows(&all, &priority, &sections, 192.0, 24.0), all);
        assert_eq!(
            fitted_rows(&all, &priority, &sections, 168.0, 24.0),
            priority
        );
        assert_eq!(
            fitted_rows(&all, &priority, &sections, 144.0, 24.0),
            sections
        );
        assert!(fitted_rows(&all, &priority, &sections, 143.0, 24.0).is_empty());
    }
}
