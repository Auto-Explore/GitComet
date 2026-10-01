//! Presentation state and caches shared by built-in and hosted changed-file lists.
use super::*;
use crate::view::rows::{
    CollapsedDirs, CommitFileFilter, CommitFileProjectionCache, CommitFileSort, FileListPlan,
    FileListPlanCache, FileTree, FileTreeItem,
};
use gitcomet_core::domain::{CommitFileChange, FileStatusKind};
use gitcomet_extension_api::FileListMode;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Change kinds in the order grouped lists show them.
pub(in crate::view) const GROUP_ORDER: [FileStatusKind; 6] = [
    FileStatusKind::Conflicted,
    FileStatusKind::Added,
    FileStatusKind::Modified,
    FileStatusKind::Renamed,
    FileStatusKind::Deleted,
    FileStatusKind::Untracked,
];

pub(in crate::view) fn group_of(kind: FileStatusKind) -> usize {
    GROUP_ORDER
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(GROUP_ORDER.len() - 1)
}

pub(in crate::view) fn group_label(group: usize) -> &'static str {
    match GROUP_ORDER.get(group) {
        Some(FileStatusKind::Conflicted) => "Conflicted",
        Some(FileStatusKind::Added) => "Added",
        Some(FileStatusKind::Modified) => "Modified",
        Some(FileStatusKind::Renamed) => "Renamed",
        Some(FileStatusKind::Deleted) => "Deleted",
        _ => "Untracked",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum GroupedRow {
    Header {
        group: usize,
        count: usize,
        collapsed: bool,
    },
    /// The file at this position of the shown files.
    File { ordinal: usize },
}

/// A grouped list's rows, all one height.
#[derive(Debug, Default)]
pub(in crate::view) struct GroupedRows {
    pub(in crate::view) rows: Vec<GroupedRow>,
    /// The row of each header, ascending.
    headers: Vec<usize>,
}

impl GroupedRows {
    /// The header of the group holding `row`.
    pub(in crate::view) fn header_for(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        after.checked_sub(1).map(|ix| self.headers[ix])
    }

    /// The first header below `row`.
    pub(in crate::view) fn next_header(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        self.headers.get(after).copied()
    }
}

/// One list's presentation state, independent of every other list.
pub(in crate::view) struct FileListController {
    pub(in crate::view) files: Arc<Vec<CommitFileChange>>,
    pub(in crate::view) files_rev: u64,
    pub(in crate::view) sort: CommitFileSort,
    pub(in crate::view) kind_filter: CommitFileFilter,
    pub(in crate::view) query: SharedString,
    pub(in crate::view) mode: FileListMode,
    pub(in crate::view) collapsed: CollapsedDirs,
    collapsed_groups: [bool; GROUP_ORDER.len()],
    pub(in crate::view) selected: Option<PathBuf>,
    pub(in crate::view) projection_cache: CommitFileProjectionCache<u64>,
    pub(in crate::view) presentations: crate::view::rows::CommitFileRowPresentationCache<u64>,
    pub(in crate::view) plan_cache: FileListPlanCache,
    shown: Option<(u64, Arc<[usize]>)>,
    grouped: Option<(u64, Arc<GroupedRows>)>,
    #[cfg(test)]
    pub(in crate::view) group_builds: usize,
}

impl FileListController {
    pub(in crate::view) fn new(mode: FileListMode) -> Self {
        Self {
            files: Arc::default(),
            files_rev: 0,
            sort: CommitFileSort::default(),
            kind_filter: CommitFileFilter::default(),
            query: SharedString::default(),
            mode,
            collapsed: CollapsedDirs::default(),
            collapsed_groups: [false; GROUP_ORDER.len()],
            selected: None,
            projection_cache: CommitFileProjectionCache::default(),
            presentations: Default::default(),
            plan_cache: FileListPlanCache::default(),
            shown: None,
            grouped: None,
            #[cfg(test)]
            group_builds: 0,
        }
    }

    pub(in crate::view) fn set_mode(&mut self, mode: FileListMode) {
        self.mode = mode;
    }

    fn plan_layout(&self) -> FileListLayout {
        match self.mode {
            FileListMode::Tree => FileListLayout::Tree,
            _ => FileListLayout::Flat,
        }
    }

    /// Rows in the current mode.
    pub(in crate::view) fn row_count(&mut self) -> usize {
        match self.mode {
            FileListMode::Grouped => self.grouped().rows.len(),
            _ => self.plan().row_len(),
        }
    }

    /// The shown files by change kind; rebuilt only when the shown files or
    /// the collapsed groups change.
    pub(in crate::view) fn grouped(&mut self) -> Arc<GroupedRows> {
        let mut hasher = rustc_hash::FxHasher::default();
        self.projection_key().hash(&mut hasher);
        self.collapsed_groups.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((cached, grouped)) = &self.grouped
            && *cached == key
        {
            return Arc::clone(grouped);
        }
        let shown = self.shown();
        let mut groups = vec![Vec::new(); GROUP_ORDER.len()];
        for (ordinal, &ix) in shown.iter().enumerate() {
            groups[group_of(self.files[ix].kind)].push(ordinal);
        }
        let mut grouped = GroupedRows::default();
        for (group, ordinals) in groups.into_iter().enumerate() {
            if ordinals.is_empty() {
                continue;
            }
            let collapsed = self.collapsed_groups[group];
            grouped.headers.push(grouped.rows.len());
            grouped.rows.push(GroupedRow::Header {
                group,
                count: ordinals.len(),
                collapsed,
            });
            if !collapsed {
                grouped.rows.extend(
                    ordinals
                        .into_iter()
                        .map(|ordinal| GroupedRow::File { ordinal }),
                );
            }
        }
        #[cfg(test)]
        {
            self.group_builds += 1;
        }
        let grouped = Arc::new(grouped);
        self.grouped = Some((key, Arc::clone(&grouped)));
        grouped
    }

    pub(in crate::view) fn toggle_group(&mut self, group: usize) {
        if let Some(collapsed) = self.collapsed_groups.get_mut(group) {
            *collapsed = !*collapsed;
        }
    }

    pub(in crate::view) fn set_files(&mut self, files: Arc<Vec<CommitFileChange>>, rev: u64) {
        self.files = files;
        self.files_rev = rev;
    }

    pub(in crate::view) fn set_query(&mut self, query: SharedString) {
        self.query = query;
    }

    pub(in crate::view) fn set_sort(&mut self, sort: CommitFileSort) {
        self.sort = sort;
    }

    fn projection_key(&self) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        self.files_rev.hash(&mut hasher);
        Arc::as_ptr(&self.files).hash(&mut hasher);
        self.sort.hash(&mut hasher);
        self.kind_filter.hash(&mut hasher);
        self.query.hash(&mut hasher);
        hasher.finish()
    }

    /// Source indices of the files shown, in display order before grouping.
    pub(in crate::view) fn shown(&mut self) -> Arc<[usize]> {
        let key = self.projection_key();
        if let Some((cached, shown)) = &self.shown
            && *cached == key
        {
            return Arc::clone(shown);
        }
        let projection =
            self.projection_cache
                .projection_for(&key, &self.files, self.sort, self.kind_filter);
        let query = self.query.to_lowercase();
        let shown: Arc<[usize]> = if query.is_empty() {
            Arc::clone(&projection.source_indices)
        } else {
            projection
                .source_indices
                .iter()
                .copied()
                .filter(|&ix| {
                    self.files[ix]
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
                })
                .collect()
        };
        self.shown = Some((key, Arc::clone(&shown)));
        shown
    }

    pub(in crate::view) fn plan(&mut self) -> Arc<FileListPlan> {
        let shown = self.shown();
        let key = self.projection_key();
        let files = Arc::clone(&self.files);
        let sort = self.sort;
        self.plan_cache.plan_for(
            key,
            self.plan_layout(),
            &self.collapsed,
            shown.len(),
            || {
                FileTree::build(
                    shown.iter().map(|&ix| FileTreeItem {
                        path: &files[ix].path,
                        additions: files[ix].additions,
                        deletions: files[ix].deletions,
                    }),
                    sort,
                )
            },
        )
    }

    /// The change a file row shows, with its label and icon.
    pub(in crate::view) fn presentation_at_ordinal(
        &mut self,
        ordinal: usize,
    ) -> Option<(
        CommitFileChange,
        crate::view::rows::CommitFileRowPresentation,
    )> {
        let shown = self.shown();
        let source = *shown.get(ordinal)?;
        let mut hasher = rustc_hash::FxHasher::default();
        self.files_rev.hash(&mut hasher);
        Arc::as_ptr(&self.files).hash(&mut hasher);
        let source_key = hasher.finish();
        let presentations = self.presentations.rows_for(&source_key, &self.files);
        Some((
            self.files.get(source)?.clone(),
            presentations.get(source)?.clone(),
        ))
    }

    pub(in crate::view) fn shown_changes(&mut self) -> Vec<CommitFileChange> {
        let shown = self.shown();
        let ordinals: Vec<usize> = match self.mode {
            FileListMode::Tree => self.plan().ordered().iter().collect(),
            FileListMode::Grouped => {
                let mut ordered = Vec::with_capacity(shown.len());
                for group in 0..GROUP_ORDER.len() {
                    ordered.extend(shown.iter().enumerate().filter_map(|(ordinal, &ix)| {
                        (group_of(self.files[ix].kind) == group).then_some(ordinal)
                    }));
                }
                ordered
            }
            _ => (0..shown.len()).collect(),
        };
        ordinals
            .into_iter()
            .map(|ordinal| self.files[shown[ordinal]].clone())
            .collect()
    }

    pub(in crate::view) fn toggle_dir(
        &mut self,
        key: Arc<Path>,
        chain: &[Arc<Path>],
        collapsed: bool,
    ) {
        if collapsed {
            self.collapsed.expand(chain);
        } else {
            self.collapsed.collapse(key, chain);
        }
    }

    /// Selects `path` if it is shown.
    pub(in crate::view) fn select(&mut self, path: &Path) -> Option<CommitFileChange> {
        let change = self
            .shown_changes()
            .into_iter()
            .find(|change| change.path == path)?;
        self.selected = Some(change.path.clone());
        Some(change)
    }

    pub(in crate::view) fn selected(&self) -> Option<CommitFileChange> {
        let selected = self.selected.as_ref()?;
        self.files
            .iter()
            .find(|change| &change.path == selected)
            .cloned()
    }
}
