use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    ConfirmDialog::new("Remove submodule", DIALOG_420_WIDTH)
        .text(theme, path.display().to_string())
        .render(
            theme,
            dialog_cancel_button(
                "submodule_remove_cancel",
                "submodule_remove_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("submodule_remove_go", "Remove")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::RemoveSubmodule {
                        repo_id,
                        path: path.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
