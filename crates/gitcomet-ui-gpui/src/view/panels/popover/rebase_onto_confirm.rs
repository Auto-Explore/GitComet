use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    onto: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    // Name the branch being moved rather than the opaque "HEAD".
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let current_branch = repo
        .and_then(|r| match &r.head_branch {
            Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => Some(head.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "HEAD".to_string());
    // The menu entry that opens this popover is gated the same way, but the
    // popover can outlive that check (an operation may start while it is
    // open); git would refuse the rebase, so hold the button too.
    let history_rewrite_busy = repo.is_some_and(|r| r.history_rewrite_busy());

    ConfirmDialog::new("Rebase", DIALOG_380_WIDTH)
        .text(theme, format!("Rebase {current_branch} onto {onto}"))
        .note(
            theme,
            "This rewrites commit history. Avoid rebasing commits already pushed to a shared branch.",
        )
        .render(
            theme,
            dialog_cancel_button("rebase_onto_cancel", "rebase_onto_cancel_hint", theme, cx),
            components::Button::new("rebase_onto_go", "Rebase")
                .disabled(history_rewrite_busy)
                .focus_handle(this.rebase_onto_submit_focus_handle.clone())
                .separated_end_slot(super::hotkey_hint(theme, "rebase_onto_go_hint", "Enter"))
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::Rebase {
                        repo_id,
                        onto: onto.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
