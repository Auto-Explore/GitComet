use super::*;
use crate::view::branch_sidebar;

/// Context menu for a `/`-prefix group row in the branch tree (`feat/`).
///
/// The branch tree's answer to the file explorer's folder menu: tree controls
/// for the group itself, plus the two operations that only make sense on a
/// whole folder of branches — seeding a new branch inside it, and clearing it
/// out.
pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> ContextMenuModel {
    let member_count = member_count(this, repo_id, section, remote, path);
    let filtered = this.active_branch_filter().is_some();
    let collapse_key = match section {
        BranchSection::Local => branch_sidebar::local_group_storage_key(path),
        BranchSection::Remote => {
            branch_sidebar::remote_group_storage_key(remote.unwrap_or_default(), path)
        }
    };
    let collapsed = this.sidebar_collapse_key_is_collapsed(repo_id, &collapse_key);

    // What the group is called in prose: `origin/feat/` for a remote group so
    // the entries cannot be mistaken for the identically named local one.
    let group_label = match (section, remote) {
        (BranchSection::Remote, Some(remote)) => format!("{remote}/{path}/"),
        _ => format!("{path}/"),
    };

    let mut items = vec![ContextMenuItem::Header(group_label.clone().into())];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(if filtered {
            format!("{} matching the filter", branch_count_label(member_count))
        } else {
            branch_count_label(member_count)
        }),
    ));
    items.push(ContextMenuItem::Separator);

    items.push(ContextMenuItem::Entry {
        label: if collapsed { "Expand" } else { "Collapse" }.into(),
        icon: Some(
            if collapsed {
                "icons/chevron_right.svg"
            } else {
                "icons/chevron_down.svg"
            }
            .into(),
        ),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ToggleSidebarCollapseKey {
            collapse_key: collapse_key.clone().into(),
        }),
    });
    for (label, icon, collapsed) in [
        (
            "Expand all under here",
            "icons/arrow_down_to_line.svg",
            false,
        ),
        (
            "Collapse all under here",
            "icons/arrow_up_to_line.svg",
            true,
        ),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetBranchGroupCollapsedRecursive {
                section,
                remote: remote.map(ToOwned::to_owned),
                path: path.to_owned(),
                collapsed,
            }),
        });
    }

    // A remote group holds remote-tracking refs, which cannot be created
    // locally — so this is a local-only offer.
    if section == BranchSection::Local {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: format!("Create branch in {group_label}…").into(),
            icon: Some("icons/plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::CreateBranchFromRefPrompt {
                    repo_id,
                    target: current_branch_target(this, repo_id),
                    source_selectable: true,
                    name_prefix: format!("{path}/"),
                },
            }),
        });
    }

    let deletable_count = deletable_count(this, repo_id, section, remote, path);

    // While a filter is live the group shows only its matches, so the entry
    // says so rather than implying it covers the whole group.
    let count_label = if filtered {
        format!("{} matching", branch_count_label(deletable_count))
    } else {
        branch_count_label(deletable_count)
    };

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: format!("Delete {count_label} in {group_label}…").into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: deletable_count == 0,
        // Resolved when the entry is activated rather than baked in here: this
        // model is rebuilt on every repaint of the open menu, and materialising
        // a few hundred branch names per frame to render one count is waste.
        // The confirm still freezes the list it is handed.
        action: Box::new(ContextMenuAction::ConfirmDeleteBranchGroup {
            repo_id,
            section,
            remote: remote.map(ToOwned::to_owned),
            path: path.to_owned(),
            group_label,
        }),
    });

    ContextMenuModel::new(items)
}

fn branch_count_label(count: usize) -> String {
    if count == 1 {
        "1 branch".to_string()
    } else {
        format!("{count} branches")
    }
}

/// Visits the branches the group row is actually showing.
///
/// Matching is against `"{path}/"` rather than the bare `path`, so a sibling
/// group whose name merely starts with the same characters (`features/`
/// against `feat/`) is never counted as a member.
///
/// The sidebar's branch filter is applied here too. The tree filters branch
/// names *before* building the group tree, so a filtered `feat/` row lists only
/// its matching members — and a menu counting all 47 while one is on screen
/// would offer to delete 46 branches the user cannot see.
///
/// Names are passed without the remote prefix, matching how the tree stores
/// them and what `git push --delete` expects. Borrowed rather than cloned so
/// the per-repaint callers can count without allocating.
fn for_each_member(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
    mut visit: impl FnMut(&str),
) {
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return;
    };
    let needle = format!("{path}/");
    let filter = this.active_branch_filter().unwrap_or_default();

    match section {
        BranchSection::Local => {
            let Loadable::Ready(branches) = &repo.branches else {
                return;
            };
            for branch in branches.iter() {
                if branch.name.starts_with(needle.as_str())
                    && branch_sidebar::branch_matches_raw_filter(&branch.name, filter)
                {
                    visit(&branch.name);
                }
            }
        }
        BranchSection::Remote => {
            let Some(remote) = remote else {
                return;
            };
            let Loadable::Ready(branches) = &repo.remote_branches else {
                return;
            };
            for branch in branches.iter() {
                if branch.remote == remote
                    && branch.name.starts_with(needle.as_str())
                    && branch_sidebar::remote_branch_matches_raw_filter(
                        remote,
                        &branch.name,
                        filter,
                    )
                {
                    visit(&branch.name);
                }
            }
        }
    }
}

fn member_count(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> usize {
    let mut count = 0;
    for_each_member(this, repo_id, section, remote, path, |_| count += 1);
    count
}

/// Whether a member is one the batch delete may touch.
///
/// The checked-out branch cannot be deleted, so including it would promise
/// something the delete cannot do. A remote group has no such constraint: the
/// local HEAD says nothing about a remote-tracking ref.
fn is_deletable(section: BranchSection, name: &str, current_branch: Option<&str>) -> bool {
    section == BranchSection::Remote || Some(name) != current_branch
}

fn deletable_count(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> usize {
    let current_branch = current_branch_name(this, repo_id);
    let mut count = 0;
    for_each_member(this, repo_id, section, remote, path, |name| {
        if is_deletable(section, name, current_branch) {
            count += 1;
        }
    });
    count
}

/// The member list the delete confirmation acts on, resolved once when the
/// entry is activated.
pub(super) fn deletable_branches(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> Vec<String> {
    let current_branch = current_branch_name(this, repo_id);
    let mut names = Vec::new();
    for_each_member(this, repo_id, section, remote, path, |name| {
        if is_deletable(section, name, current_branch) {
            names.push(name.to_owned());
        }
    });
    names
}

fn current_branch_name(this: &PopoverHost, repo_id: RepoId) -> Option<&str> {
    this.state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.head_branch {
            Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => Some(head.as_str()),
            _ => None,
        })
}

/// What a branch created from this group should branch off: the checked-out
/// branch, or `HEAD` when detached.
fn current_branch_target(this: &PopoverHost, repo_id: RepoId) -> String {
    current_branch_name(this, repo_id)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "HEAD".to_string())
}
