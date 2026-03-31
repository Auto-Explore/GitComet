use super::*;
use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    num::NonZeroU32,
};

const LOCAL_SECTION_KEY: &str = "section:branches/local";
const REMOTE_SECTION_KEY: &str = "section:branches/remote";
const WORKTREES_SECTION_KEY: &str = "section:worktrees";
const SUBMODULES_SECTION_KEY: &str = "section:submodules";
const STASH_SECTION_KEY: &str = "section:stash";
const EXPANDED_DEFAULT_SECTION_PREFIX: &str = "expanded:";
const TRAILING_BOTTOM_SPACERS: usize = 3;
const REMOTE_HEADER_GROUP_PREFIX: &str = "group:remote-header:";
const LOCAL_GROUP_PREFIX: &str = "group:local:";
const REMOTE_GROUP_PREFIX: &str = "group:remote:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BranchSection {
    Local,
    Remote,
}

type BranchSidebarDepth = u16;

pub(super) const fn local_section_storage_key() -> &'static str {
    LOCAL_SECTION_KEY
}

pub(super) const fn remote_section_storage_key() -> &'static str {
    REMOTE_SECTION_KEY
}

pub(super) const fn worktrees_section_storage_key() -> &'static str {
    WORKTREES_SECTION_KEY
}

pub(super) const fn submodules_section_storage_key() -> &'static str {
    SUBMODULES_SECTION_KEY
}

pub(super) const fn stash_section_storage_key() -> &'static str {
    STASH_SECTION_KEY
}

pub(super) fn remote_header_storage_key(name: &str) -> String {
    let mut key = String::with_capacity(REMOTE_HEADER_GROUP_PREFIX.len() + name.len());
    key.push_str(REMOTE_HEADER_GROUP_PREFIX);
    key.push_str(name);
    key
}

pub(super) fn local_group_storage_key(path: &str) -> String {
    let mut key = String::with_capacity(LOCAL_GROUP_PREFIX.len() + path.len());
    key.push_str(LOCAL_GROUP_PREFIX);
    key.push_str(path);
    key
}

pub(super) fn remote_group_storage_key(remote: &str, path: &str) -> String {
    let mut key = String::with_capacity(REMOTE_GROUP_PREFIX.len() + remote.len() + 1 + path.len());
    key.push_str(REMOTE_GROUP_PREFIX);
    key.push_str(remote);
    key.push(':');
    key.push_str(path);
    key
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BranchSidebarRow {
    SectionHeader {
        section: BranchSection,
        top_border: bool,
        collapsed: bool,
        collapse_key: SharedString,
    },
    SectionSpacer,
    Placeholder {
        section: BranchSection,
        message: SharedString,
    },
    RemoteHeader {
        name: SharedString,
        collapsed: bool,
        collapse_key: SharedString,
    },
    GroupHeader {
        label: SharedString,
        section: BranchSection,
        depth: BranchSidebarDepth,
        collapsed: bool,
        collapse_key: SharedString,
    },
    Branch {
        name: SharedString,
        section: BranchSection,
        depth: BranchSidebarDepth,
        muted: bool,
        divergence_ahead: Option<NonZeroU32>,
        divergence_behind: Option<NonZeroU32>,
        is_head: bool,
        is_upstream: bool,
    },
    WorktreesHeader {
        top_border: bool,
        collapsed: bool,
        collapse_key: SharedString,
    },
    WorktreePlaceholder {
        message: SharedString,
    },
    WorktreeItem {
        path: std::path::PathBuf,
        branch: Option<SharedString>,
        detached: bool,
        is_active: bool,
    },
    SubmodulesHeader {
        top_border: bool,
        collapsed: bool,
        collapse_key: SharedString,
    },
    SubmodulePlaceholder {
        message: SharedString,
    },
    SubmoduleItem {
        path: std::path::PathBuf,
    },
    StashHeader {
        top_border: bool,
        collapsed: bool,
        collapse_key: SharedString,
    },
    StashPlaceholder {
        message: SharedString,
    },
    StashItem {
        index: usize,
        message: SharedString,
        tooltip: SharedString,
        created_at: Option<std::time::SystemTime>,
    },
}

#[derive(Clone, Copy, Default)]
struct SlashTreeLeafMeta {
    divergence: Option<UpstreamDivergence>,
    is_head: bool,
}

#[derive(Default)]
struct SlashTree<'a> {
    is_leaf: bool,
    leaf_meta_index: Option<NonZeroU32>,
    children: BTreeMap<&'a str, SlashTree<'a>>,
}

impl<'a> SlashTree<'a> {
    fn insert(&mut self, name: &'a str) {
        self.insert_with_leaf_meta_index(name, None);
    }

    fn insert_local(&mut self, name: &'a str, leaf_meta_index: NonZeroU32) {
        self.insert_with_leaf_meta_index(name, Some(leaf_meta_index));
    }

    fn insert_with_leaf_meta_index(&mut self, name: &'a str, leaf_meta_index: Option<NonZeroU32>) {
        let mut node = self;
        let bytes = name.as_bytes();
        let mut segment_start = 0;
        while segment_start < bytes.len() {
            while segment_start < bytes.len() && bytes[segment_start] == b'/' {
                segment_start += 1;
            }
            if segment_start >= bytes.len() {
                break;
            }

            let mut segment_end = segment_start;
            while segment_end < bytes.len() && bytes[segment_end] != b'/' {
                segment_end += 1;
            }

            node = node
                .children
                .entry(&name[segment_start..segment_end])
                .or_default();
            segment_start = segment_end;
        }
        node.is_leaf = true;
        node.leaf_meta_index = leaf_meta_index;
    }
}

struct RemoteBranchGroup<'a> {
    name: &'a str,
    branches: Vec<&'a str>,
}

pub(in crate::view) fn branch_sidebar_branch_tooltip(
    full_name: &str,
    is_upstream: bool,
) -> SharedString {
    const PREFIX: &str = "Branch: ";
    const UPSTREAM_NOTE: &str = " (upstream for current branch)";

    let upstream_note = if is_upstream { UPSTREAM_NOTE } else { "" };
    let mut tooltip = String::with_capacity(PREFIX.len() + full_name.len() + upstream_note.len());
    tooltip.push_str(PREFIX);
    tooltip.push_str(full_name);
    tooltip.push_str(upstream_note);
    tooltip.into()
}

pub(in crate::view) fn branch_sidebar_branch_label(full_name: &str) -> &str {
    full_name
        .rsplit_once('/')
        .map_or(full_name, |(_, label)| label)
}

pub(in crate::view) fn branch_sidebar_worktree_label(
    branch: Option<&str>,
    detached: bool,
    path_display: &str,
) -> SharedString {
    const SEPARATOR: &str = "  ";
    const DETACHED_LABEL: &str = "(detached)";

    match branch {
        Some(branch) => {
            let mut label =
                String::with_capacity(branch.len() + SEPARATOR.len() + path_display.len());
            label.push_str(branch);
            label.push_str(SEPARATOR);
            label.push_str(path_display);
            label.into()
        }
        None if detached => {
            let mut label =
                String::with_capacity(DETACHED_LABEL.len() + SEPARATOR.len() + path_display.len());
            label.push_str(DETACHED_LABEL);
            label.push_str(SEPARATOR);
            label.push_str(path_display);
            label.into()
        }
        None => SharedString::new(path_display),
    }
}

pub(in crate::view) fn branch_sidebar_divergence_label(count: NonZeroU32) -> SharedString {
    count.get().to_string().into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct BranchSidebarSourceFingerprint(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct BranchSidebarSourceFingerprintParts {
    local_revs: (u64, u64),
    local_hash: u64,
    local_reuse_key: u64,
    remote_revs: (u64, u64, u64, u64),
    remote_hash: u64,
    remote_reuse_key: u64,
    worktree_rev: u64,
    worktree_hash: u64,
    worktree_reuse_identity: fingerprint::LoadableArcIdentity,
    submodule_rev: u64,
    submodule_hash: u64,
    submodule_reuse_identity: fingerprint::LoadableArcIdentity,
    stash_rev: u64,
    stash_hash: u64,
    stash_reuse_identity: fingerprint::LoadableArcIdentity,
}

impl BranchSidebarSourceFingerprintParts {
    fn for_repo(repo: &RepoState, reuse: Option<&Self>) -> Self {
        let local_revs = (repo.head_branch_rev, repo.branches_rev);
        let local_reuse_key = branch_sidebar_local_reuse_key(repo);
        let remote_revs = (
            repo.head_branch_rev,
            repo.branches_rev,
            repo.remotes_rev,
            repo.remote_branches_rev,
        );
        let remote_reuse_key = branch_sidebar_remote_reuse_key(repo);
        let worktree_rev = repo.worktrees_rev;
        let worktree_reuse_identity = fingerprint::loadable_arc_identity(&repo.worktrees);
        let submodule_rev = repo.submodules_rev;
        let submodule_reuse_identity = fingerprint::loadable_arc_identity(&repo.submodules);
        let stash_rev = repo.stashes_rev;
        let stash_reuse_identity = fingerprint::loadable_arc_identity(&repo.stashes);

        Self {
            local_revs,
            local_reuse_key,
            local_hash: reuse
                .filter(|parts| {
                    parts.local_revs == local_revs || parts.local_reuse_key == local_reuse_key
                })
                .map_or_else(
                    || branch_sidebar_local_source_hash(repo),
                    |parts| parts.local_hash,
                ),
            remote_revs,
            remote_reuse_key,
            remote_hash: reuse
                .filter(|parts| {
                    parts.remote_revs == remote_revs || parts.remote_reuse_key == remote_reuse_key
                })
                .map_or_else(
                    || branch_sidebar_remote_source_hash(repo),
                    |parts| parts.remote_hash,
                ),
            worktree_rev,
            worktree_reuse_identity,
            worktree_hash: reuse
                .filter(|parts| {
                    parts.worktree_rev == worktree_rev
                        || parts.worktree_reuse_identity == worktree_reuse_identity
                })
                .map_or_else(
                    || branch_sidebar_worktree_source_hash(repo),
                    |parts| parts.worktree_hash,
                ),
            submodule_rev,
            submodule_reuse_identity,
            submodule_hash: reuse
                .filter(|parts| {
                    parts.submodule_rev == submodule_rev
                        || parts.submodule_reuse_identity == submodule_reuse_identity
                })
                .map_or_else(
                    || branch_sidebar_submodule_source_hash(repo),
                    |parts| parts.submodule_hash,
                ),
            stash_rev,
            stash_reuse_identity,
            stash_hash: reuse
                .filter(|parts| {
                    parts.stash_rev == stash_rev
                        || parts.stash_reuse_identity == stash_reuse_identity
                })
                .map_or_else(
                    || branch_sidebar_stash_source_hash(repo),
                    |parts| parts.stash_hash,
                ),
        }
    }

    fn fingerprint(&self) -> BranchSidebarSourceFingerprint {
        let mut hasher = FxHasher::default();
        0u8.hash(&mut hasher);
        self.local_hash.hash(&mut hasher);
        1u8.hash(&mut hasher);
        self.remote_hash.hash(&mut hasher);
        2u8.hash(&mut hasher);
        self.worktree_hash.hash(&mut hasher);
        3u8.hash(&mut hasher);
        self.submodule_hash.hash(&mut hasher);
        4u8.hash(&mut hasher);
        self.stash_hash.hash(&mut hasher);
        BranchSidebarSourceFingerprint(hasher.finish())
    }
}

pub(in crate::view) fn branch_sidebar_source_fingerprint(
    repo: &RepoState,
    reuse: Option<&BranchSidebarSourceFingerprintParts>,
) -> (
    BranchSidebarSourceFingerprint,
    BranchSidebarSourceFingerprintParts,
) {
    let parts = BranchSidebarSourceFingerprintParts::for_repo(repo, reuse);
    (parts.fingerprint(), parts)
}

#[inline]
pub(in crate::view) fn branch_sidebar_source_matches_cached(
    repo: &RepoState,
    cached: &BranchSidebarSourceFingerprintParts,
) -> bool {
    let local_revs = (repo.head_branch_rev, repo.branches_rev);
    if cached.local_revs != local_revs
        && cached.local_reuse_key != branch_sidebar_local_reuse_key(repo)
    {
        return false;
    }

    let remote_revs = (
        repo.head_branch_rev,
        repo.branches_rev,
        repo.remotes_rev,
        repo.remote_branches_rev,
    );
    if cached.remote_revs != remote_revs
        && cached.remote_reuse_key != branch_sidebar_remote_reuse_key(repo)
    {
        return false;
    }

    let worktree_rev = repo.worktrees_rev;
    if cached.worktree_rev != worktree_rev
        && cached.worktree_reuse_identity != fingerprint::loadable_arc_identity(&repo.worktrees)
    {
        return false;
    }

    let submodule_rev = repo.submodules_rev;
    if cached.submodule_rev != submodule_rev
        && cached.submodule_reuse_identity != fingerprint::loadable_arc_identity(&repo.submodules)
    {
        return false;
    }

    let stash_rev = repo.stashes_rev;
    if cached.stash_rev != stash_rev
        && cached.stash_reuse_identity != fingerprint::loadable_arc_identity(&repo.stashes)
    {
        return false;
    }

    true
}

fn hash_branch_sidebar_local_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    fingerprint::hash_loadable_kind(&repo.head_branch, hasher);
    if let Loadable::Ready(head_branch) = &repo.head_branch {
        head_branch.hash(hasher);
    }

    fingerprint::hash_loadable_kind(&repo.branches, hasher);
    if let Loadable::Ready(branches) = &repo.branches {
        for branch in branches.iter() {
            branch.name.hash(hasher);
            match branch.divergence {
                Some(divergence) => {
                    true.hash(hasher);
                    divergence.ahead.hash(hasher);
                    divergence.behind.hash(hasher);
                }
                None => false.hash(hasher),
            }
        }
    }
}

fn branch_sidebar_local_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    hash_branch_sidebar_local_source(repo, &mut hasher);
    hasher.finish()
}

fn branch_sidebar_head_reuse_key(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    fingerprint::hash_loadable_kind(&repo.head_branch, &mut hasher);
    if let Loadable::Ready(head_branch) = &repo.head_branch {
        head_branch.hash(&mut hasher);
    }
    hasher.finish()
}

// Branch/ref collections are treated as immutable Arc snapshots in the store, so
// their pointer identities are a valid no-change signal for cache-source reuse.
fn branch_sidebar_local_reuse_key(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    branch_sidebar_head_reuse_key(repo).hash(&mut hasher);
    fingerprint::hash_loadable_arc(&repo.branches, &mut hasher);
    hasher.finish()
}

fn hash_branch_sidebar_remote_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    fingerprint::hash_loadable_kind(&repo.head_branch, hasher);
    if let Loadable::Ready(head_branch) = &repo.head_branch {
        head_branch.hash(hasher);
    }

    fingerprint::hash_loadable_kind(&repo.branches, hasher);
    if let Loadable::Ready(branches) = &repo.branches {
        for branch in branches.iter() {
            branch.name.hash(hasher);
            match &branch.upstream {
                Some(upstream) => {
                    true.hash(hasher);
                    upstream.remote.hash(hasher);
                    upstream.branch.hash(hasher);
                }
                None => false.hash(hasher),
            }
        }
    }

    fingerprint::hash_loadable_kind(&repo.remotes, hasher);
    if let Loadable::Ready(remotes) = &repo.remotes {
        for remote in remotes.iter() {
            remote.name.hash(hasher);
        }
    }

    fingerprint::hash_loadable_kind(&repo.remote_branches, hasher);
    if let Loadable::Ready(remote_branches) = &repo.remote_branches {
        for branch in remote_branches.iter() {
            branch.remote.hash(hasher);
            branch.name.hash(hasher);
        }
    }
}

fn branch_sidebar_remote_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    hash_branch_sidebar_remote_source(repo, &mut hasher);
    hasher.finish()
}

fn branch_sidebar_remote_reuse_key(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    branch_sidebar_head_reuse_key(repo).hash(&mut hasher);
    fingerprint::hash_loadable_arc(&repo.branches, &mut hasher);
    fingerprint::hash_loadable_arc(&repo.remotes, &mut hasher);
    fingerprint::hash_loadable_arc(&repo.remote_branches, &mut hasher);
    hasher.finish()
}

fn hash_branch_sidebar_worktree_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    repo.spec.workdir.hash(hasher);
    fingerprint::hash_loadable_kind(&repo.worktrees, hasher);
    if let Loadable::Ready(worktrees) = &repo.worktrees {
        for worktree in worktrees.iter() {
            worktree.path.hash(hasher);
            worktree.branch.hash(hasher);
            worktree.detached.hash(hasher);
        }
    }
}

fn branch_sidebar_worktree_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    hash_branch_sidebar_worktree_source(repo, &mut hasher);
    hasher.finish()
}

fn hash_branch_sidebar_submodule_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    fingerprint::hash_loadable_kind(&repo.submodules, hasher);
    if let Loadable::Ready(submodules) = &repo.submodules {
        for submodule in submodules.iter() {
            submodule.path.hash(hasher);
        }
    }
}

fn branch_sidebar_submodule_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    hash_branch_sidebar_submodule_source(repo, &mut hasher);
    hasher.finish()
}

fn hash_branch_sidebar_stash_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    fingerprint::hash_loadable_kind(&repo.stashes, hasher);
    if let Loadable::Ready(stashes) = &repo.stashes {
        for stash in stashes.iter() {
            stash.index.hash(hasher);
            stash.message.hash(hasher);
            stash.created_at.hash(hasher);
        }
    }
}

fn branch_sidebar_stash_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    hash_branch_sidebar_stash_source(repo, &mut hasher);
    hasher.finish()
}

fn cmp_ascii_case_insensitive(left: &[u8], right: &[u8]) -> Ordering {
    for (&left, &right) in left.iter().zip(right.iter()) {
        let ordering = left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase());
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left.len().cmp(&right.len())
}

fn cmp_case_insensitive_then_case_sensitive(left: &str, right: &str) -> Ordering {
    let ordering = if left.is_ascii() && right.is_ascii() {
        cmp_ascii_case_insensitive(left.as_bytes(), right.as_bytes())
    } else {
        left.chars()
            .flat_map(char::to_lowercase)
            .cmp(right.chars().flat_map(char::to_lowercase))
    };

    ordering.then_with(|| left.cmp(right))
}

fn branch_sidebar_depth(depth: usize) -> BranchSidebarDepth {
    u16::try_from(depth).unwrap_or(u16::MAX)
}

fn branch_sidebar_divergence_count(count: usize) -> Option<NonZeroU32> {
    if count == 0 {
        None
    } else {
        Some(NonZeroU32::new(u32::try_from(count).unwrap_or(u32::MAX)).unwrap())
    }
}

fn defaults_to_collapsed(collapse_key: &str) -> bool {
    matches!(
        collapse_key,
        WORKTREES_SECTION_KEY | SUBMODULES_SECTION_KEY | STASH_SECTION_KEY
    )
}

pub(super) fn expanded_default_section_storage_key(collapse_key: &str) -> Option<String> {
    defaults_to_collapsed(collapse_key)
        .then(|| format!("{EXPANDED_DEFAULT_SECTION_PREFIX}{collapse_key}"))
}

pub(super) fn is_collapsed(collapsed_items: &BTreeSet<String>, collapse_key: &str) -> bool {
    if collapsed_items.is_empty() {
        return defaults_to_collapsed(collapse_key);
    }

    if let Some(expanded_key) = expanded_default_section_storage_key(collapse_key) {
        return !collapsed_items.contains(expanded_key.as_str());
    }

    collapsed_items.contains(collapse_key)
}

pub(super) fn toggle_collapse_state(collapsed_items: &mut BTreeSet<String>, collapse_key: &str) {
    if let Some(expanded_key) = expanded_default_section_storage_key(collapse_key) {
        if !collapsed_items.insert(expanded_key.clone()) {
            collapsed_items.remove(expanded_key.as_str());
        }
        collapsed_items.remove(collapse_key);
        return;
    }

    if !collapsed_items.insert(collapse_key.to_string()) {
        collapsed_items.remove(collapse_key);
    }
}

pub(super) fn branch_sidebar_rows(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
) -> Vec<BranchSidebarRow> {
    let head = match &repo.head_branch {
        Loadable::Ready(head) => Some(head.as_str()),
        _ => None,
    };
    let local_collapsed = is_collapsed(collapsed_items, local_section_storage_key());
    let remote_collapsed = is_collapsed(collapsed_items, remote_section_storage_key());
    let worktrees_collapsed = is_collapsed(collapsed_items, worktrees_section_storage_key());
    let submodules_collapsed = is_collapsed(collapsed_items, submodules_section_storage_key());
    let stash_collapsed = is_collapsed(collapsed_items, stash_section_storage_key());
    let visible_rows = if local_collapsed {
        0
    } else {
        match &repo.branches {
            Loadable::Ready(branches) => branches.len(),
            _ => 0,
        }
    } + if remote_collapsed {
        0
    } else {
        match &repo.remote_branches {
            Loadable::Ready(branches) => branches.len(),
            _ => 0,
        }
    } + if worktrees_collapsed {
        0
    } else {
        match &repo.worktrees {
            Loadable::Ready(worktrees) => worktrees.len(),
            _ => 0,
        }
    } + if submodules_collapsed {
        0
    } else {
        match &repo.submodules {
            Loadable::Ready(submodules) => submodules.len(),
            _ => 0,
        }
    } + if stash_collapsed {
        0
    } else {
        match &repo.stashes {
            Loadable::Ready(stashes) => stashes.len(),
            _ => 0,
        }
    };
    let approx_rows = 16 + visible_rows + visible_rows / 8;
    let mut rows = Vec::with_capacity(approx_rows);
    let mut head_upstream_full = None;
    let mut local_upstreams: SmallVec<[(&str, &str); 4]> = SmallVec::new();

    if local_collapsed && let Loadable::Ready(branches) = &repo.branches {
        for branch in branches.iter() {
            record_local_branch_sidebar_metadata(
                branch,
                head,
                &mut local_upstreams,
                &mut head_upstream_full,
            );
        }
    }

    rows.push(BranchSidebarRow::SectionHeader {
        section: BranchSection::Local,
        top_border: false,
        collapsed: local_collapsed,
        collapse_key: local_section_storage_key().into(),
    });

    if !local_collapsed {
        match &repo.branches {
            Loadable::Ready(branches) if branches.is_empty() => {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Local,
                    message: "No branches".into(),
                });
            }
            Loadable::Ready(branches) => {
                let mut tree = SlashTree::default();
                let mut local_leaf_meta = Vec::with_capacity(branches.len());
                for branch in branches.iter() {
                    record_local_branch_sidebar_metadata(
                        branch,
                        head,
                        &mut local_upstreams,
                        &mut head_upstream_full,
                    );
                    local_leaf_meta.push(SlashTreeLeafMeta {
                        divergence: branch.divergence,
                        is_head: head.is_some_and(|current| current == branch.name.as_str()),
                    });
                    let leaf_meta_index = NonZeroU32::new(
                        u32::try_from(local_leaf_meta.len())
                            .expect("branch sidebar local leaf meta index overflow"),
                    )
                    .expect("branch sidebar local leaf meta index must be non-zero");
                    tree.insert_local(branch.name.as_str(), leaf_meta_index);
                }

                let mut name_prefix = String::new();
                let mut group_path_prefix = String::new();
                push_slash_tree_rows(
                    &tree,
                    &mut rows,
                    Some(local_leaf_meta.as_slice()),
                    head_upstream_full.as_deref(),
                    0,
                    false,
                    BranchSection::Local,
                    &mut name_prefix,
                    &mut group_path_prefix,
                    None,
                    collapsed_items,
                );
            }
            Loadable::Loading => rows.push(BranchSidebarRow::Placeholder {
                section: BranchSection::Local,
                message: "Loading".into(),
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::Placeholder {
                section: BranchSection::Local,
                message: "Not loaded".into(),
            }),
            Loadable::Error(error) => rows.push(BranchSidebarRow::Placeholder {
                section: BranchSection::Local,
                message: error.clone().into(),
            }),
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    rows.push(BranchSidebarRow::SectionHeader {
        section: BranchSection::Remote,
        top_border: true,
        collapsed: remote_collapsed,
        collapse_key: remote_section_storage_key().into(),
    });

    if !remote_collapsed {
        let known_remote_count = match &repo.remotes {
            Loadable::Ready(remotes) => remotes.len(),
            _ => 0,
        };
        let mut remotes = Vec::with_capacity(known_remote_count.max(1));
        let mut remote_indexes =
            FxHashMap::with_capacity_and_hasher(known_remote_count.max(1), Default::default());
        let mut remote_names_need_sort = false;
        let mut remote_section_is_loading_or_error = false;
        match &repo.remote_branches {
            Loadable::Ready(branches) => {
                for branch in branches.iter() {
                    let inserted = push_remote_group_branch(
                        &mut remotes,
                        &mut remote_indexes,
                        branch.remote.as_str(),
                        branch.name.as_str(),
                    );
                    if inserted {
                        remote_names_need_sort |=
                            slash_tree_label_needs_sort(branch.remote.as_str());
                    }
                }
            }
            Loadable::Loading => {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Remote,
                    message: "Loading".into(),
                });
                remote_section_is_loading_or_error = true;
            }
            Loadable::Error(error) => {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Remote,
                    message: error.clone().into(),
                });
                remote_section_is_loading_or_error = true;
            }
            Loadable::NotLoaded => {}
        }

        if !remote_section_is_loading_or_error {
            for (remote, branch) in local_upstreams.iter().copied() {
                if push_remote_group_branch(&mut remotes, &mut remote_indexes, remote, branch) {
                    remote_names_need_sort |= slash_tree_label_needs_sort(remote);
                }
            }

            if let Loadable::Ready(known) = &repo.remotes {
                for remote in known.iter() {
                    remote_names_need_sort |= slash_tree_label_needs_sort(remote.name.as_str());
                    ensure_remote_group(&mut remotes, &mut remote_indexes, remote.name.as_str());
                }
            }

            if remotes.is_empty() {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Remote,
                    message: "No remotes".into(),
                });
            } else {
                if remote_names_need_sort {
                    remotes.sort_unstable_by(|left, right| {
                        cmp_case_insensitive_then_case_sensitive(left.name, right.name)
                    });
                } else {
                    remotes.sort_unstable_by(|left, right| left.name.cmp(right.name));
                }

                for remote in remotes {
                    push_remote_branch_sidebar_rows(
                        remote.name,
                        remote.branches,
                        &mut rows,
                        head_upstream_full.as_deref(),
                        collapsed_items,
                    );
                }
            }
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    rows.push(BranchSidebarRow::WorktreesHeader {
        top_border: true,
        collapsed: worktrees_collapsed,
        collapse_key: worktrees_section_storage_key().into(),
    });

    if !worktrees_collapsed {
        match &repo.worktrees {
            Loadable::Ready(worktrees) => {
                let mut any = false;
                for worktree in worktrees.iter() {
                    any = true;
                    rows.push(BranchSidebarRow::WorktreeItem {
                        path: worktree.path.clone(),
                        branch: worktree
                            .branch
                            .as_ref()
                            .map(|branch| SharedString::new(branch.as_str())),
                        detached: worktree.detached,
                        is_active: worktree.path == repo.spec.workdir,
                    });
                }
                if !any {
                    rows.push(BranchSidebarRow::WorktreePlaceholder {
                        message: "No worktrees".into(),
                    });
                }
            }
            Loadable::Loading => rows.push(BranchSidebarRow::WorktreePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::WorktreePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::Error(error) => rows.push(BranchSidebarRow::WorktreePlaceholder {
                message: error.clone().into(),
            }),
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    rows.push(BranchSidebarRow::SubmodulesHeader {
        top_border: true,
        collapsed: submodules_collapsed,
        collapse_key: submodules_section_storage_key().into(),
    });

    if !submodules_collapsed {
        match &repo.submodules {
            Loadable::Ready(submodules) if submodules.is_empty() => {
                rows.push(BranchSidebarRow::SubmodulePlaceholder {
                    message: "No submodules".into(),
                });
            }
            Loadable::Ready(submodules) => {
                for submodule in submodules.iter() {
                    rows.push(BranchSidebarRow::SubmoduleItem {
                        path: submodule.path.clone(),
                    });
                }
            }
            Loadable::Loading => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::Error(error) => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: error.clone().into(),
            }),
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    rows.push(BranchSidebarRow::StashHeader {
        top_border: true,
        collapsed: stash_collapsed,
        collapse_key: stash_section_storage_key().into(),
    });

    if !stash_collapsed {
        match &repo.stashes {
            Loadable::Ready(stashes) if stashes.is_empty() => {
                rows.push(BranchSidebarRow::StashPlaceholder {
                    message: "No stashes".into(),
                });
            }
            Loadable::Ready(stashes) => {
                for stash in stashes.iter() {
                    let message: SharedString = stash.message.clone().into();
                    let tooltip: SharedString = if stash.message.is_empty() {
                        "Stash".into()
                    } else {
                        message.clone()
                    };
                    rows.push(BranchSidebarRow::StashItem {
                        index: stash.index,
                        message,
                        tooltip,
                        created_at: stash.created_at,
                    });
                }
            }
            Loadable::Loading => rows.push(BranchSidebarRow::StashPlaceholder {
                message: "Loading".into(),
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::StashPlaceholder {
                message: "Loading".into(),
            }),
            Loadable::Error(error) => rows.push(BranchSidebarRow::StashPlaceholder {
                message: error.clone().into(),
            }),
        }
    }

    for _ in 0..TRAILING_BOTTOM_SPACERS {
        rows.push(BranchSidebarRow::SectionSpacer);
    }

    rows
}

#[allow(clippy::too_many_arguments)]
fn push_slash_tree_rows(
    tree: &SlashTree<'_>,
    out: &mut Vec<BranchSidebarRow>,
    local_leaf_meta: Option<&[SlashTreeLeafMeta]>,
    upstream_full: Option<&str>,
    depth: usize,
    muted: bool,
    section: BranchSection,
    name_prefix: &mut String,
    group_path_prefix: &mut String,
    remote_name: Option<&str>,
    collapsed_items: &BTreeSet<String>,
) {
    if let Some((label, node)) = tree.children.first_key_value()
        && tree.children.len() == 1
    {
        push_slash_tree_child_rows(
            label,
            node,
            out,
            local_leaf_meta,
            upstream_full,
            depth,
            muted,
            section,
            name_prefix,
            group_path_prefix,
            remote_name,
            collapsed_items,
        );
        return;
    }

    let mut has_group = false;
    let mut has_leaf = false;
    let mut needs_sort = false;
    for (label, node) in tree.children.iter() {
        has_group |= !node.children.is_empty();
        has_leaf |= node.children.is_empty();
        needs_sort |= slash_tree_label_needs_sort(label);
    }

    if !needs_sort {
        if has_group && has_leaf {
            for (label, node) in tree.children.iter() {
                if node.children.is_empty() {
                    continue;
                }
                push_slash_tree_child_rows(
                    label,
                    node,
                    out,
                    local_leaf_meta,
                    upstream_full,
                    depth,
                    muted,
                    section,
                    name_prefix,
                    group_path_prefix,
                    remote_name,
                    collapsed_items,
                );
            }
            for (label, node) in tree.children.iter() {
                if !node.children.is_empty() {
                    continue;
                }
                push_slash_tree_child_rows(
                    label,
                    node,
                    out,
                    local_leaf_meta,
                    upstream_full,
                    depth,
                    muted,
                    section,
                    name_prefix,
                    group_path_prefix,
                    remote_name,
                    collapsed_items,
                );
            }
        } else {
            for (label, node) in tree.children.iter() {
                push_slash_tree_child_rows(
                    label,
                    node,
                    out,
                    local_leaf_meta,
                    upstream_full,
                    depth,
                    muted,
                    section,
                    name_prefix,
                    group_path_prefix,
                    remote_name,
                    collapsed_items,
                );
            }
        }
        return;
    }

    let mut children: SmallVec<[(&str, &SlashTree<'_>); 8]> = tree
        .children
        .iter()
        .map(|(label, node)| (*label, node))
        .collect();
    children.sort_unstable_by(|(left_label, left_node), (right_label, right_node)| {
        let left_is_group = !left_node.children.is_empty();
        let right_is_group = !right_node.children.is_empty();
        right_is_group
            .cmp(&left_is_group)
            .then_with(|| cmp_case_insensitive_then_case_sensitive(left_label, right_label))
    });
    for (label, node) in children {
        push_slash_tree_child_rows(
            label,
            node,
            out,
            local_leaf_meta,
            upstream_full,
            depth,
            muted,
            section,
            name_prefix,
            group_path_prefix,
            remote_name,
            collapsed_items,
        );
    }
}

fn slash_tree_label_needs_sort(label: &str) -> bool {
    !label.is_ascii()
        || label
            .as_bytes()
            .iter()
            .any(|byte| byte.is_ascii_uppercase())
}

fn slash_tree_segments(name: &str) -> SmallVec<[&str; 8]> {
    let bytes = name.as_bytes();
    let mut segment_start = 0;
    let mut segments = SmallVec::new();
    while segment_start < bytes.len() {
        while segment_start < bytes.len() && bytes[segment_start] == b'/' {
            segment_start += 1;
        }
        if segment_start >= bytes.len() {
            break;
        }

        let mut segment_end = segment_start;
        while segment_end < bytes.len() && bytes[segment_end] != b'/' {
            segment_end += 1;
        }
        segments.push(&name[segment_start..segment_end]);
        segment_start = segment_end;
    }
    segments
}

fn remote_branch_linear_chain<'a>(
    branches: &[&'a str],
) -> Option<(SmallVec<[&'a str; 8]>, Vec<&'a str>)> {
    let first = *branches.first()?;
    let first_segments = slash_tree_segments(first);
    if first_segments.len() <= 1 {
        return None;
    }

    let chain_len = first_segments.len() - 1;
    let chain_segments: SmallVec<[&'a str; 8]> =
        first_segments[..chain_len].iter().copied().collect();
    let mut leaf_labels = Vec::with_capacity(branches.len());
    leaf_labels.push(first_segments[chain_len]);

    for branch in branches.iter().copied().skip(1) {
        let leaf = slash_tree_leaf_after_chain(branch, chain_segments.as_slice())?;
        leaf_labels.push(leaf);
    }

    Some((chain_segments, leaf_labels))
}

fn slash_tree_leaf_after_chain<'a>(name: &'a str, chain_segments: &[&str]) -> Option<&'a str> {
    let bytes = name.as_bytes();
    let mut segment_start = 0;

    for expected in chain_segments {
        while segment_start < bytes.len() && bytes[segment_start] == b'/' {
            segment_start += 1;
        }
        if segment_start >= bytes.len() {
            return None;
        }

        let mut segment_end = segment_start;
        while segment_end < bytes.len() && bytes[segment_end] != b'/' {
            segment_end += 1;
        }
        if &name[segment_start..segment_end] != *expected {
            return None;
        }
        segment_start = segment_end;
    }

    while segment_start < bytes.len() && bytes[segment_start] == b'/' {
        segment_start += 1;
    }
    if segment_start >= bytes.len() {
        return None;
    }

    let leaf_start = segment_start;
    while segment_start < bytes.len() && bytes[segment_start] != b'/' {
        segment_start += 1;
    }
    let leaf_end = segment_start;
    while segment_start < bytes.len() {
        if bytes[segment_start] != b'/' {
            return None;
        }
        segment_start += 1;
        if segment_start < bytes.len() && bytes[segment_start] != b'/' {
            return None;
        }
    }

    Some(&name[leaf_start..leaf_end])
}

fn record_local_branch_sidebar_metadata<'a>(
    branch: &'a Branch,
    head: Option<&str>,
    local_upstreams: &mut SmallVec<[(&'a str, &'a str); 4]>,
    head_upstream_full: &mut Option<String>,
) {
    let Some(upstream) = branch.upstream.as_ref() else {
        return;
    };

    local_upstreams.push((upstream.remote.as_str(), upstream.branch.as_str()));
    if head_upstream_full.is_none() && head.is_some_and(|current| current == branch.name.as_str()) {
        let mut full = String::with_capacity(upstream.remote.len() + 1 + upstream.branch.len());
        full.push_str(&upstream.remote);
        full.push('/');
        full.push_str(&upstream.branch);
        *head_upstream_full = Some(full);
    }
}

fn push_remote_group_branch<'a>(
    remotes: &mut Vec<RemoteBranchGroup<'a>>,
    remote_indexes: &mut FxHashMap<&'a str, usize>,
    remote: &'a str,
    branch: &'a str,
) -> bool {
    if let Some(&index) = remote_indexes.get(remote) {
        remotes[index].branches.push(branch);
        return false;
    }

    remote_indexes.insert(remote, remotes.len());
    remotes.push(RemoteBranchGroup {
        name: remote,
        branches: vec![branch],
    });
    true
}

fn ensure_remote_group<'a>(
    remotes: &mut Vec<RemoteBranchGroup<'a>>,
    remote_indexes: &mut FxHashMap<&'a str, usize>,
    remote: &'a str,
) {
    if remote_indexes.contains_key(remote) {
        return;
    }

    remote_indexes.insert(remote, remotes.len());
    remotes.push(RemoteBranchGroup {
        name: remote,
        branches: Vec::new(),
    });
}

fn push_remote_branch_sidebar_rows(
    remote: &str,
    branches: Vec<&str>,
    out: &mut Vec<BranchSidebarRow>,
    upstream_full: Option<&str>,
    collapsed_items: &BTreeSet<String>,
) {
    let remote_collapse_key = remote_header_storage_key(remote);
    let remote_is_collapsed = is_collapsed(collapsed_items, &remote_collapse_key);
    out.push(BranchSidebarRow::RemoteHeader {
        name: SharedString::new(remote),
        collapsed: remote_is_collapsed,
        collapse_key: remote_collapse_key.into(),
    });
    if branches.is_empty() || remote_is_collapsed {
        return;
    }

    if let Some((chain_segments, mut leaf_labels)) = remote_branch_linear_chain(branches.as_slice())
    {
        push_remote_linear_chain_rows(
            remote,
            chain_segments.as_slice(),
            &mut leaf_labels,
            out,
            upstream_full,
            collapsed_items,
        );
        return;
    }

    let mut tree = SlashTree::default();
    // `push_slash_tree_rows()` sorts each fanout level, so sorting the flat
    // branch list here would only duplicate work.
    for branch in branches {
        tree.insert(branch);
    }

    let mut name_prefix = String::with_capacity(remote.len() + 1);
    name_prefix.push_str(remote);
    name_prefix.push('/');
    let mut group_path_prefix = String::new();
    push_slash_tree_rows(
        &tree,
        out,
        None,
        upstream_full,
        1,
        true,
        BranchSection::Remote,
        &mut name_prefix,
        &mut group_path_prefix,
        Some(remote),
        collapsed_items,
    );
}

fn push_remote_linear_chain_rows(
    remote: &str,
    chain_segments: &[&str],
    leaf_labels: &mut Vec<&str>,
    out: &mut Vec<BranchSidebarRow>,
    upstream_full: Option<&str>,
    collapsed_items: &BTreeSet<String>,
) {
    let mut name_prefix = String::with_capacity(
        remote.len()
            + 1
            + chain_segments
                .iter()
                .map(|segment| segment.len() + 1)
                .sum::<usize>(),
    );
    name_prefix.push_str(remote);
    name_prefix.push('/');
    let mut group_path_prefix = String::new();
    let mut depth = 1;

    for segment in chain_segments.iter().copied() {
        group_path_prefix.push_str(segment);
        let collapse_key = remote_group_storage_key(remote, group_path_prefix.as_str());
        let group_collapsed = is_collapsed(collapsed_items, &collapse_key);
        let mut group_label = String::with_capacity(segment.len() + 1);
        group_label.push_str(segment);
        group_label.push('/');
        out.push(BranchSidebarRow::GroupHeader {
            label: group_label.into(),
            section: BranchSection::Remote,
            depth: branch_sidebar_depth(depth),
            collapsed: group_collapsed,
            collapse_key: collapse_key.into(),
        });
        if group_collapsed {
            return;
        }

        name_prefix.push_str(segment);
        name_prefix.push('/');
        group_path_prefix.push('/');
        depth += 1;
    }

    if leaf_labels.len() > 1 {
        if leaf_labels.iter().copied().any(slash_tree_label_needs_sort) {
            leaf_labels.sort_unstable_by(|left, right| {
                cmp_case_insensitive_then_case_sensitive(left, right)
            });
        } else {
            leaf_labels.sort_unstable();
        }
        leaf_labels.dedup();
    }

    for label in leaf_labels.iter().copied() {
        push_branch_sidebar_branch_row(
            out,
            label,
            &mut name_prefix,
            None,
            None,
            upstream_full,
            BranchSection::Remote,
            depth,
            true,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_slash_tree_child_rows(
    label: &str,
    node: &SlashTree<'_>,
    out: &mut Vec<BranchSidebarRow>,
    local_leaf_meta: Option<&[SlashTreeLeafMeta]>,
    upstream_full: Option<&str>,
    depth: usize,
    muted: bool,
    section: BranchSection,
    name_prefix: &mut String,
    group_path_prefix: &mut String,
    remote_name: Option<&str>,
    collapsed_items: &BTreeSet<String>,
) {
    if node.children.is_empty() {
        if node.is_leaf {
            push_branch_sidebar_branch_row(
                out,
                label,
                name_prefix,
                node.leaf_meta_index,
                local_leaf_meta,
                upstream_full,
                section,
                depth,
                muted,
            );
        }
        return;
    }

    let group_path_mark = group_path_prefix.len();
    group_path_prefix.push_str(label);
    let collapse_key = match section {
        BranchSection::Local => local_group_storage_key(group_path_prefix.as_str()),
        BranchSection::Remote => {
            remote_group_storage_key(remote_name.unwrap_or_default(), group_path_prefix.as_str())
        }
    };
    let group_collapsed = is_collapsed(collapsed_items, &collapse_key);
    let mut group_label = String::with_capacity(label.len() + 1);
    group_label.push_str(label);
    group_label.push('/');
    out.push(BranchSidebarRow::GroupHeader {
        label: group_label.into(),
        section,
        depth: branch_sidebar_depth(depth),
        collapsed: group_collapsed,
        collapse_key: collapse_key.into(),
    });
    if group_collapsed {
        group_path_prefix.truncate(group_path_mark);
        return;
    }

    if node.is_leaf {
        push_branch_sidebar_branch_row(
            out,
            label,
            name_prefix,
            node.leaf_meta_index,
            local_leaf_meta,
            upstream_full,
            section,
            depth + 1,
            muted,
        );
    }

    let name_prefix_mark = name_prefix.len();
    name_prefix.push_str(label);
    name_prefix.push('/');
    group_path_prefix.push('/');

    push_slash_tree_rows(
        node,
        out,
        local_leaf_meta,
        upstream_full,
        depth + 1,
        muted,
        section,
        name_prefix,
        group_path_prefix,
        remote_name,
        collapsed_items,
    );

    name_prefix.truncate(name_prefix_mark);
    group_path_prefix.truncate(group_path_mark);
}

#[allow(clippy::too_many_arguments)]
fn push_branch_sidebar_branch_row(
    out: &mut Vec<BranchSidebarRow>,
    label: &str,
    name_prefix: &mut String,
    leaf_meta_index: Option<NonZeroU32>,
    local_leaf_meta: Option<&[SlashTreeLeafMeta]>,
    upstream_full: Option<&str>,
    section: BranchSection,
    depth: usize,
    muted: bool,
) {
    name_prefix.push_str(label);
    let is_upstream = section == BranchSection::Remote
        && upstream_full.is_some_and(|u| u == name_prefix.as_str());
    let leaf_meta = leaf_meta_index
        .and_then(|index| {
            local_leaf_meta.and_then(|meta| meta.get(index.get().saturating_sub(1) as usize))
        })
        .copied()
        .unwrap_or_default();
    let name = SharedString::new(name_prefix.as_str());
    name_prefix.truncate(name_prefix.len() - label.len());
    let divergence_ahead = leaf_meta
        .divergence
        .and_then(|d| branch_sidebar_divergence_count(d.ahead));
    let divergence_behind = leaf_meta
        .divergence
        .and_then(|d| branch_sidebar_divergence_count(d.behind));
    out.push(BranchSidebarRow::Branch {
        name,
        section,
        depth: branch_sidebar_depth(depth),
        muted,
        divergence_ahead,
        divergence_behind,
        is_head: leaf_meta.is_head,
        is_upstream,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{
        Branch, CommitId, FileStatus, FileStatusKind, Remote, RemoteBranch, RepoSpec, RepoStatus,
        StashEntry, Submodule, SubmoduleStatus, Upstream, Worktree,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn commit_id(id: &str) -> CommitId {
        CommitId(id.into())
    }

    fn populated_repo() -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: commit_id("aaaaaaaa"),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            divergence: None,
        }]));
        repo.branches_rev = 1;
        repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: Some("https://example.com/origin.git".to_string()),
        }]));
        repo.remotes_rev = 1;
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: commit_id("aaaaaaaa"),
        }]));
        repo.remote_branches_rev = 1;
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: PathBuf::from("/tmp/repo"),
                head: Some(commit_id("aaaaaaaa")),
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: PathBuf::from("/tmp/repo-linked"),
                head: Some(commit_id("bbbbbbbb")),
                branch: Some("feature/topic".to_string()),
                detached: false,
            },
        ]));
        repo.worktrees_rev = 1;
        repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
            path: PathBuf::from("vendor/lib"),
            head: commit_id("cccccccc"),
            status: SubmoduleStatus::UpToDate,
        }]));
        repo.submodules_rev = 1;
        repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
            index: 0,
            id: commit_id("dddddddd"),
            message: "stash message".into(),
            created_at: None,
        }]));
        repo.stashes_rev = 1;
        repo
    }

    #[test]
    fn source_fingerprint_ignores_status_only_changes() {
        let mut repo = populated_repo();
        let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            staged: vec![],
            unstaged: vec![FileStatus {
                path: PathBuf::from("src/lib.rs"),
                kind: FileStatusKind::Modified,
                conflict: None,
            }],
        }));

        let (after_fingerprint, after_parts) =
            branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

        assert_eq!(after_fingerprint, before_fingerprint);
        assert_eq!(after_parts, before_parts);
    }

    #[test]
    fn source_fingerprint_reuses_unchanged_partitions_when_worktrees_change() {
        let mut repo = populated_repo();
        let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: PathBuf::from("/tmp/repo"),
                head: Some(commit_id("aaaaaaaa")),
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: PathBuf::from("/tmp/repo-linked"),
                head: Some(commit_id("eeeeeeee")),
                branch: None,
                detached: true,
            },
        ]));
        repo.worktrees_rev = repo.worktrees_rev.wrapping_add(1);

        let (after_fingerprint, after_parts) =
            branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

        assert_ne!(after_fingerprint, before_fingerprint);
        assert_eq!(after_parts.local_hash, before_parts.local_hash);
        assert_eq!(after_parts.remote_hash, before_parts.remote_hash);
        assert_ne!(after_parts.worktree_hash, before_parts.worktree_hash);
        assert_eq!(after_parts.submodule_hash, before_parts.submodule_hash);
        assert_eq!(after_parts.stash_hash, before_parts.stash_hash);
    }

    #[test]
    fn source_fingerprint_reuses_branch_partition_hashes_when_revs_bump_without_snapshot_change() {
        let mut repo = populated_repo();
        let (before_fingerprint, before_parts) = branch_sidebar_source_fingerprint(&repo, None);

        repo.branches_rev = repo.branches_rev.wrapping_add(1);

        let (after_fingerprint, after_parts) =
            branch_sidebar_source_fingerprint(&repo, Some(&before_parts));

        assert_eq!(after_fingerprint, before_fingerprint);
        assert_ne!(after_parts.local_revs, before_parts.local_revs);
        assert_ne!(after_parts.remote_revs, before_parts.remote_revs);
        assert_eq!(after_parts.local_hash, before_parts.local_hash);
        assert_eq!(after_parts.remote_hash, before_parts.remote_hash);
        assert_eq!(after_parts.worktree_hash, before_parts.worktree_hash);
        assert_eq!(after_parts.submodule_hash, before_parts.submodule_hash);
        assert_eq!(after_parts.stash_hash, before_parts.stash_hash);
    }

    #[test]
    fn remote_rows_dedup_upstream_branches_that_also_exist_as_remote_refs() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.head_branch = Loadable::Ready("feature".to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "feature".to_string(),
            target: commit_id("aaaaaaaa"),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: "feature".to_string(),
            }),
            divergence: None,
        }]));
        repo.branches_rev = 1;
        repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: Some("https://example.com/origin.git".to_string()),
        }]));
        repo.remotes_rev = 1;
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "feature".to_string(),
            target: commit_id("aaaaaaaa"),
        }]));
        repo.remote_branches_rev = 1;

        let rows = branch_sidebar_rows(&repo, &BTreeSet::new());
        let matches = rows
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    BranchSidebarRow::Branch {
                        section: BranchSection::Remote,
                        name,
                        ..
                    } if name.as_ref() == "origin/feature"
                )
            })
            .count();

        assert_eq!(matches, 1, "remote branch rows should be deduplicated");
    }

    #[test]
    fn worktree_label_handles_branchless_and_detached_states() {
        assert_eq!(
            branch_sidebar_worktree_label(None, false, "/tmp/repo").as_ref(),
            "/tmp/repo"
        );
        assert_eq!(
            branch_sidebar_worktree_label(None, true, "/tmp/repo").as_ref(),
            "(detached)  /tmp/repo"
        );
    }

    #[test]
    fn branch_tooltip_only_appends_upstream_note_when_requested() {
        assert_eq!(
            branch_sidebar_branch_tooltip("origin/main", false).as_ref(),
            "Branch: origin/main"
        );
        assert_eq!(
            branch_sidebar_branch_tooltip("origin/main", true).as_ref(),
            "Branch: origin/main (upstream for current branch)"
        );
    }
}
