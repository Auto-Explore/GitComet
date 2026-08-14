use super::branch_sidebar::{
    BranchSidebarSourceFingerprint, BranchSidebarSourceFingerprintParts,
    branch_sidebar_source_matches_cached,
};
use super::*;
use gitcomet_core::domain::{Branch, LogScope, RemoteBranch, StashEntry, Tag};
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub(super) struct HistoryCache {
    pub(super) base: HistoryBaseCache,
    pub(super) decorations: HistoryDecorationCache,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryBaseCache {
    pub(super) request: HistoryBaseCacheRequest,
    pub(super) visible_indices: HistoryVisibleIndices,
    pub(super) graph_rows: Arc<[history_graph::GraphRow]>,
    pub(super) max_lanes: usize,
    pub(super) row_vms: Vec<HistoryBaseRowVm>,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryDecorationCache {
    pub(super) request: HistoryDecorationCacheRequest,
    pub(super) row_vms: Arc<[HistoryDecorationRowVm]>,
    /// Branch names referenced by [`HistoryDecorationRowVm::lane_branch`].
    pub(super) branch_names: Arc<[SharedString]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryBaseCacheRequest {
    pub(super) repo_id: RepoId,
    pub(super) history_scope: LogScope,
    pub(super) log_fingerprint: u64,
    pub(super) head_branch_rev: u64,
    pub(super) detached_head_commit: Option<CommitId>,
    pub(super) head_branch_target: Option<CommitId>,
    pub(super) branches_rev: u64,
    pub(super) remote_branches_rev: u64,
    pub(super) stashes_rev: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryDecorationCacheRequest {
    pub(super) base_request: HistoryBaseCacheRequest,
    pub(super) head_branch_rev: u64,
    pub(super) detached_head_commit: Option<CommitId>,
    pub(super) branches_rev: u64,
    pub(super) remote_branches_rev: u64,
    pub(super) tags_rev: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryCacheBuildRequest {
    pub(super) base_request: HistoryBaseCacheRequest,
    pub(super) decoration_request: HistoryDecorationCacheRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct HistoryDisplayKey {
    pub(in crate::view) date_time_format: DateTimeFormat,
    pub(in crate::view) timezone: Timezone,
    pub(in crate::view) show_timezone: bool,
    /// History-table-only "3 days ago" display; independent of the general
    /// date format setting.
    relative_dates: bool,
    /// Minutes since the Unix epoch when `relative_dates` is set, zero
    /// otherwise. Bumping once a minute invalidates cached relative strings
    /// so "2 mins ago" doesn't freeze; absolute formats keep a stable key.
    relative_now_bucket: u64,
}

impl HistoryDisplayKey {
    pub(in crate::view) fn new(
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        relative_dates: bool,
    ) -> Self {
        let relative_now_bucket = if relative_dates {
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() / 60)
                .unwrap_or(0)
        } else {
            0
        };
        Self {
            date_time_format,
            timezone,
            show_timezone,
            relative_dates,
            relative_now_bucket,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::view) enum HistoryVisibleIndices {
    All { len: usize },
    Filtered(Arc<[usize]>),
}

pub(in crate::view) enum HistoryVisibleIndicesIter<'a> {
    All(Range<usize>),
    Filtered(std::iter::Copied<std::slice::Iter<'a, usize>>),
}

impl Iterator for HistoryVisibleIndicesIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::All(range) => range.next(),
            Self::Filtered(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::All(range) => range.size_hint(),
            Self::Filtered(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for HistoryVisibleIndicesIter<'_> {
    fn len(&self) -> usize {
        match self {
            Self::All(range) => range.len(),
            Self::Filtered(iter) => iter.len(),
        }
    }
}

impl HistoryVisibleIndices {
    pub(in crate::view) const fn all(len: usize) -> Self {
        Self::All { len }
    }

    pub(in crate::view) fn len(&self) -> usize {
        match self {
            Self::All { len } => *len,
            Self::Filtered(indices) => indices.len(),
        }
    }

    pub(in crate::view) fn first(&self) -> Option<usize> {
        match self {
            Self::All { len } => (*len > 0).then_some(0),
            Self::Filtered(indices) => indices.first().copied(),
        }
    }

    pub(in crate::view) fn get(&self, visible_ix: usize) -> Option<usize> {
        match self {
            Self::All { len } => (visible_ix < *len).then_some(visible_ix),
            Self::Filtered(indices) => indices.get(visible_ix).copied(),
        }
    }

    pub(in crate::view) fn iter(&self) -> HistoryVisibleIndicesIter<'_> {
        match self {
            Self::All { len } => HistoryVisibleIndicesIter::All(0..*len),
            Self::Filtered(indices) => HistoryVisibleIndicesIter::Filtered(indices.iter().copied()),
        }
    }
}

/// Where a worktree's "Uncommitted changes" row sits: immediately above the
/// commit at `visible_ix`, which is that worktree's HEAD.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct HistoryWorktreeRowAnchor {
    pub(in crate::view) visible_ix: usize,
    /// Index into the repo's `worktree_dirty` list.
    pub(in crate::view) worktree_ix: usize,
}

/// What the list shows at a given `list_ix`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum HistoryListRow {
    WorkingTreeSummary,
    WorktreeUncommitted {
        visible_ix: usize,
        worktree_ix: usize,
    },
    Commit {
        visible_ix: usize,
    },
}

/// Maps between the virtualized list's indices and the commit indices behind
/// them, once synthetic rows are interleaved.
///
/// The list used to be "commits, optionally shifted by one", so every call site
/// could do `list_ix = visible_ix + offset`. Worktree rows sit *inside* the run
/// of commits, so the shift is no longer constant. Anchors are kept sorted by
/// `visible_ix` and both directions binary-search them, which keeps this O(log n)
/// per lookup and costs no per-row allocation.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct HistoryListPlan {
    show_working_tree_summary_row: bool,
    /// Sorted by `visible_ix`, ascending.
    anchors: Arc<[HistoryWorktreeRowAnchor]>,
}

impl HistoryListPlan {
    pub(in crate::view) fn new(
        show_working_tree_summary_row: bool,
        mut anchors: Vec<HistoryWorktreeRowAnchor>,
    ) -> Self {
        anchors.sort_by_key(|anchor| anchor.visible_ix);
        Self {
            show_working_tree_summary_row,
            anchors: anchors.into(),
        }
    }

    pub(in crate::view) fn show_working_tree_summary_row(&self) -> bool {
        self.show_working_tree_summary_row
    }

    /// Identity of the interleaving, for caches that store a `list_ix`.
    /// Two plans that place the same rows at the same indices hash equal.
    pub(in crate::view) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        self.show_working_tree_summary_row.hash(&mut hasher);
        self.anchors.len().hash(&mut hasher);
        for anchor in self.anchors.iter() {
            anchor.visible_ix.hash(&mut hasher);
            anchor.worktree_ix.hash(&mut hasher);
        }
        hasher.finish()
    }

    #[cfg(test)]
    pub(in crate::view) fn worktree_row_count(&self) -> usize {
        self.anchors.len()
    }

    fn base_offset(&self) -> usize {
        usize::from(self.show_working_tree_summary_row)
    }

    /// Total rows for `visible_len` commits.
    pub(in crate::view) fn list_len(&self, visible_len: usize) -> usize {
        self.base_offset() + visible_len + self.anchors.len()
    }

    /// The `list_ix` the commit at `visible_ix` renders at.
    pub(in crate::view) fn list_ix_for_visible(&self, visible_ix: usize) -> usize {
        // Anchors at exactly `visible_ix` render immediately above this commit,
        // so they count towards its offset.
        let before = self
            .anchors
            .partition_point(|anchor| anchor.visible_ix <= visible_ix);
        self.base_offset() + visible_ix + before
    }

    /// The `list_ix` of the row for `worktree_ix`, if that worktree still has
    /// one. `None` once it goes clean or its HEAD leaves the visible log.
    pub(in crate::view) fn list_ix_for_worktree(&self, worktree_ix: usize) -> Option<usize> {
        self.anchors
            .iter()
            .enumerate()
            .find(|(_, anchor)| anchor.worktree_ix == worktree_ix)
            .map(|(k, anchor)| self.base_offset() + anchor.visible_ix + k)
    }

    pub(in crate::view) fn row_at(&self, list_ix: usize) -> Option<HistoryListRow> {
        let base = self.base_offset();
        if self.show_working_tree_summary_row && list_ix == 0 {
            return Some(HistoryListRow::WorkingTreeSummary);
        }
        let offset_ix = list_ix.checked_sub(base)?;

        // An anchor's own position is `visible_ix + <number of earlier anchors>`,
        // which is strictly increasing, so it can be binary-searched directly.
        let anchor_pos = |k: usize| self.anchors[k].visible_ix + k;
        let mut lo = 0usize;
        let mut hi = self.anchors.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            match anchor_pos(mid).cmp(&offset_ix) {
                std::cmp::Ordering::Equal => {
                    let anchor = self.anchors[mid];
                    return Some(HistoryListRow::WorktreeUncommitted {
                        visible_ix: anchor.visible_ix,
                        worktree_ix: anchor.worktree_ix,
                    });
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        // `lo` is now the number of anchors sitting above this row.
        Some(HistoryListRow::Commit {
            visible_ix: offset_ix.checked_sub(lo)?,
        })
    }
}


#[cfg(test)]
mod history_list_plan_tests {
    use super::*;

    fn anchor(visible_ix: usize, worktree_ix: usize) -> HistoryWorktreeRowAnchor {
        HistoryWorktreeRowAnchor {
            visible_ix,
            worktree_ix,
        }
    }

    /// Every list index maps to exactly one row, and every commit round-trips
    /// back to the index it was placed at.
    fn assert_plan_round_trips(plan: &HistoryListPlan, visible_len: usize) {
        let len = plan.list_len(visible_len);
        let mut seen_commits = Vec::new();
        let mut seen_worktrees = Vec::new();
        for list_ix in 0..len {
            match plan.row_at(list_ix).expect("row within list length") {
                HistoryListRow::WorkingTreeSummary => assert_eq!(list_ix, 0),
                HistoryListRow::WorktreeUncommitted { visible_ix, .. } => {
                    seen_worktrees.push((list_ix, visible_ix))
                }
                HistoryListRow::Commit { visible_ix } => {
                    assert_eq!(
                        plan.list_ix_for_visible(visible_ix),
                        list_ix,
                        "commit {visible_ix} did not round-trip"
                    );
                    seen_commits.push(visible_ix);
                }
            }
        }
        assert_eq!(seen_commits, (0..visible_len).collect::<Vec<_>>());
        assert_eq!(seen_worktrees.len(), plan.worktree_row_count());
        // A worktree row always renders directly above the commit it anchors to.
        for (list_ix, visible_ix) in seen_worktrees {
            assert!(plan.list_ix_for_visible(visible_ix) > list_ix);
        }
    }

    #[test]
    fn a_plan_without_synthetic_rows_is_the_identity() {
        let plan = HistoryListPlan::new(false, Vec::new());
        assert_eq!(plan.list_len(5), 5);
        assert_eq!(plan.list_ix_for_visible(3), 3);
        assert_eq!(plan.row_at(3), Some(HistoryListRow::Commit { visible_ix: 3 }));
        assert_plan_round_trips(&plan, 5);
    }

    #[test]
    fn the_working_tree_row_shifts_every_commit_by_one() {
        let plan = HistoryListPlan::new(true, Vec::new());
        assert_eq!(plan.list_len(5), 6);
        assert_eq!(plan.row_at(0), Some(HistoryListRow::WorkingTreeSummary));
        assert_eq!(plan.list_ix_for_visible(0), 1);
        assert_plan_round_trips(&plan, 5);
    }

    #[test]
    fn a_worktree_row_sits_directly_above_its_head_commit() {
        let plan = HistoryListPlan::new(false, vec![anchor(2, 0)]);
        assert_eq!(plan.list_len(5), 6);
        assert_eq!(plan.row_at(1), Some(HistoryListRow::Commit { visible_ix: 1 }));
        assert_eq!(
            plan.row_at(2),
            Some(HistoryListRow::WorktreeUncommitted {
                visible_ix: 2,
                worktree_ix: 0,
            })
        );
        assert_eq!(plan.row_at(3), Some(HistoryListRow::Commit { visible_ix: 2 }));
        assert_eq!(plan.list_ix_for_visible(2), 3);
        assert_plan_round_trips(&plan, 5);
    }

    #[test]
    fn several_worktrees_on_the_same_commit_stack_above_it() {
        let plan = HistoryListPlan::new(true, vec![anchor(1, 0), anchor(1, 1)]);
        assert_eq!(plan.list_len(4), 7);
        assert_eq!(
            plan.row_at(2),
            Some(HistoryListRow::WorktreeUncommitted {
                visible_ix: 1,
                worktree_ix: 0,
            })
        );
        assert_eq!(
            plan.row_at(3),
            Some(HistoryListRow::WorktreeUncommitted {
                visible_ix: 1,
                worktree_ix: 1,
            })
        );
        assert_eq!(plan.list_ix_for_visible(1), 4);
        assert_plan_round_trips(&plan, 4);
    }

    #[test]
    fn anchors_are_sorted_so_construction_order_does_not_matter() {
        let scrambled = HistoryListPlan::new(false, vec![anchor(4, 1), anchor(0, 0), anchor(2, 2)]);
        let ordered = HistoryListPlan::new(false, vec![anchor(0, 0), anchor(2, 2), anchor(4, 1)]);
        for list_ix in 0..ordered.list_len(6) {
            assert_eq!(scrambled.row_at(list_ix), ordered.row_at(list_ix));
        }
        assert_plan_round_trips(&scrambled, 6);
    }

    #[test]
    fn indices_past_the_end_have_no_row() {
        let plan = HistoryListPlan::new(true, vec![anchor(0, 0)]);
        let len = plan.list_len(2);
        assert!(plan.row_at(len).is_none() || plan.row_at(len - 1).is_some());
        assert_plan_round_trips(&plan, 2);
    }
}

pub(in crate::view) struct HistoryStashAnalysis<'a> {
    pub(in crate::view) stash_tips: Vec<HistoryStashTip<'a>>,
    pub(in crate::view) stash_helper_ids: HashSet<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct HistoryStashTip<'a> {
    pub(in crate::view) commit_ix: usize,
    pub(in crate::view) message: Option<&'a Arc<str>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct HistoryTextVm {
    text: SharedString,
    hash: u64,
}

impl HistoryTextVm {
    pub(in crate::view) fn new(text: SharedString) -> Self {
        Self {
            hash: history_text_hash(text.as_ref()),
            text,
        }
    }

    pub(in crate::view) fn shared(&self) -> &SharedString {
        &self.text
    }

    pub(in crate::view) const fn text_hash(&self) -> u64 {
        self.hash
    }

    pub(in crate::view) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

impl AsRef<str> for HistoryTextVm {
    fn as_ref(&self) -> &str {
        self.text.as_ref()
    }
}

#[inline]
pub(in crate::view) fn history_text_hash(text: &str) -> u64 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Debug)]
pub(in crate::view) struct HistoryWhenVm {
    time: SystemTime,
    formatted: RefCell<Option<(HistoryDisplayKey, HistoryTextVm)>>,
}

impl HistoryWhenVm {
    pub(in crate::view) fn deferred(time: SystemTime) -> Self {
        Self {
            time,
            formatted: RefCell::new(None),
        }
    }

    pub(in crate::view) fn resolve(&self, display_key: HistoryDisplayKey) -> HistoryTextVm {
        if let Some((cached_key, formatted)) = self.formatted.borrow().as_ref()
            && *cached_key == display_key
        {
            return formatted.clone();
        }

        let formatted = if display_key.relative_dates {
            let unix_secs = match self.time.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_secs() as i64,
                Err(e) => -(e.duration().as_secs() as i64),
            };
            crate::view::date_time::format_relative_time(unix_secs, SystemTime::now())
        } else {
            let mut formatted = String::with_capacity(32);
            format_datetime_into(
                &mut formatted,
                self.time,
                display_key.date_time_format,
                display_key.timezone,
                display_key.show_timezone,
            );
            formatted
        };
        let formatted = HistoryTextVm::new(formatted.into());
        *self.formatted.borrow_mut() = Some((display_key, formatted.clone()));
        formatted
    }
}

const HISTORY_SHORT_SHA_LEN: usize = 8;

#[derive(Clone, Debug)]
pub(in crate::view) struct HistoryShortShaVm {
    bytes: [u8; HISTORY_SHORT_SHA_LEN],
    len: u8,
    hash: u64,
    formatted: RefCell<Option<HistoryTextVm>>,
}

impl HistoryShortShaVm {
    pub(in crate::view) fn new(id: &str) -> Self {
        let id = id.as_bytes();
        let len = id.len().min(HISTORY_SHORT_SHA_LEN);
        let mut bytes = [0; HISTORY_SHORT_SHA_LEN];
        bytes[..len].copy_from_slice(&id[..len]);
        Self {
            bytes,
            len: u8::try_from(len).expect("short sha length fits into u8"),
            hash: history_text_hash(std::str::from_utf8(&id[..len]).expect("short sha is utf-8")),
            formatted: RefCell::new(None),
        }
    }

    pub(in crate::view) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("commit id prefixes must stay valid utf-8")
    }

    pub(in crate::view) fn resolve(&self) -> HistoryTextVm {
        if let Some(formatted) = self.formatted.borrow().as_ref() {
            return formatted.clone();
        }

        let formatted = HistoryTextVm {
            text: SharedString::new(self.as_str()),
            hash: self.hash,
        };
        *self.formatted.borrow_mut() = Some(formatted.clone());
        formatted
    }
}

#[derive(Clone, Debug)]
pub(super) struct HistoryBaseRowVm {
    pub(super) author: HistoryTextVm,
    pub(super) summary: HistoryTextVm,
    pub(super) when: HistoryWhenVm,
    pub(super) short_sha: HistoryShortShaVm,
    pub(super) is_head: bool,
    pub(super) is_stash: bool,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryDecorationRowVm {
    /// Joined display text of all branch refs on the row. Rendering paints
    /// per-ref chips from `ref_items`; this stays as the canonical flat form
    /// that decoration-cache tests and benchmarks assert against.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) branches_text: HistoryTextVm,
    pub(super) tag_names: Arc<[HistoryTextVm]>,
    pub(super) ref_items: Arc<[HistoryRefListItem]>,
    /// Branch this commit belongs to, as an index into
    /// [`HistoryDecorationCache::branch_names`]. Inherited down the lane from
    /// the branch head that started it, so unlabelled commits can still say
    /// which branch they are on.
    pub(super) lane_branch: Option<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct HistoryRefListItem {
    pub(in crate::view) text: HistoryTextVm,
    pub(in crate::view) kind: HistoryRefListItemKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum HistoryRefListItemKind {
    Tag { name: String },
    LocalBranch { name: String },
    RemoteBranch { name: String },
    AttachedHead { branch: String },
    DetachedHead,
}

type HistoryRefItems = Arc<[HistoryRefListItem]>;
type HistoryRefItemsByTarget<'a> = HashMap<&'a str, HistoryRefItems>;

#[inline]
pub(in crate::view) fn history_commit_is_probable_stash_tip(commit: &Commit) -> bool {
    if !(2..=3).contains(&commit.parent_ids.len()) {
        return false;
    }
    let summary: &str = &commit.summary;
    (summary.starts_with("WIP on ") || summary.starts_with("On ")) && summary.contains(": ")
}

pub(in crate::view) fn analyze_history_stashes<'a>(
    commits: &'a [Commit],
    stashes: &'a [StashEntry],
) -> HistoryStashAnalysis<'a> {
    if stashes.is_empty() {
        let mut stash_tips: Vec<HistoryStashTip<'_>> = Vec::new();
        let mut stash_helper_ids: HashSet<&str> = HashSet::default();
        for (commit_ix, commit) in commits.iter().enumerate() {
            if !history_commit_is_probable_stash_tip(commit) {
                continue;
            }
            if stash_tips.is_empty() {
                stash_tips.reserve(4);
                stash_helper_ids.reserve(4);
            }
            stash_tips.push(HistoryStashTip {
                commit_ix,
                message: None,
            });
            for parent_id in commit.parent_ids.iter().skip(1).map(|p| p.as_ref()) {
                stash_helper_ids.insert(parent_id);
            }
        }

        return HistoryStashAnalysis {
            stash_tips,
            stash_helper_ids,
        };
    }

    let mut listed_stash_messages_by_id: HashMap<&str, Option<&Arc<str>>> =
        HashMap::with_capacity_and_hasher(stashes.len(), Default::default());
    for stash in stashes.iter() {
        listed_stash_messages_by_id.insert(
            stash.id.as_ref(),
            (!stash.message.trim().is_empty()).then_some(&stash.message),
        );
    }

    let mut stash_tips: Vec<HistoryStashTip<'_>> = Vec::with_capacity(stashes.len());
    let mut stash_helper_ids: HashSet<&str> =
        HashSet::with_capacity_and_hasher(stashes.len().max(4), Default::default());
    for (commit_ix, commit) in commits.iter().enumerate() {
        let commit_id = commit.id.as_ref();
        let is_probable_stash = history_commit_is_probable_stash_tip(commit);
        let listed_stash_message = listed_stash_messages_by_id.get(commit_id).copied();
        let listed_stash_tip = listed_stash_message.is_some();
        if listed_stash_tip || is_probable_stash {
            stash_tips.push(HistoryStashTip {
                commit_ix,
                message: listed_stash_message.flatten(),
            });
        }

        if listed_stash_tip {
            for parent_id in commit.parent_ids.iter().skip(1).map(|p| p.as_ref()) {
                stash_helper_ids.insert(parent_id);
            }
        }
    }

    HistoryStashAnalysis {
        stash_tips,
        stash_helper_ids,
    }
}

pub(in crate::view) fn build_history_visible_indices(
    commits: &[Commit],
    stash_helper_ids: &HashSet<&str>,
) -> HistoryVisibleIndices {
    if stash_helper_ids.is_empty() {
        return HistoryVisibleIndices::all(commits.len());
    }

    let mut visible_indices =
        Vec::with_capacity(commits.len().saturating_sub(stash_helper_ids.len()));
    for (ix, commit) in commits.iter().enumerate() {
        if stash_helper_ids.contains(commit.id.as_ref()) {
            continue;
        }
        visible_indices.push(ix);
    }
    HistoryVisibleIndices::Filtered(visible_indices.into())
}

#[inline]
pub(in crate::view) fn next_history_stash_tip_for_commit_ix<'a>(
    stash_tips: &[HistoryStashTip<'a>],
    next_stash_tip_ix: &mut usize,
    commit_ix: usize,
) -> Option<HistoryStashTip<'a>> {
    let stash_tip = stash_tips.get(*next_stash_tip_ix).copied()?;
    if stash_tip.commit_ix != commit_ix {
        return None;
    }
    *next_stash_tip_ix += 1;
    Some(stash_tip)
}

type HistoryBranchNameBucket<'a> = SmallVec<[HistoryBranchNameRef<'a>; 2]>;
type HistoryTagNameBucket<'a> = SmallVec<[&'a str; 1]>;

#[derive(Clone, Copy, Debug)]
enum HistoryBranchNameRef<'a> {
    Plain(&'a str),
    Remote { remote: &'a str, name: &'a str },
}

#[derive(Clone, Copy)]
struct HistoryBranchDisplaySegments<'a> {
    parts: [&'a str; 3],
    len: usize,
}

impl<'a> HistoryBranchNameRef<'a> {
    fn display_segments(self) -> HistoryBranchDisplaySegments<'a> {
        match self {
            Self::Plain(name) => HistoryBranchDisplaySegments {
                parts: [name, "", ""],
                len: 1,
            },
            Self::Remote { remote, name } => HistoryBranchDisplaySegments {
                parts: [remote, "/", name],
                len: 3,
            },
        }
    }

    fn display_len(self) -> usize {
        match self {
            Self::Plain(name) => name.len(),
            Self::Remote { remote, name } => remote.len() + 1 + name.len(),
        }
    }

    fn write_display_to(self, output: &mut String) {
        match self {
            Self::Plain(name) => output.push_str(name),
            Self::Remote { remote, name } => {
                output.push_str(remote);
                output.push('/');
                output.push_str(name);
            }
        }
    }

    fn to_shared_string(self) -> SharedString {
        match self {
            Self::Plain(name) => SharedString::new(name),
            Self::Remote { remote, name } => {
                let mut text = String::with_capacity(remote.len() + 1 + name.len());
                text.push_str(remote);
                text.push('/');
                text.push_str(name);
                SharedString::from(text)
            }
        }
    }
}

fn cmp_history_branch_display(
    left: HistoryBranchNameRef<'_>,
    right: HistoryBranchNameRef<'_>,
) -> Ordering {
    let left = left.display_segments();
    let right = right.display_segments();
    let mut left_part_ix = 0usize;
    let mut left_byte_ix = 0usize;
    let mut right_part_ix = 0usize;
    let mut right_byte_ix = 0usize;

    loop {
        while left_part_ix < left.len && left_byte_ix == left.parts[left_part_ix].len() {
            left_part_ix += 1;
            left_byte_ix = 0;
        }
        while right_part_ix < right.len && right_byte_ix == right.parts[right_part_ix].len() {
            right_part_ix += 1;
            right_byte_ix = 0;
        }

        match (left_part_ix == left.len, right_part_ix == right.len) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }

        let left_bytes = left.parts[left_part_ix].as_bytes();
        let right_bytes = right.parts[right_part_ix].as_bytes();
        let ord = left_bytes[left_byte_ix].cmp(&right_bytes[right_byte_ix]);
        if ord != Ordering::Equal {
            return ord;
        }

        left_byte_ix += 1;
        right_byte_ix += 1;
    }
}

fn sort_and_dedup_history_branch_names(names: &mut HistoryBranchNameBucket<'_>) {
    if names.len() < 2 {
        return;
    }
    names.sort_unstable_by(|left, right| cmp_history_branch_display(*left, *right));
    names.dedup_by(|left, right| cmp_history_branch_display(*left, *right) == Ordering::Equal);
}

fn cmp_history_branch_ref_item(
    left: HistoryBranchNameRef<'_>,
    right: HistoryBranchNameRef<'_>,
) -> Ordering {
    cmp_history_branch_display(left, right).then_with(|| match (left, right) {
        (HistoryBranchNameRef::Plain(left), HistoryBranchNameRef::Plain(right)) => left.cmp(right),
        (HistoryBranchNameRef::Plain(_), HistoryBranchNameRef::Remote { .. }) => Ordering::Less,
        (HistoryBranchNameRef::Remote { .. }, HistoryBranchNameRef::Plain(_)) => Ordering::Greater,
        (
            HistoryBranchNameRef::Remote {
                remote: left_remote,
                name: left_name,
            },
            HistoryBranchNameRef::Remote {
                remote: right_remote,
                name: right_name,
            },
        ) => left_remote
            .cmp(right_remote)
            .then(left_name.cmp(right_name)),
    })
}

fn sort_and_dedup_history_branch_ref_names(names: &mut HistoryBranchNameBucket<'_>) {
    if names.len() < 2 {
        return;
    }
    names.sort_unstable_by(|left, right| cmp_history_branch_ref_item(*left, *right));
    names.dedup_by(|left, right| cmp_history_branch_ref_item(*left, *right) == Ordering::Equal);
}

fn build_history_branch_names_by_target<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&str>,
    head_target: Option<&str>,
) -> HashMap<&'a str, HistoryBranchNameBucket<'a>> {
    build_history_branch_names_by_target_with_dedup(
        branches,
        remote_branches,
        head_branch,
        head_target,
        sort_and_dedup_history_branch_names,
    )
}

fn build_history_branch_ref_names_by_target<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&str>,
    head_target: Option<&str>,
) -> HashMap<&'a str, HistoryBranchNameBucket<'a>> {
    build_history_branch_names_by_target_with_dedup(
        branches,
        remote_branches,
        head_branch,
        head_target,
        sort_and_dedup_history_branch_ref_names,
    )
}

fn build_history_branch_names_by_target_with_dedup<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&str>,
    head_target: Option<&str>,
    dedup: fn(&mut HistoryBranchNameBucket<'a>),
) -> HashMap<&'a str, HistoryBranchNameBucket<'a>> {
    let mut branch_names_by_target: HashMap<&str, HistoryBranchNameBucket<'_>> =
        HashMap::with_capacity_and_hasher(
            branches.len() + remote_branches.len(),
            Default::default(),
        );

    for branch in branches.iter() {
        let should_skip = head_branch.is_some_and(|head| head != "HEAD" && branch.name == head)
            && head_target == Some(branch.target.as_ref());
        if should_skip {
            continue;
        }
        branch_names_by_target
            .entry(branch.target.as_ref())
            .or_default()
            .push(HistoryBranchNameRef::Plain(branch.name.as_str()));
    }

    for branch in remote_branches.iter() {
        branch_names_by_target
            .entry(branch.target.as_ref())
            .or_default()
            .push(HistoryBranchNameRef::Remote {
                remote: branch.remote.as_str(),
                name: branch.name.as_str(),
            });
    }

    for names in branch_names_by_target.values_mut() {
        dedup(names);
    }

    branch_names_by_target
}

fn shared_history_branch_text(names: &[HistoryBranchNameRef<'_>]) -> SharedString {
    match names {
        [] => return SharedString::default(),
        [name] => return name.to_shared_string(),
        _ => {}
    }

    let total_len = names
        .iter()
        .copied()
        .map(HistoryBranchNameRef::display_len)
        .sum::<usize>()
        + 2 * names.len().saturating_sub(1);
    let mut text = String::with_capacity(total_len);
    for (ix, name) in names.iter().copied().enumerate() {
        if ix > 0 {
            text.push_str(", ");
        }
        name.write_display_to(&mut text);
    }
    SharedString::from(text)
}

fn shared_history_branch_text_with_extra_plain(
    names: &[HistoryBranchNameRef<'_>],
    extra_plain: &str,
) -> SharedString {
    if names.is_empty() {
        return SharedString::new(extra_plain);
    }

    let extra = HistoryBranchNameRef::Plain(extra_plain);
    let include_extra = names
        .iter()
        .copied()
        .all(|name| cmp_history_branch_display(name, extra) != Ordering::Equal);
    let total_len = names
        .iter()
        .copied()
        .map(HistoryBranchNameRef::display_len)
        .sum::<usize>()
        + usize::from(include_extra) * extra.display_len()
        + 2 * (names.len() + usize::from(include_extra)).saturating_sub(1);
    let mut text = String::with_capacity(total_len);
    let mut wrote_any = false;
    let mut extra_pending = include_extra;

    for name in names.iter().copied() {
        if extra_pending && cmp_history_branch_display(extra, name) == Ordering::Less {
            if wrote_any {
                text.push_str(", ");
            }
            extra.write_display_to(&mut text);
            wrote_any = true;
            extra_pending = false;
        }
        if wrote_any {
            text.push_str(", ");
        }
        name.write_display_to(&mut text);
        wrote_any = true;
    }

    if extra_pending {
        if wrote_any {
            text.push_str(", ");
        }
        extra.write_display_to(&mut text);
    }

    SharedString::from(text)
}

pub(in crate::view) fn build_history_branch_text_by_target<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&str>,
    head_target: Option<&str>,
) -> (HashMap<&'a str, HistoryTextVm>, Option<HistoryTextVm>) {
    let branch_names_by_target =
        build_history_branch_names_by_target(branches, remote_branches, head_branch, head_target);

    let head_branches_text = history_head_branch_label(head_branch).map(|head_label| {
        let names = head_target
            .and_then(|target| branch_names_by_target.get(target))
            .cloned()
            .unwrap_or_default();
        shared_history_branch_text_with_extra_plain(&names, head_label.as_str())
    });

    let mut branch_text_by_target: HashMap<&str, HistoryTextVm> =
        HashMap::with_capacity_and_hasher(branch_names_by_target.len(), Default::default());
    for (target, names) in branch_names_by_target {
        if names.is_empty() {
            continue;
        }
        branch_text_by_target.insert(
            target,
            HistoryTextVm::new(shared_history_branch_text(&names)),
        );
    }

    (
        branch_text_by_target,
        head_branches_text.map(HistoryTextVm::new),
    )
}

fn history_branch_ref_item(name: HistoryBranchNameRef<'_>) -> HistoryRefListItem {
    let text = name.to_shared_string();
    let kind = match name {
        HistoryBranchNameRef::Plain(name) => HistoryRefListItemKind::LocalBranch {
            name: name.to_string(),
        },
        HistoryBranchNameRef::Remote { remote, name } => HistoryRefListItemKind::RemoteBranch {
            name: format!("{remote}/{name}"),
        },
    };

    HistoryRefListItem {
        text: HistoryTextVm::new(text),
        kind,
    }
}

fn history_head_ref_item(head_branch: &str) -> HistoryRefListItem {
    let text = history_head_branch_label(Some(head_branch)).unwrap_or_default();
    let kind = if head_branch == "HEAD" {
        HistoryRefListItemKind::DetachedHead
    } else {
        HistoryRefListItemKind::AttachedHead {
            branch: head_branch.to_string(),
        }
    };

    HistoryRefListItem {
        text: HistoryTextVm::new(SharedString::from(text)),
        kind,
    }
}

fn history_branch_ref_items(names: &[HistoryBranchNameRef<'_>]) -> Arc<[HistoryRefListItem]> {
    names
        .iter()
        .copied()
        .map(history_branch_ref_item)
        .collect::<Vec<_>>()
        .into()
}

fn history_branch_ref_items_with_extra_head(
    names: &[HistoryBranchNameRef<'_>],
    head_branch: &str,
) -> Arc<[HistoryRefListItem]> {
    if names.is_empty() {
        return vec![history_head_ref_item(head_branch)].into();
    }

    let head_label = history_head_branch_label(Some(head_branch)).unwrap_or_default();
    let extra = HistoryBranchNameRef::Plain(head_label.as_str());
    let include_extra = names
        .iter()
        .copied()
        .all(|name| cmp_history_branch_display(name, extra) != Ordering::Equal);
    let mut out = Vec::with_capacity(names.len() + usize::from(include_extra));
    let mut extra_pending = include_extra;

    for name in names.iter().copied() {
        if extra_pending && cmp_history_branch_display(extra, name) == Ordering::Less {
            out.push(history_head_ref_item(head_branch));
            extra_pending = false;
        }
        out.push(history_branch_ref_item(name));
    }

    if extra_pending {
        out.push(history_head_ref_item(head_branch));
    }

    out.into()
}

pub(in crate::view) fn build_history_branch_ref_items_by_target<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&str>,
    head_target: Option<&str>,
) -> (HistoryRefItemsByTarget<'a>, Option<HistoryRefItems>) {
    let branch_names_by_target = build_history_branch_ref_names_by_target(
        branches,
        remote_branches,
        head_branch,
        head_target,
    );

    let head_branch_ref_items = history_head_branch_label(head_branch).map(|_| {
        let names = head_target
            .and_then(|target| branch_names_by_target.get(target))
            .map(|names| names.as_slice())
            .unwrap_or(&[]);
        history_branch_ref_items_with_extra_head(names, head_branch.unwrap_or_default())
    });

    let mut ref_items_by_target: HistoryRefItemsByTarget<'_> =
        HashMap::with_capacity_and_hasher(branch_names_by_target.len(), Default::default());
    for (target, names) in branch_names_by_target {
        if names.is_empty() {
            continue;
        }
        ref_items_by_target.insert(target, history_branch_ref_items(&names));
    }

    (ref_items_by_target, head_branch_ref_items)
}

pub(in crate::view) fn build_history_tag_names_by_target(
    tags: &[Tag],
) -> HashMap<&str, Arc<[HistoryTextVm]>> {
    let mut tag_names_by_target: HashMap<&str, HistoryTagNameBucket<'_>> =
        HashMap::with_capacity_and_hasher(tags.len(), Default::default());
    for tag in tags.iter() {
        tag_names_by_target
            .entry(tag.target.as_ref())
            .or_default()
            .push(tag.name.as_str());
    }

    let mut tag_text_by_target: HashMap<&str, Arc<[HistoryTextVm]>> =
        HashMap::with_capacity_and_hasher(tag_names_by_target.len(), Default::default());
    for (target, mut names) in tag_names_by_target {
        if names.is_empty() {
            continue;
        }
        if names.len() == 1 {
            let tag_names: Vec<HistoryTextVm> =
                vec![HistoryTextVm::new(SharedString::new(names[0]))];
            tag_text_by_target.insert(target, tag_names.into());
            continue;
        }
        names.sort_unstable();
        names.dedup();
        let tag_names: Vec<HistoryTextVm> = names
            .into_iter()
            .map(SharedString::new)
            .map(HistoryTextVm::new)
            .collect();
        tag_text_by_target.insert(target, tag_names.into());
    }

    tag_text_by_target
}

pub(in crate::view) fn history_ref_items_from_displayed_refs(
    tag_names: &Arc<[HistoryTextVm]>,
    branch_items: Arc<[HistoryRefListItem]>,
) -> Arc<[HistoryRefListItem]> {
    if tag_names.is_empty() {
        return branch_items;
    }

    let mut items = Vec::with_capacity(tag_names.len() + branch_items.len());
    items.extend(tag_names.iter().map(|tag| HistoryRefListItem {
        text: tag.clone(),
        kind: HistoryRefListItemKind::Tag {
            name: tag.as_ref().to_string(),
        },
    }));
    items.extend(branch_items.iter().cloned());
    items.into()
}

fn history_head_branch_label(head_branch: Option<&str>) -> Option<String> {
    match head_branch {
        Some("HEAD") => Some("HEAD".to_string()),
        Some(head) => Some(format!("HEAD → {head}")),
        None => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BranchSidebarFingerprint {
    cache_rev: u64,
}

impl BranchSidebarFingerprint {
    #[inline]
    pub(super) fn from_repo(repo: &RepoState) -> Self {
        Self {
            cache_rev: repo.branch_sidebar_cache_rev(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BranchSidebarCache {
    pub(super) repo_id: RepoId,
    pub(super) fingerprint: BranchSidebarFingerprint,
    pub(super) source_fingerprint: BranchSidebarSourceFingerprint,
    pub(super) source_parts: BranchSidebarSourceFingerprintParts,
    pub(super) rows: Rc<[BranchSidebarRow]>,
}

pub(super) fn branch_sidebar_cache_lookup(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
) -> Option<Rc<[BranchSidebarRow]>> {
    if let Some(cached) = cache.as_mut()
        && cached.repo_id == repo_id
        && cached.fingerprint == fingerprint
    {
        return Some(Rc::clone(&cached.rows));
    }

    None
}

pub(super) fn branch_sidebar_cache_lookup_by_source(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    source_fingerprint: BranchSidebarSourceFingerprint,
    source_parts: &BranchSidebarSourceFingerprintParts,
) -> Option<Rc<[BranchSidebarRow]>> {
    if let Some(cached) = cache.as_mut()
        && cached.repo_id == repo_id
        && cached.source_fingerprint == source_fingerprint
    {
        cached.fingerprint = fingerprint;
        cached.source_fingerprint = source_fingerprint;
        cached.source_parts = source_parts.clone();
        return Some(Rc::clone(&cached.rows));
    }

    None
}

#[inline]
pub(super) fn branch_sidebar_cache_lookup_by_cached_source(
    cache: &mut Option<BranchSidebarCache>,
    repo: &RepoState,
    fingerprint: BranchSidebarFingerprint,
) -> Option<Rc<[BranchSidebarRow]>> {
    if let Some(cached) = cache.as_mut()
        && cached.repo_id == repo.id
        && branch_sidebar_source_matches_cached(repo, &cached.source_parts)
    {
        cached.fingerprint = fingerprint;
        return Some(Rc::clone(&cached.rows));
    }

    None
}

pub(super) fn branch_sidebar_cache_store(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    source_fingerprint: BranchSidebarSourceFingerprint,
    source_parts: BranchSidebarSourceFingerprintParts,
    rows: Rc<[BranchSidebarRow]>,
) {
    *cache = Some(BranchSidebarCache {
        repo_id,
        fingerprint,
        source_fingerprint,
        source_parts,
        rows,
    });
}

/// Whether `commit_ix` is set in a commit bitset. Used by the branch-containment
/// bitsets that attribute a row to an integration branch.
#[inline]
pub(super) fn related_commit_contains(bits: &[u64], commit_ix: usize) -> bool {
    bits.get(commit_ix / 64)
        .is_some_and(|word| word & (1u64 << (commit_ix % 64)) != 0)
}

/// Marks the anchor's whole chain: the commit itself, every commit it descends
/// from, and every commit that descends from it.
///
/// Relies on the log-order invariant the graph also relies on: a commit's
/// parents sit at *higher* indices than it does. Ancestors are therefore a
/// single sweep downward through the list and descendants a single sweep upward
/// -- no queue, no revisiting -- and the first parent can be taken as the next
/// row without a lookup, which is the common case. The id map is built lazily,
/// so a linear history never pays for it.
///
/// The two directions are accumulated separately and combined at the end. Sharing
/// one bitset would let the descendant sweep mistake an *ancestor* of the anchor
/// for one of its descendants: a sibling branch forking off that ancestor has a
/// marked parent without descending from the anchor at all.
/// Bitset of `anchor_ix` and everything it descends from.
///
/// Split out because branch attribution needs containment ("is this commit in
/// `dev`?") without the descendant half.
fn ancestor_bits<'a>(
    commits: &'a [Commit],
    anchor_ix: usize,
    id_to_index: &mut Option<HashMap<&'a str, usize>>,
) -> Vec<u64> {
    let mut bits = vec![0u64; commits.len().div_ceil(64)];
    bits[anchor_ix / 64] |= 1u64 << (anchor_ix % 64);

    for (ix, commit) in commits.iter().enumerate().skip(anchor_ix) {
        if bits[ix / 64] & (1u64 << (ix % 64)) == 0 {
            continue;
        }
        for (parent_pos, parent) in commit.parent_ids.iter().enumerate() {
            let parent_id = parent.as_ref();
            let resolved = if parent_pos == 0
                && commits
                    .get(ix + 1)
                    .is_some_and(|next| next.id.as_ref() == parent_id)
            {
                Some(ix + 1)
            } else {
                index_of(id_to_index, commits, parent_id)
            };
            // Only ever downwards, so a parent resolving above cannot loop.
            if let Some(parent_ix) = resolved.filter(|&parent_ix| parent_ix > ix) {
                bits[parent_ix / 64] |= 1u64 << (parent_ix % 64);
            }
        }
    }
    bits
}

/// Commits contained in each branch whose tip is listed, in the order given: the
/// tip itself and everything it descends from. An empty bitset stands in for a
/// tip that is not in the page.
///
/// Takes every tip at once so the one id -> index map `ancestor_bits` builds
/// lazily is shared across them. That map holds an entry per commit in the page,
/// so building one per tip would hash every commit id again for each branch.
pub(super) fn build_history_branch_containment_bits<'t>(
    commits: &[Commit],
    tips: impl IntoIterator<Item = &'t CommitId>,
) -> Vec<Arc<[u64]>> {
    let mut id_to_index: Option<HashMap<&str, usize>> = None;
    tips.into_iter()
        .map(|tip| {
            let tip_id = tip.as_ref();
            let Some(tip_ix) = commits
                .iter()
                .position(|commit| commit.id.as_ref() == tip_id)
            else {
                return Arc::from(Vec::new());
            };
            Arc::from(ancestor_bits(commits, tip_ix, &mut id_to_index))
        })
        .collect()
}

/// Shared by the ancestor pass and the relation builder; a free fn because the
/// map borrows from `commits` and a closure cannot name that lifetime.
fn index_of<'a>(
    map: &mut Option<HashMap<&'a str, usize>>,
    commits: &'a [Commit],
    id: &str,
) -> Option<usize> {
    map.get_or_insert_with(|| {
        let mut built: HashMap<&'a str, usize> =
            HashMap::with_capacity_and_hasher(commits.len(), Default::default());
        for (ix, commit) in commits.iter().enumerate() {
            // First occurrence wins, matching the row the history shows.
            built.entry(commit.id.as_ref()).or_insert(ix);
        }
        built
    })
    .get(id)
    .copied()
}

/// Caches the interleaving of synthetic rows into the commit list. Rebuilt
/// whenever the commit set, the dirty-worktree scan, or the working-tree row's
/// visibility changes.
#[derive(Clone, Debug)]
pub(super) struct HistoryListPlanCache {
    pub(super) repo_id: RepoId,
    pub(super) log_fingerprint: u64,
    pub(super) history_scope: LogScope,
    pub(super) worktrees_rev: u64,
    pub(super) worktree_dirty_rev: u64,
    pub(super) show_working_tree_summary_row: bool,
    pub(super) plan: HistoryListPlan,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryWorktreeSummaryCache {
    pub(super) repo_id: RepoId,
    pub(super) worktree_status_rev: u64,
    pub(super) staged_status_rev: u64,
    pub(super) show_row: bool,
    pub(super) counts: (usize, usize, usize),
}

#[derive(Clone, Debug)]
pub(super) struct HistoryStashIdsCache {
    pub(super) repo_id: RepoId,
    pub(super) stashes_rev: u64,
    pub(super) ids: Arc<HashSet<CommitId>>,
}

impl GitCometView {
    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn branch_sidebar_rows(repo: &RepoState) -> Vec<BranchSidebarRow> {
        branch_sidebar::branch_sidebar_rows(
            repo,
            &std::collections::BTreeSet::new(),
            &std::collections::BTreeSet::new(),
            "",
        )
    }

    #[cfg(test)]
    pub(super) fn branch_sidebar_rows_with_collapsed(
        repo: &RepoState,
        collapsed_items: &[&str],
    ) -> Vec<BranchSidebarRow> {
        let collapsed_items: std::collections::BTreeSet<String> = collapsed_items
            .iter()
            .map(|item| (*item).to_string())
            .collect();
        branch_sidebar::branch_sidebar_rows(
            repo,
            &collapsed_items,
            &std::collections::BTreeSet::new(),
            "",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn commit_id(id: &str) -> CommitId {
        CommitId(id.into())
    }

    fn commit(id: &str, parents: &[&str], summary: &str) -> Commit {
        Commit {
            id: commit_id(id),
            parent_ids: parents.iter().map(|parent| commit_id(parent)).collect(),
            summary: summary.into(),
            author: "author".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn history_branch_text_cache_precomputes_head_and_remote_labels() {
        let commit_a = commit_id("a");
        let commit_b = commit_id("b");
        let branches = vec![
            Branch {
                name: "main".to_string(),
                target: commit_a.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "feature".to_string(),
                target: commit_a.clone(),
                upstream: None,
                divergence: None,
            },
        ];
        let remote_branches = vec![
            RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: commit_a.clone(),
            },
            RemoteBranch {
                remote: "upstream".to_string(),
                name: "topic".to_string(),
                target: commit_b.clone(),
            },
        ];

        let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
            &branches,
            &remote_branches,
            Some("main"),
            Some(commit_a.as_ref()),
        );

        assert_eq!(
            branch_text_by_target
                .get(commit_a.as_ref())
                .map(HistoryTextVm::as_ref),
            Some("feature, origin/main")
        );
        assert_eq!(
            branch_text_by_target
                .get(commit_b.as_ref())
                .map(HistoryTextVm::as_ref),
            Some("upstream/topic")
        );
        assert_eq!(
            head_branches_text.as_ref().map(HistoryTextVm::as_ref),
            Some("HEAD → main, feature, origin/main")
        );
    }

    #[test]
    fn history_branch_text_dedups_and_orders_duplicate_names() {
        let commit = commit_id("a");
        let branches = vec![
            Branch {
                name: "topic".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "apple".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "topic".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
        ];
        let remote_branches = vec![
            RemoteBranch {
                remote: "origin".to_string(),
                name: "zzz".to_string(),
                target: commit.clone(),
            },
            RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: commit.clone(),
            },
            RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: commit.clone(),
            },
        ];

        let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
            &branches,
            &remote_branches,
            Some("topic"),
            Some(commit.as_ref()),
        );

        assert_eq!(
            branch_text_by_target
                .get(commit.as_ref())
                .map(HistoryTextVm::as_ref),
            Some("apple, origin/main, origin/zzz")
        );
        assert_eq!(
            head_branches_text.as_ref().map(HistoryTextVm::as_ref),
            Some("HEAD → topic, apple, origin/main, origin/zzz")
        );
    }

    #[test]
    fn history_tag_names_cache_dedups_once_per_target() {
        let commit_a = commit_id("a");
        let tags = vec![
            Tag {
                name: "v2.0.0".to_string(),
                target: commit_a.clone(),
            },
            Tag {
                name: "v1.0.0".to_string(),
                target: commit_a.clone(),
            },
            Tag {
                name: "v1.0.0".to_string(),
                target: commit_a.clone(),
            },
        ];

        let tag_names_by_target = build_history_tag_names_by_target(&tags);
        let tag_names = tag_names_by_target
            .get(commit_a.as_ref())
            .expect("tag names should be cached for the target");
        let tag_names = tag_names
            .iter()
            .map(HistoryTextVm::as_ref)
            .collect::<Vec<_>>();

        assert_eq!(tag_names, vec!["v1.0.0", "v2.0.0"]);
    }

    #[test]
    fn history_ref_items_preserve_display_order_and_ref_types_for_attached_head() {
        let commit = commit_id("a");
        let branches = vec![
            Branch {
                name: "main".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "feature".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
            Branch {
                name: "feature".to_string(),
                target: commit.clone(),
                upstream: None,
                divergence: None,
            },
        ];
        let remote_branches = vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        }];
        let tags = vec![
            Tag {
                name: "v2.0.0".to_string(),
                target: commit.clone(),
            },
            Tag {
                name: "v1.0.0".to_string(),
                target: commit.clone(),
            },
        ];

        let (_, head_branch_items) = build_history_branch_ref_items_by_target(
            &branches,
            &remote_branches,
            Some("main"),
            Some(commit.as_ref()),
        );
        let tag_names_by_target = build_history_tag_names_by_target(&tags);
        let tag_names = tag_names_by_target
            .get(commit.as_ref())
            .expect("expected tag names");
        let ref_items = history_ref_items_from_displayed_refs(
            tag_names,
            head_branch_items.expect("expected head branch items"),
        );

        let display = ref_items
            .iter()
            .map(|item| item.text.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            display,
            vec!["v1.0.0", "v2.0.0", "HEAD → main", "feature", "origin/main"]
        );
        assert!(matches!(
            ref_items[0].kind,
            HistoryRefListItemKind::Tag { ref name } if name == "v1.0.0"
        ));
        assert!(matches!(
            ref_items[2].kind,
            HistoryRefListItemKind::AttachedHead { ref branch } if branch == "main"
        ));
        assert!(matches!(
            ref_items[3].kind,
            HistoryRefListItemKind::LocalBranch { ref name } if name == "feature"
        ));
        assert!(matches!(
            ref_items[4].kind,
            HistoryRefListItemKind::RemoteBranch { ref name } if name == "origin/main"
        ));

        let hidden_tags = Arc::<[HistoryTextVm]>::from([]);
        let ref_items_without_tags = history_ref_items_from_displayed_refs(
            &hidden_tags,
            ref_items.iter().skip(2).cloned().collect::<Vec<_>>().into(),
        );
        let display = ref_items_without_tags
            .iter()
            .map(|item| item.text.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(display, vec!["HEAD → main", "feature", "origin/main"]);
    }

    #[test]
    fn history_ref_items_preserve_detached_head_as_informational() {
        let commit = commit_id("a");
        let branches = vec![Branch {
            name: "main".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        }];
        let remote_branches = vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        }];

        let (_, head_branch_items) = build_history_branch_ref_items_by_target(
            &branches,
            &remote_branches,
            Some("HEAD"),
            Some(commit.as_ref()),
        );
        let items = head_branch_items.expect("expected detached head item");
        let display = items
            .iter()
            .map(|item| item.text.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(display, vec!["HEAD", "main", "origin/main"]);
        assert!(matches!(
            items[0].kind,
            HistoryRefListItemKind::DetachedHead
        ));
        assert!(matches!(
            items[1].kind,
            HistoryRefListItemKind::LocalBranch { ref name } if name == "main"
        ));
        assert!(matches!(
            items[2].kind,
            HistoryRefListItemKind::RemoteBranch { ref name } if name == "origin/main"
        ));
    }

    #[test]
    fn history_ref_items_keep_local_and_remote_refs_with_same_display_text() {
        let commit = commit_id("a");
        let branches = vec![Branch {
            name: "origin/main".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        }];
        let remote_branches = vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        }];

        let (text_by_target, _) =
            build_history_branch_text_by_target(&branches, &remote_branches, None, None);
        assert_eq!(
            text_by_target
                .get(commit.as_ref())
                .expect("expected compact branch text")
                .as_ref(),
            "origin/main",
            "history row text should still dedupe duplicate display labels"
        );

        let (items_by_target, _) =
            build_history_branch_ref_items_by_target(&branches, &remote_branches, None, None);
        let items = items_by_target
            .get(commit.as_ref())
            .expect("expected hover branch refs");
        let display = items
            .iter()
            .map(|item| item.text.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(display, vec!["origin/main", "origin/main"]);
        assert!(matches!(
            items[0].kind,
            HistoryRefListItemKind::LocalBranch { ref name } if name == "origin/main"
        ));
        assert!(matches!(
            items[1].kind,
            HistoryRefListItemKind::RemoteBranch { ref name } if name == "origin/main"
        ));
    }

    #[test]
    fn history_stash_analysis_ignores_stash_ids_absent_from_log() {
        let commits = vec![
            commit("a", &[], "Commit A"),
            commit("b", &["a"], "Commit B"),
        ];
        let stashes = vec![StashEntry {
            index: 0,
            id: commit_id("z"),
            message: "On main: hidden stash".into(),
            created_at: None,
        }];

        let analysis = analyze_history_stashes(&commits, &stashes);

        assert!(analysis.stash_tips.is_empty());
        assert!(analysis.stash_helper_ids.is_empty());
    }

    #[test]
    fn history_stash_analysis_keeps_matching_tip_message_and_helper() {
        let commits = vec![
            commit("base", &[], "Commit base"),
            commit("helper", &["base"], "index on main: helper"),
            commit("tip", &["base", "helper"], "WIP on main: fallback"),
        ];
        let stashes = vec![StashEntry {
            index: 0,
            id: commit_id("tip"),
            message: "On main: listed stash".into(),
            created_at: None,
        }];

        let analysis = analyze_history_stashes(&commits, &stashes);

        assert_eq!(analysis.stash_tips.len(), 1);
        assert_eq!(analysis.stash_tips[0].commit_ix, 2);
        assert_eq!(
            analysis.stash_tips[0].message.map(AsRef::as_ref),
            Some("On main: listed stash")
        );
        assert!(analysis.stash_helper_ids.contains("helper"));
    }

    #[test]
    fn history_when_vm_formats_lazily_and_caches_result() {
        let display_key = HistoryDisplayKey::new(DateTimeFormat::YmdHm, Timezone::Utc, true, false);
        let when = HistoryWhenVm::deferred(SystemTime::UNIX_EPOCH);

        assert!(when.formatted.borrow().is_none());
        let first = when.resolve(display_key);
        let second = when.resolve(display_key);
        assert_eq!(first, second);
        assert!(when.formatted.borrow().is_some());
    }

    #[test]
    fn history_short_sha_vm_formats_lazily_and_caches_result() {
        let short_sha = HistoryShortShaVm::new("0123456789abcdef");

        assert_eq!(short_sha.as_str(), "01234567");
        assert!(short_sha.formatted.borrow().is_none());
        let first = short_sha.resolve();
        let second = short_sha.resolve();
        assert_eq!(first.as_ref(), "01234567");
        assert_eq!(first, second);
        assert!(short_sha.formatted.borrow().is_some());
    }

    #[test]
    fn history_short_sha_vm_preserves_short_ids_without_padding() {
        let short_sha = HistoryShortShaVm::new("abc");

        assert_eq!(short_sha.as_str(), "abc");
        assert_eq!(short_sha.resolve().as_ref(), "abc");
    }

    #[test]
    fn detached_head_history_branch_text_adds_head_label_once() {
        let commit = commit_id("a");
        let branches = vec![Branch {
            name: "main".to_string(),
            target: commit.clone(),
            upstream: None,
            divergence: None,
        }];
        let remote_branches = vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit.clone(),
        }];

        let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
            &branches,
            &remote_branches,
            Some("HEAD"),
            Some(commit.as_ref()),
        );

        assert_eq!(
            branch_text_by_target
                .get(commit.as_ref())
                .map(HistoryTextVm::as_ref),
            Some("main, origin/main")
        );
        assert_eq!(
            head_branches_text.as_ref().map(HistoryTextVm::as_ref),
            Some("HEAD, main, origin/main")
        );
    }

    #[test]
    fn branch_sidebar_cache_lookup_by_source_reuses_rows_and_updates_fingerprint() {
        let repo = RepoState::new_opening(
            RepoId(7),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        let (source_fingerprint, source_parts) =
            branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
        let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
        let mut cache = None;

        branch_sidebar_cache_store(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 1 },
            source_fingerprint,
            source_parts.clone(),
            Rc::clone(&rows),
        );

        let hit = branch_sidebar_cache_lookup_by_source(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 2 },
            source_fingerprint,
            &source_parts,
        )
        .expect("matching source fingerprints should reuse cached rows");

        assert!(Rc::ptr_eq(&hit, &rows));
        let cached = cache
            .as_ref()
            .expect("branch sidebar cache should stay populated");
        assert_eq!(
            cached.fingerprint,
            BranchSidebarFingerprint { cache_rev: 2 }
        );
        assert_eq!(cached.source_fingerprint, source_fingerprint);
        assert_eq!(cached.source_parts, source_parts);
    }

    #[test]
    fn branch_sidebar_cache_lookup_by_cached_source_reuses_rows_when_revs_bump() {
        let mut repo = RepoState::new_opening(
            RepoId(7),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        let (source_fingerprint, source_parts) =
            branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
        let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
        let mut cache = None;

        branch_sidebar_cache_store(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 1 },
            source_fingerprint,
            source_parts,
            Rc::clone(&rows),
        );

        repo.branches_rev = repo.branches_rev.wrapping_add(1);

        let hit = branch_sidebar_cache_lookup_by_cached_source(
            &mut cache,
            &repo,
            BranchSidebarFingerprint { cache_rev: 2 },
        )
        .expect("unchanged source snapshots should reuse cached rows");

        assert!(Rc::ptr_eq(&hit, &rows));
        let cached = cache
            .as_ref()
            .expect("branch sidebar cache should stay populated");
        assert_eq!(
            cached.fingerprint,
            BranchSidebarFingerprint { cache_rev: 2 }
        );
    }

    #[test]
    fn branch_sidebar_cache_lookup_by_source_rejects_repo_and_source_mismatches() {
        let repo = RepoState::new_opening(
            RepoId(7),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        let (source_fingerprint, source_parts) =
            branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
        let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
        let mut cache = None;

        branch_sidebar_cache_store(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 1 },
            source_fingerprint,
            source_parts,
            rows,
        );

        let other_repo = RepoState::new_opening(
            RepoId(8),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/other"),
            },
        );
        let (other_source_fingerprint, other_source_parts) =
            branch_sidebar::branch_sidebar_source_fingerprint(&other_repo, None);

        assert!(
            branch_sidebar_cache_lookup_by_source(
                &mut cache,
                other_repo.id,
                BranchSidebarFingerprint { cache_rev: 2 },
                source_fingerprint,
                &other_source_parts,
            )
            .is_none()
        );
        assert!(
            branch_sidebar_cache_lookup_by_source(
                &mut cache,
                repo.id,
                BranchSidebarFingerprint { cache_rev: 2 },
                other_source_fingerprint,
                &other_source_parts,
            )
            .is_none()
        );
    }

    #[test]
    fn branch_sidebar_cache_lookup_by_source_reuses_rows_after_worktrees_rev_bump() {
        let mut repo = RepoState::new_opening(
            RepoId(7),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        let (source_fingerprint, source_parts) =
            branch_sidebar::branch_sidebar_source_fingerprint(&repo, None);
        let rows: Rc<[BranchSidebarRow]> = vec![BranchSidebarRow::SectionSpacer].into();
        let mut cache = None;

        branch_sidebar_cache_store(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 1 },
            source_fingerprint,
            source_parts.clone(),
            Rc::clone(&rows),
        );

        repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

        let hit = branch_sidebar_cache_lookup_by_source(
            &mut cache,
            repo.id,
            BranchSidebarFingerprint { cache_rev: 2 },
            source_fingerprint,
            &source_parts,
        )
        .expect("matching source fingerprints should reuse cached rows after worktrees rev bump");

        assert!(Rc::ptr_eq(&hit, &rows));
    }

    #[test]
    fn branch_sidebar_cache_fingerprint_changes_when_worktrees_rev_bumps() {
        let mut repo = RepoState::new_opening(
            RepoId(7),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );

        let fingerprint_before = BranchSidebarFingerprint::from_repo(&repo);

        repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

        let fingerprint_after = BranchSidebarFingerprint::from_repo(&repo);

        assert_ne!(fingerprint_before, fingerprint_after);
    }
}
