use super::*;
use gpui::Pixels;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::num::NonZeroUsize;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct LruCacheMetrics {
    pub(in crate::view) hits: u64,
    pub(in crate::view) misses: u64,
    pub(in crate::view) evictions: u64,
    pub(in crate::view) clears: u64,
}

#[derive(Debug)]
pub(in crate::view) struct InstrumentedLruCache<
    K: std::hash::Hash + Eq,
    V,
    S: std::hash::BuildHasher = lru::DefaultHasher,
> {
    cache: lru::LruCache<K, V, S>,
    metrics: LruCacheMetrics,
}

impl<K: std::hash::Hash + Eq, V> InstrumentedLruCache<K, V> {
    pub(in crate::view) fn new(cap: usize) -> Self {
        Self {
            cache: lru::LruCache::new(non_zero_lru_capacity(cap)),
            metrics: LruCacheMetrics::default(),
        }
    }
}

impl<K: std::hash::Hash + Eq, V, S: std::hash::BuildHasher> InstrumentedLruCache<K, V, S> {
    pub(in crate::view) fn with_hasher(cap: usize, hash_builder: S) -> Self {
        Self {
            cache: lru::LruCache::with_hasher(non_zero_lru_capacity(cap), hash_builder),
            metrics: LruCacheMetrics::default(),
        }
    }

    pub(in crate::view) fn get(&mut self, key: &K) -> Option<&V> {
        let value = self.cache.get(key);
        if value.is_some() {
            self.metrics.hits = self.metrics.hits.saturating_add(1);
        } else {
            self.metrics.misses = self.metrics.misses.saturating_add(1);
        }
        value
    }

    #[cfg(test)]
    pub(in crate::view) fn peek(&self, key: &K) -> Option<&V> {
        self.cache.peek(key)
    }

    pub(in crate::view) fn put(&mut self, key: K, value: V) -> Option<V> {
        let will_evict =
            self.cache.peek(&key).is_none() && self.cache.len() >= self.cache.cap().get();
        let previous = self.cache.put(key, value);
        if will_evict {
            self.metrics.evictions = self.metrics.evictions.saturating_add(1);
        }
        previous
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[allow(dead_code)]
    pub(in crate::view) fn len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(in crate::view) fn clear(&mut self) {
        self.cache.clear();
        self.metrics.clears = self.metrics.clears.saturating_add(1);
    }

    #[cfg(test)]
    pub(in crate::view) fn metrics(&self) -> LruCacheMetrics {
        self.metrics
    }
}

fn non_zero_lru_capacity(cap: usize) -> NonZeroUsize {
    NonZeroUsize::new(cap).expect("LRU cache capacity must be > 0")
}

pub(in crate::view) type LruCache<K, V> = InstrumentedLruCache<K, V>;
/// LRU cache backed by FxHasher for fast hashing of u64 keys (text layout caches).
pub(in crate::view) type FxLruCache<K, V> =
    InstrumentedLruCache<K, V, BuildHasherDefault<FxHasher>>;

pub(in crate::view) fn new_lru_cache<K: std::hash::Hash + Eq, V>(cap: usize) -> LruCache<K, V> {
    InstrumentedLruCache::new(cap)
}

/// Create a new FxHasher-backed LRU cache with the given capacity.
pub(in crate::view) fn new_fx_lru_cache<K: std::hash::Hash + Eq, V>(
    cap: usize,
) -> FxLruCache<K, V> {
    InstrumentedLruCache::with_hasher(cap, BuildHasherDefault::default())
}

/// A VecDeque-backed recency queue for caches that keep their own entry maps
/// and only need the touch/evict-order half of an LRU.
#[derive(Clone, Debug)]
pub(in crate::view) struct LruTouchQueue<K> {
    order: std::collections::VecDeque<K>,
}

impl<K> Default for LruTouchQueue<K> {
    fn default() -> Self {
        Self {
            order: std::collections::VecDeque::new(),
        }
    }
}

impl<K: Eq> LruTouchQueue<K> {
    pub(in crate::view) fn touch(&mut self, key: K) {
        if self.order.back() == Some(&key) {
            return;
        }
        if let Some(pos) = self.order.iter().position(|existing| *existing == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
    }

    pub(in crate::view) fn remove(&mut self, key: &K) -> bool {
        if let Some(pos) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(pos);
            true
        } else {
            false
        }
    }

    pub(in crate::view) fn back(&self) -> Option<&K> {
        self.order.back()
    }

    #[cfg(test)]
    pub(in crate::view) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) fn clear(&mut self) {
        self.order.clear();
    }

    pub(in crate::view) fn pop_oldest(&mut self) -> Option<K> {
        self.order.pop_front()
    }
}

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
}

impl CommitFileSort {
    pub(in crate::view) const ALL: [Self; 6] = [
        Self::PathAscending,
        Self::PathDescending,
        Self::FileTypeAscending,
        Self::FileTypeDescending,
        Self::EditSizeAscending,
        Self::EditSizeDescending,
    ];

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
    Some(u64::from(file.additions?) + u64::from(file.deletions?))
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

impl<K: Eq + Clone> CommitFileProjectionCache<K> {
    pub(in crate::view) fn projection_for(
        &mut self,
        key: &K,
        files: &[gitcomet_core::domain::CommitFileChange],
        sort: CommitFileSort,
        filter: CommitFileFilter,
    ) -> Arc<CommitFileProjection> {
        if let Some(entry) = self.cached.as_ref()
            && entry.key == *key
        {
            return Arc::clone(&entry.projection);
        }

        let projection = Arc::new(build_commit_file_projection(files, sort, filter));
        self.cached = Some(CommitFileProjectionCacheEntry {
            key: key.clone(),
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
        let signature = commit_file_row_presentation_signature(files);
        if let Some(reused_rows) = self.cached.as_ref().and_then(|entry| {
            if entry.key == *key || entry.signature == signature {
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

/// [`tinted_row_bg`] for a fill that is itself translucent: it has to be
/// flattened onto the row's surface first, or the tint would land under it and
/// disappear.
#[inline]
pub(in crate::view) fn tinted_row_overlay_bg(
    surface: gpui::Rgba,
    overlay: gpui::Rgba,
    tint: Option<gpui::Rgba>,
) -> gpui::Rgba {
    match tint {
        None => overlay,
        Some(_) => tinted_row_bg(composite_over(surface, overlay), tint),
    }
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
                    .absolute()
                    // Out past the slot's corner: the type glyphs fill their
                    // box, so a badge tucked inside would sit on top of one.
                    .top(scaled(-3.0))
                    .right(scaled(-4.0))
                    .size(scaled(FILE_ROW_BADGE_DISC_PX))
                    .rounded_full()
                    .bg(disc.resting)
                    .when_some(disc.hover, |badge, (group, hovered)| {
                        badge.group_hover(group, |badge| badge.bg(hovered))
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
pub(in crate::view) use diff::BlameLabelCache;

/// A comparison/multi-selection card's per-commit strings, prepared once.
pub(in crate::view) struct CommitCard {
    pub(in crate::view) short_sha: gpui::SharedString,
    pub(in crate::view) summary: gpui::SharedString,
    pub(in crate::view) author: gpui::SharedString,
    pub(in crate::view) unix_secs: i64,
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
            unix_secs,
        }
    }
}

mod diff_canvas;
mod file_list;
pub(in crate::view) use file_list::{
    CollapsedDirs, DirectoryRowDetail, DirectoryRowProps, FileListId, FileListPlan,
    FileListPlanCache, FileListRow, FileOrdinal, FileTree, FileTreeItem, RowIx, directory_row,
    directory_row_detail_for_width, file_list_projection_key, file_list_projection_key_scoped,
    file_row_indent_px,
};
mod diff_text;
mod history;
pub(in crate::view) use history::history_row_height;
mod history_canvas;
pub(in crate::view) mod history_graph_paint;
mod markdown_document;
mod markdown_flow_text;
pub(in crate::view) mod sidebar;
mod status;

#[cfg(feature = "benchmarks")]
pub(crate) mod benchmarks;

pub(in crate::view) use self::conflict_resolver::{
    resolved_output_gutter_width, resolved_output_line_no_width,
};
pub(in crate::view) use self::diff::{BlameRenderCtx, build_row_blame_paint};
pub(in crate::view) use self::diff_canvas::blame_gutter_row_canvas;
pub(in crate::view) use self::history::{
    MarkdownPreviewImageSource, MarkdownPreviewPictureSizes, MarkdownPreviewQuery,
    MarkdownPreviewRevealRequest, MarkdownRemoteImageAccess, markdown_preview_alert_bar_color,
    markdown_preview_alert_label, markdown_preview_flow_image, markdown_preview_highlighted_text,
    markdown_preview_image_source, markdown_preview_inline_image, markdown_preview_marker_label,
    markdown_preview_remote_image_url, markdown_preview_reveal_offset_y,
    markdown_preview_row_background, markdown_preview_row_extent,
    markdown_preview_styled_row_with_query, worktree_markdown_preview_bar_color,
};
pub(in crate::view) use self::markdown_document::{
    MarkdownDocumentBlockCache, MarkdownDocumentBlockScrolls, MarkdownDocumentContext,
    render_markdown_document,
};
#[cfg(test)]
pub(in crate::view) use self::markdown_flow_text::{
    MarkdownFlowPaintPhase, begin_markdown_flow_paint_phase_capture_for_tests,
    clear_markdown_selection_paint_log_for_tests, markdown_flow_paint_phases_for_tests,
    markdown_selection_paint_log_for_tests,
};
pub(in crate::view) use self::markdown_flow_text::{
    markdown_flow_painted_offset, markdown_flow_row_offset,
};
pub(in crate::view) use self::sidebar::active_workspace_paths_by_branch;
pub(in crate::view) use self::sidebar::listed_workspace_paths_by_branch;

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use diff_text::has_pending_prepared_diff_syntax_chunk_builds_for_document;
// Every pair kind is painted alike, so only assertions name one; see
// `DiffTextPairMatch::kind`.
pub(in crate::view) use diff_text::{
    BackgroundPreparedDiffSyntaxDocument, DiffSearchMatchEmphasis, DiffSyntaxBudget,
    DiffSyntaxEdit, DiffSyntaxLanguage, DiffSyntaxMode, LiveSyntaxDocument, LiveSyntaxSnapshot,
    LiveSyntaxSyncOutcome, PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES,
    PrepareDiffSyntaxDocumentResult, PreparedDiffSyntaxDocument, PreparedDiffSyntaxLine,
    PreparedDiffSyntaxReparseSeed, SyntaxPair, diff_syntax_language_for_code_fence_info,
    diff_syntax_language_for_path, diff_wrap_ranges_for_text,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds, inject_background_prepared_diff_syntax_document,
    live_syntax_document_supported, live_syntax_reparse,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_document_is_available, prepared_diff_syntax_line_for_inline_diff_row,
    prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_occurrences_at_display_offset,
    prepared_diff_syntax_pair_at_display_offset, prepared_diff_syntax_reparse_seed,
    query_highlight_colors, request_syntax_highlights_for_prepared_document_byte_range,
    resolved_output_line_text, shared_byte_affix_bounds, syntax_highlights_for_line,
    whitespace_visible_line_text,
};
#[cfg(test)]
pub(in crate::view) use diff_text::{OCCURRENCE_MAX_TEXT_BYTES, SyntaxPairKind};

pub(in crate::view) use self::diff_canvas::{
    AnnotArea, DIFF_ANNOTATION_COLUMN_WIDTH_PX, DIFF_ANNOTATION_MAX_WIDTH_PX,
    DIFF_ANNOTATION_MIN_WIDTH_PX, DiffStageHover, DiffStageSlot, DiffTextWrapSlice,
    DiffWrapByteRange, diff_change_bar_width as diff_canvas_change_bar_width,
    diff_inline_text_start as diff_canvas_inline_text_start,
    diff_row_horizontal_padding as diff_canvas_row_horizontal_padding,
    diff_single_column_text_start as diff_canvas_single_column_text_start,
    diff_text_wrap_char_width as diff_canvas_text_wrap_char_width, is_streamable_diff_text,
    whitespace_visible_diff_offset_map,
};
#[cfg(test)]
pub(in crate::view) use self::diff_canvas::{
    DiffPaintRecord, clear_diff_paint_log_for_tests, diff_paint_log_for_tests,
};

#[cfg(test)]
pub(in crate::view) use diff_text::{
    PreparedDiffSyntaxParseMode, prepare_diff_syntax_document_in_background_text,
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
};

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitFileChange, FileStatusKind};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// The five bundled themes, so a tint invariant is proved against every
    /// palette that ships rather than the default dark one alone.
    fn bundled_themes() -> Vec<AppTheme> {
        [
            "gitcomet_dark",
            "gitcomet_light",
            "tokyo_night",
            "amber_dark",
            "sunset_veil",
        ]
        .into_iter()
        .map(|key| AppTheme::from_key(key).unwrap_or_else(|| panic!("bundled theme `{key}`")))
        .collect()
    }

    #[test]
    fn file_row_icon_names_the_file_type() {
        let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");

        let (icon, color) =
            file_row_icon(Path::new("src/main.rs"), FileStatusKind::Modified, &theme);
        assert_eq!(icon, "icons/file_icons/rust.svg");
        assert_ne!(
            color, theme.colors.foreground.secondary,
            "a known type should carry its brand tint, not the neutral fallback",
        );

        // Same file, every non-conflict kind: the glyph is the type, not the change.
        for kind in [
            FileStatusKind::Untracked,
            FileStatusKind::Added,
            FileStatusKind::Deleted,
            FileStatusKind::Renamed,
        ] {
            assert_eq!(
                file_row_icon(Path::new("src/main.rs"), kind, &theme).0,
                "icons/file_icons/rust.svg",
            );
        }

        let (icon, _) = file_row_icon(Path::new("nope.wat-is-this"), FileStatusKind::Added, &theme);
        assert_eq!(icon, "icons/file_icons/file.svg", "unknown types fall back");
    }

    #[test]
    fn conflicts_keep_their_warning_glyph() {
        let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");
        let (icon, color) =
            file_row_icon(Path::new("src/main.rs"), FileStatusKind::Conflicted, &theme);

        assert_eq!(icon, "icons/warning.svg");
        assert_eq!(color, theme.colors.status.danger.foreground);
    }

    #[test]
    fn only_added_deleted_and_renamed_wear_a_badge() {
        let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");

        for bare in [FileStatusKind::Modified, FileStatusKind::Conflicted] {
            assert_eq!(
                file_row_kind_badge(bare, &theme),
                None,
                "{bare:?} goes bare: the pencil reads worst at badge size and a \
                 conflict already shows a warning triangle",
            );
        }
        for kind in [
            FileStatusKind::Untracked,
            FileStatusKind::Added,
            FileStatusKind::Deleted,
            FileStatusKind::Renamed,
        ] {
            let (icon, color) = file_row_kind_badge(kind, &theme).expect("badged kind");
            let visuals = commit_file_kind_visuals(kind);
            assert_eq!(icon, visuals.icon, "{kind:?} badge reuses the kind glyph");
            assert_eq!(color, visuals.color(&theme));
        }
    }

    /// The badge disc is punched out of the row, so its resting fill has to be
    /// the row's resting fill and its hover fill the row's hover fill -- a disc
    /// that does not move leaves a stale circle under the pointer.
    #[test]
    fn badge_disc_tracks_the_row_it_sits_on() {
        for theme in bundled_themes() {
            for kind in [FileStatusKind::Modified, FileStatusKind::Deleted] {
                let tint = file_kind_row_tint(kind, &theme);
                let resting = tinted_row_bg(theme.colors.surface.canvas, tint);
                let hovered = tinted_row_bg(theme.colors.interaction.hover_background, tint);
                assert_ne!(
                    resting, hovered,
                    "{kind:?} disc must have somewhere to move to",
                );
            }
        }
    }

    #[test]
    fn only_modified_rows_go_untinted() {
        for theme in bundled_themes() {
            assert_eq!(
                file_kind_row_tint(FileStatusKind::Modified, &theme),
                None,
                "modified is the untinted default",
            );
            for kind in [
                FileStatusKind::Untracked,
                FileStatusKind::Added,
                FileStatusKind::Deleted,
                FileStatusKind::Renamed,
                FileStatusKind::Conflicted,
            ] {
                assert!(
                    file_kind_row_tint(kind, &theme).is_some(),
                    "{kind:?} should be tinted",
                );
            }
        }
    }

    /// The invariant the whole treatment rests on: `.bg()` replaces rather than
    /// layers, so an opaque tint would flatten resting, hover and press into one
    /// colour and the row would stop answering the mouse.
    #[test]
    fn every_tint_is_translucent_and_leaves_hover_visible() {
        for theme in bundled_themes() {
            for kind in [
                FileStatusKind::Untracked,
                FileStatusKind::Added,
                FileStatusKind::Deleted,
                FileStatusKind::Renamed,
                FileStatusKind::Conflicted,
            ] {
                let tint = file_kind_row_tint(kind, &theme).expect("tinted kind");
                assert!(tint.alpha < 1.0, "{kind:?} tint must stay translucent");

                let resting = tinted_row_bg(theme.colors.surface.canvas, Some(tint));
                let hovered = tinted_row_bg(theme.colors.interaction.hover_background, Some(tint));
                let pressed =
                    tinted_row_bg(theme.colors.interaction.pressed_background, Some(tint));

                assert_ne!(resting, hovered, "{kind:?} row must still answer hover");
                assert_ne!(hovered, pressed, "{kind:?} row must still answer press");
                assert_ne!(
                    resting, theme.colors.surface.canvas,
                    "{kind:?} tint must actually shift the surface",
                );
            }
        }
    }

    /// A translucent selection fill has to be flattened before the tint goes on
    /// top, or the tint lands underneath it and vanishes.
    #[test]
    fn translucent_fills_keep_their_tint() {
        let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");
        let selected = with_alpha(theme.colors.accent.foreground, 0.16);
        let tint = file_kind_row_tint(FileStatusKind::Deleted, &theme).expect("tinted kind");

        assert_eq!(
            tinted_row_overlay_bg(theme.colors.surface.canvas, selected, None),
            selected,
            "an untinted row keeps its original translucent fill",
        );
        let tinted = tinted_row_overlay_bg(theme.colors.surface.canvas, selected, Some(tint));
        assert_eq!(tinted.alpha, 1.0, "the flattened fill is opaque");
        assert_ne!(
            tinted,
            composite_over(theme.colors.surface.canvas, selected),
            "the tint must survive the flatten",
        );
    }

    fn reset_line_number_string_cache() {
        LINE_NUMBER_STRINGS.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.clear();
            cache.push(SharedString::default());
        });
    }

    fn line_number_string_cache_len() -> usize {
        LINE_NUMBER_STRINGS.with(|cache| cache.borrow().len())
    }

    #[test]
    fn line_number_cache_does_not_grow_for_uncached_large_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(8)), SharedString::from("8"));
        assert_eq!(line_number_string_cache_len(), 9);

        let uncached_line = (MAX_CACHED_LINE_NUMBER as u32).saturating_add(1);
        assert_eq!(
            line_number_string(Some(uncached_line)),
            uncached_line.to_string()
        );
        assert_eq!(line_number_string_cache_len(), 9);
    }

    #[test]
    fn line_number_cache_still_caches_small_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string(Some(3)), SharedString::from("3"));
        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string_cache_len(), 4);
    }

    #[test]
    fn lru_cache_evicts_least_recently_used() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(8);
        for key in 0..8u64 {
            cache.put(key, key);
        }
        assert_eq!(cache.len(), 8);

        // Insert a 9th entry — should evict key 0 (LRU)
        cache.put(999, 999);
        assert_eq!(cache.len(), 8);
        assert!(cache.peek(&999).is_some());
        assert!(cache.peek(&0).is_none(), "LRU entry should be evicted");
        assert!(cache.peek(&7).is_some(), "MRU entry should remain");
    }

    #[test]
    fn lru_cache_promotes_on_get() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(4);
        for key in 0..4u64 {
            cache.put(key, key);
        }

        // Access key 0 to promote it to MRU
        assert_eq!(cache.get(&0), Some(&0));

        // Insert 4 more entries — key 0 should survive (was promoted)
        cache.put(10, 10);
        cache.put(11, 11);
        cache.put(12, 12);

        assert!(cache.peek(&0).is_some(), "promoted entry should survive");
        assert!(
            cache.peek(&1).is_none(),
            "unpromoted old entry should be evicted"
        );
    }

    #[test]
    fn lru_cache_metrics_track_hits_misses_evictions_and_clears() {
        let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(2);

        assert_eq!(cache.get(&1), None);
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 0,
                misses: 1,
                evictions: 0,
                clears: 0,
            }
        );

        cache.put(1, 10);
        cache.put(2, 20);
        assert_eq!(cache.get(&1), Some(&10));
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 0,
                clears: 0,
            }
        );

        cache.put(3, 30);
        assert_eq!(
            cache.peek(&2),
            None,
            "least-recently used entry should evict"
        );
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 1,
                clears: 0,
            }
        );

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(
            cache.metrics(),
            LruCacheMetrics {
                hits: 1,
                misses: 1,
                evictions: 1,
                clears: 1,
            }
        );
    }

    #[test]
    fn commit_file_row_presentation_cache_reuses_same_key_and_invalidates_on_new_key() {
        let mut cache: CommitFileRowPresentationCache<u64> =
            CommitFileRowPresentationCache::default();
        let files = vec![
            CommitFileChange {
                path: PathBuf::from("src/lib.rs"),
                kind: FileStatusKind::Modified,
                is_submodule: false,
                additions: None,
                deletions: None,
            },
            CommitFileChange {
                path: PathBuf::from("README.md"),
                kind: FileStatusKind::Added,
                is_submodule: false,
                additions: None,
                deletions: None,
            },
        ];

        let first = cache.rows_for(&7, &files);
        let reused = cache.rows_for(
            &7,
            &[CommitFileChange {
                path: PathBuf::from("should/not/appear.rs"),
                kind: FileStatusKind::Deleted,
                is_submodule: false,
                additions: None,
                deletions: None,
            }],
        );

        assert!(Arc::ptr_eq(&first, &reused));
        assert_eq!(
            first
                .iter()
                .map(|row| row.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["src/lib.rs", "README.md"]
        );
        assert_eq!(
            first[0].visuals,
            commit_file_kind_visuals(FileStatusKind::Modified)
        );
        assert_eq!(
            first[1].visuals,
            commit_file_kind_visuals(FileStatusKind::Added)
        );

        let replacement = cache.rows_for(
            &8,
            &[CommitFileChange {
                path: PathBuf::from("docs/guide.md"),
                kind: FileStatusKind::Renamed,
                is_submodule: false,
                additions: None,
                deletions: None,
            }],
        );

        assert!(!Arc::ptr_eq(&first, &replacement));
        assert_eq!(
            replacement
                .iter()
                .map(|row| row.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["docs/guide.md"]
        );
        assert_eq!(
            replacement[0].visuals,
            commit_file_kind_visuals(FileStatusKind::Renamed)
        );
    }

    fn commit_file(
        path: &str,
        kind: FileStatusKind,
        additions: Option<u32>,
        deletions: Option<u32>,
    ) -> CommitFileChange {
        CommitFileChange {
            path: PathBuf::from(path),
            kind,
            is_submodule: false,
            additions,
            deletions,
        }
    }

    #[test]
    fn commit_file_projection_counts_the_whole_commit_and_filters_by_kind() {
        let files = vec![
            commit_file("modified.rs", FileStatusKind::Modified, Some(1), Some(2)),
            commit_file("conflicted.rs", FileStatusKind::Conflicted, None, None),
            commit_file("removed.rs", FileStatusKind::Deleted, Some(0), Some(3)),
            commit_file("added.rs", FileStatusKind::Added, Some(4), Some(0)),
            commit_file("untracked.rs", FileStatusKind::Untracked, None, None),
            commit_file("renamed.rs", FileStatusKind::Renamed, Some(0), Some(0)),
        ];

        let projection = build_commit_file_projection(
            &files,
            CommitFileSort::PathAscending,
            CommitFileFilter::Modified,
        );

        assert_eq!(projection.source_indices.as_ref(), &[1, 0]);
        assert_eq!(
            projection.counts,
            CommitFileKindCounts {
                all: 6,
                modified: 2,
                removed: 1,
                added: 2,
                renamed: 1,
            }
        );
        assert_eq!(
            CommitFileFilter::ALL.map(|filter| projection.counts.for_filter(filter)),
            [6, 2, 1, 2, 1]
        );
        assert_eq!(
            CommitFileFilter::ALL.map(CommitFileFilter::label),
            ["All", "Modified", "Deleted", "Added", "Renamed"]
        );
        assert_eq!(
            CommitFileFilter::Removed.tooltip_in("this commit", 1),
            "Show files deleted by this commit (1)"
        );
        assert_eq!(
            CommitFileFilter::Removed.tooltip_in("this worktree", 2),
            "Show files deleted by this worktree (2)"
        );
    }

    #[test]
    fn commit_file_projection_sorts_paths_case_insensitively_with_stable_ties() {
        let files = vec![
            commit_file("src/zeta.rs", FileStatusKind::Modified, None, None),
            commit_file("src/Alpha.rs", FileStatusKind::Modified, None, None),
            commit_file("src/alpha.rs", FileStatusKind::Modified, None, None),
            commit_file("src/Alpha.rs", FileStatusKind::Modified, None, None),
        ];

        let ascending = build_commit_file_projection(
            &files,
            CommitFileSort::PathAscending,
            CommitFileFilter::All,
        );
        let descending = build_commit_file_projection(
            &files,
            CommitFileSort::PathDescending,
            CommitFileFilter::All,
        );

        assert_eq!(ascending.source_indices.as_ref(), &[1, 3, 2, 0]);
        assert_eq!(descending.source_indices.as_ref(), &[0, 2, 3, 1]);
    }

    #[test]
    fn commit_file_projection_sorts_edit_size_with_unknown_stats_last() {
        let files = vec![
            commit_file("z-large.rs", FileStatusKind::Modified, Some(7), Some(3)),
            commit_file("b-small.rs", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("unknown.rs", FileStatusKind::Modified, None, None),
            commit_file("a-small.rs", FileStatusKind::Modified, Some(0), Some(2)),
        ];

        let ascending = build_commit_file_projection(
            &files,
            CommitFileSort::EditSizeAscending,
            CommitFileFilter::All,
        );
        let descending = build_commit_file_projection(
            &files,
            CommitFileSort::EditSizeDescending,
            CommitFileFilter::All,
        );

        assert_eq!(ascending.source_indices.as_ref(), &[3, 1, 0, 2]);
        assert_eq!(descending.source_indices.as_ref(), &[0, 3, 1, 2]);
    }

    #[test]
    fn file_type_sort_groups_by_extension_then_path() {
        let files = vec![
            commit_file("src/ui/view.ts", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("Makefile", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("src/main.rs", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("Cargo.toml", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("src/ui/app.ts", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file(
                "src/core/lib.RS",
                FileStatusKind::Modified,
                Some(1),
                Some(1),
            ),
        ];

        let ascending = build_commit_file_projection(
            &files,
            CommitFileSort::FileTypeAscending,
            CommitFileFilter::All,
        );

        // "" (Makefile) < rs < toml < ts, and `.RS` groups with `.rs`.
        assert_eq!(ascending.source_indices.as_ref(), &[1, 5, 2, 3, 4, 0]);
    }

    #[test]
    fn file_type_descending_flips_the_groups_but_not_the_paths_inside_them() {
        let files = vec![
            commit_file("b.rs", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("a.ts", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("a.rs", FileStatusKind::Modified, Some(1), Some(1)),
            commit_file("b.ts", FileStatusKind::Modified, Some(1), Some(1)),
        ];

        let descending = build_commit_file_projection(
            &files,
            CommitFileSort::FileTypeDescending,
            CommitFileFilter::All,
        );

        // ts before rs, but a.* before b.* within each -- reading a group is
        // still alphabetical, only the group order reverses.
        assert_eq!(descending.source_indices.as_ref(), &[1, 3, 2, 0]);
    }

    /// The direction word is the same across options; only the noun changes.
    #[test]
    fn sort_labels_name_what_is_ordered_and_which_way() {
        assert_eq!(CommitFileSort::PathAscending.label(), "Path: Ascending");
        assert_eq!(CommitFileSort::PathDescending.label(), "Path: Descending");
        assert_eq!(
            CommitFileSort::FileTypeAscending.label(),
            "File type: Ascending"
        );
        assert_eq!(
            CommitFileSort::FileTypeDescending.label(),
            "File type: Descending"
        );

        for sort in CommitFileSort::ALL {
            assert!(
                !sort.label().contains('–'),
                "{sort:?} still reads as an A–Z range",
            );
        }
    }

    #[test]
    fn commit_file_projection_cache_reuses_a_key_and_invalidates_on_change() {
        let files = vec![commit_file(
            "src/lib.rs",
            FileStatusKind::Modified,
            Some(1),
            Some(2),
        )];
        let mut cache = CommitFileProjectionCache::<u64>::default();

        let first = cache.projection_for(
            &1,
            &files,
            CommitFileSort::PathAscending,
            CommitFileFilter::All,
        );
        let reused = cache.projection_for(
            &1,
            &[],
            CommitFileSort::PathDescending,
            CommitFileFilter::Removed,
        );
        let replacement = cache.projection_for(
            &2,
            &files,
            CommitFileSort::PathDescending,
            CommitFileFilter::All,
        );

        assert!(Arc::ptr_eq(&first, &reused));
        assert!(!Arc::ptr_eq(&first, &replacement));
    }

    /// A linked worktree's file list contains untracked files, which commit and
    /// range lists never do. The table slot for them used to hold a
    /// "shouldn't happen" question mark, which is what those rows rendered.
    #[test]
    fn untracked_files_read_as_additions() {
        let untracked = commit_file_kind_visuals(FileStatusKind::Untracked);
        assert_eq!(untracked.icon, "icons/plus.svg");
        assert_eq!(
            untracked.icon,
            commit_file_kind_visuals(FileStatusKind::Added).icon,
            "an untracked file reads as an addition, as it does everywhere else"
        );
    }

    /// `kind_key` is a separate ordering from the table's own: callers count into
    /// buckets by it (`benchmarks/repo_history.rs` reads bucket 0 as "added") and
    /// it is hashed into the row signature. Retoning an entry must never move it.
    #[test]
    fn kind_keys_are_stable_buckets() {
        assert_eq!(commit_file_kind_visuals(FileStatusKind::Added).kind_key, 0);
        assert_eq!(
            commit_file_kind_visuals(FileStatusKind::Modified).kind_key,
            1
        );
        assert_eq!(
            commit_file_kind_visuals(FileStatusKind::Deleted).kind_key,
            2
        );
        assert_eq!(
            commit_file_kind_visuals(FileStatusKind::Renamed).kind_key,
            3
        );
        assert_eq!(
            commit_file_kind_visuals(FileStatusKind::Untracked).kind_key,
            4
        );
        assert_eq!(
            commit_file_kind_visuals(FileStatusKind::Conflicted).kind_key,
            5
        );
    }

    /// `rows_for` returns cached rows when the *key* matches, without looking at
    /// the files. The worktree list keys on the scan revision, which does not
    /// move when a different worktree is selected, so the key has to carry the
    /// worktree too — otherwise one worktree shows another's files.
    #[test]
    fn a_key_that_does_not_move_with_the_selection_serves_stale_rows() {
        let mut cache: CommitFileRowPresentationCache<(u64, std::path::PathBuf)> =
            CommitFileRowPresentationCache::default();

        let first = cache.rows_for(
            &(1, PathBuf::from("/wt/a")),
            &[CommitFileChange {
                path: PathBuf::from("a.rs"),
                kind: FileStatusKind::Modified,
                is_submodule: false,
                additions: None,
                deletions: None,
            }],
        );
        assert_eq!(first[0].label.as_ref(), "a.rs");

        // Same revision, different worktree: the path in the key is what keeps
        // these apart.
        let second = cache.rows_for(
            &(1, PathBuf::from("/wt/b")),
            &[CommitFileChange {
                path: PathBuf::from("b.rs"),
                kind: FileStatusKind::Modified,
                is_submodule: false,
                additions: None,
                deletions: None,
            }],
        );
        assert_eq!(
            second[0].label.as_ref(),
            "b.rs",
            "a second worktree must not be served the first one's rows"
        );
    }

    #[test]
    fn commit_file_row_presentation_cache_reuses_identical_files_across_new_keys() {
        let mut cache: CommitFileRowPresentationCache<u64> =
            CommitFileRowPresentationCache::default();
        let files = vec![
            CommitFileChange {
                path: PathBuf::from("src/lib.rs"),
                kind: FileStatusKind::Modified,
                is_submodule: false,
                additions: None,
                deletions: None,
            },
            CommitFileChange {
                path: PathBuf::from("README.md"),
                kind: FileStatusKind::Added,
                is_submodule: false,
                additions: None,
                deletions: None,
            },
        ];

        let first = cache.rows_for(&7, &files);
        let reused = cache.rows_for(&8, &files);

        assert!(Arc::ptr_eq(&first, &reused));
        assert_eq!(
            reused
                .iter()
                .map(|row| row.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["src/lib.rs", "README.md"]
        );
    }

    #[test]
    fn commit_file_row_presentation_cache_handles_empty_file_lists() {
        let mut cache: CommitFileRowPresentationCache<u64> =
            CommitFileRowPresentationCache::default();

        let first = cache.rows_for(&1, &[]);
        let second = cache.rows_for(&1, &[]);

        assert!(first.is_empty());
        assert!(Arc::ptr_eq(&first, &second));
    }

    /// The resolver's text is shaped from `window.rem_size()`, so its row box and
    /// line-number cell have to grow with UI scale too -- a flat 20px row holds a
    /// 41px line box at 200% and spills into the row below.
    #[test]
    fn conflict_row_geometry_scales_with_ui_scale() {
        for percent in [80, 100, 150, 200] {
            let factor = percent as f32 / 100.0;
            let height: f32 = AppTheme::gitcomet_dark().editor_row_height(percent).into();
            let line_no: f32 = conflict_line_no_width(percent).into();
            let base = crate::appearance::Appearance::default().editor_line_height();
            assert!(
                (height - base * factor).abs() < 0.01,
                "row height at {percent}% should be {}, got {height}",
                base * factor,
            );
            // Also measured at the editor font, so anchor it at 100%.
            let line_no_base: f32 = conflict_line_no_width(100).into();
            assert!(
                (line_no - line_no_base * factor).abs() < 0.01,
                "line-number width at {percent}% should be {}, got {line_no}",
                line_no_base * factor,
            );
        }

        // Strictly monotonic across the presets, so no two zoom levels collapse
        // onto the same geometry.
        let heights = crate::ui_scale::UI_SCALE_PRESETS
            .iter()
            .map(|percent| f32::from(AppTheme::gitcomet_dark().editor_row_height(*percent)))
            .collect::<Vec<_>>();
        assert!(
            heights.windows(2).all(|pair| pair[0] < pair[1]),
            "row heights should grow with every preset, got {heights:?}"
        );
    }

    /// The conflict rows and the diff rows are the same 20px design row; keeping the
    /// two helpers in agreement is what stops the resolver drifting away from the
    /// diff view it sits beside.
    #[test]
    fn conflict_row_height_matches_the_diff_row_height() {
        for percent in crate::ui_scale::UI_SCALE_PRESETS.iter().copied() {
            assert_eq!(
                AppTheme::gitcomet_dark().editor_row_height(percent),
                AppTheme::gitcomet_dark().editor_row_height(percent),
                "conflict and diff rows disagree at {percent}%"
            );
        }
    }
}
