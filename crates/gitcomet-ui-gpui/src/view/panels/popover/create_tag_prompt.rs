use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_tag(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let message_scroll = this.create_tag_message_scroll.clone();

    // Derive input box colours the same way TextInputStyle::from_theme does.
    let input_bg = {
        let base = theme.colors.surface_bg_elevated;
        let mix = if theme.is_dark { 1.0_f32 } else { 0.0_f32 };
        gpui::Rgba {
            r: base.r + (mix - base.r) * 0.03,
            g: base.g + (mix - base.g) * 0.03,
            b: base.b + (mix - base.b) * 0.03,
            a: base.a,
        }
    };
    let border_idle = theme.colors.border;
    let hover_alpha = if theme.is_dark { 0.55_f32 } else { 0.40_f32 };
    let border_hover = crate::theme::with_alpha(theme.colors.text_muted, hover_alpha);
    let focus_alpha = if theme.is_dark { 0.98_f32 } else { 0.92_f32 };
    let border_focus = crate::theme::with_alpha(theme.colors.accent, focus_alpha);
    let radius = px(theme.radii.row);

    let focused = this
        .create_tag_message_input
        .read(cx)
        .focus_handle()
        .is_focused(window);

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
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .border_1()
                        .rounded(radius)
                        .bg(input_bg)
                        .when(focused, |d| d.border_color(border_focus))
                        .when(!focused, |d| {
                            d.border_color(border_idle)
                                .hover(move |s| s.border_color(border_hover))
                        })
                        .min_h(px(72.0))
                        .max_h(px(72.0))
                        .child(
                            restrict_scroll_to_vertical_axis(
                                div()
                                    .id("create_tag_message_scroll_surface")
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .w_full()
                                    .min_w(px(0.0))
                                    .px(px(8.0))
                                    .py(px(10.0))
                                    .overflow_y_scroll()
                                    .track_scroll(&message_scroll),
                            )
                            .child(this.create_tag_message_input.clone()),
                        )
                        .child(
                            components::Scrollbar::new(
                                "create_tag_message_scrollbar",
                                message_scroll,
                            )
                            .show_track()
                            .render(theme),
                        ),
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
