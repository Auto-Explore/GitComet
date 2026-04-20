use super::*;

fn hotkey_hint(theme: AppTheme, debug_selector: &'static str, label: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child(label)
}

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let can_stash = this.can_submit_stash(cx);
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
                    components::Button::new("stash_cancel", "Cancel")
                        .separated_end_slot(hotkey_hint(theme, "stash_cancel_hint", "Esc"))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_inline_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("stash_go", "Stash")
                        .separated_end_slot(hotkey_hint(theme, "stash_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_stash)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_stash(window, cx);
                        }),
                ),
        )
}
