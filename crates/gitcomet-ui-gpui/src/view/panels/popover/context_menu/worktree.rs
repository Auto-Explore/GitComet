use super::*;

use crate::kit::components::secondary_shortcut;

pub(super) fn model(
    repo_id: RepoId,
    path: &std::path::Path,
    branch: Option<&str>,
    editor_available: bool,
    location_available: bool,
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
    items.push(ContextMenuItem::Entry {
        label: "Open folder location".into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: !location_available,
        action: Box::new(ContextMenuAction::OpenRepositoryLocation {
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

impl PopoverHost {
    /// Snapshot availability once per menu opening, away from the UI thread.
    /// A stat on an offline network share may never return; the menu remains
    /// usable with the location action disabled while the check is pending.
    pub(in super::super) fn prepare_worktree_menu_location(
        &mut self,
        kind: Option<&PopoverKind>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.worktree_menu_location_task = None;
        self.worktree_menu_location = None;
        let Some(PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu { path, .. }),
            ..
        }) = kind
        else {
            return;
        };
        let path = path.clone();
        self.worktree_menu_location = Some((path.clone(), false));
        let check_path = path.clone();
        let check = cx
            .background_executor()
            .spawn(async move { check_path.is_dir() });
        self.worktree_menu_location_task = Some(cx.spawn(async move |host, cx| {
            let available = check.await;
            let _ = host.update(cx, |host, cx| {
                if let Some((current_path, current_available)) = &mut host.worktree_menu_location
                    && *current_path == path
                {
                    *current_available = available;
                    cx.notify();
                }
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_location_uses_cached_availability_without_checking_disk() {
        let path = std::path::Path::new("/unavailable/checkout");
        for available in [false, true] {
            let menu = model(RepoId(1), path, None, false, available);
            let disabled = menu.items.iter().find_map(|item| match item {
                ContextMenuItem::Entry {
                    action, disabled, ..
                } if matches!(
                    action.as_ref(),
                    ContextMenuAction::OpenRepositoryLocation { .. }
                ) =>
                {
                    Some(*disabled)
                }
                _ => None,
            });
            assert_eq!(disabled, Some(!available));
        }
    }

    #[test]
    fn model_includes_open_in_new_tab() {
        let repo_id = RepoId(1);
        let path = std::path::PathBuf::from("/tmp/worktree");
        let model = model(repo_id, &path, None, false, true);

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
        let model = model(repo_id, &path, Some("feature/workspace"), false, true);

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
            model(RepoId(1), path, None, available, true)
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
