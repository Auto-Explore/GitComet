use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_commit = this.can_submit_commit_prompt(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

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
                .child("Commit Changes"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .w_full()
                .min_w(px(0.0))
                .child(
                    restrict_scroll_to_vertical_axis(
                        div()
                            .id("commit_prompt_message_scroll_surface")
                            .relative()
                            .w_full()
                            .min_w(px(0.0))
                            .max_h(px(200.0))
                            .pr(components::Scrollbar::visible_gutter(
                                this.commit_prompt_message_scroll.clone(),
                                components::ScrollbarAxis::Vertical,
                            ))
                            .overflow_y_scroll()
                            .track_scroll(&this.commit_prompt_message_scroll),
                    )
                    .child(this.commit_prompt_message_input.clone()),
                )
                .child(
                    components::Scrollbar::new(
                        "commit_prompt_message_scrollbar",
                        this.commit_prompt_message_scroll.clone(),
                    )
                    .render(theme),
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
                    components::Button::new("commit_prompt_cancel", "Cancel")
                        .focus_handle(this.commit_prompt_cancel_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "commit_prompt_cancel_hint",
                            "Esc",
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("commit_prompt_submit", "Commit")
                        .focus_handle(this.commit_prompt_submit_focus_handle.clone())
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_commit)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            this.submit_commit_prompt(window, cx);
                        }),
                ),
        )
}
