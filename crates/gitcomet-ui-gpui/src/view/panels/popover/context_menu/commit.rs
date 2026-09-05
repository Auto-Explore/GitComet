use super::*;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

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

/// Returns `true` when `commit_id` is already reachable from HEAD — i.e. it is
/// part of the current branch's history (including HEAD itself). The check walks
/// the parents of the loaded log page from HEAD. The right-clicked commit is by
/// construction present in that page, so a path through the page reaching it
/// proves ancestry; a truncated page can only produce a false negative, which
/// git corrects at merge time with "Already up to date".
fn commit_is_ancestor_of_head(this: &PopoverHost, repo_id: RepoId, commit_id: &CommitId) -> bool {
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return false;
    };
    repo_commit_is_ancestor_of_head(repo, commit_id)
}

fn repo_commit_is_ancestor_of_head(repo: &RepoState, commit_id: &CommitId) -> bool {
    let Some(head) = repo.head_commit_id() else {
        return false;
    };
    if head == *commit_id {
        return true;
    }
    let Loadable::Ready(page) = &repo.log else {
        return false;
    };
    let mut by_id: FxHashMap<CommitId, &Commit> = FxHashMap::default();
    for commit in &page.commits {
        by_id.insert(commit.id.clone(), commit);
    }
    let mut seen: FxHashSet<CommitId> = FxHashSet::default();
    let mut queue: VecDeque<CommitId> = VecDeque::from([head]);
    while let Some(current) = queue.pop_front() {
        if current == *commit_id {
            return true;
        }
        if !seen.insert(current.clone()) {
            continue;
        }
        if let Some(commit) = by_id.get(&current) {
            for parent in &commit.parent_ids {
                queue.push_back(parent.clone());
            }
        }
    }
    false
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
    let mut entry_tooltips = FxHashMap::default();
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
        .and_then(|repo| repo.navigation.comparison_mark.clone());
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
    let add_tag_disabled = !this.state.git_log_settings.show_history_tags;
    let add_tag_ix = items.len();
    items.push(ContextMenuItem::Entry {
        label: "Add tag…".into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: Some("T".into()),
        disabled: add_tag_disabled,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::CreateTagPrompt {
                repo_id,
                target: sha.clone(),
            },
        }),
    });
    if add_tag_disabled {
        entry_tooltips.insert(
            add_tag_ix,
            "Enable “Show tags in history view” in Settings > Git log to add tags.".into(),
        );
    }
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

    // "Merge into current" merges the right-clicked commit into the checked-out
    // branch through the shared MergeRef pipeline. It is a no-op when the commit
    // is already part of HEAD's history (including HEAD itself), so it is
    // disabled rather than letting git reject it as "Already up to date". It
    // also contends for the same repository operation slot as history rewrites.
    //
    // Resolved through `state.repos` rather than `active_repo()`, and through
    // the dialog's own two helpers, because this entry opens that dialog: with
    // several repositories open, a right-click on a commit outside the active
    // one made the entry read "into HEAD" while the confirmation it opened read
    // "into main", and left the entry enabled for a repository the dialog was
    // about to refuse. `commit_is_ancestor_of_head` already looks the repository
    // up this way.
    let merge_repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let merge_into_current_disabled =
        !super::super::merge_commit_confirm::merge_commit_repo_is_ready(merge_repo)
            || commit_is_ancestor_of_head(this, repo_id, commit_id);
    let merge_destination =
        super::super::merge_commit_confirm::merge_commit_destination_label(merge_repo);
    items.push(ContextMenuItem::Entry {
        label: format!("Merge {short} into {merge_destination}").into(),
        icon: Some("icons/swap.svg".into()),
        shortcut: Some("M".into()),
        disabled: merge_into_current_disabled,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::MergeCommitConfirm {
                repo_id,
                commit_id: commit_id.clone(),
            },
        }),
    });

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

    ContextMenuModel::new(items).with_entry_tooltips(entry_tooltips)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitParentIds, LogPage, RepoSpec};
    use gitcomet_state::model::{Loadable, RepoId, RepoState};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn repo_state() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: CommitId(id.into()),
            parent_ids: parents
                .iter()
                .map(|p| CommitId((*p).into()))
                .collect::<CommitParentIds>(),
            summary: "summary".into(),
            author: "author".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    fn with_log(mut repo: RepoState, head: &str, commits: Vec<Commit>) -> RepoState {
        repo.detached_head_commit = Some(CommitId(head.into()));
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits,
            next_cursor: None,
        }));
        repo
    }

    #[test]
    fn head_commit_is_its_own_ancestor() {
        let repo = with_log(repo_state(), "a", vec![commit("a", &[])]);

        assert!(repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("a".into())
        ));
    }

    #[test]
    fn commit_reachable_through_parent_chain_is_ancestor() {
        // c -> b -> a (HEAD is c)
        let repo = with_log(
            repo_state(),
            "c",
            vec![commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])],
        );

        assert!(repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("a".into())
        ));
    }

    #[test]
    fn commit_on_unrelated_branch_is_not_ancestor() {
        // HEAD is b, which does not descend from the side-branch commit x
        let repo = with_log(
            repo_state(),
            "b",
            vec![commit("b", &["a"]), commit("a", &[])],
        );

        assert!(!repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("x".into())
        ));
    }

    #[test]
    fn no_head_commit_is_not_ancestor() {
        let repo = repo_state();

        assert!(!repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("a".into())
        ));
    }

    #[test]
    fn merge_entry_disabled_when_commit_is_ancestor_of_head() {
        let repo = with_log(
            repo_state(),
            "c",
            vec![commit("c", &["b"]), commit("b", &[])],
        );

        assert!(repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("b".into())
        ));
        assert!(!repo_commit_is_ancestor_of_head(
            &repo,
            &CommitId("missing".into())
        ));
    }
}
