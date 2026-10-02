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

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    src_ix: usize,
    cx: &gpui::App,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Hunk".into())];
    items.push(ContextMenuItem::Separator);

    let pane = this.main_pane.read(cx);
    let diff_target = pane.rendered_diff_target();
    let (disabled, label, icon, shortcut) = diff_hunk_primary_metadata(diff_target);
    let patch = this.build_unified_patch_for_hunk_src_ix(repo_id, src_ix, cx);

    items.push(ContextMenuItem::Entry {
        label: label.into(),
        icon: Some(icon.into()),
        shortcut: shortcut.map(Into::into),
        disabled: disabled || patch.is_none(),
        action: Box::new(diff_hunk_primary_action(repo_id, src_ix, diff_target)),
    });

    let is_unstaged = diff_target.is_some_and(|target| {
        matches!(
            target,
            DiffTarget::WorkingTree {
                area: DiffArea::Unstaged,
                ..
            }
        )
    });

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

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstaged_target_uses_stage_shortcut_and_action() {
        let target =
            DiffTarget::working_tree(std::path::PathBuf::from("src/lib.rs"), DiffArea::Unstaged);

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
        let target =
            DiffTarget::working_tree(std::path::PathBuf::from("src/lib.rs"), DiffArea::Staged);

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
