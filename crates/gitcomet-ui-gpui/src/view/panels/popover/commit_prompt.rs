use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_commit = this.can_submit_commit_prompt(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title("Commit Changes"))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(
            div().px_2().py_1().w_full().min_w(px(0.0)).child(
                components::ScrollContainer::vertical(
                    "commit_prompt_message_scroll_surface",
                    "commit_prompt_message_scrollbar",
                    this.commit_prompt_message_scroll.clone(),
                    px(200.0),
                )
                .render(theme, this.commit_prompt_message_input.clone()),
            ),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button("commit_prompt_cancel", "commit_prompt_cancel_hint", theme)
                        .focus_handle(this.commit_prompt_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("commit_prompt_submit", "Commit")
                        .focus_handle(this.commit_prompt_focus.submit.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "commit_prompt_submit_hint",
                            crate::view::shortcut_labels::secondary_shortcut("Enter"),
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_commit)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            this.submit_commit_prompt(window, cx);
                        }),
                ),
        )
}
