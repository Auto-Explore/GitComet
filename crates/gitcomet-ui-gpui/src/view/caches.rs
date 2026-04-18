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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryBaseCacheRequest {
    pub(super) repo_id: RepoId,
    pub(super) history_scope: LogScope,
    pub(super) log_fingerprint: u64,
    pub(super) head_branch_rev: u64,
    pub(super) detached_head_commit: Option<CommitId>,
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
}

impl HistoryDisplayKey {
    pub(in crate::view) const fn new(
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
    ) -> Self {
        Self {
            date_time_format,
            timezone,
            show_timezone,
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

        let mut formatted = String::with_capacity(32);
        format_datetime_into(
            &mut formatted,
            self.time,
            display_key.date_time_format,
            display_key.timezone,
            display_key.show_timezone,
        );
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
    pub(super) branches_text: HistoryTextVm,
    pub(super) tag_names: Arc<[HistoryTextVm]>,
}

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
        sort_and_dedup_history_branch_names(names);
    }

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
        branch_sidebar::branch_sidebar_rows(repo, &std::collections::BTreeSet::new())
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
        branch_sidebar::branch_sidebar_rows(repo, &collapsed_items)
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
        let display_key = HistoryDisplayKey::new(DateTimeFormat::YmdHm, Timezone::Utc, true);
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
}
