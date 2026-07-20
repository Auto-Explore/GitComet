use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let lease = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.pending_force_push_lease.clone());
    let body = if let Some(lease) = lease.as_ref() {
        format!(
            "This will update {}/{} only if it still points at {} and {} is still checked out at {}.",
            lease.remote, lease.branch, lease.expected, lease.local_branch, lease.local_head
        )
    } else {
        "This will overwrite remote history if your branch has diverged.".to_string()
    };
    let command = if let Some(lease) = lease.as_ref() {
        format!(
            "git push --force-with-lease=refs/heads/{}:{} {} {}:refs/heads/{}",
            lease.branch, lease.expected, lease.remote, lease.local_head, lease.branch
        )
    } else {
        "git push --force-with-lease".to_string()
    };
    let button_label = if lease.is_some() {
        "Force push with lease"
    } else {
        "Force push"
    };

    ConfirmDialog::new("Force push", DIALOG_420_WIDTH)
        .text(theme, body)
        .command(theme, command)
        .render(
            theme,
            dialog_cancel_button("force_push_cancel", "force_push_cancel_hint", theme, cx),
            components::Button::new("force_push_go", button_label)
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    if let Some(lease) = lease.clone() {
                        this.store
                            .dispatch(Msg::ForcePushWithLease { repo_id, lease });
                    } else {
                        this.store.dispatch(Msg::ForcePush { repo_id });
                    }
                    this.close_popover(cx);
                }),
            cx,
        )
}
