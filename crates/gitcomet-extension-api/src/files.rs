//! File-list descriptors. Decoration revisions never invalidate the file plan.
use gitcomet_ui_kit::gpui::SharedString;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

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

#[derive(Clone, Debug)]
pub struct FileListFilterChip {
    pub label: SharedString,
    pub query: SharedString,
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
