use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let can_stash = this.can_submit_stash(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(theme, "Create stash"))
        .child(super::popover_rule(theme))
        .child(
            div()
                .px_2()
                .py_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.stash_message_input.clone()),
        )
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button("stash_cancel", "stash_cancel_hint", theme)
                        .focus_handle(this.stash_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("stash_go", "Stash")
                        .focus_handle(this.stash_focus.submit.clone())
                        .separated_end_slot(super::hotkey_hint(theme, "stash_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_stash)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_stash(window, cx);
                        }),
                ),
        )
}
