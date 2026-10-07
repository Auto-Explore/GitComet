//! Presentation state and caches shared by built-in and hosted changed-file lists.
use super::*;
use crate::view::rows::{
    CollapsedDirs, CommitFileFilter, CommitFileProjectionCache, CommitFileSort, FileListPlan,
    FileListPlanCache, FileTree, FileTreeItem,
};
use gitcomet_core::domain::{CommitFileChange, FileStatusKind};
use gitcomet_extension_api::{FileListGroups, FileListMode, FileListVisible};
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
#[derive(Debug)]
pub(in crate::view) struct GroupedRows {
    pub(in crate::view) rows: Vec<GroupedRow>,
    /// The row of each header, ascending.
    headers: Vec<usize>,
    /// Each group's label, by group index.
    pub(in crate::view) labels: Arc<[SharedString]>,
}

impl GroupedRows {
    /// The row of each header, ascending.
    pub(in crate::view) fn headers(&self) -> Arc<[usize]> {
        self.headers.iter().copied().collect()
    }

    /// The header of the group holding `row`.
    #[cfg(test)]
    pub(in crate::view) fn header_for(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        after.checked_sub(1).map(|ix| self.headers[ix])
    }

    /// The first header below `row`.
    #[cfg(test)]
    pub(in crate::view) fn next_header(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        self.headers.get(after).copied()
    }
}

/// What a grouped list groups by.
enum Grouping {
    ByKind,
    /// The caller's groups; `labels` ends with "Other".
    Custom {
        groups: FileListGroups,
        labels: Arc<[SharedString]>,
    },
}

/// The shown files' ordinals by group.
struct GroupBuckets {
    groups: Vec<Vec<usize>>,
    labels: Arc<[SharedString]>,
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
    grouping: Grouping,
    kind_labels: Arc<[SharedString]>,
    /// Per group; reset when the group labels change.
    collapsed_groups: Vec<bool>,
    visible: Option<FileListVisible>,
    pub(in crate::view) selected: Option<PathBuf>,
    pub(in crate::view) projection_cache: CommitFileProjectionCache<u64>,
    pub(in crate::view) presentations: crate::view::rows::CommitFileRowPresentationCache<u64>,
    pub(in crate::view) plan_cache: FileListPlanCache,
    shown: Option<(u64, Arc<[usize]>)>,
    buckets: Option<(u64, Arc<GroupBuckets>)>,
    grouped: Option<(u64, Arc<GroupedRows>)>,
    /// Row builds (collapse included) and regroup passes.
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) group_builds: usize,
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) bucket_builds: usize,
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
            grouping: Grouping::ByKind,
            kind_labels: (0..GROUP_ORDER.len())
                .map(|group| SharedString::new_static(group_label(group)))
                .collect(),
            collapsed_groups: vec![false; GROUP_ORDER.len()],
            visible: None,
            selected: None,
            projection_cache: CommitFileProjectionCache::default(),
            presentations: Default::default(),
            plan_cache: FileListPlanCache::default(),
            shown: None,
            buckets: None,
            grouped: None,
            #[cfg(any(test, feature = "benchmarks"))]
            group_builds: 0,
            #[cfg(any(test, feature = "benchmarks"))]
            bucket_builds: 0,
        }
    }

    /// Groups by `groups` instead of change kinds (`None`: by kind). New
    /// labels expand every group.
    pub(in crate::view) fn set_groups(&mut self, groups: Option<FileListGroups>) {
        let grouping = match groups {
            None => Grouping::ByKind,
            Some(groups) => Grouping::Custom {
                labels: groups
                    .labels
                    .iter()
                    .cloned()
                    .chain([SharedString::new_static("Other")])
                    .collect(),
                groups,
            },
        };
        let labels = match &grouping {
            Grouping::ByKind => &self.kind_labels,
            Grouping::Custom { labels, .. } => labels,
        };
        if labels[..] != self.group_labels()[..] {
            self.collapsed_groups = vec![false; labels.len()];
        }
        self.grouping = grouping;
    }

    /// Shows only `visible`'s paths (`None`: all).
    pub(in crate::view) fn set_visible(&mut self, visible: Option<FileListVisible>) {
        self.visible = visible;
    }

    /// Whether `visible` is the set this list shows.
    pub(in crate::view) fn shows_only(&self, visible: &FileListVisible) -> bool {
        self.visible.as_ref().is_some_and(|current| {
            current.revision == visible.revision && Arc::ptr_eq(&current.paths, &visible.paths)
        })
    }

    fn group_labels(&self) -> Arc<[SharedString]> {
        match &self.grouping {
            Grouping::ByKind => Arc::clone(&self.kind_labels),
            Grouping::Custom { labels, .. } => Arc::clone(labels),
        }
    }

    /// Changes when the buckets would: the shown files or the grouping.
    fn grouping_key(&self) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        self.projection_key().hash(&mut hasher);
        if let Grouping::Custom { groups, labels } = &self.grouping {
            groups.revision.hash(&mut hasher);
            (std::rc::Rc::as_ptr(&groups.group_of) as *const () as usize).hash(&mut hasher);
            (Arc::as_ptr(labels) as *const () as usize).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The shown files by group: one pass over them, asking a custom
    /// grouping once per file.
    fn buckets(&mut self) -> Arc<GroupBuckets> {
        let key = self.grouping_key();
        if let Some((cached, buckets)) = &self.buckets
            && *cached == key
        {
            return Arc::clone(buckets);
        }
        let shown = self.shown();
        let labels = self.group_labels();
        let mut groups = vec![Vec::new(); labels.len()];
        match &self.grouping {
            Grouping::ByKind => {
                for (ordinal, &ix) in shown.iter().enumerate() {
                    groups[group_of(self.files[ix].kind)].push(ordinal);
                }
            }
            Grouping::Custom { groups: custom, .. } => {
                let other = labels.len() - 1;
                for (ordinal, &ix) in shown.iter().enumerate() {
                    let group = (custom.group_of)(&self.files[ix].path)
                        .filter(|group| *group < other)
                        .unwrap_or(other);
                    groups[group].push(ordinal);
                }
            }
        }
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.bucket_builds += 1;
        }
        let buckets = Arc::new(GroupBuckets { groups, labels });
        self.buckets = Some((key, Arc::clone(&buckets)));
        buckets
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

    /// The grouped rows; rebuilt only when the shown files, the grouping or
    /// the collapsed groups change, and regrouped only for the first two.
    pub(in crate::view) fn grouped(&mut self) -> Arc<GroupedRows> {
        let mut hasher = rustc_hash::FxHasher::default();
        self.grouping_key().hash(&mut hasher);
        self.collapsed_groups.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((cached, grouped)) = &self.grouped
            && *cached == key
        {
            return Arc::clone(grouped);
        }
        let buckets = self.buckets();
        let mut grouped = GroupedRows {
            rows: Vec::new(),
            headers: Vec::new(),
            labels: Arc::clone(&buckets.labels),
        };
        for (group, ordinals) in buckets.groups.iter().enumerate() {
            if ordinals.is_empty() {
                continue;
            }
            let collapsed = self.collapsed_groups.get(group).copied().unwrap_or(false);
            grouped.headers.push(grouped.rows.len());
            grouped.rows.push(GroupedRow::Header {
                group,
                count: ordinals.len(),
                collapsed,
            });
            if !collapsed {
                grouped
                    .rows
                    .extend(ordinals.iter().map(|&ordinal| GroupedRow::File { ordinal }));
            }
        }
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.group_builds += 1;
        }
        let grouped = Arc::new(grouped);
        self.grouped = Some((key, Arc::clone(&grouped)));
        grouped
    }

    /// Which groups are collapsed, by group index.
    pub(in crate::view) fn collapsed_groups(&self) -> &[bool] {
        &self.collapsed_groups
    }

    /// The change-kind groups' labels, for a built-in list grouped by kind.
    pub(in crate::view) fn kind_labels(&self) -> Arc<[SharedString]> {
        Arc::clone(&self.kind_labels)
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
        if let Some(visible) = &self.visible {
            visible.revision.hash(&mut hasher);
            Arc::as_ptr(&visible.paths).hash(&mut hasher);
        }
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
        // Hashed once per rebuild: path comparisons in a tree set are slow.
        let visible: Option<rustc_hash::FxHashSet<&Path>> = self
            .visible
            .as_ref()
            .map(|visible| visible.paths.iter().map(PathBuf::as_path).collect());
        let shown: Arc<[usize]> = if query.is_empty() && visible.is_none() {
            Arc::clone(&projection.source_indices)
        } else {
            projection
                .source_indices
                .iter()
                .copied()
                .filter(|&ix| {
                    let path = &self.files[ix].path;
                    visible
                        .as_ref()
                        .is_none_or(|visible| visible.contains(path.as_path()))
                        && (query.is_empty()
                            || path.to_string_lossy().to_lowercase().contains(&query))
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
        let kinds = Arc::clone(&self.kind_labels);
        self.plan_cache.plan_for(
            key,
            crate::view::rows::PlanShape {
                layout: self.plan_layout(),
                collapsed: &self.collapsed,
                collapsed_groups: &self.collapsed_groups,
                file_count: shown.len(),
            },
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
            || {
                (
                    shown.iter().map(|&ix| group_of(files[ix].kind)).collect(),
                    kinds,
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
            FileListMode::Grouped => self.buckets().groups.iter().flatten().copied().collect(),
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
