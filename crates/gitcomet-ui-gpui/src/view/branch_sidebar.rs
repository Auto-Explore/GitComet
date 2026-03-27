use super::*;
use rustc_hash::FxHasher;
use std::{
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
    format!("group:remote-header:{name}")
}

pub(super) fn local_group_storage_key(path: &str) -> String {
    format!("group:local:{path}")
}

pub(super) fn remote_group_storage_key(remote: &str, path: &str) -> String {
    format!("group:remote:{remote}:{path}")
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

#[derive(Default)]
struct SlashTree {
    is_leaf: bool,
    children: BTreeMap<String, SlashTree>,
}

impl SlashTree {
    fn insert(&mut self, name: &str) {
        let mut node = self;
        for part in name.split('/').filter(|part| !part.is_empty()) {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.is_leaf = true;
    }
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
    remote_revs: (u64, u64, u64, u64),
    remote_hash: u64,
    worktree_rev: u64,
    worktree_hash: u64,
    submodule_rev: u64,
    submodule_hash: u64,
    stash_rev: u64,
    stash_hash: u64,
}

impl BranchSidebarSourceFingerprintParts {
    fn for_repo(repo: &RepoState, reuse: Option<&Self>) -> Self {
        let local_revs = (repo.head_branch_rev, repo.branches_rev);
        let remote_revs = (
            repo.head_branch_rev,
            repo.branches_rev,
            repo.remotes_rev,
            repo.remote_branches_rev,
        );
        let worktree_rev = repo.worktrees_rev;
        let submodule_rev = repo.submodules_rev;
        let stash_rev = repo.stashes_rev;

        Self {
            local_revs,
            local_hash: reuse
                .filter(|parts| parts.local_revs == local_revs)
                .map_or_else(
                    || branch_sidebar_local_source_hash(repo),
                    |parts| parts.local_hash,
                ),
            remote_revs,
            remote_hash: reuse
                .filter(|parts| parts.remote_revs == remote_revs)
                .map_or_else(
                    || branch_sidebar_remote_source_hash(repo),
                    |parts| parts.remote_hash,
                ),
            worktree_rev,
            worktree_hash: reuse
                .filter(|parts| parts.worktree_rev == worktree_rev)
                .map_or_else(
                    || branch_sidebar_worktree_source_hash(repo),
                    |parts| parts.worktree_hash,
                ),
            submodule_rev,
            submodule_hash: reuse
                .filter(|parts| parts.submodule_rev == submodule_rev)
                .map_or_else(
                    || branch_sidebar_submodule_source_hash(repo),
                    |parts| parts.submodule_hash,
                ),
            stash_rev,
            stash_hash: reuse
                .filter(|parts| parts.stash_rev == stash_rev)
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

fn cmp_case_insensitive_then_case_sensitive(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
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
    let approx_rows =
        16 + match &repo.branches {
            Loadable::Ready(branches) => branches.len(),
            _ => 0,
        } + match &repo.remote_branches {
            Loadable::Ready(branches) => branches.len(),
            _ => 0,
        } + match &repo.worktrees {
            Loadable::Ready(worktrees) => worktrees.len(),
            _ => 0,
        } + match &repo.submodules {
            Loadable::Ready(submodules) => submodules.len(),
            _ => 0,
        } + match &repo.stashes {
            Loadable::Ready(stashes) => stashes.len(),
            _ => 0,
        };
    let mut rows = Vec::with_capacity(approx_rows);
    let head_upstream_full = match (&repo.branches, &repo.head_branch) {
        (Loadable::Ready(branches), Loadable::Ready(head)) => branches
            .iter()
            .find(|branch| branch.name == *head)
            .and_then(|branch| branch.upstream.as_ref())
            .map(|upstream| {
                let mut full =
                    String::with_capacity(upstream.remote.len() + 1 + upstream.branch.len());
                full.push_str(&upstream.remote);
                full.push('/');
                full.push_str(&upstream.branch);
                full
            }),
        _ => None,
    };

    let local_collapsed = is_collapsed(collapsed_items, local_section_storage_key());
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
                let head = match &repo.head_branch {
                    Loadable::Ready(head) => Some(head.as_str()),
                    _ => None,
                };
                let mut local_meta: HashMap<String, (Option<UpstreamDivergence>, bool)> =
                    HashMap::default();
                local_meta.reserve(branches.len());
                for branch in branches.iter() {
                    local_meta.insert(
                        branch.name.clone(),
                        (
                            branch.divergence,
                            head.is_some_and(|current| current == branch.name),
                        ),
                    );
                }

                let mut tree = SlashTree::default();
                for branch in branches.iter() {
                    tree.insert(&branch.name);
                }

                let mut name_prefix = String::new();
                let mut group_path_prefix = String::new();
                push_slash_tree_rows(
                    &tree,
                    &mut rows,
                    Some(&local_meta),
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

    let remote_collapsed = is_collapsed(collapsed_items, remote_section_storage_key());
    rows.push(BranchSidebarRow::SectionHeader {
        section: BranchSection::Remote,
        top_border: true,
        collapsed: remote_collapsed,
        collapse_key: remote_section_storage_key().into(),
    });

    if !remote_collapsed {
        let mut remotes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut remote_section_is_loading_or_error = false;
        match &repo.remote_branches {
            Loadable::Ready(branches) => {
                for branch in branches.iter() {
                    remotes
                        .entry(branch.remote.clone())
                        .or_default()
                        .push(branch.name.clone());
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
            if let Loadable::Ready(local_branches) = &repo.branches {
                for local in local_branches.iter() {
                    if let Some(upstream) = &local.upstream {
                        remotes
                            .entry(upstream.remote.clone())
                            .or_default()
                            .push(upstream.branch.clone());
                    }
                }
            }

            if let Loadable::Ready(known) = &repo.remotes {
                for remote in known.iter() {
                    remotes.entry(remote.name.clone()).or_default();
                }
            }

            if remotes.is_empty() {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Remote,
                    message: "No remotes".into(),
                });
            } else {
                let mut remotes = remotes.into_iter().collect::<Vec<_>>();
                remotes.sort_by(|(left, _), (right, _)| {
                    cmp_case_insensitive_then_case_sensitive(left, right)
                });

                for (remote, mut branches) in remotes {
                    branches.sort_by(|left, right| {
                        cmp_case_insensitive_then_case_sensitive(left, right)
                    });
                    branches.dedup();

                    let remote_collapse_key = remote_header_storage_key(&remote);
                    let remote_is_collapsed = is_collapsed(collapsed_items, &remote_collapse_key);
                    rows.push(BranchSidebarRow::RemoteHeader {
                        name: remote.clone().into(),
                        collapsed: remote_is_collapsed,
                        collapse_key: remote_collapse_key.into(),
                    });
                    if branches.is_empty() || remote_is_collapsed {
                        continue;
                    }

                    let mut tree = SlashTree::default();
                    for branch in branches {
                        tree.insert(&branch);
                    }

                    let mut name_prefix = String::with_capacity(remote.len() + 1);
                    name_prefix.push_str(&remote);
                    name_prefix.push('/');
                    let mut group_path_prefix = String::new();
                    push_slash_tree_rows(
                        &tree,
                        &mut rows,
                        None,
                        head_upstream_full.as_deref(),
                        1,
                        true,
                        BranchSection::Remote,
                        &mut name_prefix,
                        &mut group_path_prefix,
                        Some(remote.as_str()),
                        collapsed_items,
                    );
                }
            }
        }
    }

    rows.push(BranchSidebarRow::SectionSpacer);

    let worktrees_collapsed = is_collapsed(collapsed_items, worktrees_section_storage_key());
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

    let submodules_collapsed = is_collapsed(collapsed_items, submodules_section_storage_key());
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

    let stash_collapsed = is_collapsed(collapsed_items, stash_section_storage_key());
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
    tree: &SlashTree,
    out: &mut Vec<BranchSidebarRow>,
    local_meta: Option<&HashMap<String, (Option<UpstreamDivergence>, bool)>>,
    upstream_full: Option<&str>,
    depth: usize,
    muted: bool,
    section: BranchSection,
    name_prefix: &mut String,
    group_path_prefix: &mut String,
    remote_name: Option<&str>,
    collapsed_items: &BTreeSet<String>,
) {
    let mut children = tree.children.iter().collect::<Vec<_>>();
    children.sort_by(|(left_label, left_node), (right_label, right_node)| {
        let left_is_group = !left_node.children.is_empty();
        let right_is_group = !right_node.children.is_empty();
        right_is_group
            .cmp(&left_is_group)
            .then_with(|| cmp_case_insensitive_then_case_sensitive(left_label, right_label))
    });

    for (label, node) in children {
        if node.children.is_empty() {
            if node.is_leaf {
                push_branch_sidebar_branch_row(
                    out,
                    label,
                    name_prefix,
                    local_meta,
                    upstream_full,
                    section,
                    depth,
                    muted,
                );
            }
            continue;
        }

        let group_path_mark = group_path_prefix.len();
        group_path_prefix.push_str(label);
        let collapse_key = match section {
            BranchSection::Local => local_group_storage_key(group_path_prefix.as_str()),
            BranchSection::Remote => remote_group_storage_key(
                remote_name.unwrap_or_default(),
                group_path_prefix.as_str(),
            ),
        };
        let group_collapsed = is_collapsed(collapsed_items, &collapse_key);
        out.push(BranchSidebarRow::GroupHeader {
            label: format!("{label}/").into(),
            section,
            depth: branch_sidebar_depth(depth),
            collapsed: group_collapsed,
            collapse_key: collapse_key.into(),
        });
        if group_collapsed {
            group_path_prefix.truncate(group_path_mark);
            continue;
        }

        if node.is_leaf {
            push_branch_sidebar_branch_row(
                out,
                label,
                name_prefix,
                local_meta,
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
            local_meta,
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
}

#[allow(clippy::too_many_arguments)]
fn push_branch_sidebar_branch_row(
    out: &mut Vec<BranchSidebarRow>,
    label: &str,
    name_prefix: &mut String,
    local_meta: Option<&HashMap<String, (Option<UpstreamDivergence>, bool)>>,
    upstream_full: Option<&str>,
    section: BranchSection,
    depth: usize,
    muted: bool,
) {
    name_prefix.push_str(label);
    let is_upstream = section == BranchSection::Remote
        && upstream_full.is_some_and(|u| u == name_prefix.as_str());
    let (divergence, is_head) = local_meta
        .and_then(|meta| meta.get(name_prefix.as_str()))
        .copied()
        .unwrap_or((None, false));
    let name = SharedString::new(name_prefix.as_str());
    name_prefix.truncate(name_prefix.len() - label.len());
    let divergence_ahead = divergence.and_then(|d| branch_sidebar_divergence_count(d.ahead));
    let divergence_behind = divergence.and_then(|d| branch_sidebar_divergence_count(d.behind));
    out.push(BranchSidebarRow::Branch {
        name,
        section,
        depth: branch_sidebar_depth(depth),
        muted,
        divergence_ahead,
        divergence_behind,
        is_head,
        is_upstream,
    });
}
