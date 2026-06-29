use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_tag(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let message_scroll = this.create_tag_message_scroll.clone();

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Create tag"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(format!("Target: {target}")),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_tag_input.clone()),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .pt_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Annotation message"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(
                    restrict_scroll_to_vertical_axis(
                        div()
                            .id("create_tag_message_scroll_surface")
                            .relative()
                            .w_full()
                            .min_w(px(0.0))
                            .max_h(scaled_px(140.0))
                            .pr(components::Scrollbar::visible_gutter(
                                message_scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            ))
                            .overflow_y_scroll()
                            .track_scroll(&message_scroll),
                    )
                    .child(this.create_tag_message_input.clone()),
                )
                .child(
                    components::Scrollbar::new("create_tag_message_scrollbar", message_scroll)
                        .render(theme),
                ),
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
                    components::Button::new("create_tag_cancel", "Cancel")
                        .focus_handle(this.create_tag_cancel_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "create_tag_cancel_hint",
                            "Esc",
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("create_tag_go", "Create")
                        .focus_handle(this.create_tag_submit_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "create_tag_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_create)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_create_tag(cx);
                        }),
                ),
        )
}
