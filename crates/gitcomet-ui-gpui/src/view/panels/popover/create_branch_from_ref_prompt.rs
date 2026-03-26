use super::*;

fn checkout_toggle(theme: AppTheme, enabled: bool) -> gpui::Stateful<gpui::Div> {
    let border = if enabled {
        theme.colors.success
    } else {
        theme.colors.border
    };
    let background = if enabled {
        with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.18 } else { 0.12 },
        )
    } else {
        gpui::rgba(0x00000000)
    };

    div()
        .id("create_branch_checkout_toggle")
        .debug_selector(|| "create_branch_checkout_toggle".to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(theme.radii.row))
        .hover(move |this| this.bg(theme.colors.hover))
        .active(move |this| this.bg(theme.colors.active))
        .cursor(CursorStyle::PointingHand)
        .child(
            div()
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .bg(background)
                .when(enabled, |this| {
                    this.child(crate::view::icons::svg_icon(
                        "icons/check.svg",
                        theme.colors.success,
                        px(10.0),
                    ))
                }),
        )
        .child(div().text_sm().child("Checkout"))
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_branch(cx);

    div()
        .flex()
        .flex_col()
        .w(px(540.0))
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
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Source branch: {target}")),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("New branch name"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(
            checkout_toggle(theme, this.create_branch_checkout_enabled).on_click(cx.listener(
                |this, _e: &ClickEvent, _w, cx| {
                    this.create_branch_checkout_enabled = !this.create_branch_checkout_enabled;
                    cx.notify();
                },
            )),
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
                    components::Button::new("create_branch_from_ref_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_inline_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("create_branch_from_ref_go", "Create")
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_create)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_create_branch(window, cx);
                        }),
                ),
        )
}
