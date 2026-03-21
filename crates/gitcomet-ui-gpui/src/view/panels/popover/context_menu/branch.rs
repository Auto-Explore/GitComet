use super::*;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    name: &String,
) -> ContextMenuModel {
    let header: SharedString = match section {
        BranchSection::Local => "Local branch".into(),
        BranchSection::Remote => "Remote branch".into(),
    };
    let mut items = vec![ContextMenuItem::Header(header)];
    items.push(ContextMenuItem::Label(name.clone().into()));
    items.push(ContextMenuItem::Separator);

    let repo = this.state.repos.iter().find(|r| r.id == repo_id);

    let active_branch_name = repo.and_then(|r| match &r.head_branch {
        Loadable::Ready(branch) => Some(branch.clone()),
        _ => None,
    });
    let active_branch = repo.and_then(|r| match (&r.branches, active_branch_name.as_ref()) {
        (Loadable::Ready(branches), Some(head)) => {
            branches.iter().find(|branch| branch.name == *head)
        }
        _ => None,
    });
    let active_upstream_full = active_branch.and_then(|branch| {
        branch
            .upstream
            .as_ref()
            .map(|upstream| format!("{}/{}", upstream.remote, upstream.branch))
    });
    let active_branch_has_no_upstream =
        active_branch.is_some_and(|branch| branch.upstream.is_none());
    let is_current_branch = active_branch_name
        .as_ref()
        .is_some_and(|branch| branch == name);

    items.push(ContextMenuItem::Entry {
        label: "Checkout".into(),
        icon: Some("⎇".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(match section {
            BranchSection::Local => ContextMenuAction::CheckoutBranch {
                repo_id,
                name: name.clone(),
            },
            BranchSection::Remote => {
                if let Some((remote, branch)) = name.split_once('/') {
                    ContextMenuAction::OpenPopover {
                        kind: PopoverKind::CheckoutRemoteBranchPrompt {
                            repo_id,
                            remote: remote.to_string(),
                            branch: branch.to_string(),
                        },
                    }
                } else {
                    ContextMenuAction::CheckoutBranch {
                        repo_id,
                        name: name.clone(),
                    }
                }
            }
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Create branch".into(),
        icon: Some("+".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                target: name.clone(),
            },
        }),
    });
    if section == BranchSection::Local {
        items.push(ContextMenuItem::Separator);
        if !is_current_branch {
            items.push(ContextMenuItem::Entry {
                label: "Pull into current".into(),
                icon: Some("↓".into()),
                shortcut: Some("P".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::PullBranch {
                    repo_id,
                    remote: ".".to_string(),
                    branch: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Merge into current".into(),
                icon: Some("⇄".into()),
                shortcut: Some("M".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("⇉".into()),
                shortcut: Some("S".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::SquashRef {
                    repo_id,
                    reference: name.clone(),
                }),
            });
        }
        items.push(ContextMenuItem::Entry {
            label: "Delete branch".into(),
            icon: Some("🗑".into()),
            shortcut: None,
            disabled: is_current_branch,
            action: Box::new(ContextMenuAction::DeleteBranch {
                repo_id,
                name: name.clone(),
            }),
        });
    }

    if section == BranchSection::Remote {
        items.push(ContextMenuItem::Separator);
        if let Some((remote, branch)) = name.split_once('/') {
            items.push(ContextMenuItem::Entry {
                label: "Pull into current".into(),
                icon: Some("↓".into()),
                shortcut: Some("P".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::PullBranch {
                    repo_id,
                    remote: remote.to_string(),
                    branch: branch.to_string(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Merge into current".into(),
                icon: Some("⇄".into()),
                shortcut: Some("M".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("⇉".into()),
                shortcut: Some("S".into()),
                disabled: false,
                action: Box::new(ContextMenuAction::SquashRef {
                    repo_id,
                    reference: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Entry {
                label: "Delete remote branch…".into(),
                icon: Some("🗑".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::OpenPopover {
                    kind: PopoverKind::remote(
                        repo_id,
                        RemotePopoverKind::DeleteBranchConfirm {
                            remote: remote.to_string(),
                            branch: branch.to_string(),
                        },
                    ),
                }),
            });
            if active_branch_has_no_upstream
                && let Some(active_branch_name) = active_branch_name.clone()
                && name.split_once('/').is_some()
            {
                items.push(ContextMenuItem::Entry {
                    label: "Set as tracking upstream".into(),
                    icon: Some("link".into()),
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::SetUpstreamBranch {
                        repo_id,
                        branch: active_branch_name,
                        upstream: name.clone(),
                    }),
                });
            }
            if active_upstream_full.is_some() {
                items.push(ContextMenuItem::Entry {
                    label: "Unlink upstream branch".into(),
                    icon: Some("unlink".into()),
                    shortcut: None,
                    disabled: active_upstream_full.as_deref() != Some(name.as_str()),
                    action: Box::new(ContextMenuAction::UnsetUpstreamBranch {
                        repo_id,
                        branch: active_branch_name.unwrap_or_default(),
                    }),
                });
            }
            items.push(ContextMenuItem::Separator);
        }
        items.push(ContextMenuItem::Entry {
            label: "Fetch all".into(),
            icon: Some("↓".into()),
            shortcut: Some("F".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune merged branches".into(),
            icon: Some("🧹".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune local tags".into(),
            icon: Some("🏷".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        });
    }

    ContextMenuModel::new(items)
}
