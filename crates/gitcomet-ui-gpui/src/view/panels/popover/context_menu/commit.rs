use super::*;

pub(super) fn model(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> ContextMenuModel {
    let sha = commit_id.as_ref().to_string();
    let short: SharedString = sha.get(0..8).unwrap_or(&sha).to_string().into();

    let commit_summary = this
        .active_repo()
        .and_then(|r| match &r.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|c| c.id == *commit_id)
                .map(|c| format!("{} — {}", c.author, c.summary)),
            _ => None,
        })
        .unwrap_or_default();

    let branch_names: Vec<String> = this
        .active_repo()
        .and_then(|r| match &r.branches {
            Loadable::Ready(branches) => Some(
                branches
                    .iter()
                    .filter(|b| b.target == *commit_id)
                    .map(|b| b.name.clone())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();

    let header_text: SharedString = match branch_names.as_slice() {
        [] => format!("Commit {short}").into(),
        [name] => name.clone().into(),
        names => names.join(", ").into(),
    };
    let mut items = vec![ContextMenuItem::Header(
        components::ContextMenuText::new(header_text).max_lines(2),
    )];
    if !commit_summary.is_empty() {
        items.push(ContextMenuItem::Label(
            components::ContextMenuText::new(commit_summary).max_lines(4),
        ));
    }
    items.push(ContextMenuItem::Separator);

    // "Squash N commits" appears only when the right-clicked commit is part
    // of the active multi-selection and the whole selection passes the squash
    // criteria (contiguous linear first-parent chain, non-root base). The
    // range may end at HEAD or sit anywhere in the chain.
    let squash_plan = this
        .active_repo()
        .filter(|repo| repo.id == repo_id)
        .and_then(|repo| {
            let selection = &repo.history_state.multi_selection;
            if !(selection.is_multi() && selection.contains(commit_id)) {
                return None;
            }
            let Loadable::Ready(page) = &repo.log else {
                return None;
            };
            let head = repo.head_commit_id()?;
            gitcomet_core::squash::squash_eligibility(&page.commits, &selection.commits, &head)
        });
    if let Some(plan) = squash_plan {
        let label = format!("Squash {} commits", plan.commit_count).into();
        items.push(ContextMenuItem::Entry {
            label,
            icon: Some("icons/git_commit.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SquashSelectedCommits { repo_id }),
        });
        items.push(ContextMenuItem::Separator);
    }
    items.push(ContextMenuItem::Entry {
        label: "Open diff".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SelectDiff {
            repo_id,
            target: DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: None,
            },
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Browse repository at this point".into(),
        icon: Some("icons/history.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Export patch…".into(),
        icon: Some("icons/arrow_down.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::ExportPatch {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Add tag…".into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: Some("T".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateTagPrompt {
                repo_id,
                target: sha.clone(),
            },
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Checkout (detached)".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: Some("D".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::CheckoutCommit {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Cherry-pick".into(),
        icon: Some("icons/arrow_up.svg".into()),
        shortcut: Some("P".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::CherryPickCommit {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Revert".into(),
        icon: Some("icons/undo.svg".into()),
        shortcut: Some("R".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::RevertCommit {
            repo_id,
            commit_id: commit_id.clone(),
        }),
    });
    let current_branch: SharedString = this
        .active_repo()
        .and_then(|r| match &r.head_branch {
            Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => {
                Some(head.as_str().into())
            }
            _ => None,
        })
        .unwrap_or_else(|| short.clone());

    // Rebasing the current branch onto the commit it already points to is a
    // no-op (plain rebase) or produces an empty `HEAD..HEAD` todo list
    // (interactive), so skip both entries on the HEAD commit. The topmost
    // commit is still editable via an interactive rebase from the commit below.
    let is_head_commit = this
        .active_repo()
        .filter(|repo| repo.id == repo_id)
        .and_then(|repo| repo.head_commit_id())
        .is_some_and(|head| head == *commit_id);
    if !is_head_commit {
        // Prefer a branch name at the target commit; fall back to the abbreviated
        // sha for the label and the full sha for the actual rebase target.
        let target_label: SharedString = branch_names
            .first()
            .map(|s| s.as_str())
            .unwrap_or(&short)
            .into();
        let onto_ref = branch_names.first().cloned().unwrap_or_else(|| sha.clone());
        items.push(ContextMenuItem::Entry {
            label: format!("Rebase {current_branch} onto {target_label}").into(),
            icon: Some("icons/arrow_up.svg".into()),
            shortcut: Some("B".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::RebaseOntoConfirm {
                    repo_id,
                    onto: onto_ref,
                },
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: format!("Interactive rebase {current_branch} onto {target_label}").into(),
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("I".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::LoadInteractiveRebaseSetup {
                repo_id,
                base: sha.clone(),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    for (label, icon, mode) in [
        (
            "Reset (--soft) to here",
            "icons/refresh.svg",
            ResetMode::Soft,
        ),
        (
            "Reset (--mixed) to here",
            "icons/refresh.svg",
            ResetMode::Mixed,
        ),
        (
            "Reset (--hard) to here",
            "icons/refresh.svg",
            ResetMode::Hard,
        ),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::ResetPrompt {
                    repo_id,
                    target: sha.clone(),
                    mode,
                },
            }),
        });
    }

    ContextMenuModel::new(items)
}
