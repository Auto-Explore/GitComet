use super::*;

pub(super) fn model(repo_id: RepoId) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Worktrees".into())];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Add worktree…".into(),
        icon: Some("icons/plus.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Refresh worktrees".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::LoadWorktrees { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open worktree…".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::OpenPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Remove worktree…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::RemovePicker),
        }),
    });

    ContextMenuModel::new(items)
}
