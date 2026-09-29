//! Shared sidebar matching. The owner caches this object across paints; all
//! projections, highlights and bulk actions use the same compiled expression.
use super::branch_sidebar::BranchSidebarRow;
use gitcomet_core::text_search::{TextSearchMatcher, TextSearchOptions};

pub(super) struct SidebarSearch {
    pub query: String,
    pub matcher: TextSearchMatcher,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::branch_sidebar::{BranchMenuTarget, BranchSection};

    fn branch(name: &str) -> BranchSidebarRow {
        BranchSidebarRow::Branch {
            name: name.to_owned().into(),
            target: BranchMenuTarget::local(name),
            section: BranchSection::Local,
            depth: 0,
            muted: false,
            divergence_ahead: None,
            divergence_behind: None,
            is_head: false,
            is_upstream: false,
        }
    }

    #[test]
    fn options_groups_and_highlights_use_one_matcher() {
        let mut options = TextSearchOptions::default();
        let search = SidebarSearch::new("release", options);
        assert_eq!(
            search
                .project(&[branch("Release/v1"), branch("prerelease")])
                .len(),
            2
        );
        options.match_case = true;
        assert_eq!(
            SidebarSearch::new("release", options)
                .project(&[branch("Release/v1"), branch("prerelease")])
                .len(),
            1
        );
        options.whole_word = true;
        assert!(
            SidebarSearch::new("release", options)
                .project(&[branch("Release/v1"), branch("prerelease")])
                .is_empty()
        );
        options.regex = true;
        let search = SidebarSearch::new(r"^origin/release/$", options);
        assert!(search.matches_remote("origin", "release/v1"));
        assert!(!search.matches_remote("upstream", "release/v1"));
        assert!(!search.matches_remote("origin", "prerelease/v1"));
        let search = SidebarSearch::new(
            r"topic-\d+$",
            TextSearchOptions {
                regex: true,
                ..Default::default()
            },
        );
        let mut ranges = Vec::new();
        search
            .matcher
            .find_ranges_into("origin/topic-123", &mut ranges, 16);
        assert_eq!(ranges, [7..16]);
        let invalid = SidebarSearch::new(
            "[",
            TextSearchOptions {
                regex: true,
                ..Default::default()
            },
        );
        assert!(invalid.matcher.regex_error().is_some());
        assert!(invalid.project(&[branch("[")]).is_empty());
    }

    #[test]
    fn auxiliary_paths_and_stash_labels_are_searchable() {
        let rows = [
            BranchSidebarRow::WorktreeItem {
                path: "/tmp/project-sandbox".into(),
                branch: Some("topic/release".into()),
                detached: false,
                is_active: false,
            },
            BranchSidebarRow::SubmoduleItem {
                path: "vendor/release".into(),
                status: gitcomet_core::domain::SubmoduleStatus::UpToDate,
                recorded_head: gitcomet_core::domain::CommitId("a".into()),
                checked_out_head: None,
            },
            BranchSidebarRow::StashItem {
                id: gitcomet_core::domain::CommitId("s".into()),
                index: 12,
                message: "release work".into(),
                tooltip: "release work".into(),
                created_at: None,
            },
        ];
        assert_eq!(
            SidebarSearch::new("release", Default::default())
                .project(&rows)
                .len(),
            3
        );
        assert_eq!(
            SidebarSearch::new("sandbox", Default::default())
                .project(&rows)
                .len(),
            1
        );
        assert!(matches!(
            SidebarSearch::new("stash@{12}", Default::default())
                .project(&rows)
                .as_slice(),
            [BranchSidebarRow::StashItem { .. }]
        ));
    }
}

impl SidebarSearch {
    pub fn new(query: &str, options: TextSearchOptions) -> Self {
        Self {
            query: query.trim().to_owned(),
            matcher: TextSearchMatcher::new(query.trim(), options),
        }
    }

    /// Group matches include their subtree, even for an anchored expression.
    /// Full ref spellings avoid guessing the boundary of a slash-named remote.
    pub fn matches_ref(&self, name: &str) -> bool {
        self.matcher.is_empty()
            || self.matcher.is_match(name)
            || name.match_indices('/').any(|(end, _)| {
                self.matcher.is_match(&name[..end]) || self.matcher.is_match(&name[..=end])
            })
    }

    pub fn matches_remote(&self, remote: &str, name: &str) -> bool {
        self.matches_ref(&format!("{remote}/{name}"))
    }

    /// Input rows have expanded ancestors. Retain only matches and their
    /// context, preserving loading/error rows rather than reporting no results.
    pub fn project(&self, rows: &[BranchSidebarRow]) -> Vec<BranchSidebarRow> {
        if self.matcher.is_empty() {
            return rows
                .iter()
                .filter(|row| !matches!(row, BranchSidebarRow::SectionSpacer))
                .cloned()
                .collect();
        }
        if self.matcher.regex_error().is_some() {
            return Vec::new();
        }
        let mut keep = vec![false; rows.len()];
        let mut ancestors: Vec<(usize, usize)> = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let (level, header) = match row {
                BranchSidebarRow::SectionHeader { .. }
                | BranchSidebarRow::WorktreesHeader { .. }
                | BranchSidebarRow::SubmodulesHeader { .. }
                | BranchSidebarRow::StashHeader { .. } => (0, true),
                BranchSidebarRow::RemoteHeader { .. } => (1, true),
                BranchSidebarRow::GroupHeader { depth, .. } => (usize::from(*depth) + 2, true),
                BranchSidebarRow::Branch { depth, .. } => (usize::from(*depth) + 2, false),
                BranchSidebarRow::SectionSpacer => continue,
                _ => (1, false),
            };
            while ancestors.last().is_some_and(|(depth, _)| *depth >= level) {
                ancestors.pop();
            }
            if header {
                ancestors.push((level, ix));
                continue;
            }
            let matched = match row {
                BranchSidebarRow::Branch { name, .. } => self.matches_ref(name),
                BranchSidebarRow::WorktreeItem { path, branch, .. } => {
                    self.matcher.is_match(&path.to_string_lossy())
                        || branch
                            .as_ref()
                            .is_some_and(|branch| self.matcher.is_match(branch))
                }
                BranchSidebarRow::SubmoduleItem { path, .. } => {
                    self.matcher.is_match(&path.to_string_lossy())
                }
                BranchSidebarRow::StashItem { index, message, .. } => self
                    .matcher
                    .is_match(&format!("stash@{{{index}}}: {message}")),
                BranchSidebarRow::Placeholder { message, .. }
                | BranchSidebarRow::WorktreePlaceholder { message }
                | BranchSidebarRow::SubmodulePlaceholder { message, .. }
                | BranchSidebarRow::StashPlaceholder { message } => !message.starts_with("No "),
                _ => false,
            };
            if matched {
                keep[ix] = true;
                for (_, parent) in &ancestors {
                    keep[*parent] = true;
                }
            }
        }
        rows.iter()
            .zip(keep)
            .filter(|(_, keep)| *keep)
            .map(|(row, _)| row.clone())
            .collect()
    }
}
