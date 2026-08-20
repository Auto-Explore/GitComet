use super::*;

fn same_as_fetch_toggle(
    theme: AppTheme,
    enabled: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    let scaled_px = super::popover_scaled_px_fn(cx);
    let border = if enabled {
        theme.colors.status.success.foreground
    } else {
        theme.colors.stroke.default
    };
    let background = if enabled {
        with_alpha(
            theme.colors.status.success.foreground,
            if theme.is_dark { 0.18 } else { 0.12 },
        )
    } else {
        gpui::rgba(0x00000000)
    };

    focusable_toggle_row(
        "remote_edit_same_push_url_toggle",
        "remote_edit_same_push_url_toggle",
        theme,
        focus_handle,
        cx,
    )
    .flex()
    .gap_2()
    .justify_start()
    .child(
        div()
            .size(scaled_px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(border)
            .rounded(scaled_px(theme.radii.control * 0.5))
            .bg(background)
            .when(enabled, |this| {
                this.child(crate::view::icons::svg_icon(
                    "icons/check.svg",
                    theme.colors.status.success.foreground,
                    scaled_px(10.0),
                ))
            }),
    )
    .child(div().text_sm().child("Push URL same as Fetch URL"))
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    name: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_submit = this.can_submit_remote_edit(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);
    let same_as_fetch = this.remote_edit_same_push_url;

    div()
        .flex()
        .flex_col()
        .w(scaled_px(680.0))
        .child(popover_title(format!("Edit remote '{name}'")))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        // Remote Name
        .child(input_label(theme, "Remote Name"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_name_input.clone()),
        )
        // Username helper
        .child(input_label(theme, "Username (embedded in URL)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_edit_username_input.clone()),
        )
        // Fetch URL
        .child(input_label(theme, "Fetch URL"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_url_edit_input.clone()),
        )
        // Toggle Push same as Fetch
        .child(
            div().px_2().py_1().child(
                same_as_fetch_toggle(
                    theme,
                    same_as_fetch,
                    &this.remote_edit_same_push_toggle_focus_handle,
                    cx,
                )
                .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                    this.remote_edit_same_push_url = !this.remote_edit_same_push_url;
                    if this.remote_edit_same_push_url {
                        let fetch_url = this
                            .remote_url_edit_input
                            .read_with(cx, |i, _| i.text().trim().to_string());
                        let theme = this.theme;
                        this.remote_edit_push_url_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(fetch_url, cx);
                            cx.notify();
                        });
                    }
                    cx.notify();
                })),
            ),
        )
        // Push URL (only shown or enabled when not same_as_fetch)
        .when(!same_as_fetch, |this_div| {
            this_div.child(input_label(theme, "Push URL")).child(
                div()
                    .px_2()
                    .pb_1()
                    .w_full()
                    .min_w(px(0.0))
                    .child(this.remote_edit_push_url_input.clone()),
            )
        })
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        // Footer buttons
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button("edit_remote_cancel", "edit_remote_cancel_hint", theme)
                        .focus_handle(this.remote_edit_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("edit_remote_go", "Save")
                        .focus_handle(this.remote_edit_focus.submit.clone())
                        .disabled(!can_submit)
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "edit_remote_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_remote_edit(cx);
                        }),
                ),
        )
}
