use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    ConfirmDialog::new("Pull: choose strategy", DIALOG_440_WIDTH)
        .text(
            theme,
            "Fast-forward isn't possible. Choose whether to merge or rebase.",
        )
        .text(
            theme,
            "Merge combines your changes with the upstream branch.",
        )
        .text(theme, "Rebase replays your commits on the upstream branch.")
        .render(
            theme,
            dialog_cancel_button(
                "pull_reconcile_cancel",
                "pull_reconcile_cancel_hint",
                theme,
                cx,
            ),
            div()
                .flex()
                .gap_1()
                .child(
                    components::Button::new("pull_reconcile_merge", "Merge")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::Pull {
                                repo_id,
                                mode: PullMode::Merge,
                            });
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("pull_reconcile_rebase", "Rebase")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::Pull {
                                repo_id,
                                mode: PullMode::Rebase,
                            });
                            this.close_popover(cx);
                        }),
                ),
            cx,
        )
}
