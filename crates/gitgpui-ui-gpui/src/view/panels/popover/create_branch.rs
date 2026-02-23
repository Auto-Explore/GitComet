use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let is_empty = this
        .create_branch_input
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
                .child("Create branch"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    zed::Button::new("create_branch_cancel", "Cancel")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    zed::Button::new("create_branch_go", "Create")
                        .style(zed::ButtonStyle::Filled)
                        .disabled(is_empty)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            let name = this
                                .create_branch_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            if let Some(repo_id) = this.active_repo_id()
                                && !name.is_empty()
                            {
                                this.store
                                    .dispatch(Msg::CreateBranchAndCheckout { repo_id, name });
                            }
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
