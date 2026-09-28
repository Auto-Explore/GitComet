//! Geometry and identity for the expanded sidebar. All coordinates here are
//! uniform-list slots, never the visual height of a section spacer.
use super::branch_sidebar::{self, BranchMenuTarget, BranchSection, BranchSidebarRow};
use gpui::SharedString;
use rustc_hash::FxHashMap;

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
}

pub(super) fn header_key(row: &BranchSidebarRow) -> Option<&SharedString> {
    match row {
        BranchSidebarRow::SectionHeader { collapse_key, .. }
        | BranchSidebarRow::WorktreesHeader { collapse_key, .. }
        | BranchSidebarRow::SubmodulesHeader { collapse_key, .. }
        | BranchSidebarRow::StashHeader { collapse_key, .. }
        | BranchSidebarRow::RemoteHeader { collapse_key, .. }
        | BranchSidebarRow::GroupHeader { collapse_key, .. } => Some(collapse_key),
        _ => None,
    }
}

impl SidebarStructure {
    pub(super) fn new(rows: &[BranchSidebarRow]) -> Self {
        let mut result = Self::default();
        for (ix, row) in rows.iter().enumerate() {
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
        query: &str,
    ) -> Vec<usize> {
        let (name, remote) = match target {
            BranchMenuTarget::Local { name } => {
                if !branch_sidebar::branch_matches_raw_filter(name, query) {
                    return Vec::new();
                }
                (name.as_str(), None)
            }
            BranchMenuTarget::Remote { remote, branch } => {
                if !branch_sidebar::remote_branch_matches_raw_filter(remote, branch, query) {
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
