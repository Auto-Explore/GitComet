use super::*;

pub(super) fn model(_this: &PopoverHost, repo_id: RepoId, name: &str) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header("Remote".into())];
    items.push(ContextMenuItem::Label(name.to_owned().into()));
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Fetch all".into(),
        icon: Some("icons/arrow_down.svg".into()),
        shortcut: Some("F".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::FetchAll { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Prune merged branches".into(),
        icon: Some("icons/broom.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: "Prune local tags".into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
    });
    items.push(ContextMenuItem::Separator);

    for (label, kind) in [
        ("Edit fetch URL…", RemoteUrlKind::Fetch),
        ("Edit push URL…", RemoteUrlKind::Push),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/pencil.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(
                    repo_id,
                    RemotePopoverKind::EditUrlPrompt {
                        name: name.to_owned(),
                        kind,
                    },
                ),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: "Remove remote…".into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::remote(
                repo_id,
                RemotePopoverKind::RemoveConfirm {
                    name: name.to_owned(),
                },
            ),
        }),
    });

    ContextMenuModel::new(items)
}
