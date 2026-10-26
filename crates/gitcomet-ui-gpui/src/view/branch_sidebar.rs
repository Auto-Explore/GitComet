use super::*;
use gitcomet_core::domain::SubmoduleStatus;
use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    num::NonZeroU32,
};

mod group_members;
pub(in crate::view) use group_members::{
    for_each_group_member, group_focus_state, group_focus_targets,
};

const PINNED_LOCAL_SECTION_KEY: &str = "section:pinned/local";
const PINNED_REMOTE_SECTION_KEY: &str = "section:pinned/remote";
const LOCAL_SECTION_KEY: &str = "section:branches/local";
const REMOTE_SECTION_KEY: &str = "section:branches/remote";
const PIN_LOCAL_PREFIX: &str = "local:";
const PIN_REMOTE_PREFIX: &str = "remote:";
const WORKTREES_SECTION_KEY: &str = "section:worktrees";
const SUBMODULES_SECTION_KEY: &str = "section:submodules";
const ANNEX_SECTION_KEY: &str = "section:annex";
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum BranchMenuTarget {
    Local { name: String },
    Remote { remote: String, branch: String },
}

impl BranchMenuTarget {
    pub(super) fn local(name: impl Into<String>) -> Self {
        Self::Local { name: name.into() }
    }

    pub(super) fn remote(remote: impl Into<String>, branch: impl Into<String>) -> Self {
        Self::Remote {
            remote: remote.into(),
            branch: branch.into(),
        }
    }

    pub(super) const fn section(&self) -> BranchSection {
        match self {
            Self::Local { .. } => BranchSection::Local,
            Self::Remote { .. } => BranchSection::Remote,
        }
    }

    pub(super) fn display_name(&self) -> String {
        match self {
            Self::Local { name } => name.clone(),
            Self::Remote { remote, branch } => format!("{remote}/{branch}"),
        }
    }

    pub(super) fn remote_parts(&self) -> Option<(&str, &str)> {
        match self {
            Self::Remote { remote, branch } => Some((remote, branch)),
            Self::Local { .. } => None,
        }
    }
}

type BranchSidebarDepth = u16;

/// Build the persisted key identifying a pinned branch (`local:<name>` or
/// `remote:<remote>/<name>`).
pub(super) fn branch_pin_storage_key(section: BranchSection, name: &str) -> String {
    let prefix = match section {
        BranchSection::Local => PIN_LOCAL_PREFIX,
        BranchSection::Remote => PIN_REMOTE_PREFIX,
    };
    let mut key = String::with_capacity(prefix.len() + name.len());
    key.push_str(prefix);
    key.push_str(name);
    key
}

/// Whether a stored pin key would render a row in `section`.
///
/// The row builder drops a pin whose branch no longer exists and one the branch
/// filter excludes, so anything reporting "how many are pinned" has to ask the
/// same question or it disagrees with what is on screen.
#[cfg(test)]
pub(super) fn pinned_branch_renders(
    repo: &RepoState,
    key: &str,
    section: BranchSection,
    raw_filter: &str,
) -> bool {
    if let Some((key_section, remote, path)) = parse_group_pin_key(key) {
        if key_section != section {
            return false;
        }
        let prefix = format!("{path}/");
        return match section {
            BranchSection::Local => repo.branches.ready().is_some_and(|branches| {
                branches.iter().any(|branch| {
                    branch.name.starts_with(&prefix)
                        && branch_matches_raw_filter(&branch.name, raw_filter)
                })
            }),
            BranchSection::Remote => repo.remote_branches.ready().is_some_and(|branches| {
                branches.iter().any(|branch| {
                    Some(branch.remote.as_str()) == remote
                        && branch.name.starts_with(&prefix)
                        && remote_branch_matches_raw_filter(
                            &branch.remote,
                            &branch.name,
                            raw_filter,
                        )
                })
            }),
        };
    }
    let Some((key_section, name)) = parse_branch_pin_key(key) else {
        return false;
    };
    if key_section != section {
        return false;
    }
    if !branch_matches_raw_filter(name, raw_filter) {
        return false;
    }

    match section {
        BranchSection::Local => matches!(
            &repo.branches,
            Loadable::Ready(branches) if branches.iter().any(|branch| branch.name == name)
        ),
        BranchSection::Remote => {
            let Loadable::Ready(branches) = &repo.remote_branches else {
                return false;
            };
            let mut matching = branches
                .iter()
                .filter(|branch| format!("{}/{}", branch.remote, branch.name) == name);
            matching.next().is_some() && matching.next().is_none()
        }
    }
}

/// Parse a stored pin key back into its section and branch name. Unknown
/// prefixes yield `None` so stale keys are ignored rather than mis-rendered.
fn parse_branch_pin_key(key: &str) -> Option<(BranchSection, &str)> {
    if let Some(name) = key.strip_prefix(PIN_LOCAL_PREFIX) {
        Some((BranchSection::Local, name))
    } else {
        key.strip_prefix(PIN_REMOTE_PREFIX)
            .map(|name| (BranchSection::Remote, name))
    }
}

/// Group pins reuse the canonical group key. The remote/path boundary is
/// unambiguous even for remote names containing slashes (`:` is not a ref-name
/// character). Existing branch pin keys continue to load unchanged.
fn parse_group_pin_key(key: &str) -> Option<(BranchSection, Option<&str>, &str)> {
    if let Some(path) = key.strip_prefix(LOCAL_GROUP_PREFIX) {
        return (!path.is_empty()).then_some((BranchSection::Local, None, path));
    }
    let (remote, path) = key.strip_prefix(REMOTE_GROUP_PREFIX)?.split_once(':')?;
    (!remote.is_empty() && !path.is_empty()).then_some((BranchSection::Remote, Some(remote), path))
}

pub(super) fn pinned_row_storage_key(row: &BranchSidebarRow) -> Option<(BranchSection, String)> {
    match row {
        BranchSidebarRow::Branch {
            section,
            name,
            depth: 0,
            ..
        } => Some((*section, branch_pin_storage_key(*section, name))),
        BranchSidebarRow::GroupHeader {
            section,
            collapse_key,
            depth: 0,
            ..
        } => Some((*section, collapse_key.to_string())),
        _ => None,
    }
}

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

pub(super) const fn annex_section_storage_key() -> &'static str {
    ANNEX_SECTION_KEY
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
    ContributionHeader {
        section: usize,
        title: SharedString,
        collapsed: bool,
        collapse_key: SharedString,
    },
    ContributionItem {
        section: usize,
        row: usize,
        label: SharedString,
        key: SharedString,
    },
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
        hidden_in_history: bool,
    },
    GroupHeader {
        label: SharedString,
        /// The group's full slash path with no trailing separator (`feat`,
        /// `feat/sub`), which `label` alone cannot give: it is only the last
        /// segment. Everything acting on the group's members needs the whole
        /// path to match branch names against.
        path: SharedString,
        /// The owning remote for a `Remote` group, `None` for a local one. Also
        /// absent from `label`, and needed to tell `origin/feat` apart from
        /// `upstream/feat`.
        remote: Option<SharedString>,
        section: BranchSection,
        depth: BranchSidebarDepth,
        collapsed: bool,
        collapse_key: SharedString,
    },
    Branch {
        name: SharedString,
        target: BranchMenuTarget,
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
        can_load: bool,
    },
    SubmoduleItem {
        path: std::path::PathBuf,
        status: SubmoduleStatus,
        recorded_head: CommitId,
        checked_out_head: Option<CommitId>,
    },
    /// git-annex: the repositories and special remotes content can live in.
    AnnexHeader {
        collapsed: bool,
        collapse_key: SharedString,
        /// e.g. "numcopies 2" or "adjusted (unlocked)".
        summary: Option<SharedString>,
    },
    AnnexPlaceholder {
        message: SharedString,
        /// This clone has not run `git annex init` yet.
        can_init: bool,
        /// Files were left stale by an interrupted command; offer a restage.
        can_restage: bool,
    },
    AnnexRepositoryItem {
        uuid: SharedString,
        name: SharedString,
        /// Type and trust, e.g. "directory · untrusted".
        detail: SharedString,
        here: bool,
        untrusted: bool,
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
        id: CommitId,
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

#[cfg(any(test, feature = "benchmarks"))]
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
    annex_refs_hidden: bool,
    annex_rev: u64,
    annex_reuse_identity: fingerprint::LoadableArcIdentity,
    annex_hash: u64,
    local_revs: (u64, u64),
    local_hash: u64,
    local_reuse_key: u64,
    remote_revs: (u64, u64, u64, u64),
    remote_hash: u64,
    remote_reuse_key: u64,
    history_hidden_remotes: BTreeSet<String>,
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
        let annex_refs_hidden = repo.annex_refs_hidden;
        let annex_rev = repo.large_file_support_rev;
        let annex_reuse_identity = fingerprint::loadable_arc_identity(&repo.large_file_support);
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
            annex_refs_hidden,
            annex_rev,
            annex_reuse_identity,
            annex_hash: reuse
                .filter(|parts| {
                    parts.annex_refs_hidden == annex_refs_hidden
                        && parts.annex_rev == annex_rev
                        && parts.annex_reuse_identity == annex_reuse_identity
                })
                .map_or_else(
                    || branch_sidebar_annex_source_hash(repo),
                    |parts| parts.annex_hash,
                ),
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
            history_hidden_remotes: repo.history_hidden_remotes().clone(),
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
        5u8.hash(&mut hasher);
        self.annex_hash.hash(&mut hasher);
        6u8.hash(&mut hasher);
        self.history_hidden_remotes.hash(&mut hasher);
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
    if cached.annex_refs_hidden != repo.annex_refs_hidden
        || cached.annex_rev != repo.large_file_support_rev
        || cached.annex_reuse_identity
            != fingerprint::loadable_arc_identity(&repo.large_file_support)
    {
        return false;
    }
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
    if cached.history_hidden_remotes != *repo.history_hidden_remotes() {
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

fn branch_sidebar_annex_source_hash(repo: &RepoState) -> u64 {
    let mut hasher = FxHasher::default();
    repo.annex_refs_hidden.hash(&mut hasher);
    fingerprint::hash_loadable_kind(&repo.large_file_support, &mut hasher);
    if let Loadable::Ready(support) = &repo.large_file_support {
        support.annex.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_branch_sidebar_submodule_source<H: Hasher>(repo: &RepoState, hasher: &mut H) {
    fingerprint::hash_loadable_kind(&repo.submodules, hasher);
    if let Loadable::Ready(submodules) = &repo.submodules {
        for submodule in submodules.iter() {
            submodule.path.hash(hasher);
            submodule.recorded_head.hash(hasher);
            submodule.checked_out_head.hash(hasher);
            match submodule.status {
                SubmoduleStatus::UpToDate => 0u8.hash(hasher),
                SubmoduleStatus::NotInitialized => 1u8.hash(hasher),
                SubmoduleStatus::HeadMismatch => 2u8.hash(hasher),
                SubmoduleStatus::MergeConflict => 3u8.hash(hasher),
                SubmoduleStatus::MissingMapping => 4u8.hash(hasher),
                SubmoduleStatus::Unknown(value) => {
                    5u8.hash(hasher);
                    value.hash(hasher);
                }
            }
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
            stash.id.hash(hasher);
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

/// Every group path at or below `path`, derived from the branch names in the
/// section — `path` itself, plus one entry per intermediate directory of each
/// member branch.
///
/// Membership is tested against `"{path}/"`, never the bare `path`, so a
/// sibling group whose name merely starts with the same characters
/// (`features/` against `feat/`) is not swept in.
///
/// The trailing segment of a branch name is the branch, not a group, so it is
/// dropped: `feat/a/b` under `feat` contributes `feat` and `feat/a`.
pub(super) fn group_paths_at_or_below<'a>(
    path: &str,
    branch_names: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // Always present, so the invoked group toggles even when its membership
    // cannot be resolved.
    if !path.is_empty() {
        out.insert(path.to_string());
    }

    let needle = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    for name in branch_names {
        let Some(rest) = name.strip_prefix(needle.as_str()) else {
            continue;
        };
        let mut accumulated = path.to_string();
        let mut segments: Vec<&str> = rest.split('/').collect();
        segments.pop();
        for segment in segments {
            if !accumulated.is_empty() {
                accumulated.push('/');
            }
            accumulated.push_str(segment);
            out.insert(accumulated.clone());
        }
    }
    out
}

/// Drive a collapse key to an explicit state, rather than flipping it.
///
/// Routes through the same `expanded:` inversion `toggle_collapse_state` uses:
/// sections that default to collapsed store the *expanded* key instead, so a
/// naive insert/remove would set the wrong one. Group and pinned keys are not
/// in that set today, but going through here keeps them correct if they ever
/// join it.
pub(super) fn set_collapse_state(
    collapsed_items: &mut BTreeSet<String>,
    collapse_key: &str,
    collapsed: bool,
) {
    if is_collapsed(collapsed_items, collapse_key) == collapsed {
        return;
    }
    toggle_collapse_state(collapsed_items, collapse_key);
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

#[cfg(any(test, feature = "benchmarks"))]
pub(super) fn branch_sidebar_rows(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
    pinned_branches: &BTreeSet<String>,
    branch_filter: &str,
) -> Vec<BranchSidebarRow> {
    let mut rows = pinned_rows(
        repo,
        pinned_branches,
        &branch_filter.trim().to_ascii_lowercase(),
        Some(collapsed_items),
    );
    rows.extend(sidebar_rows(repo, collapsed_items, branch_filter, false));
    rows
}

/// Base tree with open top-level sections. The shared presentation adds pins,
/// search results and an optional section scope for a minimized flyout.
pub(super) fn expanded_sidebar_rows(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
    branch_filter: &str,
) -> Vec<BranchSidebarRow> {
    sidebar_rows(repo, collapsed_items, branch_filter, true)
}

pub(super) fn is_top_level_collapse_key(key: &str) -> bool {
    key.starts_with("section:extension:")
        || matches!(
            key.strip_prefix(EXPANDED_DEFAULT_SECTION_PREFIX)
                .unwrap_or(key),
            PINNED_LOCAL_SECTION_KEY
                | PINNED_REMOTE_SECTION_KEY
                | LOCAL_SECTION_KEY
                | REMOTE_SECTION_KEY
                | WORKTREES_SECTION_KEY
                | SUBMODULES_SECTION_KEY
                | ANNEX_SECTION_KEY
                | STASH_SECTION_KEY
        )
}

fn sidebar_rows(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
    branch_filter: &str,
    always_expanded: bool,
) -> Vec<BranchSidebarRow> {
    let head = match &repo.head_branch {
        Loadable::Ready(head) => Some(head.as_str()),
        _ => None,
    };
    // The branch filter narrows only the Local/Remote (and pinned) branch
    // sections; a live query force-expands them so matches are always visible.
    let filter = branch_filter.trim().to_ascii_lowercase();
    let filtering = !filter.is_empty();
    let local_collapsed = !always_expanded
        && !filtering
        && is_collapsed(collapsed_items, local_section_storage_key());
    let remote_collapsed = !always_expanded
        && !filtering
        && is_collapsed(collapsed_items, remote_section_storage_key());
    let worktrees_collapsed =
        !always_expanded && is_collapsed(collapsed_items, worktrees_section_storage_key());
    let submodules_collapsed =
        !always_expanded && is_collapsed(collapsed_items, submodules_section_storage_key());
    let stash_collapsed =
        !always_expanded && is_collapsed(collapsed_items, stash_section_storage_key());
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

    if local_collapsed && let Loadable::Ready(branches) = &repo.branches {
        for branch in branches.iter() {
            record_local_branch_sidebar_metadata(branch, head, &mut head_upstream_full);
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
                    // Record the checked-out branch's upstream even when the
                    // filter hides its local row, so a real matching remote row
                    // retains its upstream tint.
                    record_local_branch_sidebar_metadata(branch, head, &mut head_upstream_full);
                    if !matches_branch_filter(&branch.name, &filter) {
                        continue;
                    }
                    if repo.annex_refs_hidden
                        && head != Some(branch.name.as_str())
                        && gitcomet_core::annex::is_annex_ref(&branch.name)
                    {
                        continue;
                    }
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
                    if !matches_remote_branch_filter(
                        branch.remote.as_str(),
                        branch.name.as_str(),
                        &filter,
                    ) {
                        continue;
                    }
                    if repo.annex_refs_hidden && gitcomet_core::annex::is_annex_ref(&branch.name) {
                        continue;
                    }
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
            // Empty remote groups (and the "No remotes" hint) only make sense in
            // the unfiltered view; while filtering, a group is shown only if it
            // has a matching branch.
            if !filtering && let Loadable::Ready(known) = &repo.remotes {
                for remote in known.iter() {
                    remote_names_need_sort |= slash_tree_label_needs_sort(remote.name.as_str());
                    ensure_remote_group(&mut remotes, &mut remote_indexes, remote.name.as_str());
                }
            }

            if remotes.is_empty() {
                if !filtering {
                    rows.push(BranchSidebarRow::Placeholder {
                        section: BranchSection::Remote,
                        message: "No remotes".into(),
                    });
                }
            } else {
                if remote_names_need_sort {
                    remotes.sort_unstable_by(|left, right| {
                        cmp_case_insensitive_then_case_sensitive(left.name, right.name)
                    });
                } else {
                    remotes.sort_unstable_by(|left, right| left.name.cmp(right.name));
                }

                let hidden_in_history = repo.history_hidden_remotes();
                for remote in remotes {
                    push_remote_branch_sidebar_rows(
                        remote.name,
                        remote.branches,
                        &mut rows,
                        head_upstream_full.as_deref(),
                        collapsed_items,
                        hidden_in_history.contains(remote.name),
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
            Loadable::Loading | Loadable::NotLoaded => {
                rows.push(BranchSidebarRow::WorktreePlaceholder {
                    message: "Loading".into(),
                })
            }
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
                    can_load: false,
                });
            }
            Loadable::Ready(submodules) => {
                for submodule in submodules.iter() {
                    rows.push(BranchSidebarRow::SubmoduleItem {
                        path: submodule.path.clone(),
                        status: submodule.status,
                        recorded_head: submodule.recorded_head.clone(),
                        checked_out_head: submodule.checked_out_head.clone(),
                    });
                }
            }
            Loadable::Loading => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Loading".into(),
                can_load: false,
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Not loaded".into(),
                can_load: true,
            }),
            Loadable::Error(error) => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: error.clone().into(),
                can_load: true,
            }),
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    push_annex_section_rows(repo, collapsed_items, always_expanded, &mut rows);

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
                        id: stash.id.clone(),
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

fn record_local_branch_sidebar_metadata(
    branch: &Branch,
    head: Option<&str>,
    head_upstream_full: &mut Option<String>,
) {
    let Some(upstream) = branch.upstream.as_ref() else {
        return;
    };

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

/// Only repositories that use git-annex get the section.
fn push_annex_section_rows(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
    always_expanded: bool,
    rows: &mut Vec<BranchSidebarRow>,
) {
    let Loadable::Ready(support) = &repo.large_file_support else {
        return;
    };
    let annex = &support.annex;
    if !annex.in_use() {
        return;
    }
    let collapsed = !always_expanded && is_collapsed(collapsed_items, annex_section_storage_key());
    let summary = match (repo.annex_adjusted_branch(), annex.numcopies) {
        (Some((_, mode)), _) => Some(format!("adjusted ({mode})").into()),
        (None, Some(copies)) => Some(format!("numcopies {copies}").into()),
        (None, None) => None,
    };
    rows.push(BranchSidebarRow::AnnexHeader {
        collapsed,
        collapse_key: annex_section_storage_key().into(),
        summary,
    });
    if collapsed {
        rows.push(BranchSidebarRow::SectionSpacer);
        return;
    }
    if !annex.initialized() {
        rows.push(BranchSidebarRow::AnnexPlaceholder {
            message: "Not initialized in this clone".into(),
            can_init: true,
            can_restage: false,
        });
    } else {
        if annex.restage_pending {
            rows.push(BranchSidebarRow::AnnexPlaceholder {
                message: "Some annexed files need a refresh".into(),
                can_init: false,
                can_restage: true,
            });
        }
        let visible: Vec<_> = annex
            .repositories
            .iter()
            .filter(|repository| !repository.is_builtin())
            .collect();
        if visible.is_empty() {
            rows.push(BranchSidebarRow::AnnexPlaceholder {
                message: "Repositories unknown (is git-annex installed?)".into(),
                can_init: false,
                can_restage: false,
            });
        }
        for repository in visible {
            let kind = repository
                .special_type
                .as_deref()
                .unwrap_or(if repository.here {
                    "this clone"
                } else {
                    "git repository"
                });
            rows.push(BranchSidebarRow::AnnexRepositoryItem {
                uuid: repository.uuid.clone().into(),
                name: repository.display_description().into(),
                detail: format!("{kind} · {}", repository.trust.label()).into(),
                here: repository.here,
                untrusted: repository.trust == gitcomet_core::large_files::AnnexTrust::Untrusted,
            });
        }
    }
    rows.push(BranchSidebarRow::SectionSpacer);
}

fn push_remote_branch_sidebar_rows(
    remote: &str,
    branches: Vec<&str>,
    out: &mut Vec<BranchSidebarRow>,
    upstream_full: Option<&str>,
    collapsed_items: &BTreeSet<String>,
    hidden_in_history: bool,
) {
    let remote_collapse_key = remote_header_storage_key(remote);
    let remote_is_collapsed = is_collapsed(collapsed_items, &remote_collapse_key);
    out.push(BranchSidebarRow::RemoteHeader {
        name: SharedString::new(remote),
        collapsed: remote_is_collapsed,
        collapse_key: remote_collapse_key.into(),
        hidden_in_history,
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
            path: group_path_prefix.as_str().into(),
            remote: Some(remote.into()),
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
            Some(remote),
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
                remote_name,
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
        path: group_path_prefix.as_str().into(),
        remote: remote_name.map(SharedString::from),
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
            remote_name,
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

/// Case-insensitive substring match of a branch name against the sidebar filter.
/// `filter` is expected to be already trimmed and lowercased; an empty filter
/// matches everything.
fn matches_branch_filter(name: &str, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    name.to_ascii_lowercase().contains(filter)
}

/// [`matches_branch_filter`] for callers holding a raw, un-normalised query.
///
/// The row builder lowercases and trims once up front; menus acting on the same
/// rows get here instead of repeating that normalisation and drifting from it.
#[cfg(test)]
pub(super) fn branch_matches_raw_filter(name: &str, filter: &str) -> bool {
    matches_branch_filter(name, &filter.trim().to_ascii_lowercase())
}

/// [`matches_remote_branch_filter`] for a raw query, matching against the full
/// `remote/name` form the tree filters on.
#[cfg(test)]
pub(super) fn remote_branch_matches_raw_filter(remote: &str, branch: &str, filter: &str) -> bool {
    matches_remote_branch_filter(remote, branch, &filter.trim().to_ascii_lowercase())
}

/// Matches a remote branch against the filter using its full `remote/name` form,
/// so a query can hit either the remote or the branch portion.
fn matches_remote_branch_filter(remote: &str, branch: &str, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let full = format!("{remote}/{branch}");
    full.to_ascii_lowercase().contains(filter)
}

/// Unfiltered pins in local-then-remote order. Missing or ambiguous refs are
/// skipped, matching the section-scoped pin lists in rail popovers.
pub(super) fn expanded_pinned_rows(
    repo: &RepoState,
    pins: &BTreeSet<String>,
    collapsed_items: &BTreeSet<String>,
) -> Vec<BranchSidebarRow> {
    pinned_rows(repo, pins, "", Some(collapsed_items))
}

/// Visible explicit roots under the same matcher used by the sidebar. Expand
/// groups before filtering, so a matching descendant keeps its root available.
pub(super) fn matching_pinned_roots(
    repo: &RepoState,
    pins: &BTreeSet<String>,
    search: &super::sidebar_search::SidebarSearch,
) -> Vec<BranchSidebarRow> {
    search
        .project(&expanded_pinned_rows(repo, pins, &BTreeSet::new()))
        .into_iter()
        .filter(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { depth: 0, .. }
                    | BranchSidebarRow::GroupHeader { depth: 0, .. }
            )
        })
        .collect()
}

/// Only explicitly pinned roots, for counts and bulk unpin. Descendants do
/// not count as extra pins, and closed groups still participate.
#[cfg(test)]
pub(super) fn pinned_root_rows(
    repo: &RepoState,
    pins: &BTreeSet<String>,
    filter: &str,
) -> Vec<BranchSidebarRow> {
    pinned_rows(repo, pins, &filter.trim().to_ascii_lowercase(), None)
}

fn pinned_rows(
    repo: &RepoState,
    pins: &BTreeSet<String>,
    filter: &str,
    collapsed_items: Option<&BTreeSet<String>>,
) -> Vec<BranchSidebarRow> {
    let head = match &repo.head_branch {
        Loadable::Ready(head) => Some(head.as_str()),
        _ => None,
    };
    let (mut local, remote) = build_pinned_branch_rows(repo, head, pins, filter, collapsed_items);
    local.extend(remote);
    local
}

/// Build separate local and remote pin lists in persisted (sorted) order.
fn build_pinned_branch_rows(
    repo: &RepoState,
    head: Option<&str>,
    pinned_branches: &BTreeSet<String>,
    filter: &str,
    collapsed_items: Option<&BTreeSet<String>>,
) -> (Vec<BranchSidebarRow>, Vec<BranchSidebarRow>) {
    if pinned_branches.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // The upstream of HEAD, used to tint a pinned remote branch as "Upstream"
    // exactly as the Remote section does.
    let head_upstream_full: Option<String> =
        head.and_then(|head| match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|branch| branch.name == head)
                .and_then(|branch| {
                    branch
                        .upstream
                        .as_ref()
                        .map(|upstream| format!("{}/{}", upstream.remote, upstream.branch))
                }),
            _ => None,
        });

    // Resolve all pins in linear passes rather than scanning every branch for
    // each pin. Remote spellings can be ambiguous in legacy persisted keys.
    let local_index: FxHashMap<&str, _> = match &repo.branches {
        Loadable::Ready(branches) => branches.iter().map(|b| (b.name.as_str(), b)).collect(),
        _ => FxHashMap::default(),
    };
    let mut remote_index = FxHashMap::default();
    if let Loadable::Ready(branches) = &repo.remote_branches {
        for branch in branches.iter() {
            remote_index
                .entry(format!("{}/{}", branch.remote, branch.name))
                .and_modify(|value| *value = None)
                .or_insert(Some(branch));
        }
    }
    let mut local_rows: Vec<BranchSidebarRow> = Vec::new();
    let mut remote_rows: Vec<BranchSidebarRow> = Vec::new();

    // Build each pinned section/remote's tree once, then look up pinned
    // subtrees by path. No per-pin scans of the repository and no flattened
    // copies of unrelated worktree/stash rows. Presentation caching keeps
    // this work out of scrolling and repainting.
    let mut local_tree = SlashTree::default();
    let mut local_meta = Vec::new();
    if pinned_branches
        .iter()
        .any(|key| key.starts_with(LOCAL_GROUP_PREFIX))
    {
        for branch in local_index.values() {
            if !matches_branch_filter(&branch.name, filter) {
                continue;
            }
            local_meta.push(SlashTreeLeafMeta {
                divergence: branch.divergence,
                is_head: head == Some(branch.name.as_str()),
            });
            local_tree.insert_local(
                &branch.name,
                NonZeroU32::new(
                    u32::try_from(local_meta.len()).expect("branch pin metadata index overflow"),
                )
                .unwrap(),
            );
        }
    }
    let mut remote_trees: FxHashMap<&str, SlashTree<'_>> = pinned_branches
        .iter()
        .filter_map(|key| parse_group_pin_key(key).and_then(|(_, remote, _)| remote))
        .map(|remote| (remote, SlashTree::default()))
        .collect();
    if !remote_trees.is_empty()
        && let Loadable::Ready(branches) = &repo.remote_branches
    {
        for branch in branches.iter() {
            if let Some(tree) = remote_trees.get_mut(branch.remote.as_str())
                && matches_remote_branch_filter(&branch.remote, &branch.name, filter)
            {
                tree.insert(&branch.name);
            }
        }
    }

    for key in pinned_branches.iter() {
        if let Some((section, remote, path)) = parse_group_pin_key(key) {
            let (tree, out, meta) = match section {
                BranchSection::Local => (&local_tree, &mut local_rows, Some(local_meta.as_slice())),
                BranchSection::Remote => {
                    let Some(tree) = remote.and_then(|remote| remote_trees.get(remote)) else {
                        continue;
                    };
                    (tree, &mut remote_rows, None)
                }
            };
            let Some(node) = path
                .split('/')
                .try_fold(tree, |node, label| node.children.get(label))
            else {
                continue;
            };
            if node.children.is_empty() {
                continue;
            }
            let label: SharedString = match remote {
                Some(remote) => format!("{remote}/{path}/").into(),
                None => format!("{path}/").into(),
            };
            let Some(collapsed_items) = collapsed_items else {
                out.push(BranchSidebarRow::GroupHeader {
                    label,
                    path: path.into(),
                    remote: remote.map(SharedString::from),
                    section,
                    depth: 0,
                    collapsed: false,
                    collapse_key: key.clone().into(),
                });
                continue;
            };
            let (parent, leaf) = path.rsplit_once('/').map_or(("", path), |(parent, leaf)| {
                (&path[..parent.len() + 1], leaf)
            });
            let mut name_prefix = remote.map_or_else(String::new, |remote| format!("{remote}/"));
            name_prefix.push_str(parent);
            let root_ix = out.len();
            push_slash_tree_child_rows(
                leaf,
                node,
                out,
                meta,
                head_upstream_full.as_deref(),
                0,
                section == BranchSection::Remote,
                section,
                &mut name_prefix,
                &mut parent.to_owned(),
                remote,
                collapsed_items,
            );
            if let BranchSidebarRow::GroupHeader {
                label: root_label, ..
            } = &mut out[root_ix]
            {
                *root_label = label;
            }
            continue;
        }
        let Some((section, name)) = parse_branch_pin_key(key) else {
            continue;
        };
        match section {
            BranchSection::Local => {
                if !matches_branch_filter(name, filter) {
                    continue;
                }
                let Some(branch) = local_index.get(name) else {
                    continue;
                };
                local_rows.push(BranchSidebarRow::Branch {
                    name: SharedString::new(name),
                    target: BranchMenuTarget::local(name),
                    section: BranchSection::Local,
                    depth: 0,
                    muted: false,
                    divergence_ahead: branch
                        .divergence
                        .and_then(|d| branch_sidebar_divergence_count(d.ahead)),
                    divergence_behind: branch
                        .divergence
                        .and_then(|d| branch_sidebar_divergence_count(d.behind)),
                    is_head: head == Some(name),
                    is_upstream: false,
                });
            }
            BranchSection::Remote => {
                if !matches_branch_filter(name, filter) {
                    continue;
                }
                let Some(Some(branch)) = remote_index.get(name) else {
                    continue;
                };
                remote_rows.push(BranchSidebarRow::Branch {
                    name: SharedString::new(name),
                    target: BranchMenuTarget::remote(&branch.remote, &branch.name),
                    section: BranchSection::Remote,
                    depth: 0,
                    muted: false,
                    divergence_ahead: None,
                    divergence_behind: None,
                    is_head: false,
                    is_upstream: head_upstream_full.as_deref() == Some(name),
                });
            }
        }
    }

    (local_rows, remote_rows)
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
    remote: Option<&str>,
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
    let full_name = name_prefix.as_str();
    let target = match remote {
        Some(remote) => BranchMenuTarget::remote(
            remote,
            full_name
                .strip_prefix(remote)
                .and_then(|name| name.strip_prefix('/'))
                .unwrap_or(full_name),
        ),
        None => BranchMenuTarget::local(full_name),
    };
    let name = SharedString::new(full_name);
    name_prefix.truncate(name_prefix.len() - label.len());
    let divergence_ahead = leaf_meta
        .divergence
        .and_then(|d| branch_sidebar_divergence_count(d.ahead));
    let divergence_behind = leaf_meta
        .divergence
        .and_then(|d| branch_sidebar_divergence_count(d.behind));
    out.push(BranchSidebarRow::Branch {
        name,
        target,
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
mod tests;
