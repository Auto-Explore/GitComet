use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StatusSection {
    CombinedUnstaged,
    Untracked,
    Unstaged,
    Staged,
}

impl StatusSection {
    pub(crate) const fn diff_area(self) -> DiffArea {
        match self {
            Self::CombinedUnstaged | Self::Untracked | Self::Unstaged => DiffArea::Unstaged,
            Self::Staged => DiffArea::Staged,
        }
    }

    pub(crate) const fn id_label(self) -> &'static str {
        match self {
            Self::CombinedUnstaged | Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
            Self::Staged => "staged",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusSectionFilter {
    All,
    UntrackedOnly,
    ExcludeUntracked,
}

#[derive(Clone)]
pub(crate) struct StatusSectionEntries<'a> {
    entries: &'a [FileStatus],
    indexes: StatusSectionIndexes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StatusSectionIndexes {
    All,
    Filtered(std::sync::Arc<[usize]>),
}

impl<'a> StatusSectionEntries<'a> {
    pub(crate) fn from_repo(repo: &'a RepoState, section: StatusSection) -> Option<Self> {
        let (entries, filter) = match section {
            StatusSection::CombinedUnstaged => {
                (repo.worktree_status_entries()?, StatusSectionFilter::All)
            }
            StatusSection::Untracked => (
                repo.worktree_status_entries()?,
                StatusSectionFilter::UntrackedOnly,
            ),
            StatusSection::Unstaged => (
                repo.worktree_status_entries()?,
                StatusSectionFilter::ExcludeUntracked,
            ),
            StatusSection::Staged => (repo.staged_status_entries()?, StatusSectionFilter::All),
        };
        let indexes = match filter {
            StatusSectionFilter::All => StatusSectionIndexes::All,
            StatusSectionFilter::UntrackedOnly | StatusSectionFilter::ExcludeUntracked => {
                StatusSectionIndexes::Filtered(
                    entries
                        .iter()
                        .enumerate()
                        .filter_map(|(ix, entry)| {
                            status_section_filter_matches(filter, entry).then_some(ix)
                        })
                        .collect::<Vec<_>>()
                        .into(),
                )
            }
        };
        Some(Self { entries, indexes })
    }

    /// The section's entries in a caller-supplied display order. `indexes` is
    /// in the backing slice's index space.
    pub(crate) fn from_repo_with_order(
        repo: &'a RepoState,
        section: StatusSection,
        indexes: std::sync::Arc<[usize]>,
    ) -> Option<Self> {
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries()?,
            _ => repo.worktree_status_entries()?,
        };
        Some(Self {
            entries,
            indexes: StatusSectionIndexes::Filtered(indexes),
        })
    }

    /// The section's entries in the backing slice's own order, before any UI
    /// sort. Callers that render or navigate must go through the pane's
    /// ordered accessor instead, or they walk a different order than the rows.
    pub(crate) fn source_order_indexes(
        repo: &'a RepoState,
        section: StatusSection,
    ) -> Option<std::sync::Arc<[usize]>> {
        let entries = Self::from_repo(repo, section)?;
        Some(match &entries.indexes {
            StatusSectionIndexes::All => (0..entries.entries.len()).collect::<Vec<_>>().into(),
            StatusSectionIndexes::Filtered(indexes) => std::sync::Arc::clone(indexes),
        })
    }

    pub(crate) fn iter(&self) -> StatusSectionIter<'a, '_> {
        let inner = match &self.indexes {
            StatusSectionIndexes::All => StatusSectionIterInner::All(self.entries.iter()),
            StatusSectionIndexes::Filtered(indexes) => StatusSectionIterInner::Filtered {
                entries: self.entries,
                indexes: indexes.iter(),
            },
        };
        StatusSectionIter { inner }
    }

    pub(crate) fn len(&self) -> usize {
        match &self.indexes {
            StatusSectionIndexes::All => self.entries.len(),
            StatusSectionIndexes::Filtered(indexes) => indexes.len(),
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&'a FileStatus> {
        match &self.indexes {
            StatusSectionIndexes::All => self.entries.get(index),
            StatusSectionIndexes::Filtered(indexes) => indexes
                .get(index)
                .and_then(|source_ix| self.entries.get(*source_ix)),
        }
    }

    pub(crate) fn path_vec(&self) -> Vec<std::path::PathBuf> {
        self.iter().map(|entry| entry.path.clone()).collect()
    }

    pub(crate) fn contains_path(&self, path: &std::path::Path) -> bool {
        self.iter().any(|entry| entry.path == path)
    }
}

pub(crate) struct StatusSectionIter<'a, 'indexes> {
    inner: StatusSectionIterInner<'a, 'indexes>,
}

pub(crate) enum StatusSectionIterInner<'a, 'indexes> {
    All(std::slice::Iter<'a, FileStatus>),
    Filtered {
        entries: &'a [FileStatus],
        indexes: std::slice::Iter<'indexes, usize>,
    },
}

impl<'a, 'indexes> Iterator for StatusSectionIter<'a, 'indexes> {
    type Item = &'a FileStatus;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            StatusSectionIterInner::All(iter) => iter.next(),
            StatusSectionIterInner::Filtered { entries, indexes } => {
                indexes.next().and_then(|ix| entries.get(*ix))
            }
        }
    }
}

pub(crate) fn status_section_filter_matches(
    filter: StatusSectionFilter,
    entry: &FileStatus,
) -> bool {
    match filter {
        StatusSectionFilter::All => true,
        StatusSectionFilter::UntrackedOnly => entry.kind == FileStatusKind::Untracked,
        StatusSectionFilter::ExcludeUntracked => entry.kind != FileStatusKind::Untracked,
    }
}

pub(crate) fn status_section_rev(repo: &RepoState, section: StatusSection) -> u64 {
    match section {
        StatusSection::Staged => repo.staged_status_cache_rev(),
        StatusSection::CombinedUnstaged | StatusSection::Untracked | StatusSection::Unstaged => {
            repo.worktree_status_cache_rev()
        }
    }
}

/// The rev anything derived from a section must key on: its status lane and
/// its line-stats lane. Counts arrive without `status_section_rev` moving, so a
/// cache keyed on that alone keeps serving pre-stats rows.
pub(crate) fn status_section_content_rev(repo: &RepoState, section: StatusSection) -> u64 {
    gitcomet_state::model::mix_status_cache_revs([
        status_section_rev(repo, section),
        repo.line_stats_rev(section.diff_area()),
    ])
}

/// `None` while unloaded, and always `None` for Untracked — those files are in
/// neither index lane.
pub(crate) fn status_section_line_stats(
    repo: &RepoState,
    section: StatusSection,
) -> Option<&rustc_hash::FxHashMap<std::path::PathBuf, gitcomet_core::domain::LineStats>> {
    status_section_has_line_stats(section)
        .then(|| repo.line_stats_for_area(section.diff_area()))
        .flatten()
}

pub(crate) const fn status_section_has_line_stats(section: StatusSection) -> bool {
    !matches!(section, StatusSection::Untracked)
}

pub(crate) fn status_section_is_loading(repo: &RepoState, section: StatusSection) -> bool {
    match section {
        StatusSection::Staged => repo.staged_status_is_loading(),
        StatusSection::CombinedUnstaged | StatusSection::Untracked | StatusSection::Unstaged => {
            repo.worktree_status_is_loading()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StatusMultiSelection {
    /// An explicit empty selection must not fall back to the previewed file.
    pub(crate) explicit_section: Option<StatusSection>,
    pub(crate) untracked: Vec<std::path::PathBuf>,
    pub(crate) untracked_anchor: Option<std::path::PathBuf>,
    pub(crate) unstaged: Vec<std::path::PathBuf>,
    pub(crate) unstaged_anchor: Option<std::path::PathBuf>,
    pub(crate) unstaged_anchor_index: Option<usize>,
    pub(crate) unstaged_anchor_order_rev: Option<u64>,
    pub(crate) staged: Vec<std::path::PathBuf>,
    pub(crate) staged_anchor: Option<std::path::PathBuf>,
    pub(crate) staged_anchor_index: Option<usize>,
    pub(crate) staged_anchor_order_rev: Option<u64>,
}

impl StatusMultiSelection {
    pub(crate) fn select_all(
        &mut self,
        section: StatusSection,
        paths: Vec<std::path::PathBuf>,
        order_rev: u64,
    ) {
        let previous_anchor = match section {
            StatusSection::CombinedUnstaged | StatusSection::Unstaged => &self.unstaged_anchor,
            StatusSection::Untracked => &self.untracked_anchor,
            StatusSection::Staged => &self.staged_anchor,
        };
        let anchor_index = previous_anchor
            .as_ref()
            .and_then(|anchor| paths.iter().position(|path| path == anchor))
            .or_else(|| (!paths.is_empty()).then_some(0));
        let anchor = anchor_index.map(|ix| paths[ix].clone());
        *self = Self {
            explicit_section: Some(section),
            ..Default::default()
        };
        match section {
            StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
                self.unstaged = paths;
                self.unstaged_anchor = anchor;
                self.unstaged_anchor_index = anchor_index;
                self.unstaged_anchor_order_rev = Some(order_rev);
            }
            StatusSection::Untracked => {
                self.untracked = paths;
                self.untracked_anchor = anchor;
            }
            StatusSection::Staged => {
                self.staged = paths;
                self.staged_anchor = anchor;
                self.staged_anchor_index = anchor_index;
                self.staged_anchor_order_rev = Some(order_rev);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.untracked.is_empty() && self.unstaged.is_empty() && self.staged.is_empty()
    }

    pub(crate) fn selected_paths_for_area(&self, area: DiffArea) -> &[std::path::PathBuf] {
        match area {
            DiffArea::Unstaged => {
                if !self.unstaged.is_empty() {
                    self.unstaged.as_slice()
                } else {
                    self.untracked.as_slice()
                }
            }
            DiffArea::Staged => self.staged.as_slice(),
        }
    }

    pub(crate) fn selected_count_for_area(&self, area: DiffArea) -> usize {
        self.selected_paths_for_area(area).len()
    }

    pub(crate) fn first_selected_for_area(&self, area: DiffArea) -> Option<&std::path::PathBuf> {
        self.selected_paths_for_area(area).first()
    }

    pub(crate) fn take_selected_paths_for_area(self, area: DiffArea) -> Vec<std::path::PathBuf> {
        match area {
            DiffArea::Unstaged => {
                if !self.unstaged.is_empty() {
                    self.unstaged
                } else {
                    self.untracked
                }
            }
            DiffArea::Staged => self.staged,
        }
    }
}

#[cfg(test)]
pub(crate) fn reconcile_status_multi_selection(
    selection: &mut StatusMultiSelection,
    status: &gitcomet_core::domain::RepoStatus,
) {
    let mut untracked_paths: FxHashSet<&std::path::Path> =
        FxHashSet::with_capacity_and_hasher(status.unstaged.len(), Default::default());
    let mut unstaged_paths: FxHashSet<&std::path::Path> =
        FxHashSet::with_capacity_and_hasher(status.unstaged.len(), Default::default());
    for entry in status.unstaged.iter() {
        unstaged_paths.insert(entry.path.as_path());
        if entry.kind == FileStatusKind::Untracked {
            untracked_paths.insert(entry.path.as_path());
        }
    }

    selection
        .untracked
        .retain(|p| untracked_paths.contains(&p.as_path()));
    if selection
        .untracked_anchor
        .as_ref()
        .is_some_and(|a| !untracked_paths.contains(&a.as_path()))
    {
        selection.untracked_anchor = None;
    }

    selection
        .unstaged
        .retain(|p| unstaged_paths.contains(&p.as_path()));
    if selection
        .unstaged_anchor
        .as_ref()
        .is_some_and(|a| !unstaged_paths.contains(&a.as_path()))
    {
        selection.unstaged_anchor = None;
        selection.unstaged_anchor_index = None;
        selection.unstaged_anchor_order_rev = None;
    }

    let mut staged_paths: FxHashSet<&std::path::Path> =
        FxHashSet::with_capacity_and_hasher(status.staged.len(), Default::default());
    for entry in status.staged.iter() {
        staged_paths.insert(entry.path.as_path());
    }

    selection
        .staged
        .retain(|p| staged_paths.contains(&p.as_path()));
    if selection
        .staged_anchor
        .as_ref()
        .is_some_and(|a| !staged_paths.contains(&a.as_path()))
    {
        selection.staged_anchor = None;
        selection.staged_anchor_index = None;
        selection.staged_anchor_order_rev = None;
    }
}

pub(crate) fn reconcile_status_multi_selection_with_repo(
    selection: &mut StatusMultiSelection,
    repo: &RepoState,
) {
    if let Some(worktree) = repo.worktree_status_entries() {
        let mut untracked_paths: FxHashSet<&std::path::Path> =
            FxHashSet::with_capacity_and_hasher(worktree.len(), Default::default());
        let mut unstaged_paths: FxHashSet<&std::path::Path> =
            FxHashSet::with_capacity_and_hasher(worktree.len(), Default::default());
        for entry in worktree {
            unstaged_paths.insert(entry.path.as_path());
            if entry.kind == FileStatusKind::Untracked {
                untracked_paths.insert(entry.path.as_path());
            }
        }

        selection
            .untracked
            .retain(|p| untracked_paths.contains(&p.as_path()));
        if selection
            .untracked_anchor
            .as_ref()
            .is_some_and(|a| !untracked_paths.contains(&a.as_path()))
        {
            selection.untracked_anchor = None;
        }

        selection
            .unstaged
            .retain(|p| unstaged_paths.contains(&p.as_path()));
        if selection
            .unstaged_anchor
            .as_ref()
            .is_some_and(|a| !unstaged_paths.contains(&a.as_path()))
        {
            selection.unstaged_anchor = None;
            selection.unstaged_anchor_index = None;
            selection.unstaged_anchor_order_rev = None;
        }
    }

    if let Some(staged) = repo.staged_status_entries() {
        let mut staged_paths: FxHashSet<&std::path::Path> =
            FxHashSet::with_capacity_and_hasher(staged.len(), Default::default());
        for entry in staged {
            staged_paths.insert(entry.path.as_path());
        }

        selection
            .staged
            .retain(|p| staged_paths.contains(&p.as_path()));
        if selection
            .staged_anchor
            .as_ref()
            .is_some_and(|a| !staged_paths.contains(&a.as_path()))
        {
            selection.staged_anchor = None;
            selection.staged_anchor_index = None;
            selection.staged_anchor_order_rev = None;
        }
    }
}
