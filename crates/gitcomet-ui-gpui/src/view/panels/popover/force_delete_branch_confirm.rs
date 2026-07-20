use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    ConfirmDialog::new("Delete branch anyway?", DIALOG_420_WIDTH)
        .mono_value(theme, name.clone())
        .text(
            theme,
            "This will permanently delete the local branch, even if it is not fully merged.",
        )
        .command(theme, format!("git branch -D {name}"))
        .render(
            theme,
            dialog_cancel_button(
                "force_delete_branch_cancel",
                "force_delete_branch_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("force_delete_branch_go", "Delete anyway")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ForceDeleteBranch {
                        repo_id,
                        name: name.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
