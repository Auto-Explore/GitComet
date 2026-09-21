use super::*;
use gitcomet_core::domain::Upstream;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    target: &BranchMenuTarget,
) -> ContextMenuModel {
    let section = target.section();
    let name = target.display_name();
    let header: SharedString = match section {
        BranchSection::Local => "Local branch".into(),
        BranchSection::Remote => "Remote branch".into(),
    };
    let mut items = vec![ContextMenuItem::Header(header.into())];
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
    let active_upstream = active_branch.and_then(|branch| branch.upstream.as_ref());
    let active_branch_has_no_upstream =
        active_branch.is_some_and(|branch| branch.upstream.is_none());
    // The target's own parts: exact even when `remote/branch` is ambiguous.
    let target_upstream = target.remote_parts().map(|(remote, branch)| Upstream {
        remote: remote.to_string(),
        branch: branch.to_string(),
    });
    // Setting an upstream needs the remote-tracking ref to exist; unlinking
    // only needs the target to be the configured upstream.
    let exact_remote_branch = target_upstream.clone().filter(|upstream| {
        repo.and_then(|repo| repo.remote_branches.ready())
            .is_some_and(|branches| {
                branches.iter().any(|candidate| {
                    candidate.remote == upstream.remote && candidate.name == upstream.branch
                })
            })
    });
    let is_current_branch = active_branch_name
        .as_ref()
        .is_some_and(|branch| branch == &name);
    // Name the branch being moved rather than the opaque "HEAD".
    let current_branch_label = active_branch_name
        .clone()
        .unwrap_or_else(|| "HEAD".to_string());
    // Git refuses to start a rebase while another rebase, cherry-pick,
    // revert, or unconcluded merge is in flight; grey the entry out instead
    // of letting the click surface that refusal.
    let history_rewrite_disabled = repo.is_some_and(|r| r.history_rewrite_busy());
    let branch_commit_id: Option<CommitId> = match target {
        BranchMenuTarget::Local { name } => repo.and_then(|repo| {
            repo.branches
                .ready()?
                .iter()
                .find(|branch| branch.name == *name)
                .map(|branch| branch.target.clone())
        }),
        BranchMenuTarget::Remote { remote, branch } => repo.and_then(|repo| {
            repo.remote_branches
                .ready()?
                .iter()
                .find(|candidate| candidate.remote == *remote && candidate.name == *branch)
                .map(|candidate| candidate.target.clone())
        }),
    };
    // Local Git actions operate on the exact loaded tip. This is independent
    // of the remote-tracking ref's destination, which can be renamed by a
    // custom fetch refspec.
    let action_reference = match section {
        BranchSection::Local => name.clone(),
        BranchSection::Remote => branch_commit_id
            .as_ref()
            .map(|id| id.as_ref().to_string())
            .unwrap_or_else(|| name.clone()),
    };

    items.push(ContextMenuItem::Entry {
        label: "Checkout".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(match section {
            BranchSection::Local => ContextMenuAction::CheckoutBranch {
                repo_id,
                name: name.clone(),
            },
            BranchSection::Remote => {
                if let Some((remote, branch)) = target.remote_parts() {
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
        icon: Some("icons/plus.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                target: action_reference.clone(),
                source_selectable: false,
                name_prefix: String::new(),
            },
        }),
    });
    if section == BranchSection::Local {
        items.push(ContextMenuItem::Entry {
            label: "Rename branch".into(),
            icon: Some("icons/pencil.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::RenameBranchPrompt {
                    repo_id,
                    name: name.clone(),
                    is_current_branch: false,
                },
            }),
        });
    }

    // Comparison: mark this branch's tip, or compare it against a mark.
    if let Some(commit_id) = branch_commit_id {
        let comparison_mark = repo.and_then(|r| r.navigation.comparison_mark.clone());
        items.push(ContextMenuItem::Entry {
            label: format!("Mark {name} for comparison").into(),
            icon: Some("icons/git_branch.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::MarkForComparison {
                repo_id,
                commit_id: commit_id.clone(),
                label: name.clone(),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Compare with working tree".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CompareWithWorkingTree {
                repo_id,
                commit_id: commit_id.clone(),
                label: name.clone(),
            }),
        });
        if let Some(mark) = comparison_mark.filter(|mark| mark.commit_id != commit_id) {
            items.push(ContextMenuItem::Entry {
                label: format!("Compare with {}", mark.label).into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::CompareWithMarked {
                    repo_id,
                    commit_id,
                    label: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Clear comparison mark".into(),
                icon: Some("icons/generic_close.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::ClearComparisonMark { repo_id }),
            });
        }
    }
    items.push(ContextMenuItem::Entry {
        label: "Copy branch name".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::CopyText { text: name.clone() }),
    });
    let pinned = this.is_branch_pinned(repo_id, section, &name);
    items.push(ContextMenuItem::Entry {
        label: if pinned {
            "Unpin branch".into()
        } else {
            "Pin branch".into()
        },
        icon: Some("icons/pin.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ToggleBranchPin {
            repo_id,
            section,
            name: name.clone(),
        }),
    });
    if section == BranchSection::Local {
        items.push(ContextMenuItem::Separator);
        if !is_current_branch {
            items.push(ContextMenuItem::Entry {
                label: "Pull into current".into(),
                icon: Some("icons/arrow_down.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PullBranch {
                    repo_id,
                    remote: ".".to_string(),
                    branch: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Merge into current".into(),
                icon: Some("icons/swap.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("icons/arrow_right.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::SquashRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: format!("Rebase {current_branch_label} onto {name}").into(),
                icon: Some("icons/arrow_up.svg".into()),
                shortcut: None,
                disabled: history_rewrite_disabled,
                action: Box::new(ContextMenuAction::OpenPopover {
                    kind: PopoverKind::RebaseOntoConfirm {
                        repo_id,
                        onto: action_reference.clone(),
                    },
                }),
            });
        }
        items.push(ContextMenuItem::Entry {
            label: "Delete branch".into(),
            icon: Some("icons/trash.svg".into()),
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
        if let Some((remote, branch)) = target.remote_parts() {
            items.push(ContextMenuItem::Entry {
                label: "Pull into current".into(),
                icon: Some("icons/arrow_down.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PullBranch {
                    repo_id,
                    remote: remote.to_string(),
                    branch: branch.to_string(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Merge into current".into(),
                icon: Some("icons/swap.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("icons/arrow_right.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::SquashRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: format!("Rebase {current_branch_label} onto {name}").into(),
                icon: Some("icons/arrow_up.svg".into()),
                shortcut: None,
                disabled: history_rewrite_disabled,
                action: Box::new(ContextMenuAction::OpenPopover {
                    kind: PopoverKind::RebaseOntoConfirm {
                        repo_id,
                        onto: action_reference.clone(),
                    },
                }),
            });
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Entry {
                label: "Delete remote branch…".into(),
                icon: Some("icons/trash.svg".into()),
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
                && let Some(upstream) = exact_remote_branch.clone()
            {
                items.push(ContextMenuItem::Entry {
                    label: "Set as tracking upstream".into(),
                    icon: Some("icons/link.svg".into()),
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::SetUpstreamBranch {
                        repo_id,
                        branch: active_branch_name,
                        upstream,
                    }),
                });
            }
            if active_upstream.is_some() {
                items.push(ContextMenuItem::Entry {
                    label: "Unlink upstream branch".into(),
                    icon: Some("icons/unlink.svg".into()),
                    shortcut: None,
                    disabled: active_upstream != target_upstream.as_ref(),
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
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune merged branches".into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Prune local tags".into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        });
    }

    ContextMenuModel::new(items)
}
