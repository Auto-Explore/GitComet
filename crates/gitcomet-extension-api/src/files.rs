//! File-list descriptors. Decoration revisions never invalidate the file plan.
use gitcomet_ui_kit::gpui::SharedString;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileListSort {
    #[default]
    PathAscending,
    PathDescending,
    FileTypeAscending,
    FileTypeDescending,
    EditSizeAscending,
    EditSizeDescending,
    /// Files changed the same way next to each other: the largest set of
    /// files whose removed and added lines match (each line trimmed) first,
    /// then files whose edit no other file shares, then files with no line
    /// diff (binary, too large, or a source that reports none). Path order
    /// applies within each.
    Edits,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileListFilter {
    #[default]
    All,
    Modified,
    Removed,
    Added,
    Renamed,
}

/// A chip above the list; clicking it applies its filter, clicking it again
/// (shown active) clears it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FileListFilterChip {
    /// Filters by path text, like [`crate::FileList::set_filter`].
    #[non_exhaustive]
    Query {
        label: SharedString,
        query: SharedString,
    },
    /// Shows only these paths, like [`crate::FileList::set_visible`]. Replacing
    /// the chips while it is active applies the new chip of the same label.
    #[non_exhaustive]
    Visible {
        label: SharedString,
        visible: FileListVisible,
    },
}

impl FileListFilterChip {
    pub fn query(label: impl Into<SharedString>, query: impl Into<SharedString>) -> Self {
        Self::Query {
            label: label.into(),
            query: query.into(),
        }
    }

    pub fn visible(label: impl Into<SharedString>, visible: FileListVisible) -> Self {
        Self::Visible {
            label: label.into(),
            visible,
        }
    }

    pub fn label(&self) -> &SharedString {
        match self {
            Self::Query { label, .. } | Self::Visible { label, .. } => label,
        }
    }
}

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FileListMarks {
    /// Bump when replacing row data. The owner checks this in O(1).
    pub revision: u64,
    pub rows: Arc<BTreeMap<PathBuf, crate::RowMark>>,
}

impl FileListMarks {
    pub fn new(revision: u64, rows: impl Into<Arc<BTreeMap<PathBuf, crate::RowMark>>>) -> Self {
        Self {
            revision,
            rows: rows.into(),
        }
    }
}

/// The only paths a list shows, on top of its kind filter and query.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FileListVisible {
    /// Bump when replacing the paths. The owner checks this in O(1).
    pub revision: u64,
    pub paths: Arc<BTreeSet<PathBuf>>,
}

impl FileListVisible {
    pub fn new(revision: u64, paths: impl Into<Arc<BTreeSet<PathBuf>>>) -> Self {
        Self {
            revision,
            paths: paths.into(),
        }
    }
}

/// Which group a path belongs to: an index into the labels, or `None`.
pub type GroupOf = Rc<dyn Fn(&Path) -> Option<usize>>;

/// Groups of a grouped list ([`crate::FileListMode::Grouped`]) in place of
/// change kinds: labels in display order, and each shown file's group, asked
/// once per file when the files, filters or groups change (never while
/// scrolling), so keep `group_of` a hash lookup or cheaper. Files in no group
/// show last, under "Other".
#[derive(Clone)]
#[non_exhaustive]
pub struct FileListGroups {
    /// Bump whenever `group_of` would answer differently.
    pub revision: u64,
    pub labels: Arc<[SharedString]>,
    pub group_of: GroupOf,
}

impl FileListGroups {
    pub fn new(
        revision: u64,
        labels: impl Into<Arc<[SharedString]>>,
        group_of: impl Fn(&Path) -> Option<usize> + 'static,
    ) -> Self {
        Self {
            revision,
            labels: labels.into(),
            group_of: Rc::new(group_of),
        }
    }
}

impl fmt::Debug for FileListGroups {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileListGroups")
            .field("revision", &self.revision)
            .field("labels", &self.labels)
            .finish_non_exhaustive()
    }
}
