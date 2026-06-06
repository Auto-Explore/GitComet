use super::*;

fn hotkey_hint(theme: AppTheme, debug_selector: &'static str, label: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child(label)
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    index: usize,
    message: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_branch(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let reference = format!("stash@{{{index}}}");
    let source = if message.is_empty() {
        reference
    } else {
        format!("{reference} {message}")
    };

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
                .child("Create branch from stash"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .line_clamp(1)
                .child(source),
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
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("create_branch_from_stash_cancel", "Cancel")
                        .focus_handle(this.create_branch_cancel_focus_handle.clone())
                        .separated_end_slot(hotkey_hint(
                            theme,
                            "create_branch_from_stash_cancel_hint",
                            "Esc",
                        ))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("create_branch_from_stash_go", "Create")
                        .focus_handle(this.create_branch_submit_focus_handle.clone())
                        .separated_end_slot(hotkey_hint(
                            theme,
                            "create_branch_from_stash_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_create)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_create_branch(window, cx);
                        }),
                ),
        )
}
