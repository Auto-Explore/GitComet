use super::*;

use crate::view::shortcut_labels::secondary_shortcut;

fn diff_hunk_primary_metadata(
    diff_target: Option<&DiffTarget>,
) -> (bool, &'static str, &'static str, Option<String>) {
    match diff_target {
        Some(DiffTarget::WorkingTree { area, .. }) => match area {
            DiffArea::Unstaged => (
                false,
                "Stage hunk",
                "icons/plus.svg",
                Some(secondary_shortcut("S")),
            ),
            DiffArea::Staged => (
                false,
                "Unstage hunk",
                "icons/minus.svg",
                Some(secondary_shortcut("U")),
            ),
        },
        _ => (true, "Stage/Unstage hunk", "icons/plus.svg", None),
    }
}

fn diff_hunk_primary_action(
    repo_id: RepoId,
    src_ix: usize,
    diff_target: Option<&DiffTarget>,
) -> ContextMenuAction {
    match diff_target {
        Some(DiffTarget::WorkingTree {
            area: DiffArea::Staged,
            ..
        }) => ContextMenuAction::UnstageHunk { repo_id, src_ix },
        _ => ContextMenuAction::StageHunk { repo_id, src_ix },
    }
}

pub(super) fn model(this: &PopoverHost, repo_id: RepoId, src_ix: usize) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Hunk".into())];
    items.push(ContextMenuItem::Separator);

    let diff_target = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.diff_state.diff_target.as_ref());
    let (disabled, label, icon, shortcut) = diff_hunk_primary_metadata(diff_target);

    items.push(ContextMenuItem::Entry {
        label: label.into(),
        icon: Some(icon.into()),
        shortcut: shortcut.map(Into::into),
        disabled,
        action: Box::new(diff_hunk_primary_action(repo_id, src_ix, diff_target)),
    });

    let is_unstaged = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.diff_state.diff_target.as_ref())
        .is_some_and(|target| {
            matches!(
                target,
                DiffTarget::WorkingTree {
                    area: DiffArea::Unstaged,
                    ..
                }
            )
        });
    let patch = this.build_unified_patch_for_hunk_src_ix(repo_id, src_ix);
    let move_patch = patch.clone();

    items.push(ContextMenuItem::Entry {
        label: "Discard hunk".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: Some(secondary_shortcut("D").into()),
        disabled: !is_unstaged || patch.is_none(),
        action: Box::new(ContextMenuAction::ApplyWorktreePatch {
            repo_id,
            patch: patch.unwrap_or_default(),
            reverse: true,
        }),
    });

    // Move the hunk into a virtual branch: the hunk is removed from the
    // worktree and parked in the branch's stored patch collection (recoverable
    // by applying the branch). Only available for unstaged worktree hunks.
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let has_branches = repo
        .map(|r| !r.virtual_branches.is_empty())
        .unwrap_or(false);
    let move_path = repo
        .and_then(|r| r.diff_state.diff_target.as_ref())
        .and_then(|target| match target {
            DiffTarget::WorkingTree { path, .. } => Some(path.clone()),
            _ => None,
        });
    items.push(ContextMenuItem::Entry {
        label: "Move hunk to virtual branch…".into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: !is_unstaged || move_patch.is_none() || move_path.is_none() || !has_branches,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::VirtualBranchMovePicker {
                repo_id,
                patch: move_patch.unwrap_or_default(),
                path: move_path.unwrap_or_default(),
            },
        }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstaged_target_uses_stage_shortcut_and_action() {
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };

        let (disabled, label, icon, shortcut) = diff_hunk_primary_metadata(Some(&target));
        assert!(!disabled);
        assert_eq!(label, "Stage hunk");
        assert_eq!(icon, "icons/plus.svg");
        assert_eq!(shortcut, Some(secondary_shortcut("S")));
        assert!(matches!(
            diff_hunk_primary_action(RepoId(9), 4, Some(&target)),
            ContextMenuAction::StageHunk {
                repo_id,
                src_ix: 4
            } if repo_id == RepoId(9)
        ));
    }

    #[test]
    fn staged_target_uses_unstage_shortcut_and_action() {
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Staged,
        };

        let (disabled, label, icon, shortcut) = diff_hunk_primary_metadata(Some(&target));
        assert!(!disabled);
        assert_eq!(label, "Unstage hunk");
        assert_eq!(icon, "icons/minus.svg");
        assert_eq!(shortcut, Some(secondary_shortcut("U")));
        assert!(matches!(
            diff_hunk_primary_action(RepoId(10), 5, Some(&target)),
            ContextMenuAction::UnstageHunk {
                repo_id,
                src_ix: 5
            } if repo_id == RepoId(10)
        ));
    }
}
