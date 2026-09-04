use super::*;

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let repo_id = this.active_repo_id();
    let repo = this.active_repo();
    let push_disabled = repo.is_none_or(|repo| matches!(push_request(repo), PushRequest::NotReady));
    let force_push_disabled = repo.is_none_or(|repo| !head_branch_has_live_upstream(repo));
    let repo_id = repo_id.unwrap_or(RepoId(0));
    let tracking_branch_name = super::active_branch_tracking_upstream_name(this);
    let force_push_label = if this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.pending.force_push_lease.as_ref())
        .is_some()
    {
        "Force push published amend with lease…"
    } else {
        "Force push (with lease)…"
    };

    ContextMenuModel::new(vec![
        ContextMenuItem::Header(
            super::action_menu_title("Push", tracking_branch_name.as_deref()).into(),
        ),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Push".into(),
            icon: Some("icons/arrow_up.svg".into()),
            shortcut: None,
            disabled: push_disabled,
            action: Box::new(ContextMenuAction::Push { repo_id }),
        },
        ContextMenuItem::Entry {
            label: force_push_label.into(),
            icon: Some("icons/warning.svg".into()),
            shortcut: Some("F".into()),
            disabled: force_push_disabled,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::ForcePushConfirm { repo_id },
            }),
        },
    ])
}
