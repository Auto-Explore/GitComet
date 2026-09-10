//! Flat/tree presentation shared by every changed-file list.
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
mod render;
#[cfg(test)]
mod tests;

pub(in crate::view) use build::{FileTree, FileTreeItem};
pub(in crate::view) use render::{
    DirectoryRowDetail, DirectoryRowProps, directory_row, directory_row_detail_for_width,
    file_row_indent_px,
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
}

impl FileListId {
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

    pub(in crate::view) fn is_tree(&self) -> bool {
        matches!(self, Self::Tree { .. })
    }

    pub(in crate::view) fn row_len(&self) -> usize {
        match self {
            Self::Flat { len } => *len,
            Self::Tree { rows, .. } => rows.len(),
        }
    }

    pub(in crate::view) fn row_at(&self, row: RowIx) -> Option<FileListRow> {
        match self {
            Self::Flat { len } => (row.0 < *len).then_some(FileListRow::File {
                ordinal: FileOrdinal(row.0),
                depth: 0,
            }),
            Self::Tree { rows, .. } => rows.get(row.0).cloned(),
        }
    }

    /// Ordinals in display order, ignoring collapse.
    #[cfg(test)]
    pub(in crate::view) fn file_count(&self) -> usize {
        match self {
            Self::Flat { len } => *len,
            Self::Tree { ordered, .. } => ordered.len(),
        }
    }

    /// The ordinal a row shows, or `None` for a directory row.
    #[cfg(test)]
    pub(in crate::view) fn ordinal_at(&self, row: RowIx) -> Option<FileOrdinal> {
        match self.row_at(row)? {
            FileListRow::File { ordinal, .. } => Some(ordinal),
            FileListRow::Directory { .. } => None,
        }
    }

    pub(in crate::view) fn ordered(&self) -> FileListOrdered<'_> {
        match self {
            Self::Flat { len } => FileListOrdered::Identity(*len),
            Self::Tree { ordered, .. } => FileListOrdered::Ordered(ordered),
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
            } => display_ix_by_ordinal.get(ordinal.0).copied(),
        }
    }

    pub(in crate::view) fn row_ix_for_ordinal(&self, ordinal: FileOrdinal) -> Option<RowIx> {
        match self {
            Self::Flat { len } => (ordinal.0 < *len).then_some(RowIx(ordinal.0)),
            Self::Tree {
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
    set: FxHashSet<Arc<Path>>,
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
            self.set.remove(segment);
        }
        self.set.insert(key);
        self.rev = self.rev.wrapping_add(1);
    }

    pub(in crate::view) fn expand(&mut self, chain: &[Arc<Path>]) {
        let mut changed = false;
        for segment in chain {
            changed |= self.set.remove(segment);
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

/// Two-stage plan cache. The trie is keyed on the projection alone so a chevron
/// click only re-runs the flatten, not the grouping.
#[derive(Default)]
pub(in crate::view) struct FileListPlanCache {
    tree: Option<(u64, FileTree)>,
    plan: Option<(u64, FileListLayout, u64, Arc<FileListPlan>)>,
}

impl FileListPlanCache {
    pub(in crate::view) fn plan_for(
        &mut self,
        projection_key: u64,
        layout: FileListLayout,
        collapsed: &CollapsedDirs,
        file_count: usize,
        build: impl FnOnce() -> FileTree,
    ) -> Arc<FileListPlan> {
        let collapsed_rev = collapsed.rev();
        if let Some((key, cached_layout, rev, plan)) = self.plan.as_ref()
            && *key == projection_key
            && *cached_layout == layout
            && *rev == collapsed_rev
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
        };
        self.plan = Some((projection_key, layout, collapsed_rev, Arc::clone(&plan)));
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
