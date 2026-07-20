use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    ConfirmDialog::new("Remove remote", DIALOG_420_WIDTH)
        .text(theme, format!("Remote: {name}"))
        .render(
            theme,
            dialog_cancel_button(
                "remove_remote_cancel",
                "remove_remote_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("remove_remote_go", "Remove")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::RemoveRemote {
                        repo_id,
                        name: name.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
