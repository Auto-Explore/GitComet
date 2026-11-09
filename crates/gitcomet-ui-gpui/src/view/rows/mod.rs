use super::*;
use gitcomet_core::edit_signature::EditSignature;
use gpui::Pixels;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};

pub(in crate::view) const MAX_LINES_FOR_SYNTAX_HIGHLIGHTING: usize = 4_000;
const MAX_CACHED_LINE_NUMBER: usize = 16_384;

/// Design width of a conflict diff-column line-number cell, shared by the div
/// path (`conflict_diff_line_number_cell`, `conflict_input_row_min_width`) and
/// the canvas path (`conflict_line_no_width`) so the gutter stays aligned.
///
/// Design units, not device pixels: read it through [`conflict_line_no_width`].
pub(in crate::view) const CONFLICT_DIFF_LINE_NO_WIDTH_PX: f32 = 38.0;

/// Design width of the accent/marker bar a conflict row paints at its left edge
/// to flag the active conflict. Mirrors `diff_canvas::DIFF_CHANGE_BAR_WIDTH_PX`.
pub(in crate::view) const CONFLICT_ROW_ACCENT_BAR_WIDTH_PX: f32 = 3.0;

/// Horizontal row padding, `px_2` on each side. The div path spends it through
/// `.px_2()` and the canvas path through `conflict_canvas::px_2`, both of which are
/// rem-derived and so already UI-scaled; this constant exists for the width sums
/// that have to account for it without laying it out.
pub(in crate::view) const CONFLICT_ROW_PADDING_X_PX: f32 = 8.0;

#[inline]
pub(in crate::view) fn conflict_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

#[inline]
pub(in crate::view) fn conflict_line_no_width(scale: impl Into<ui_scale::UiScale>) -> Pixels {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    conflict_scaled_px(
        CONFLICT_DIFF_LINE_NO_WIDTH_PX * scale.appearance.editor_font_size_px as f32 / 13.0,
        ui_scale_percent,
    )
}

#[cfg(test)]
pub(in crate::view) use gitcomet_text::cache::LruCacheMetrics;
pub(in crate::view) use gitcomet_text::cache::{
    FxLruCache, LruCache, new_fx_lru_cache, new_lru_cache,
};

#[derive(Clone, Debug)]
pub(in crate::view) struct CommitFileRowPresentation {
    pub(in crate::view) label: SharedString,
    pub(in crate::view) visuals: CommitFileKindVisuals,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(in crate::view) enum CommitFileSort {
    #[default]
    PathAscending,
    PathDescending,
    FileTypeAscending,
    FileTypeDescending,
    EditSizeAscending,
    EditSizeDescending,
    /// Files changed the same way next to each other: the largest set of
    /// files sharing one edit first, then the files whose edit is their own.
    Edits,
}

impl From<CommitFileSort> for gitcomet_state::model::RepositoryFileSort {
    fn from(sort: CommitFileSort) -> Self {
        match sort {
            CommitFileSort::PathAscending => Self::PathAscending,
            CommitFileSort::PathDescending => Self::PathDescending,
            CommitFileSort::FileTypeAscending => Self::FileTypeAscending,
            CommitFileSort::FileTypeDescending => Self::FileTypeDescending,
            CommitFileSort::EditSizeAscending => Self::EditSizeAscending,
            CommitFileSort::EditSizeDescending => Self::EditSizeDescending,
            CommitFileSort::Edits => Self::Edits,
        }
    }
}

impl From<gitcomet_state::model::RepositoryFileSort> for CommitFileSort {
    fn from(sort: gitcomet_state::model::RepositoryFileSort) -> Self {
        use gitcomet_state::model::RepositoryFileSort;
        match sort {
            RepositoryFileSort::PathAscending => Self::PathAscending,
            RepositoryFileSort::PathDescending => Self::PathDescending,
            RepositoryFileSort::FileTypeAscending => Self::FileTypeAscending,
            RepositoryFileSort::FileTypeDescending => Self::FileTypeDescending,
            RepositoryFileSort::EditSizeAscending => Self::EditSizeAscending,
            RepositoryFileSort::EditSizeDescending => Self::EditSizeDescending,
            RepositoryFileSort::Edits => Self::Edits,
        }
    }
}

impl CommitFileSort {
    pub(in crate::view) const ALL: [Self; 7] = [
        Self::PathAscending,
        Self::PathDescending,
        Self::FileTypeAscending,
        Self::FileTypeDescending,
        Self::EditSizeAscending,
        Self::EditSizeDescending,
        Self::Edits,
    ];

    /// The name the setting is stored under.
    pub(in crate::view) const fn key(self) -> &'static str {
        match self {
            Self::PathAscending => "path_ascending",
            Self::PathDescending => "path_descending",
            Self::FileTypeAscending => "file_type_ascending",
            Self::FileTypeDescending => "file_type_descending",
            Self::EditSizeAscending => "edit_size_smallest",
            Self::EditSizeDescending => "edit_size_largest",
            Self::Edits => "edits",
        }
    }

    pub(in crate::view) fn from_key(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.key() == raw)
    }

    pub(in crate::view) const fn needs_line_stats(self) -> bool {
        match self {
            Self::EditSizeAscending | Self::EditSizeDescending | Self::Edits => true,
            Self::PathAscending
            | Self::PathDescending
            | Self::FileTypeAscending
            | Self::FileTypeDescending => false,
        }
    }

    /// Each option reads "Ascending"/"Descending" its own way -- path A→Z, file
    /// type by extension A→Z -- so the direction word is the same everywhere and
    /// the noun in front says what is being ordered. Edit size keeps
    /// Smallest/Largest, which names the ends of that scale better than a
    /// direction word does.
    pub(in crate::view) const fn label(self) -> &'static str {
        match self {
            Self::PathAscending => "Path: Ascending",
            Self::PathDescending => "Path: Descending",
            Self::FileTypeAscending => "File type: Ascending",
            Self::FileTypeDescending => "File type: Descending",
            Self::EditSizeAscending => "Edit size: Smallest",
            Self::EditSizeDescending => "Edit size: Largest",
            Self::Edits => "Edits: Repeated first",
        }
    }
}

/// The Edits sort's order over one list: files sharing an edit form a
/// cluster, clusters come largest first (ties by their first path), then the
/// files whose edit no other file shares, then those with no edit known.
/// Path order applies inside each of those, so the result is stable.
struct EditClusters {
    by_edit: rustc_hash::FxHashMap<EditSignature, (usize, String)>,
}

impl EditClusters {
    fn new<'a>(items: impl Iterator<Item = (Option<EditSignature>, &'a str)>) -> Self {
        let mut by_edit = rustc_hash::FxHashMap::<EditSignature, (usize, String)>::default();
        for (edit, path_key) in items {
            let Some(edit) = edit else { continue };
            let entry = by_edit
                .entry(edit)
                .or_insert_with(|| (0, path_key.to_owned()));
            entry.0 += 1;
            if path_key < entry.1.as_str() {
                path_key.clone_into(&mut entry.1);
            }
        }
        Self { by_edit }
    }

    /// Compared before the path. The edit itself is last, so two clusters
    /// whose first paths compare equal still do not interleave.
    fn rank(
        &self,
        edit: Option<EditSignature>,
    ) -> (u8, std::cmp::Reverse<usize>, &str, Option<EditSignature>) {
        match edit.and_then(|edit| Some((edit, self.by_edit.get(&edit)?))) {
            Some((edit, (count, first))) if *count > 1 => {
                (0, std::cmp::Reverse(*count), first.as_str(), Some(edit))
            }
            Some(_) => (1, std::cmp::Reverse(0), "", None),
            None => (2, std::cmp::Reverse(0), "", None),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(in crate::view) enum CommitFileFilter {
    #[default]
    All,
    Modified,
    Removed,
    Added,
    Renamed,
}

impl CommitFileFilter {
    pub(in crate::view) const ALL: [Self; 5] = [
        Self::All,
        Self::Modified,
        Self::Removed,
        Self::Added,
        Self::Renamed,
    ];

    pub(in crate::view) const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Modified => "Modified",
            Self::Removed => "Deleted",
            Self::Added => "Added",
            Self::Renamed => "Renamed",
        }
    }

    pub(in crate::view) const fn icon(self) -> &'static str {
        match self {
            Self::All => "icons/file.svg",
            Self::Modified => "icons/pencil.svg",
            Self::Removed => "icons/minus.svg",
            Self::Added => "icons/plus.svg",
            Self::Renamed => "icons/swap.svg",
        }
    }

    /// `scope` names what the counts belong to — "this commit", "this worktree",
    /// "this comparison" — so the same tabs read correctly above every list.
    pub(in crate::view) fn tooltip_in(self, scope: &str, count: usize) -> String {
        match self {
            Self::All => format!("Show every file changed by {scope} ({count})"),
            Self::Modified => format!("Show files modified by {scope} ({count})"),
            Self::Removed => format!("Show files deleted by {scope} ({count})"),
            Self::Added => format!("Show files added by {scope} ({count})"),
            Self::Renamed => format!("Show files renamed by {scope} ({count})"),
        }
    }

    fn matches(self, kind: FileStatusKind) -> bool {
        match self {
            Self::All => true,
            Self::Modified => {
                matches!(kind, FileStatusKind::Modified | FileStatusKind::Conflicted)
            }
            Self::Removed => kind == FileStatusKind::Deleted,
            Self::Added => matches!(kind, FileStatusKind::Added | FileStatusKind::Untracked),
            Self::Renamed => kind == FileStatusKind::Renamed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct CommitFileKindCounts {
    pub(in crate::view) all: usize,
    pub(in crate::view) modified: usize,
    pub(in crate::view) removed: usize,
    pub(in crate::view) added: usize,
    pub(in crate::view) renamed: usize,
}

impl CommitFileKindCounts {
    pub(in crate::view) const fn for_filter(self, filter: CommitFileFilter) -> usize {
        match filter {
            CommitFileFilter::All => self.all,
            CommitFileFilter::Modified => self.modified,
            CommitFileFilter::Removed => self.removed,
            CommitFileFilter::Added => self.added,
            CommitFileFilter::Renamed => self.renamed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct CommitFileProjection {
    pub(in crate::view) source_indices: Arc<[usize]>,
    pub(in crate::view) counts: CommitFileKindCounts,
}

#[derive(Clone, Debug)]
struct CommitFileProjectionCacheEntry<K: Eq + Clone> {
    key: K,
    query: String,
    options: gitcomet_core::text_search::TextSearchOptions,
    projection: Arc<CommitFileProjection>,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct CommitFileProjectionCache<K: Eq + Clone> {
    cached: Option<CommitFileProjectionCacheEntry<K>>,
}

impl<K: Eq + Clone> Default for CommitFileProjectionCache<K> {
    fn default() -> Self {
        Self { cached: None }
    }
}

fn commit_file_edit_size(file: &gitcomet_core::domain::CommitFileChange) -> Option<u64> {
    Some(u64::from(file.line_stats.additions?) + u64::from(file.line_stats.deletions?))
}

fn commit_file_path_sort_key(path: &std::path::Path) -> String {
    super::path_display::path_display_string(path).to_lowercase()
}

/// Groups a file with others of its kind. The extension alone, lowercased, so
/// `.RS` and `.rs` land together; a file without one (Makefile, LICENSE) gets
/// the empty key and they collect at the top. Path order breaks the ties inside
/// a group, which is what keeps a group readable once you are in it.
fn commit_file_type_sort_key(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default()
}

fn compare_commit_file_paths(
    left: &(usize, String, String),
    right: &(usize, String, String),
    files: &[gitcomet_core::domain::CommitFileChange],
) -> std::cmp::Ordering {
    left.1
        .cmp(&right.1)
        .then_with(|| {
            files[left.0]
                .path
                .as_os_str()
                .cmp(files[right.0].path.as_os_str())
        })
        .then_with(|| left.0.cmp(&right.0))
}

/// Display order for one status section. Edit-size sorts use the lane's stats,
/// placing unknown counts last and breaking ties by path.
pub(in crate::view) fn status_section_sorted_indexes(
    entries: &[gitcomet_core::domain::FileStatus],
    indexes: &[usize],
    sort: CommitFileSort,
    stats: Option<&rustc_hash::FxHashMap<std::path::PathBuf, gitcomet_core::domain::LineStats>>,
) -> std::sync::Arc<[usize]> {
    let mut sortable: Vec<(usize, String, String)> = indexes
        .iter()
        .filter_map(|ix| {
            entries.get(*ix).map(|entry| {
                (
                    *ix,
                    commit_file_path_sort_key(&entry.path),
                    matches!(
                        sort,
                        CommitFileSort::FileTypeAscending | CommitFileSort::FileTypeDescending
                    )
                    .then(|| commit_file_type_sort_key(&entry.path))
                    .unwrap_or_default(),
                )
            })
        })
        .collect();

    // Same shape as `build_commit_file_projection`: files with unknown sizes
    // sort last, and path order breaks every tie so the result is stable.
    let edit_size = |ix: usize| -> Option<u64> {
        let entry = entries.get(ix)?;
        let stats = stats?.get(&entry.path)?;
        Some(u64::from(stats.additions?) + u64::from(stats.deletions?))
    };
    let edit = |ix: usize| -> Option<EditSignature> { stats?.get(&entries.get(ix)?.path)?.edit };
    let clusters = (sort == CommitFileSort::Edits)
        .then(|| EditClusters::new(sortable.iter().map(|item| (edit(item.0), item.1.as_str()))));
    let by_path = |left: &(usize, String, String), right: &(usize, String, String)| {
        left.1
            .cmp(&right.1)
            .then_with(|| {
                entries[left.0]
                    .path
                    .as_os_str()
                    .cmp(entries[right.0].path.as_os_str())
            })
            .then_with(|| left.0.cmp(&right.0))
    };

    sortable.sort_by(|left, right| match sort {
        CommitFileSort::PathAscending => by_path(left, right),
        CommitFileSort::PathDescending => by_path(left, right).reverse(),
        CommitFileSort::FileTypeAscending | CommitFileSort::FileTypeDescending => {
            let left_type = &left.2;
            let right_type = &right.2;
            // Only the group order flips; inside a group the path stays A→Z, the
            // same way a descending edit-size sort still falls back to path order.
            let type_order = if sort == CommitFileSort::FileTypeAscending {
                left_type.cmp(right_type)
            } else {
                right_type.cmp(left_type)
            };
            type_order.then_with(|| by_path(left, right))
        }
        CommitFileSort::EditSizeAscending | CommitFileSort::EditSizeDescending => {
            match (edit_size(left.0), edit_size(right.0)) {
                (Some(left_size), Some(right_size)) => {
                    let order = if sort == CommitFileSort::EditSizeAscending {
                        left_size.cmp(&right_size)
                    } else {
                        right_size.cmp(&left_size)
                    };
                    order.then_with(|| by_path(left, right))
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => by_path(left, right),
            }
        }
        CommitFileSort::Edits => clusters
            .as_ref()
            .map(|clusters| {
                clusters
                    .rank(edit(left.0))
                    .cmp(&clusters.rank(edit(right.0)))
            })
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| by_path(left, right)),
    });
    sortable
        .into_iter()
        .map(|(ix, _, _)| ix)
        .collect::<Vec<_>>()
        .into()
}

fn build_commit_file_projection(
    files: &[gitcomet_core::domain::CommitFileChange],
    sort: CommitFileSort,
    filter: CommitFileFilter,
) -> CommitFileProjection {
    let mut counts = CommitFileKindCounts {
        all: files.len(),
        ..Default::default()
    };
    for file in files {
        match file.kind {
            FileStatusKind::Untracked | FileStatusKind::Added => counts.added += 1,
            FileStatusKind::Modified | FileStatusKind::Conflicted => counts.modified += 1,
            FileStatusKind::Deleted => counts.removed += 1,
            FileStatusKind::Renamed => counts.renamed += 1,
        }
    }

    let mut sortable: Vec<(usize, String, String)> = files
        .iter()
        .enumerate()
        .filter(|(_, file)| filter.matches(file.kind))
        .map(|(source_ix, file)| {
            (
                source_ix,
                commit_file_path_sort_key(&file.path),
                matches!(
                    sort,
                    CommitFileSort::FileTypeAscending | CommitFileSort::FileTypeDescending
                )
                .then(|| commit_file_type_sort_key(&file.path))
                .unwrap_or_default(),
            )
        })
        .collect();

    let clusters = (sort == CommitFileSort::Edits).then(|| {
        EditClusters::new(
            sortable
                .iter()
                .map(|item| (files[item.0].line_stats.edit, item.1.as_str())),
        )
    });
    sortable.sort_by(|left, right| match sort {
        CommitFileSort::PathAscending => compare_commit_file_paths(left, right, files),
        CommitFileSort::PathDescending => compare_commit_file_paths(left, right, files).reverse(),
        CommitFileSort::FileTypeAscending | CommitFileSort::FileTypeDescending => {
            let left_type = &left.2;
            let right_type = &right.2;
            // Only the group order flips; inside a group the path stays A→Z, the
            // same way a descending edit-size sort still falls back to path order.
            let type_order = if sort == CommitFileSort::FileTypeAscending {
                left_type.cmp(right_type)
            } else {
                right_type.cmp(left_type)
            };
            type_order.then_with(|| compare_commit_file_paths(left, right, files))
        }
        CommitFileSort::EditSizeAscending | CommitFileSort::EditSizeDescending => {
            let left_size = commit_file_edit_size(&files[left.0]);
            let right_size = commit_file_edit_size(&files[right.0]);
            match (left_size, right_size) {
                (Some(left_size), Some(right_size)) => {
                    let size_order = if sort == CommitFileSort::EditSizeAscending {
                        left_size.cmp(&right_size)
                    } else {
                        right_size.cmp(&left_size)
                    };
                    size_order.then_with(|| compare_commit_file_paths(left, right, files))
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => compare_commit_file_paths(left, right, files),
            }
        }
        CommitFileSort::Edits => clusters
            .as_ref()
            .map(|clusters| {
                clusters
                    .rank(files[left.0].line_stats.edit)
                    .cmp(&clusters.rank(files[right.0].line_stats.edit))
            })
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| compare_commit_file_paths(left, right, files)),
    });

    CommitFileProjection {
        source_indices: sortable
            .into_iter()
            .map(|(source_ix, _, _)| source_ix)
            .collect::<Vec<_>>()
            .into(),
        counts,
    }
}

/// The file filter's matcher, `None` for an empty query (no filtering).
pub(in crate::view) fn path_query_matcher(
    query: &str,
    options: gitcomet_core::text_search::TextSearchOptions,
) -> Option<gitcomet_core::text_search::TextSearchMatcher> {
    (!query.is_empty()).then(|| gitcomet_core::text_search::TextSearchMatcher::new(query, options))
}

/// Whether the filter keeps `path`. An invalid regex keeps nothing, as in the
/// other searches.
pub(in crate::view) fn path_matches_query(
    path: &std::path::Path,
    matcher: &gitcomet_core::text_search::TextSearchMatcher,
) -> bool {
    matcher.regex_error().is_none()
        && matcher.is_match(&super::path_display::path_display_string(path))
}

impl<K: Eq + Clone> CommitFileProjectionCache<K> {
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn projection_for(
        &mut self,
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
        sort: CommitFileSort,
        filter: CommitFileFilter,
    ) -> Arc<CommitFileProjection> {
        self.projection_for_query(key, files, sort, filter, "", Default::default())
    }

    pub(in crate::view) fn projection_for_query(
        &mut self,
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
        sort: CommitFileSort,
        filter: CommitFileFilter,
        query: &str,
        options: gitcomet_core::text_search::TextSearchOptions,
    ) -> Arc<CommitFileProjection> {
        if let Some(entry) = self.cached.as_ref()
            && entry.key == *key
            && entry.query == query
            && entry.options == options
        {
            return Arc::clone(&entry.projection);
        }

        let mut projection = build_commit_file_projection(files, sort, filter);
        if let Some(matcher) = path_query_matcher(query, options) {
            projection.source_indices = projection
                .source_indices
                .iter()
                .copied()
                .filter(|&ix| path_matches_query(&files[ix].path, &matcher))
                .collect();
        }
        let projection = Arc::new(projection);
        self.cached = Some(CommitFileProjectionCacheEntry {
            key: key.clone(),
            query: query.to_owned(),
            options,
            projection: Arc::clone(&projection),
        });
        projection
    }

    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn clear(&mut self) {
        self.cached = None;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CommitFileRowPresentationSignature {
    file_count: usize,
    total_path_bytes: usize,
    primary_hash: u64,
    secondary_hash: u64,
}

fn commit_file_row_presentation_signature(
    files: &[gitcomet_core::domain::CommitFileChange],
) -> CommitFileRowPresentationSignature {
    let mut primary = FxHasher::default();
    let mut secondary = FxHasher::default();
    0x9e37_79b9_7f4a_7c15u64.hash(&mut secondary);

    let mut total_path_bytes = 0usize;
    for (ix, file) in files.iter().enumerate() {
        let path_bytes = file.path.as_os_str().as_encoded_bytes();
        let kind_key = commit_file_visuals(file).kind_key;

        total_path_bytes = total_path_bytes.saturating_add(path_bytes.len());

        ix.hash(&mut primary);
        kind_key.hash(&mut primary);
        path_bytes.hash(&mut primary);
        // The chip is part of the row's presentation.
        file.large_file.hash(&mut primary);

        kind_key.hash(&mut secondary);
        ix.hash(&mut secondary);
        path_bytes.len().hash(&mut secondary);
        path_bytes.hash(&mut secondary);
    }

    files.len().hash(&mut primary);
    total_path_bytes.hash(&mut primary);
    files.len().hash(&mut secondary);
    total_path_bytes.hash(&mut secondary);

    CommitFileRowPresentationSignature {
        file_count: files.len(),
        total_path_bytes,
        primary_hash: primary.finish(),
        secondary_hash: secondary.finish(),
    }
}

#[derive(Clone, Debug)]
struct CommitFileRowPresentationCacheEntry<K: Eq + Clone> {
    key: K,
    signature: CommitFileRowPresentationSignature,
    rows: Arc<[CommitFileRowPresentation]>,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct CommitFileRowPresentationCache<K: Eq + Clone> {
    cached: Option<CommitFileRowPresentationCacheEntry<K>>,
}

impl<K: Eq + Clone> Default for CommitFileRowPresentationCache<K> {
    fn default() -> Self {
        Self { cached: None }
    }
}

impl<K: Eq + Clone> CommitFileRowPresentationCache<K> {
    fn build_entry(
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
        signature: CommitFileRowPresentationSignature,
    ) -> CommitFileRowPresentationCacheEntry<K> {
        let rows: Arc<[CommitFileRowPresentation]> = files
            .iter()
            .map(|file| CommitFileRowPresentation {
                label: super::path_display::path_display_shared_fast(&file.path),
                visuals: commit_file_visuals(file),
            })
            .collect::<Vec<_>>()
            .into();

        CommitFileRowPresentationCacheEntry {
            key: key.clone(),
            signature,
            rows,
        }
    }

    pub(in crate::view) fn rows_for(
        &mut self,
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[CommitFileRowPresentation]> {
        if let Some(entry) = &self.cached
            && entry.key == *key
        {
            return Arc::clone(&entry.rows);
        }
        let signature = commit_file_row_presentation_signature(files);
        if let Some(reused_rows) = self.cached.as_ref().and_then(|entry| {
            if entry.signature == signature {
                Some(entry.rows.clone())
            } else {
                None
            }
        }) {
            self.cached = Some(CommitFileRowPresentationCacheEntry {
                key: key.clone(),
                signature,
                rows: reused_rows.clone(),
            });
            return reused_rows;
        }

        let entry = Self::build_entry(key, files, signature);
        let rows = entry.rows.clone();
        self.cached = Some(entry);
        rows
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[allow(dead_code)]
    pub(in crate::view) fn clear(&mut self) {
        self.cached = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CommitFileKindTone {
    Success,
    Warning,
    Danger,
    Accent,
}

impl CommitFileKindTone {
    #[inline]
    pub(in crate::view) fn color(self, theme: &AppTheme) -> gpui::Rgba {
        match self {
            Self::Success => theme.colors.status.success.foreground,
            Self::Warning => theme.colors.status.warning.foreground,
            Self::Danger => theme.colors.status.danger.foreground,
            Self::Accent => theme.colors.accent.foreground,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct CommitFileKindVisuals {
    pub(in crate::view) icon: &'static str,
    pub(in crate::view) kind_key: u8,
    tone: CommitFileKindTone,
}

impl CommitFileKindVisuals {
    #[inline]
    pub(in crate::view) fn color(self, theme: &AppTheme) -> gpui::Rgba {
        self.tone.color(theme)
    }
}

/// Indexed by `FileStatusKind as usize`, so the order here is the enum's order,
/// not `kind_key`'s. `kind_key` is a separate, stable bucket index that callers
/// count by (`benchmarks/repo_history.rs` reads bucket 0 as "added") and that is
/// hashed into the presentation signature — change an entry's icon or tone
/// freely, but never its `kind_key`.
const COMMIT_FILE_KIND_VISUALS: [CommitFileKindVisuals; 6] = [
    // Untracked. Commit and range file lists cannot contain one, but a linked
    // worktree's list can, and it reads as an addition there — the same fold the
    // status pane, the submodule change rows and the diff title already apply.
    CommitFileKindVisuals {
        icon: "icons/plus.svg",
        kind_key: 4,
        tone: CommitFileKindTone::Success,
    },
    CommitFileKindVisuals {
        icon: "icons/pencil.svg",
        kind_key: 1,
        tone: CommitFileKindTone::Warning,
    },
    CommitFileKindVisuals {
        icon: "icons/plus.svg",
        kind_key: 0,
        tone: CommitFileKindTone::Success,
    },
    CommitFileKindVisuals {
        icon: "icons/minus.svg",
        kind_key: 2,
        tone: CommitFileKindTone::Danger,
    },
    CommitFileKindVisuals {
        icon: "icons/swap.svg",
        kind_key: 3,
        tone: CommitFileKindTone::Accent,
    },
    CommitFileKindVisuals {
        icon: "icons/warning.svg",
        kind_key: 5,
        tone: CommitFileKindTone::Danger,
    },
];

#[inline]
pub(in crate::view) const fn commit_file_kind_visuals(
    kind: FileStatusKind,
) -> CommitFileKindVisuals {
    COMMIT_FILE_KIND_VISUALS[kind as usize]
}

/// Row wash strength. Dark surfaces need a touch more of it than light ones to
/// read at all. These four are the only knobs the tint has -- turn them here.
const ROW_TINT_ALPHA_DARK: f32 = 0.13;
const ROW_TINT_ALPHA_LIGHT: f32 = 0.11;
const CONFLICT_ROW_TINT_ALPHA_DARK: f32 = 0.20;
const CONFLICT_ROW_TINT_ALPHA_LIGHT: f32 = 0.16;

/// Background wash marking a file row's change kind, or `None` for a kind that
/// keeps the plain surface. Modified is that untinted default: in an unstaged
/// list nearly every row is one, so tinting it would drown the add/delete
/// signal the wash exists for.
///
/// Always translucent, and that is load-bearing -- [`tinted_row_bg`] folds this
/// into the hover, selection and pressed fills too, and an opaque tint would
/// flatten all three into one colour.
pub(in crate::view) fn file_kind_row_tint(
    kind: FileStatusKind,
    theme: &AppTheme,
) -> Option<gpui::Rgba> {
    let (color, alpha) = match kind {
        FileStatusKind::Modified => return None,
        FileStatusKind::Untracked | FileStatusKind::Added => (
            theme.colors.status.success.foreground,
            row_tint_alpha(theme.is_dark),
        ),
        FileStatusKind::Deleted => (
            theme.colors.status.danger.foreground,
            row_tint_alpha(theme.is_dark),
        ),
        FileStatusKind::Renamed => (
            theme.colors.accent.foreground,
            row_tint_alpha(theme.is_dark),
        ),
        // Louder than the rest: a conflict is the one kind that blocks you.
        FileStatusKind::Conflicted => (
            theme.colors.status.danger.foreground,
            if theme.is_dark {
                CONFLICT_ROW_TINT_ALPHA_DARK
            } else {
                CONFLICT_ROW_TINT_ALPHA_LIGHT
            },
        ),
    };
    Some(with_alpha(color, alpha))
}

/// Background wash for an explorer row that is normally out of sight; ignored
/// wins over hidden. Hidden rows lean toward the canvas, away from the hover
/// overlay, so they never read as hovered. Translucent like the kind tints.
pub(in crate::view) fn explorer_row_tint(
    theme: &AppTheme,
    hidden: bool,
    ignored: bool,
) -> Option<gpui::Rgba> {
    if ignored {
        Some(with_alpha(theme.colors.status.warning.foreground, 0.10))
    } else if hidden {
        Some(with_alpha(theme.colors.surface.canvas, 0.45))
    } else {
        None
    }
}

#[inline]
fn row_tint_alpha(is_dark: bool) -> f32 {
    if is_dark {
        ROW_TINT_ALPHA_DARK
    } else {
        ROW_TINT_ALPHA_LIGHT
    }
}

/// Fold a row tint into whatever background the row would otherwise wear, so a
/// tinted row still answers hover, selection and press.
#[inline]
pub(in crate::view) fn tinted_row_bg(base: gpui::Rgba, tint: Option<gpui::Rgba>) -> gpui::Rgba {
    tint.map_or(base, |tint| composite_over(base, tint))
}

/// Leading glyph for a file row: the file-type icon in its brand tint. A
/// conflict keeps its warning glyph instead -- the row wash cannot say "this
/// one needs your hands", and the file type is the least useful thing to know
/// about a row you have to go fix.
pub(in crate::view) fn file_row_icon(
    path: &std::path::Path,
    kind: FileStatusKind,
    theme: &AppTheme,
) -> (&'static str, gpui::Rgba) {
    if kind == FileStatusKind::Conflicted {
        return ("icons/warning.svg", theme.colors.status.danger.foreground);
    }
    let icon = crate::view::file_icons::file_icon_for_path(path);
    let color = crate::view::file_icons::file_icon_color(icon, theme.is_dark)
        .unwrap_or(theme.colors.foreground.secondary);
    (icon, color)
}

/// Design size of the kind badge riding on a file row's type icon, and of the
/// disc it sits on. The glyphs are drawn for a 16px box, so below ~10 the
/// pencil turns to mush and the minus reads as nothing at all; the disc is what
/// buys back the contrast the shrink costs.
const FILE_ROW_BADGE_PX: f32 = 10.0;
const FILE_ROW_BADGE_DISC_PX: f32 = 12.0;

/// The change-kind glyph a file row wears on the corner of its type icon, or
/// `None` for the two kinds that go bare.
///
/// Modified is the untouched default throughout -- no wash, and no badge
/// either: the pencil is the badge you would see most and the one that reads
/// worst at this size, and a list where almost every row wears it says nothing.
/// A conflict's own glyph is already the warning triangle, so a second badge on
/// top of it adds nothing.
pub(in crate::view) fn file_row_kind_badge(
    kind: FileStatusKind,
    theme: &AppTheme,
) -> Option<(&'static str, gpui::Rgba)> {
    let visuals = commit_file_kind_visuals(kind);
    match kind {
        FileStatusKind::Modified | FileStatusKind::Conflicted => None,
        _ => Some((visuals.icon, visuals.color(theme))),
    }
}

/// A file row's leading icon slot: the file-type glyph, with the change kind
/// badged on its top-right corner.
pub(in crate::view) fn file_row_icon_slot(
    icon: &'static str,
    color: gpui::Rgba,
    badge: Option<(&'static str, gpui::Rgba)>,
    disc: FileRowBadgeDisc,
    icon_px: f32,
    slot_px: f32,
    ui_scale_percent: u32,
) -> gpui::Div {
    let scaled = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    div()
        .w(scaled(slot_px))
        .h(scaled(slot_px))
        .flex_none()
        // Anchors the badge; the row is `items_center`, so without it the badge
        // would hang off the row box rather than the icon.
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .child(svg_icon(icon, color, scaled(icon_px)))
        .when_some(badge, |slot, (badge_icon, badge_color)| {
            slot.child(
                div()
                    .id("file_row_kind_badge")
                    .absolute()
                    // Out past the slot's corner: the type glyphs fill their
                    // box, so a badge tucked inside would sit on top of one.
                    .top(scaled(-3.0))
                    .right(scaled(-4.0))
                    .size(scaled(FILE_ROW_BADGE_DISC_PX))
                    .rounded_full()
                    .bg(disc.resting)
                    .when_some(disc.hover, |badge, (group, hovered)| {
                        badge.group_hover_style(group, |badge| badge.bg(hovered))
                    })
                    .when_some(disc.pressed, |badge, (group, pressed)| {
                        badge.group_active_style(group, |badge| badge.bg(pressed))
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(svg_icon(badge_icon, badge_color, scaled(FILE_ROW_BADGE_PX))),
            )
        })
}

/// The fills the badge disc wears. It borrows the row's own background so the
/// glyph reads against the type icon underneath -- which means it has to track
/// the row through hover too, or a lit row shows a stale circle under the
/// pointer.
#[derive(Clone)]
pub(in crate::view) struct FileRowBadgeDisc {
    pub(in crate::view) resting: gpui::Rgba,
    /// The row's hover group and the fill it takes inside it. `None` on lists
    /// whose rows carry no group.
    pub(in crate::view) hover: Option<(SharedString, gpui::Rgba)>,
    pressed: Option<(SharedString, gpui::Rgba)>,
}

impl FileRowBadgeDisc {
    /// A disc on a surface that does not react to the pointer, such as the
    /// diff's header.
    pub(in crate::view) fn still(background: gpui::Rgba) -> Self {
        Self {
            resting: background,
            hover: None,
            pressed: None,
        }
    }
}

/// One state/color calculation for file rows and the opaque discs laid over
/// their icons. Change-kind tints remain visible in every interaction state.
pub(in crate::view) struct FileRowInteraction {
    style: crate::kit::interaction::InteractionStyle,
    state: crate::kit::interaction::InteractionState,
    surface: gpui::Rgba,
}

impl FileRowInteraction {
    pub(in crate::view) fn new(
        theme: AppTheme,
        tint: Option<gpui::Rgba>,
        selected: bool,
        open: bool,
    ) -> Self {
        let mut canvas = theme.colors.surface.canvas;
        if canvas.alpha < 1.0 {
            // Custom themes may have translucent canvases. The row and its icon
            // mask need a shared opaque backing to flatten interaction fills.
            let appearance = if theme.is_dark {
                gpui::WindowAppearance::Dark
            } else {
                gpui::WindowAppearance::Light
            };
            let backing = AppTheme::default_for_window_appearance(appearance);
            canvas = composite_over(backing.colors.surface.canvas, canvas);
        }
        let surface = tinted_row_bg(canvas, tint);
        Self {
            style: crate::kit::interaction::InteractionStyle::new(theme).on_surface(surface),
            state: crate::kit::interaction::InteractionState::default()
                .selected(
                    selected,
                    crate::theme::composite_over(
                        surface,
                        crate::theme::with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.16 } else { 0.10 },
                        ),
                    ),
                )
                .open(open),
            surface,
        }
    }

    pub(in crate::view) fn disabled(mut self, disabled: bool) -> Self {
        self.state = self.state.disabled(disabled);
        self
    }

    pub(in crate::view) fn badge_disc(&self, group: SharedString) -> FileRowBadgeDisc {
        use crate::kit::interaction::InteractionFeedback;
        FileRowBadgeDisc {
            pressed: Some((
                group.clone(),
                self.style.resolved_background(
                    self.surface,
                    self.state,
                    InteractionFeedback::Pressed,
                ),
            )),
            resting: self.style.resolved_background(
                self.surface,
                self.state,
                InteractionFeedback::Resting,
            ),
            hover: Some((
                group.clone(),
                self.style.resolved_background(
                    self.surface,
                    self.state,
                    InteractionFeedback::Hovered,
                ),
            )),
        }
    }

    pub(in crate::view) fn apply(
        self,
        row: gpui::Stateful<gpui::Div>,
    ) -> gpui::Stateful<gpui::Div> {
        self.style.apply(row, self.state)
    }
}

#[inline]
fn commit_file_visuals(file: &gitcomet_core::domain::CommitFileChange) -> CommitFileKindVisuals {
    let base = commit_file_kind_visuals(file.kind);
    if file.is_submodule {
        CommitFileKindVisuals {
            icon: "icons/box.svg",
            kind_key: base.kind_key | 0x40,
            tone: base.tone,
        }
    } else {
        base
    }
}

thread_local! {
    static LINE_NUMBER_STRINGS: RefCell<Vec<SharedString>> =
        RefCell::new(vec![SharedString::default()]);
}

fn line_number_string(n: Option<u32>) -> SharedString {
    let Some(n) = n else {
        return SharedString::default();
    };
    let ix = n as usize;
    if ix > MAX_CACHED_LINE_NUMBER {
        return n.to_string().into();
    }
    LINE_NUMBER_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() <= ix {
            let start = cache.len();
            cache.reserve(ix + 1 - start);
            for v in start..=ix {
                cache.push(v.to_string().into());
            }
        }
        cache[ix].clone()
    })
}

mod blame;
mod canvas;
#[cfg(test)]
mod canvas_tests;
mod canvas_text;
mod conflict_canvas;
mod conflict_resolver;
mod diff;
pub(in crate::view) use diff::blame_presentation::BlameLabelCache;

/// A comparison/multi-selection card's per-commit strings, prepared once.
pub(in crate::view) struct CommitCard {
    pub(in crate::view) short_sha: gpui::SharedString,
    pub(in crate::view) summary: gpui::SharedString,
    pub(in crate::view) author: gpui::SharedString,
    pub(in crate::view) unix_secs: Option<i64>,
}

impl CommitCard {
    pub(in crate::view) fn new(commit: gitcomet_core::domain::Commit) -> Self {
        let short_sha: gpui::SharedString = commit
            .id
            .as_ref()
            .get(0..8)
            .unwrap_or(commit.id.as_ref())
            .to_string()
            .into();
        let unix_secs = commit
            .time
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            short_sha,
            summary: gpui::SharedString::from(std::sync::Arc::clone(&commit.summary)),
            author: gpui::SharedString::from(std::sync::Arc::clone(&commit.author)),
            unix_secs: Some(unix_secs),
        }
    }

    pub(in crate::view) fn unloaded(id: &gitcomet_core::domain::CommitId) -> Self {
        Self {
            short_sha: id
                .as_ref()
                .get(..8)
                .unwrap_or(id.as_ref())
                .to_owned()
                .into(),
            summary: "Commit details not loaded".into(),
            author: "".into(),
            unix_secs: None,
        }
    }
}

mod diff_canvas;
mod file_list;
pub(in crate::view) use file_list::{
    ChangedFileRow, CollapsedDirs, DirectoryRowDetail, DirectoryRowProps, DirectoryToggle,
    FileListId, FileListMultiSelection, FileListPlan, FileListPlanCache, FileListRow, FileOrdinal,
    FileTree, FileTreeItem, GroupHeaderProps, PlanShape, RowIx, StickyGroupHeaders,
    changed_file_directory_row, changed_file_row, directory_row, directory_row_detail_for_width,
    file_list_folder_menu_invoker, file_list_projection_key, file_list_projection_key_scoped,
    file_row_indent_px, group_header_row,
};
mod diff_text;
mod history;
pub(in crate::view) use history::history_row_height;
mod history_canvas;
#[cfg(test)]
pub(in crate::view) use history_canvas::{
    history_mark_chip, history_mark_menu_chip, history_mark_menu_invoker, history_row_menu_is_open,
};
pub(in crate::view) mod history_graph_paint;
pub(in crate::view) mod history_node;
mod history_shapes;
mod markdown_document;
mod markdown_flow_text;
pub(in crate::view) mod sidebar;
pub(in crate::view) mod status;
pub(in crate::view) use status::apply_file_list_selection_click;

#[cfg(feature = "benchmarks")]
pub(crate) mod benchmarks;

pub(in crate::view) use self::conflict_resolver::{
    resolved_output_gutter_width, resolved_output_line_no_width,
};
pub(in crate::view) use self::diff::blame_presentation::{BlameRenderCtx, build_row_blame_paint};
pub(in crate::view) use self::diff_canvas::blame_gutter_row_canvas;
#[cfg(test)]
pub(in crate::view) use self::history::take_markdown_image_stats_for_tests;
pub(in crate::view) use self::history::{
    MarkdownDrawnPictures, MarkdownImageRoot, MarkdownPictureContext, MarkdownPreviewHoveredLink,
    MarkdownPreviewImageSource, MarkdownPreviewPictureSizes, MarkdownPreviewQuery,
    MarkdownPreviewRevealRequest, MarkdownRemoteImageAccess, markdown_preview_alert_bar_color,
    markdown_preview_alert_label, markdown_preview_document_path, markdown_preview_flow_image,
    markdown_preview_highlighted_text, markdown_preview_image_source,
    markdown_preview_inline_image, markdown_preview_justify, markdown_preview_local_link_missing,
    markdown_preview_local_link_target, markdown_preview_marker_label,
    markdown_preview_remote_image_url, markdown_preview_reveal_offset_y,
    markdown_preview_row_background, markdown_preview_row_extent,
    markdown_preview_styled_row_with_query, markdown_preview_task_checkbox,
    percent_decode_link_path, worktree_markdown_preview_bar_color,
};
#[cfg(test)]
pub(in crate::view) use self::markdown_document::markdown_block_estimate_for_tests;
pub(in crate::view) use self::markdown_document::{
    MarkdownChangeExtents, MarkdownDocumentBlockCache, MarkdownDocumentBlockScrolls,
    MarkdownDocumentContext, MarkdownDocumentLayoutCache, MarkdownRowBoxes,
    render_markdown_diff_split, render_markdown_document, render_markdown_document_with_blocks,
};
#[cfg(test)]
pub(in crate::view) use self::markdown_flow_text::{
    MarkdownFlowPaintPhase, begin_markdown_flow_font_capture_for_tests,
    begin_markdown_flow_paint_phase_capture_for_tests,
    clear_markdown_selection_paint_log_for_tests, markdown_flow_fonts_for_tests,
    markdown_flow_paint_phases_for_tests, markdown_selection_paint_log_for_tests,
    take_markdown_flow_texts_built_for_tests,
};
pub(in crate::view) use self::markdown_flow_text::{
    markdown_flow_painted_offset, markdown_flow_range_rects, markdown_flow_row_offset,
};
pub(in crate::view) use self::sidebar::active_worktree_paths_by_branch;
pub(in crate::view) use self::sidebar::{
    branch_checked_out_at, branch_held_elsewhere, listed_worktree_paths_by_branch,
};

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use diff_text::has_pending_prepared_diff_syntax_chunk_builds_for_document;
// Every pair kind is painted alike, so only assertions name one; see
// `DiffTextPairMatch::kind`.
pub(in crate::view) use diff_text::{
    BackgroundPreparedDiffSyntaxDocument, DiffSearchMatchEmphasis, DiffSyntaxBudget,
    DiffSyntaxEdit, DiffSyntaxLanguage, DiffSyntaxMode, LiveSyntaxDocument, LiveSyntaxSnapshot,
    LiveSyntaxSyncOutcome, PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES,
    PrepareDiffSyntaxDocumentResult, PreparedDiffSyntaxDocument, PreparedDiffSyntaxLine,
    PreparedDiffSyntaxReparseSeed, SyntaxPair, default_diff_syntax_budget,
    diff_syntax_language_for_path, diff_wrap_ranges_for_text,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds, inject_background_prepared_diff_syntax_document,
    line_display_text, live_syntax_document_supported, live_syntax_reparse,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_document_is_available, prepared_diff_syntax_line_for_inline_diff_row,
    prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_occurrences_at_display_offset,
    prepared_diff_syntax_pair_at_display_offset, prepared_diff_syntax_reparse_seed,
    query_highlight_style, request_syntax_highlights_for_prepared_document_byte_range,
    resolved_output_line_text, shared_byte_affix_bounds, syntax_highlights_for_line,
    whitespace_visible_line_text,
};
#[cfg(test)]
pub(in crate::view) use diff_text::{OCCURRENCE_MAX_TEXT_BYTES, SyntaxPairKind};

pub(in crate::view) use self::diff_canvas::annotation::{
    DIFF_ANNOTATION_COLUMN_WIDTH_PX, DIFF_ANNOTATION_MAX_WIDTH_PX, DIFF_ANNOTATION_MIN_WIDTH_PX,
};

pub(in crate::view) use self::diff_canvas::{
    AnnotArea, DiffStageHover, DiffStageSlot, DiffTextWrapSlice, DiffWrapByteRange,
    diff_change_bar_width as diff_canvas_change_bar_width,
    diff_inline_text_start as diff_canvas_inline_text_start,
    diff_row_horizontal_padding as diff_canvas_row_horizontal_padding,
    diff_single_column_text_start as diff_canvas_single_column_text_start,
    diff_text_wrap_char_width as diff_canvas_text_wrap_char_width, horizontal_column,
    is_streamable_diff_text, whitespace_visible_diff_offset_map,
};
#[cfg(test)]
pub(in crate::view) use self::diff_canvas::{
    DiffPaintRecord, FocusedChangeBlockPaint, clear_diff_paint_log_for_tests,
    clear_focused_change_block_paint_log_for_tests, diff_paint_log_for_tests,
    focused_change_block_paint_log_for_tests,
};

#[cfg(test)]
pub(in crate::view) use diff_text::{
    PreparedDiffSyntaxParseMode, prepare_diff_syntax_document_in_background_text,
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
};

#[cfg(test)]
mod tests;
