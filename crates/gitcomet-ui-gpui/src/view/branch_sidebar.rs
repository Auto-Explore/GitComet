use super::*;
use std::collections::BTreeSet;

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
        depth: usize,
        collapsed: bool,
        collapse_key: SharedString,
    },
    Branch {
        label: SharedString,
        name: SharedString,
        section: BranchSection,
        depth: usize,
        muted: bool,
        divergence: Option<UpstreamDivergence>,
        divergence_ahead: Option<SharedString>,
        divergence_behind: Option<SharedString>,
        tooltip: SharedString,
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
        label: SharedString,
        tooltip: SharedString,
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
        label: SharedString,
        tooltip: SharedString,
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
        for part in name.split('/').filter(|p| !p.is_empty()) {
            node = node.children.entry(part.to_string()).or_default();
        }
        node.is_leaf = true;
    }
}

fn cmp_case_insensitive_then_case_sensitive(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
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
            .find(|b| b.name == *head)
            .and_then(|b| b.upstream.as_ref())
            .map(|u| format!("{}/{}", u.remote, u.branch)),
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
                    Loadable::Ready(h) => Some(h.as_str()),
                    _ => None,
                };
                let mut local_meta: HashMap<String, (Option<UpstreamDivergence>, bool)> =
                    HashMap::default();
                local_meta.reserve(branches.len());
                for branch in branches.iter() {
                    local_meta.insert(
                        branch.name.clone(),
                        (branch.divergence, head.is_some_and(|h| h == branch.name)),
                    );
                }

                let mut tree = SlashTree::default();
                for branch in branches.iter() {
                    tree.insert(&branch.name);
                }
                push_slash_tree_rows(
                    &tree,
                    &mut rows,
                    Some(&local_meta),
                    head_upstream_full.as_deref(),
                    0,
                    false,
                    BranchSection::Local,
                    "",
                    "",
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
            Loadable::Error(e) => rows.push(BranchSidebarRow::Placeholder {
                section: BranchSection::Local,
                message: e.clone().into(),
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
            Loadable::Error(e) => {
                rows.push(BranchSidebarRow::Placeholder {
                    section: BranchSection::Remote,
                    message: e.clone().into(),
                });
                remote_section_is_loading_or_error = true;
            }
            Loadable::NotLoaded => {}
        }

        if !remote_section_is_loading_or_error {
            if let Loadable::Ready(local_branches) = &repo.branches {
                // Some repos have upstream tracking configured before any local
                // remote-tracking refs are present. Surface those upstreams so the
                // Remote section still reflects tracked branches.
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
                // Ensure remotes with no local remote-tracking branches are still visible
                // (e.g. newly added remotes before an initial fetch).
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
                    let name_prefix = format!("{remote}/");
                    push_slash_tree_rows(
                        &tree,
                        &mut rows,
                        None,
                        head_upstream_full.as_deref(),
                        1,
                        true,
                        BranchSection::Remote,
                        &name_prefix,
                        "",
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
                    let label: SharedString = if let Some(branch) = &worktree.branch {
                        format!("{branch}  {}", worktree.path.display()).into()
                    } else if worktree.detached {
                        format!("(detached)  {}", worktree.path.display()).into()
                    } else {
                        worktree.path.display().to_string().into()
                    };
                    let tooltip = label.clone();
                    rows.push(BranchSidebarRow::WorktreeItem {
                        path: worktree.path.clone(),
                        label,
                        tooltip,
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
            Loadable::Error(e) => rows.push(BranchSidebarRow::WorktreePlaceholder {
                message: e.clone().into(),
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
                    let label: SharedString = submodule.path.display().to_string().into();
                    let tooltip: SharedString = submodule.path.display().to_string().into();
                    rows.push(BranchSidebarRow::SubmoduleItem {
                        path: submodule.path.clone(),
                        label,
                        tooltip,
                    });
                }
            }
            Loadable::Loading => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::NotLoaded => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: "Loading".into(),
            }),
            Loadable::Error(e) => rows.push(BranchSidebarRow::SubmodulePlaceholder {
                message: e.clone().into(),
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
            Loadable::Error(e) => rows.push(BranchSidebarRow::StashPlaceholder {
                message: e.clone().into(),
            }),
        }
    }

    // Add rows to the end of the side to make room for side panel toggler.
    for _ in 0..TRAILING_BOTTOM_SPACERS {
        rows.push(BranchSidebarRow::SectionSpacer);
    }

    rows
}

fn push_branch_leaf(
    out: &mut Vec<BranchSidebarRow>,
    label: &str,
    full_name: String,
    local_meta: Option<&HashMap<String, (Option<UpstreamDivergence>, bool)>>,
    upstream_full: Option<&str>,
    depth: usize,
    muted: bool,
    section: BranchSection,
) {
    let is_upstream = upstream_full.is_some_and(|u| u == full_name.as_str());
    let (divergence, is_head) = local_meta
        .and_then(|meta| meta.get(&full_name))
        .copied()
        .unwrap_or((None, false));
    let divergence_ahead = divergence
        .filter(|d| d.ahead > 0)
        .map(|d| d.ahead.to_string().into());
    let divergence_behind = divergence
        .filter(|d| d.behind > 0)
        .map(|d| d.behind.to_string().into());
    let upstream_note = if is_upstream && section == BranchSection::Remote {
        " (upstream for current branch)"
    } else {
        ""
    };
    let tooltip: SharedString = format!("Branch: {full_name}{upstream_note}").into();
    out.push(BranchSidebarRow::Branch {
        label: label.to_string().into(),
        name: full_name.into(),
        section,
        depth,
        muted,
        divergence,
        divergence_ahead,
        divergence_behind,
        tooltip,
        is_head,
        is_upstream,
    });
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
    name_prefix: &str,
    group_path_prefix: &str,
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
                push_branch_leaf(
                    out,
                    label,
                    format!("{name_prefix}{label}"),
                    local_meta,
                    upstream_full,
                    depth,
                    muted,
                    section,
                );
            }
            continue;
        }

        let group_path = format!("{group_path_prefix}{label}");
        let collapse_key = match section {
            BranchSection::Local => local_group_storage_key(&group_path),
            BranchSection::Remote => {
                remote_group_storage_key(remote_name.unwrap_or_default(), &group_path)
            }
        };
        let group_collapsed = is_collapsed(collapsed_items, &collapse_key);
        out.push(BranchSidebarRow::GroupHeader {
            label: format!("{label}/").into(),
            section,
            depth,
            collapsed: group_collapsed,
            collapse_key: collapse_key.into(),
        });
        if group_collapsed {
            continue;
        }

        if node.is_leaf {
            push_branch_leaf(
                out,
                label,
                format!("{name_prefix}{label}"),
                local_meta,
                upstream_full,
                depth + 1,
                muted,
                section,
            );
        }

        let next_name_prefix = format!("{name_prefix}{label}/");
        let next_group_path_prefix = format!("{group_path}/");
        push_slash_tree_rows(
            node,
            out,
            local_meta,
            upstream_full,
            depth + 1,
            muted,
            section,
            &next_name_prefix,
            &next_group_path_prefix,
            remote_name,
            collapsed_items,
        );
    }
}
