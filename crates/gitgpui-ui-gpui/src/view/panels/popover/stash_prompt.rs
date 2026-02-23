use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let is_empty = this
        .stash_message_input
        .read_with(cx, |i, _| i.text().trim().is_empty());

    div()
        .flex()
        .flex_col()
        .w(px(420.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Create stash"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.stash_message_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    zed::Button::new("stash_cancel", "Cancel")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    zed::Button::new("stash_go", "Stash")
                        .style(zed::ButtonStyle::Filled)
                        .disabled(is_empty)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            let message = this
                                .stash_message_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            if let Some(repo_id) = this.active_repo_id() {
                                this.store.dispatch(Msg::Stash {
                                    repo_id,
                                    message,
                                    include_untracked: true,
                                });
                            }
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
