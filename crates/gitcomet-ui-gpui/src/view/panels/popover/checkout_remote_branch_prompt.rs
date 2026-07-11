use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    remote: String,
    branch: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let upstream = format!("{remote}/{branch}");
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title("Checkout remote branch"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Remote branch: {upstream}")),
        )
        .child(input_label(theme, "Local branch name"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button(
                        "checkout_remote_branch_cancel",
                        "checkout_remote_branch_cancel_hint",
                        theme,
                    )
                    .focus_handle(this.checkout_remote_branch_cancel_focus_handle.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("checkout_remote_branch_go", "Checkout")
                        .focus_handle(this.checkout_remote_branch_submit_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "checkout_remote_branch_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_checkout_remote_branch(cx);
                        }),
                ),
        )
}
