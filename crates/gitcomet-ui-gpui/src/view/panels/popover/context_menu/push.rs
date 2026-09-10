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

    let mut model = ContextMenuModel::new(vec![
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
    ]);
    for mode in gitcomet_core::tag_push::TagPushMode::ALL {
        let request = repo.and_then(|repo| super::super::tag_push::request(repo, mode));
        let preview = repo
            .zip(request.as_ref())
            .and_then(|(repo, request)| super::super::tag_push::preview(repo, request));
        let label = if request.is_some() {
            format!(
                "{} · {}",
                mode.label(),
                super::super::tag_push::summary(preview)
            )
        } else {
            mode.label().to_string()
        };
        let ix = model.items.len();
        model.items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: request.is_none() || push_disabled,
            action: Box::new(ContextMenuAction::PushWithTags { repo_id, mode }),
        });
        if let Some(request) = request.as_ref() {
            model
                .entry_tooltips
                .insert(ix, super::super::tag_push::tooltip(request, preview).into());
        }
    }
    // Last, and fenced off: the one entry here that rewrites published history
    // should not sit under a cursor aimed at the ordinary pushes above it.
    model.items.push(ContextMenuItem::Separator);
    model.items.push(ContextMenuItem::Entry {
        label: force_push_label.into(),
        icon: Some("icons/warning.svg".into()),
        shortcut: Some("F".into()),
        disabled: force_push_disabled,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::ForcePushConfirm { repo_id },
        }),
    });
    model
}
