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
    let merge_disabled = !super::super::merge_commit_confirm::merge_commit_is_allowed(
        repo,
        &this.state.large_file_settings,
    );
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
    if let Some(repo) = repo {
        items.extend(worktree_entries(repo, repo_id, target));
    }
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
                    commit_id: commit_id.clone(),
                    label: name.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Compare with merge base".into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::CompareWithMergeBase {
                    repo_id,
                    commit_id: commit_id.clone(),
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
    // Focus sits with the other view-shaping entries, above the actions that
    // change the repository.
    items.push(super::focus::entry(
        this,
        repo_id,
        match target {
            BranchMenuTarget::Local { name } => {
                gitcomet_core::domain::HistoryFocus::local_branch(name)
            }
            BranchMenuTarget::Remote { remote, branch } => {
                gitcomet_core::domain::HistoryFocus::remote_branch(remote, branch)
            }
        },
    ));
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
                disabled: merge_disabled,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("icons/arrow_right.svg".into()),
                shortcut: None,
                disabled: merge_disabled,
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
                disabled: merge_disabled,
                action: Box::new(ContextMenuAction::MergeRef {
                    repo_id,
                    reference: action_reference.clone(),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: "Squash into current".into(),
                icon: Some("icons/arrow_right.svg".into()),
                shortcut: None,
                disabled: merge_disabled,
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

/// What a new worktree for the branch starts from.
enum WorktreeStart<'a> {
    /// A local branch: checked out as-is, or the start of a new branch when
    /// it is checked out somewhere already.
    Local(String),
    /// A remote branch no local branch tracks yet.
    Track { remote: &'a str, branch: &'a str },
}

/// Starts a new worktree for the branch, after opening the other worktree
/// that has it checked out, if any.
fn worktree_entries(
    repo: &RepoState,
    repo_id: RepoId,
    target: &BranchMenuTarget,
) -> Vec<ContextMenuItem> {
    let start = match target {
        BranchMenuTarget::Local { name } => WorktreeStart::Local(name.clone()),
        BranchMenuTarget::Remote { remote, branch } => repo
            .branches
            .ready()
            .and_then(|branches| {
                branches.iter().find(|candidate| {
                    candidate.upstream.as_ref().is_some_and(|upstream| {
                        upstream.remote == *remote && upstream.branch == *branch
                    })
                })
            })
            .map_or(WorktreeStart::Track { remote, branch }, |local| {
                WorktreeStart::Local(local.name.clone())
            }),
    };
    let local = match &start {
        WorktreeStart::Local(local) => Some(local.as_str()),
        WorktreeStart::Track { .. } => None,
    };
    let mut items = Vec::new();
    if let Some(path) =
        local.and_then(|local| crate::view::rows::branch_held_elsewhere(repo, local))
    {
        items.push(ContextMenuItem::Entry {
            label: "Open worktree".into(),
            icon: Some("icons/git_worktree.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepo { path }),
        });
    }
    // Git checks a branch out in one worktree only.
    let checked_out =
        local.is_some_and(|local| crate::view::rows::branch_checked_out_at(repo, local).is_some());
    let prefill = match start {
        WorktreeStart::Local(local) => WorktreeAddPrefill {
            source: local,
            create_branch: checked_out,
            new_branch: String::new(),
            open_after: true,
        },
        WorktreeStart::Track { remote, branch } => WorktreeAddPrefill {
            source: format!("{remote}/{branch}"),
            create_branch: true,
            new_branch: branch.to_string(),
            open_after: true,
        },
    };
    items.push(ContextMenuItem::Entry {
        label: "Open in new worktree".into(),
        icon: Some("icons/git_worktree.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenWorktreeAddPrompt { repo_id, prefill }),
    });
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, RepoSpec, Worktree};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn branch(name: &str, upstream: Option<(&str, &str)>) -> Branch {
        Branch {
            name: name.to_string(),
            target: CommitId(Arc::from("1111111111111111111111111111111111111111")),
            upstream: upstream.map(|(remote, branch)| Upstream {
                remote: remote.to_string(),
                branch: branch.to_string(),
            }),
            divergence: None,
        }
    }

    fn worktree(path: &str, branch: &str) -> Worktree {
        Worktree {
            bare: false,
            path: PathBuf::from(path),
            head: None,
            branch: Some(branch.to_string()),
            detached: false,
        }
    }

    /// `/tmp/ws/main` on `main`, with `feat/held` checked out in `/tmp/ws/held`.
    fn repo() -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/ws/main"),
            },
        );
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![
            branch("main", Some(("origin", "main"))),
            branch("feat/free", None),
            branch("feat/held", Some(("origin", "held"))),
            branch("local-x", Some(("origin", "x"))),
        ]));
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            worktree("/tmp/ws/main", "main"),
            worktree("/tmp/ws/held", "feat/held"),
        ]));
        repo
    }

    fn local(name: &str) -> BranchMenuTarget {
        BranchMenuTarget::Local {
            name: name.to_string(),
        }
    }

    fn remote(remote: &str, branch: &str) -> BranchMenuTarget {
        BranchMenuTarget::Remote {
            remote: remote.to_string(),
            branch: branch.to_string(),
        }
    }

    /// Each entry's label, whether it is disabled, and its action.
    fn entries_of(
        repo: &RepoState,
        target: BranchMenuTarget,
    ) -> Vec<(String, bool, ContextMenuAction)> {
        worktree_entries(repo, RepoId(1), &target)
            .into_iter()
            .map(|item| match item {
                ContextMenuItem::Entry {
                    label,
                    disabled,
                    action,
                    ..
                } => (label.to_string(), disabled, *action),
                _ => panic!("the worktree items are entries"),
            })
            .collect()
    }

    fn entry(target: BranchMenuTarget) -> (String, bool, ContextMenuAction) {
        let [entry] = <[_; 1]>::try_from(entries_of(&repo(), target))
            .unwrap_or_else(|_| panic!("one worktree entry"));
        entry
    }

    /// The "Open worktree" entry, then the "Open in new worktree" one.
    fn open_and_new_entries(target: BranchMenuTarget) -> [(String, bool, ContextMenuAction); 2] {
        <[_; 2]>::try_from(entries_of(&repo(), target))
            .unwrap_or_else(|_| panic!("two worktree entries"))
    }

    /// A new branch off `source`, named after the new folder.
    fn new_branch_from(source: &str) -> WorktreeAddPrefill {
        WorktreeAddPrefill {
            source: source.to_string(),
            create_branch: true,
            new_branch: String::new(),
            open_after: true,
        }
    }

    fn prefill(action: ContextMenuAction) -> WorktreeAddPrefill {
        match action {
            ContextMenuAction::OpenWorktreeAddPrompt { repo_id, prefill } => {
                assert_eq!(repo_id, RepoId(1));
                prefill
            }
            _ => panic!("expected the Add-worktree action"),
        }
    }

    #[test]
    fn a_free_local_branch_is_checked_out_in_a_new_worktree() {
        let (label, disabled, action) = entry(local("feat/free"));
        assert_eq!(label, "Open in new worktree");
        assert_eq!(
            crate::view::menu_presentation::context_menu_tone(
                &label,
                Some("icons/git_worktree.svg")
            ),
            gitcomet_ui_kit::components::ContextMenuTone::Success,
            "the menu's own label keeps the green icon"
        );
        assert!(!disabled);
        assert_eq!(
            prefill(action),
            WorktreeAddPrefill {
                source: "feat/free".to_string(),
                create_branch: false,
                new_branch: String::new(),
                open_after: true,
            }
        );
    }

    #[test]
    fn the_current_branch_starts_a_new_branch_in_a_new_worktree() {
        let (label, disabled, action) = entry(local("main"));
        assert_eq!(label, "Open in new worktree");
        assert!(!disabled);
        assert_eq!(
            prefill(action),
            new_branch_from("main"),
            "git refuses to check main out a second time"
        );
    }

    #[test]
    fn the_current_branch_starts_a_new_branch_before_worktrees_load() {
        let mut repo = repo();
        repo.worktrees = Loadable::NotLoaded;
        let [(_, disabled, action)] = <[_; 1]>::try_from(entries_of(&repo, local("main")))
            .unwrap_or_else(|_| panic!("one worktree entry"));
        assert!(!disabled);
        assert_eq!(prefill(action), new_branch_from("main"));
    }

    #[test]
    fn a_branch_held_by_another_worktree_opens_it_or_starts_a_new_branch() {
        let [(open_label, _, open), (new_label, disabled, new)] =
            open_and_new_entries(local("feat/held"));
        assert_eq!(open_label, "Open worktree");
        assert!(matches!(
            open,
            ContextMenuAction::OpenRepo { path } if path == std::path::Path::new("/tmp/ws/held")
        ));
        assert_eq!(new_label, "Open in new worktree");
        assert!(!disabled);
        assert_eq!(prefill(new), new_branch_from("feat/held"));
    }

    #[test]
    fn a_remote_branch_reuses_the_local_branch_tracking_it() {
        let (_, disabled, action) = entry(remote("origin", "x"));
        assert!(!disabled);
        let prefill = prefill(action);
        assert_eq!(prefill.source, "local-x");
        assert!(!prefill.create_branch);
    }

    #[test]
    fn a_remote_branch_whose_tracking_branch_is_held_opens_it_or_starts_a_new_branch() {
        let [(open_label, _, open), (_, _, new)] = open_and_new_entries(remote("origin", "held"));
        assert_eq!(open_label, "Open worktree");
        assert!(matches!(
            open,
            ContextMenuAction::OpenRepo { path } if path == std::path::Path::new("/tmp/ws/held")
        ));
        assert_eq!(prefill(new), new_branch_from("feat/held"));
    }

    #[test]
    fn an_untracked_remote_branch_gets_a_new_tracking_branch() {
        let (label, disabled, action) = entry(remote("origin", "feat/new"));
        assert_eq!(label, "Open in new worktree");
        assert!(!disabled);
        assert_eq!(
            prefill(action),
            WorktreeAddPrefill {
                source: "origin/feat/new".to_string(),
                create_branch: true,
                new_branch: "feat/new".to_string(),
                open_after: true,
            }
        );
    }
}
