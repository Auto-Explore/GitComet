use super::*;

use crate::view::shortcut_labels::secondary_shortcut;

pub(super) fn model(
    repo_id: RepoId,
    path: &std::path::Path,
    branch: Option<&str>,
    editor_available: bool,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Worktree".into())];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Open in new tab".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenRepo {
            path: path.to_path_buf(),
        }),
    });
    if editor_available {
        items.push(ContextMenuItem::Entry {
            label: "Open in code editor".into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: Some(secondary_shortcut("Shift+E").into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenInCodeEditor {
                repo_id: None,
                path: path.to_path_buf(),
            }),
        });
    }
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Remove…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(
                repo_id,
                WorktreePopoverKind::RemoveConfirm {
                    path: path.to_path_buf(),
                    branch: branch.map(ToOwned::to_owned),
                },
            ),
        }),
    });

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_includes_open_in_new_tab() {
        let repo_id = RepoId(1);
        let path = std::path::PathBuf::from("/tmp/worktree");
        let model = model(repo_id, &path, None, false);

        let open_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Open in new tab" =>
                {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected Open in new tab entry");

        assert!(matches!(
            open_action,
            ContextMenuAction::OpenRepo { path: open_path } if open_path == path
        ));
    }

    #[test]
    fn model_routes_remove_through_branch_aware_confirm_when_branch_is_provided() {
        let repo_id = RepoId(1);
        let path = std::path::PathBuf::from("/tmp/worktree");
        let model = model(repo_id, &path, Some("feature/workspace"), false);

        let remove_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Remove…" => {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected Remove entry");

        assert!(matches!(
            remove_action,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::Repo {
                    repo_id: rid,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::RemoveConfirm {
                        path: remove_path,
                        branch: Some(branch),
                    }),
                },
            } if rid == repo_id && remove_path == path && branch == "feature/workspace"
        ));
    }

    #[test]
    fn window_editor_override_exposes_the_worktree_action() {
        let path = std::path::Path::new("/tmp/worktree");
        let has_editor = |available| {
            model(RepoId(1), path, None, available)
                .items
                .iter()
                .any(|item| {
                    matches!(item, ContextMenuItem::Entry { action, .. }
                if matches!(**action, ContextMenuAction::OpenInCodeEditor { .. }))
                })
        };
        assert!(!has_editor(false));
        assert!(has_editor(true));
    }
}
