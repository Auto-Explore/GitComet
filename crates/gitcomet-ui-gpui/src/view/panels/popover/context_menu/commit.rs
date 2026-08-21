use super::*;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};

type InteractiveRebaseSourceColor = (String, u8);
type MultiCherryPickPlan = (
    Vec<InteractiveRebaseEntry>,
    Vec<InteractiveRebaseSourceColor>,
);

fn multi_cherry_pick_plan(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
) -> Option<MultiCherryPickPlan> {
    let repo = this.active_repo().filter(|repo| repo.id == repo_id)?;
    let selection = &repo.history_state.multi_selection;
    if !(selection.is_multi() && selection.contains(commit_id)) {
        return None;
    }
    let Loadable::Ready(page) = &repo.log else {
        return None;
    };

    let branches = match &repo.branches {
        Loadable::Ready(branches) => Some(branches),
        _ => None,
    };
    let remote_branches = match &repo.remote_branches {
        Loadable::Ready(branches) => Some(branches),
        _ => None,
    };
    let branch_heads = branches
        .into_iter()
        .flat_map(|branches| branches.iter().map(|branch| branch.target.as_ref()))
        .chain(
            remote_branches
                .into_iter()
                .flat_map(|branches| branches.iter().map(|branch| branch.target.as_ref())),
        )
        .collect::<Vec<_>>();
    let head_target = repo.head_commit_id();
    let graph_rows = crate::view::history_graph::compute_graph(
        &page.commits,
        this.theme,
        branch_heads,
        head_target.as_ref().map(|head| head.as_ref()),
    );

    let mut selected = page
        .commits
        .iter()
        .enumerate()
        .filter(|(_, commit)| selection.contains(&commit.id))
        .collect::<Vec<_>>();
    if selected.len() < 2 {
        return None;
    }
    // Use reversed page order as the stable oldest-first baseline. Before the
    // editor becomes available, the repository-backed setup load follows the
    // complete graph through unselected commits and corrects any ancestry
    // inversions caused by clock skew.
    selected.reverse();

    let mut entries = Vec::with_capacity(selected.len());
    let mut source_colors = Vec::with_capacity(selected.len());
    for (page_ix, commit) in selected {
        entries.push(InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Pick,
            commit_id: commit.id.as_ref().to_string(),
            summary: commit.summary.to_string(),
            // The log page only carries the subject; the state layer loads
            // the full messages right after the setup opens so a reword
            // edit does not start from a body-less seed.
            message: commit.summary.to_string(),
            new_message: None,
        });
        let color_ix = graph_rows
            .get(page_ix)
            .map(|row| row.node_color_ix)
            .unwrap_or(0);
        source_colors.push((commit.id.as_ref().to_string(), color_ix));
    }

    Some((entries, source_colors))
}

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
    let multi_cherry_pick_plan = multi_cherry_pick_plan(this, repo_id, commit_id);
    let has_multi_cherry_pick = multi_cherry_pick_plan.is_some();
    let is_head_commit = this
        .active_repo()
        .filter(|repo| repo.id == repo_id)
        .and_then(|repo| repo.head_commit_id())
        .is_some_and(|head| head == *commit_id);
    // Cherry-pick, revert, squash, and rebase all contend for git's single
    // sequencer slot, so any in-flight operation (or a merge waiting to be
    // concluded) disables starting every one of them, not just its own kind.
    let history_rewrite_disabled = this
        .active_repo()
        .filter(|repo| repo.id == repo_id)
        .is_some_and(|repo| repo.history_rewrite_busy());

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
            disabled: history_rewrite_disabled,
            action: Box::new(ContextMenuAction::SquashSelectedCommits { repo_id }),
        });
        items.push(ContextMenuItem::Separator);
    }
    if !is_head_commit && let Some((entries, source_colors)) = multi_cherry_pick_plan {
        let label = format!("Cherry-pick {} commits…", entries.len()).into();
        items.push(ContextMenuItem::Entry {
            label,
            icon: Some("icons/arrow_up.svg".into()),
            shortcut: Some("P".into()),
            disabled: history_rewrite_disabled,
            action: Box::new(ContextMenuAction::OpenInteractiveCherryPickSetup {
                repo_id,
                entries,
                source_colors,
            }),
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
    if let Some(permalink) = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.remotes {
            Loadable::Ready(remotes) => crate::view::permalink::commit_permalink(remotes, &sha),
            _ => None,
        })
    {
        items.push(ContextMenuItem::Entry {
            label: "Copy commit permalink".into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText { text: permalink }),
        });
    }
    // Comparison: mark this commit as a base, or compare it against a mark.
    // Look the repo up by id rather than via `active_repo`, so a menu opened for
    // a non-active repo offers the same comparison entries the branch and tag
    // menus do — the dispatched `Msg` carries `repo_id` and works either way.
    let comparison_mark = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.comparison_mark.clone());
    items.push(ContextMenuItem::Entry {
        label: format!("Mark {short} for comparison").into(),
        icon: Some("icons/git_commit.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::MarkForComparison {
            repo_id,
            commit_id: commit_id.clone(),
            label: short.to_string(),
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
            label: short.to_string(),
        }),
    });
    if let Some(mark) = comparison_mark.filter(|mark| mark.commit_id != *commit_id) {
        items.push(ContextMenuItem::Entry {
            label: format!("Compare with {}", mark.label).into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CompareWithMarked {
                repo_id,
                commit_id: commit_id.clone(),
                label: short.to_string(),
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
        label: "Create branch from this commit".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateBranchFromRefPrompt {
                repo_id,
                target: sha.clone(),
                source_selectable: false,
                name_prefix: String::new(),
            },
        }),
    });
    if !has_multi_cherry_pick && !is_head_commit {
        items.push(ContextMenuItem::Entry {
            label: "Cherry-pick".into(),
            icon: Some("icons/arrow_up.svg".into()),
            shortcut: Some("P".into()),
            disabled: history_rewrite_disabled,
            action: Box::new(ContextMenuAction::CherryPickCommit {
                repo_id,
                commit_id: commit_id.clone(),
            }),
        });
    }
    items.push(ContextMenuItem::Entry {
        label: "Revert".into(),
        icon: Some("icons/undo.svg".into()),
        shortcut: Some("R".into()),
        disabled: history_rewrite_disabled,
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
            disabled: history_rewrite_disabled,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::RebaseOntoConfirm {
                    repo_id,
                    onto: onto_ref,
                },
            }),
        });
        // Count the commits the interactive rebase will rewrite (`this..HEAD`).
        // The count is only exact on a strictly linear chain — merges pull in
        // side-branch commits — so fall back to the onto-style label, which
        // claims no count, whenever exactness is not guaranteed.
        let children_count = this
            .active_repo()
            .filter(|repo| repo.id == repo_id)
            .and_then(|repo| {
                let head = repo.head_commit_id()?;
                match &repo.log {
                    Loadable::Ready(page) => gitcomet_core::squash::linear_first_parent_distance(
                        &page.commits,
                        &head,
                        commit_id,
                    ),
                    _ => None,
                }
            });
        let irebase_label: SharedString = match children_count {
            Some(count) => {
                let noun = if count == 1 { "child" } else { "children" };
                format!("Interactive rebase {count} {noun} of {short}").into()
            }
            None => format!("Interactive rebase {current_branch} onto {target_label}").into(),
        };
        items.push(ContextMenuItem::Entry {
            label: irebase_label,
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("I".into()),
            disabled: history_rewrite_disabled,
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
