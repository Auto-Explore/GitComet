use super::*;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};

/// Stable topological order of `commits` (already oldest-first by page
/// position) using only parent edges within the set: every commit sorts
/// after its selected ancestors, and unrelated commits keep their input
/// order. The history is a DAG, so the scan always makes progress; the
/// defensive fallback appends any remainder rather than dropping commits.
fn topo_order_oldest_first(
    commits: Vec<(usize, &gitcomet_core::domain::Commit)>,
) -> Vec<(usize, &gitcomet_core::domain::Commit)> {
    let index_of: std::collections::HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(ix, (_, commit))| (commit.id.as_ref(), ix))
        .collect();
    let mut pending_parents = vec![0usize; commits.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); commits.len()];
    for (ix, (_, commit)) in commits.iter().enumerate() {
        for parent in &commit.parent_ids {
            if let Some(&parent_ix) = index_of.get(parent.as_ref()) {
                children[parent_ix].push(ix);
                pending_parents[ix] += 1;
            }
        }
    }

    let mut ordered = Vec::with_capacity(commits.len());
    let mut emitted = vec![false; commits.len()];
    while let Some(next) = (0..commits.len()).find(|&ix| !emitted[ix] && pending_parents[ix] == 0)
    {
        emitted[next] = true;
        ordered.push(commits[next]);
        for &child in &children[next] {
            pending_parents[child] -= 1;
        }
    }
    for (ix, item) in commits.iter().enumerate() {
        if !emitted[ix] {
            ordered.push(*item);
        }
    }
    ordered
}

fn multi_cherry_pick_plan(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
) -> Option<(Vec<InteractiveRebaseEntry>, Vec<(String, u8)>)> {
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
    // The page is sorted newest-first by commit time, which under clock skew
    // can place a parent above its child; merely reversing it would then
    // schedule the child before its parent and replay the picks reversed (or
    // hit an avoidable conflict). Reverse for the oldest-first baseline, then
    // topologically order by parent links so a parent always precedes its
    // child, keeping the page order as the tie-break.
    selected.reverse();
    let selected = topo_order_oldest_first(selected);

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
            .and_then(|row| row.lanes_now.get(usize::from(row.node_col)))
            .map(|lane| lane.color_ix)
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

#[cfg(test)]
mod tests {
    use super::topo_order_oldest_first;
    use gitcomet_core::domain::{Commit, CommitId};

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: CommitId(id.into()),
            parent_ids: parents.iter().map(|p| CommitId((*p).into())).collect(),
            summary: id.into(),
            author: "author".into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn topo_order_puts_parents_before_children_and_keeps_stable_order() {
        // Clock skew placed child `b` before its parent `a` in the input;
        // unrelated `x` must keep its position between them.
        let b = commit("b", &["a"]);
        let x = commit("x", &["outside"]);
        let a = commit("a", &["outside"]);
        let input = vec![(2, &b), (1, &x), (0, &a)];

        let ordered = topo_order_oldest_first(input)
            .into_iter()
            .map(|(_, commit)| commit.id.as_ref().to_string())
            .collect::<Vec<_>>();

        assert_eq!(ordered, ["x", "a", "b"]);
    }

    #[test]
    fn topo_order_keeps_already_valid_order() {
        let a = commit("a", &["outside"]);
        let b = commit("b", &["a"]);
        let c = commit("c", &["b"]);
        let input = vec![(2, &a), (1, &b), (0, &c)];

        let ordered = topo_order_oldest_first(input)
            .into_iter()
            .map(|(_, commit)| commit.id.as_ref().to_string())
            .collect::<Vec<_>>();

        assert_eq!(ordered, ["a", "b", "c"]);
    }
}
