use super::*;

pub(super) fn model(repo_id: RepoId) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Submodules".into())];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Add submodule…".into(),
        icon: Some("icons/plus.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(repo_id, SubmodulePopoverKind::AddPrompt),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Update submodules".into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::UpdateSubmodules { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Open submodule…".into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(repo_id, SubmodulePopoverKind::OpenPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Remove submodule…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::submodule(repo_id, SubmodulePopoverKind::RemovePicker),
        }),
    });

    ContextMenuModel::new(items)
}
