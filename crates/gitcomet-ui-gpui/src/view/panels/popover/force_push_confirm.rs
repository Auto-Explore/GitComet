use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let lease = this.force_push_lease.clone();
    let dialog = ConfirmDialog::new("Force push", DIALOG_420_WIDTH);
    let dialog = if let Some(lease) = lease.as_ref() {
        dialog
            .text(
                theme,
                format!(
                    "This will update {}/{} only if it still points at {} and {} is still checked out at {}.",
                    lease.remote, lease.branch, lease.expected, lease.local_branch, lease.local_head
                ),
            )
            .command(
                theme,
                format!(
                    "git push --force-with-lease=refs/heads/{}:{} {} {}:refs/heads/{}",
                    lease.branch, lease.expected, lease.remote, lease.local_head, lease.branch
                ),
            )
    } else {
        dialog.text(
            theme,
            "The upstream branch's tip has not loaded, so there is nothing to check the push against.",
        )
    };
    dialog.render(
        theme,
        dialog_cancel_button("force_push_cancel", "force_push_cancel_hint", theme, cx),
        components::Button::new("force_push_go", "Force push with lease")
            .style(components::ButtonStyle::Danger)
            .disabled(lease.is_none())
            .on_click(theme, cx, move |this, _e, _w, cx| {
                if let Some(lease) = lease.clone() {
                    this.store
                        .dispatch(Msg::ForcePushWithLease { repo_id, lease });
                    this.close_popover(cx);
                }
            }),
        cx,
    )
}
