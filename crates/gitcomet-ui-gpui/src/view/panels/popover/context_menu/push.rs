use super::*;

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let repo_id = this.active_repo_id();
    let disabled = repo_id.is_none();
    let repo_id = repo_id.unwrap_or(RepoId(0));
    let tracking_branch_name = super::active_branch_tracking_upstream_name(this);

    ContextMenuModel::new(vec![
        ContextMenuItem::Header(super::action_menu_title(
            "Push",
            tracking_branch_name.as_deref(),
        )),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Push".into(),
            icon: Some("↑".into()),
            shortcut: None,
            disabled,
            action: Box::new(ContextMenuAction::Push { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Force push (with lease)…".into(),
            icon: Some("⚠".into()),
            shortcut: Some("F".into()),
            disabled,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::ForcePushConfirm { repo_id },
            }),
        },
    ])
}
