use super::*;

fn annotated_toggle(
    theme: AppTheme,
    enabled: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
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

    focusable_toggle_row(
        "create_tag_annotated_toggle",
        "create_tag_annotated_toggle",
        theme,
        focus_handle,
        cx,
    )
    .flex()
    .gap_2()
    .justify_start()
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
    .child(div().text_sm().child("Annotated tag"))
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_tag(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);
    let message_scroll = this.create_tag_message_scroll.clone();
    let annotated = this.create_tag_annotated;

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title("Create tag"))
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
            annotated_toggle(
                theme,
                annotated,
                &this.create_tag_annotated_focus_handle,
                cx,
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                this.create_tag_annotated = !this.create_tag_annotated;
                cx.notify();
            })),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Annotated tags can be GPG signed and include a message"),
        )
        .when(annotated, |panel| {
            panel
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
                    div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                        components::ScrollContainer::vertical(
                            "create_tag_message_scroll_surface",
                            "create_tag_message_scrollbar",
                            message_scroll,
                            scaled_px(140.0),
                        )
                        .render(theme, this.create_tag_message_input.clone()),
                    ),
                )
        })
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button("create_tag_cancel", "create_tag_cancel_hint", theme)
                        .focus_handle(this.create_tag_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("create_tag_go", "Create")
                        .focus_handle(this.create_tag_focus.submit.clone())
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
