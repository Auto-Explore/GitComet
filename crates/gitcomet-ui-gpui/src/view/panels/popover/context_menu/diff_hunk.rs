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

/// The workspace-assign entry for this hunk, when there is one to offer.
///
/// Assigning a hunk is a workspace action, so it is only offered when the diff
/// on screen really is an uncommitted change to a file. For a commit's diff the
/// question has no answer — that change is already committed to something — and
/// a hunk with no rendered diff has no fingerprint to record.
///
/// Pure, so the rule can be tested without standing up a popover host.
fn workspace_assign_action(
    repo_id: RepoId,
    src_ix: usize,
    diff_target: Option<&DiffTarget>,
    fingerprint: Option<gitcomet_core::workspace::HunkFingerprint>,
) -> Option<ContextMenuAction> {
    let DiffTarget::WorkingTree { path, .. } = diff_target? else {
        return None;
    };
    Some(ContextMenuAction::AssignHunkToWorkspace {
        repo_id,
        path: path.clone(),
        hunk: fingerprint?,
    })
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

    // Assigning a hunk is a workspace action, so it is only offered when the
    // diff on screen really is an uncommitted change to a file — not a commit's
    // diff, where "which branch commits this" has no answer.
    let fingerprint = pane
        .rendered_patch_diff_loadable()
        .and_then(|Loadable::Ready(diff)| {
            crate::view::diff_utils::hunk_fingerprint(diff.lines.as_slice(), src_ix)
        });
    if let Some(action) = workspace_assign_action(repo_id, src_ix, diff_target, fingerprint) {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: "Assign to a branch...".into(),
            icon: Some("icons/git_branch.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(action),
        });
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uncommitted_hunk_can_be_assigned_to_a_workspace_branch() {
        let target =
            DiffTarget::working_tree(std::path::PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
        let fingerprint = gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1);

        assert!(matches!(
            workspace_assign_action(RepoId(1), 3, Some(&target), Some(fingerprint)),
            Some(ContextMenuAction::AssignHunkToWorkspace {
                repo_id: RepoId(1),
                path,
                hunk,
            }) if path == std::path::PathBuf::from("src/lib.rs") && hunk == fingerprint
        ));
    }

    #[test]
    fn a_commits_diff_cannot_be_assigned_to_a_branch() {
        // The change is already committed to something, so there is no honest
        // answer to "which virtual branch commits this" — offering the entry
        // would be offering a question with no answer.
        let target = DiffTarget::commit(
            gitcomet_core::domain::CommitId("abc123".into()),
            std::path::PathBuf::from("src/lib.rs"),
        );
        let fingerprint = gitcomet_core::workspace::HunkFingerprint::of(&["NEW\n"], 1);
        assert!(workspace_assign_action(RepoId(1), 3, Some(&target), Some(fingerprint)).is_none());
    }

    #[test]
    fn a_hunk_with_no_rendered_diff_is_not_offered() {
        // Nothing rendered means no fingerprint to record, and an assignment
        // keyed by one would be an entry the user can fill in that never
        // resolves at commit time.
        let target =
            DiffTarget::working_tree(std::path::PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
        assert!(workspace_assign_action(RepoId(1), 3, Some(&target), None).is_none());
    }

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
