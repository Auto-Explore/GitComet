use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    index: usize,
    message: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let reference = format!("stash@{{{index}}}");
    let label = if message.is_empty() {
        reference.clone()
    } else {
        format!("{reference} {message}")
    };

    ConfirmDialog::new("Drop stash?", DIALOG_420_WIDTH)
        .mono_value(theme, label)
        .text(theme, "This permanently removes this stash entry.")
        .command(theme, format!("git stash drop {reference}"))
        .render(
            theme,
            cancel_button("stash_drop_confirm_cancel", "stash_drop_cancel_hint", theme).on_click(
                theme,
                cx,
                move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::LoadStashes { repo_id });
                    this.close_popover(cx);
                },
            ),
            components::Button::new("stash_drop_confirm_go", "Drop")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::DropStash { repo_id, index });
                    this.store.dispatch(Msg::LoadStashes { repo_id });
                    this.close_popover(cx);
                }),
            cx,
        )
}
