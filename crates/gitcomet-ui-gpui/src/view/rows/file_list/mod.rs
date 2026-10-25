//! Flat, tree and grouped presentation shared by every changed-file list.
//!
//! The lists differ in where their files come from, so the plan works purely in
//! *ordinal* space: position within a list's already sorted-and-filtered
//! projection. Each caller keeps its own ordinal -> source mapping.
//!
//! Three index spaces meet here, and the compiler cannot tell them apart:
//!
//! - [`RowIx`] — display row, including directory rows.
//! - [`FileOrdinal`] — position in the projection. Collapse-independent.
//! - source index — into the caller's own backing slice, which each list
//!   resolves through its own projection.

use crate::view::FileListLayout;
use gpui::SharedString;
use rustc_hash::{FxHashMap, FxHashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod build;
mod changed_rows;
mod render;
#[cfg(test)]
mod tests;

pub(in crate::view) use build::{FileTree, FileTreeItem};
pub(in crate::view) use changed_rows::{
    ChangedFileRow, DirectoryToggle, changed_file_directory_row, changed_file_row,
};
pub(in crate::view) use render::{
    DirectoryRowDetail, DirectoryRowProps, GroupHeaderProps, StickyGroupHeaders, directory_row,
    directory_row_detail_for_width, file_list_folder_menu_invoker, file_row_indent_px,
    group_header_row,
};

/// Display row index, directory rows included.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::view) struct RowIx(pub usize);

/// Position within a list's sorted, filtered projection.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::view) struct FileOrdinal(pub usize);

/// Which list a plan, collapsed set or per-list override belongs to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum FileListId {
    Status(crate::view::StatusSection),
    CommitFiles,
    WorktreeFiles,
    RangeFiles,
}

/// Ctrl/Shift-click selection in a commit or comparison file list, with the
/// status lists' anchor rules.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct FileListMultiSelection {
    pub(in crate::view) paths: Vec<std::path::PathBuf>,
    pub(in crate::view) anchor: Option<std::path::PathBuf>,
    pub(in crate::view) anchor_index: Option<usize>,
    pub(in crate::view) anchor_order_rev: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum FileListRow {
    Directory {
        /// Deepest segment of a folded chain — the node whose children hide.
        key: Arc<Path>,
        /// Folded, e.g. `gitcomet-ui-gpui/src/view`.
        label: SharedString,
        depth: usize,
        collapsed: bool,
        /// Every segment of the folded chain, deepest last. Collapse state is
        /// keyed on all of them so it survives the chain splitting or merging.
        chain: DirChain,
        /// Into [`FileListPlan::ordered`]; the subtree's files in tree order.
        subtree: Range<usize>,
        additions: Option<u64>,
        deletions: Option<u64>,
    },
    File {
        ordinal: FileOrdinal,
        depth: usize,
    },
    /// The header of a group of files, in a grouped plan.
    Group {
        group: usize,
        label: SharedString,
        count: usize,
        collapsed: bool,
    },
}

/// Sentinel for an ordinal whose row is hidden under a collapsed directory.
const HIDDEN: usize = usize::MAX;

/// A folded directory chain, deepest segment last.
pub(in crate::view) type DirChain = Arc<[Arc<Path>]>;
/// One collapsed folder's span over `ordered`, paired with the chain to expand.
pub(in crate::view) type CollapsedSpan = (Range<usize>, DirChain);

#[derive(Clone, Debug)]
pub(in crate::view) enum FileListPlan {
    /// Rows map 1:1 onto ordinals, so nothing is materialised.
    Flat { len: usize },
    Tree {
        rows: Arc<[FileListRow]>,
        /// Ordinals in tree order. Collapse-independent: navigation steps
        /// through files a collapsed folder is hiding rather than skipping them.
        ordered: Arc<[usize]>,
        /// Ordinal -> row, or [`HIDDEN`].
        row_ix_by_ordinal: Arc<[usize]>,
        /// Ordinal -> position in `ordered`, i.e. the inverse permutation.
        display_ix_by_ordinal: Arc<[usize]>,
        /// Every collapsed folder's span over `ordered`, nested ones included.
        collapsed_spans: Arc<[CollapsedSpan]>,
    },
    /// Files under a header per non-empty group, in group order; each group
    /// keeps the projection's order.
    Grouped {
        rows: Arc<[FileListRow]>,
        /// Ordinals in group order. Collapse-independent, as in a tree.
        ordered: Arc<[usize]>,
        /// Ordinal -> row, or [`HIDDEN`].
        row_ix_by_ordinal: Arc<[usize]>,
        /// Ordinal -> position in `ordered`.
        display_ix_by_ordinal: Arc<[usize]>,
        /// The row of each header, ascending.
        headers: Arc<[usize]>,
        /// Ordinal -> its group.
        group_by_ordinal: Arc<[usize]>,
    },
}

impl FileListId {
    pub(in crate::view) fn preference_key(self) -> gitcomet_state::model::RepositoryListKind {
        use gitcomet_state::model::RepositoryListKind as Kind;
        match self {
            Self::CommitFiles => Kind::CommitFiles,
            Self::RangeFiles => Kind::RangeFiles,
            Self::WorktreeFiles => Kind::WorktreeFiles,
            Self::Status(crate::view::StatusSection::CombinedUnstaged) => Kind::CombinedUnstaged,
            Self::Status(crate::view::StatusSection::Untracked) => Kind::Untracked,
            Self::Status(crate::view::StatusSection::Unstaged) => Kind::Unstaged,
            Self::Status(crate::view::StatusSection::Staged) => Kind::Staged,
        }
    }

    /// How the filter tooltips name what the counts belong to.
    pub(in crate::view) const fn filter_scope(self) -> &'static str {
        match self {
            Self::Status(_) => "the working tree",
            Self::CommitFiles => "this commit",
            Self::WorktreeFiles => "this worktree",
            Self::RangeFiles => "this comparison",
        }
    }
}

impl FileListPlan {
    pub(in crate::view) fn flat(len: usize) -> Self {
        Self::Flat { len }
    }

    /// Files under `labels`' groups: `groups` gives each ordinal's group and
    /// `collapsed` which groups hide their files. Empty groups get no header.
    pub(in crate::view) fn grouped(
        groups: &[usize],
        labels: &[SharedString],
        collapsed: &[bool],
    ) -> Self {
        let last = labels.len().saturating_sub(1);
        let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); labels.len()];
        for (ordinal, &group) in groups.iter().enumerate() {
            if let Some(bucket) = buckets.get_mut(group.min(last)) {
                bucket.push(ordinal);
            }
        }
        let mut rows = Vec::with_capacity(groups.len() + labels.len());
        let mut headers = Vec::new();
        let mut ordered = Vec::with_capacity(groups.len());
        let mut row_ix_by_ordinal = vec![HIDDEN; groups.len()];
        let mut display_ix_by_ordinal = vec![0; groups.len()];
        let mut group_by_ordinal = vec![0; groups.len()];
        for (group, ordinals) in buckets.iter().enumerate() {
            if ordinals.is_empty() {
                continue;
            }
            let hidden = collapsed.get(group).copied().unwrap_or(false);
            headers.push(rows.len());
            rows.push(FileListRow::Group {
                group,
                label: labels[group].clone(),
                count: ordinals.len(),
                collapsed: hidden,
            });
            for &ordinal in ordinals {
                display_ix_by_ordinal[ordinal] = ordered.len();
                group_by_ordinal[ordinal] = group;
                ordered.push(ordinal);
                if !hidden {
                    row_ix_by_ordinal[ordinal] = rows.len();
                    rows.push(FileListRow::File {
                        ordinal: FileOrdinal(ordinal),
                        depth: 0,
                    });
                }
            }
        }
        Self::Grouped {
            rows: rows.into(),
            ordered: ordered.into(),
            row_ix_by_ordinal: row_ix_by_ordinal.into(),
            display_ix_by_ordinal: display_ix_by_ordinal.into(),
            headers: headers.into(),
            group_by_ordinal: group_by_ordinal.into(),
        }
    }

    /// Whether files sit under their folders, shown by leaf name.
    pub(in crate::view) fn is_tree(&self) -> bool {
        matches!(self, Self::Tree { .. })
    }

    #[cfg(test)]
    pub(in crate::view) fn is_grouped(&self) -> bool {
        matches!(self, Self::Grouped { .. })
    }

    /// Whether the files are drawn in another order than the projection's,
    /// so navigation and ranges have to follow [`Self::ordered`].
    pub(in crate::view) fn reorders(&self) -> bool {
        !matches!(self, Self::Flat { .. })
    }

    pub(in crate::view) fn row_len(&self) -> usize {
        match self {
            Self::Flat { len } => *len,
            Self::Tree { rows, .. } | Self::Grouped { rows, .. } => rows.len(),
        }
    }

    pub(in crate::view) fn row_at(&self, row: RowIx) -> Option<FileListRow> {
        match self {
            Self::Flat { len } => (row.0 < *len).then_some(FileListRow::File {
                ordinal: FileOrdinal(row.0),
                depth: 0,
            }),
            Self::Tree { rows, .. } | Self::Grouped { rows, .. } => rows.get(row.0).cloned(),
        }
    }

    /// The row of each group header, ascending; `None` unless grouped.
    pub(in crate::view) fn headers(&self) -> Option<Arc<[usize]>> {
        match self {
            Self::Grouped { headers, .. } => Some(Arc::clone(headers)),
            Self::Flat { .. } | Self::Tree { .. } => None,
        }
    }

    /// Ordinals in display order, ignoring collapse.
    #[cfg(test)]
    pub(in crate::view) fn file_count(&self) -> usize {
        match self {
            Self::Flat { len } => *len,
            Self::Tree { ordered, .. } | Self::Grouped { ordered, .. } => ordered.len(),
        }
    }

    /// The ordinal a row shows, or `None` for a directory or group row.
    #[cfg(test)]
    pub(in crate::view) fn ordinal_at(&self, row: RowIx) -> Option<FileOrdinal> {
        match self.row_at(row)? {
            FileListRow::File { ordinal, .. } => Some(ordinal),
            FileListRow::Directory { .. } | FileListRow::Group { .. } => None,
        }
    }

    pub(in crate::view) fn ordered(&self) -> FileListOrdered<'_> {
        match self {
            Self::Flat { len } => FileListOrdered::Identity(*len),
            Self::Tree { ordered, .. } | Self::Grouped { ordered, .. } => {
                FileListOrdered::Ordered(ordered)
            }
        }
    }

    /// Where `ordinal` sits among the displayed files — not the ordinal
    /// itself, since a tree hoists directories above files at each level.
    ///
    /// Collapse-independent like [`Self::ordered`]: a shift-click range that
    /// skipped hidden files would act on less than the rows it spans.
    pub(in crate::view) fn display_position(&self, ordinal: FileOrdinal) -> Option<usize> {
        match self {
            Self::Flat { len } => (ordinal.0 < *len).then_some(ordinal.0),
            Self::Tree {
                display_ix_by_ordinal,
                ..
            }
            | Self::Grouped {
                display_ix_by_ordinal,
                ..
            } => display_ix_by_ordinal.get(ordinal.0).copied(),
        }
    }

    pub(in crate::view) fn row_ix_for_ordinal(&self, ordinal: FileOrdinal) -> Option<RowIx> {
        match self {
            Self::Flat { len } => (ordinal.0 < *len).then_some(RowIx(ordinal.0)),
            Self::Tree {
                row_ix_by_ordinal, ..
            }
            | Self::Grouped {
                row_ix_by_ordinal, ..
            } => match row_ix_by_ordinal.get(ordinal.0).copied() {
                Some(HIDDEN) | None => None,
                Some(row) => Some(RowIx(row)),
            },
        }
    }

    /// Folded chains to expand before `ordinal` has a row.
    pub(in crate::view) fn reveal(&self, ordinal: FileOrdinal) -> Vec<DirChain> {
        let Self::Tree {
            collapsed_spans, ..
        } = self
        else {
            return Vec::new();
        };
        if self.row_ix_for_ordinal(ordinal).is_some() {
            return Vec::new();
        }
        let Some(position) = self.display_position(ordinal) else {
            return Vec::new();
        };
        collapsed_spans
            .iter()
            .filter(|(span, _)| span.contains(&position))
            .map(|(_, chain)| Arc::clone(chain))
            .collect()
    }

    /// The collapsed group to expand before `ordinal` has a row.
    pub(in crate::view) fn reveal_group(&self, ordinal: FileOrdinal) -> Option<usize> {
        let Self::Grouped {
            group_by_ordinal, ..
        } = self
        else {
            return None;
        };
        if self.row_ix_for_ordinal(ordinal).is_some() {
            return None;
        }
        group_by_ordinal.get(ordinal.0).copied()
    }
}

pub(in crate::view) enum FileListOrdered<'a> {
    Identity(usize),
    Ordered(&'a [usize]),
}

impl FileListOrdered<'_> {
    #[cfg(test)]
    pub(in crate::view) fn len(&self) -> usize {
        match self {
            Self::Identity(len) => *len,
            Self::Ordered(values) => values.len(),
        }
    }

    pub(in crate::view) fn iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        match self {
            Self::Identity(len) => Box::new(0..*len),
            Self::Ordered(values) => Box::new(values.iter().copied()),
        }
    }
}

/// Per-list collapsed directories. Stores *collapsed*, not expanded, so the
/// default is fully expanded and a file that appears in a new folder is never
/// hidden.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct CollapsedDirs {
    set: Arc<FxHashSet<Arc<Path>>>,
    rev: u64,
}

impl CollapsedDirs {
    pub(in crate::view) fn rev(&self) -> u64 {
        self.rev
    }

    pub(in crate::view) fn set(&self) -> &FxHashSet<Arc<Path>> {
        &self.set
    }

    /// Collapse `key`, dropping the rest of its folded chain: the chain's
    /// boundaries move as files come and go, and a key naming a segment that
    /// later merges away would strand the collapse on no row at all.
    pub(in crate::view) fn collapse(&mut self, key: Arc<Path>, chain: &[Arc<Path>]) {
        for segment in chain {
            Arc::make_mut(&mut self.set).remove(segment);
        }
        Arc::make_mut(&mut self.set).insert(key);
        self.rev = self.rev.wrapping_add(1);
    }

    pub(in crate::view) fn expand(&mut self, chain: &[Arc<Path>]) {
        let mut changed = false;
        for segment in chain {
            changed |= Arc::make_mut(&mut self.set).remove(segment);
        }
        if changed {
            self.rev = self.rev.wrapping_add(1);
        }
    }

    /// Expand the folder `key` names and every folder below it.
    pub(in crate::view) fn expand_under(&mut self, key: &Path, chain: &[Arc<Path>]) {
        let before = self.set.len();
        Arc::make_mut(&mut self.set).retain(|dir| !dir.starts_with(key) && !chain.contains(dir));
        if self.set.len() != before {
            self.rev = self.rev.wrapping_add(1);
        }
    }

    /// Collapse every folder in `dirs`. A row is collapsed when any segment of
    /// its chain is, so marking each directory path covers folded rows too.
    pub(in crate::view) fn collapse_all(&mut self, dirs: impl IntoIterator<Item = Arc<Path>>) {
        let mut changed = false;
        for dir in dirs {
            changed |= Arc::make_mut(&mut self.set).insert(dir);
        }
        if changed {
            self.rev = self.rev.wrapping_add(1);
        }
    }
}

fn arc_path(path: PathBuf) -> Arc<Path> {
    Arc::from(path.as_path())
}

type DirLookup = FxHashMap<Arc<std::ffi::OsStr>, usize>;

/// What a plan is drawn in, beside the projection it is built from.
pub(in crate::view) struct PlanShape<'a> {
    pub(in crate::view) layout: FileListLayout,
    /// Collapsed folders, for a tree.
    pub(in crate::view) collapsed: &'a CollapsedDirs,
    /// Collapsed groups by index, for a grouped plan.
    pub(in crate::view) collapsed_groups: &'a [bool],
    pub(in crate::view) file_count: usize,
}

/// Each ordinal's group and the groups' labels, for a grouped plan.
pub(in crate::view) type GroupsOfOrdinals = (Vec<usize>, Arc<[SharedString]>);

/// Two-stage plan cache. The trie is keyed on the projection alone so a chevron
/// click only re-runs the flatten, not the grouping.
#[derive(Default)]
pub(in crate::view) struct FileListPlanCache {
    tree: Option<(u64, FileTree)>,
    plan: Option<(u64, FileListLayout, u64, Arc<FileListPlan>)>,
    /// Plans built rather than reused; budgets and tests read it.
    #[cfg(any(test, feature = "benchmarks"))]
    builds: u64,
}

impl FileListPlanCache {
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn builds(&self) -> u64 {
        self.builds
    }

    /// The plan last returned, for drawing decorations beside its rows.
    pub(in crate::view) fn current(&self) -> Option<Arc<FileListPlan>> {
        self.plan.as_ref().map(|(_, _, _, plan)| Arc::clone(plan))
    }

    /// `build` makes the trie for a tree, `groups` each ordinal's group for a
    /// grouped plan; neither runs for a layout that does not need it.
    pub(in crate::view) fn plan_for(
        &mut self,
        projection_key: u64,
        shape: PlanShape<'_>,
        build: impl FnOnce() -> FileTree,
        groups: impl FnOnce() -> GroupsOfOrdinals,
    ) -> Arc<FileListPlan> {
        let PlanShape {
            layout,
            collapsed,
            collapsed_groups,
            file_count,
        } = shape;
        let collapse_key = match layout {
            FileListLayout::Flat => 0,
            FileListLayout::Tree => collapsed.rev(),
            FileListLayout::Groups => {
                use std::hash::{Hash, Hasher};
                let mut hasher = rustc_hash::FxHasher::default();
                collapsed_groups.hash(&mut hasher);
                hasher.finish()
            }
        };
        if let Some((key, cached_layout, rev, plan)) = self.plan.as_ref()
            && *key == projection_key
            && *cached_layout == layout
            && *rev == collapse_key
        {
            return Arc::clone(plan);
        }

        let plan = match layout {
            FileListLayout::Flat => Arc::new(FileListPlan::flat(file_count)),
            FileListLayout::Tree => {
                if !matches!(self.tree.as_ref(), Some((key, _)) if *key == projection_key) {
                    self.tree = Some((projection_key, build()));
                }
                let (_, tree) = self.tree.as_ref().expect("tree cached above");
                Arc::new(tree.flatten(collapsed))
            }
            FileListLayout::Groups => {
                let (groups, labels) = groups();
                Arc::new(FileListPlan::grouped(&groups, &labels, collapsed_groups))
            }
        };
        self.plan = Some((projection_key, layout, collapse_key, Arc::clone(&plan)));
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.builds += 1;
        }
        plan
    }
}

/// Cache key for one list's projection. Every input the tree is built from has
/// to be in here, or a stale trie survives a sort or filter change.
pub(in crate::view) fn file_list_projection_key(
    repo: u64,
    rev: u64,
    sort: crate::view::rows::CommitFileSort,
    filter: crate::view::rows::CommitFileFilter,
) -> u64 {
    file_list_projection_key_scoped(repo, rev, sort, filter, None)
}

/// [`file_list_projection_key`] plus what else identifies the list's subject:
/// every worktree in one scan shares a `rev`.
pub(in crate::view) fn file_list_projection_key_scoped(
    repo: u64,
    rev: u64,
    sort: crate::view::rows::CommitFileSort,
    filter: crate::view::rows::CommitFileFilter,
    scope: Option<&Path>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    repo.hash(&mut hasher);
    rev.hash(&mut hasher);
    sort.hash(&mut hasher);
    filter.hash(&mut hasher);
    scope.hash(&mut hasher);
    hasher.finish()
}
