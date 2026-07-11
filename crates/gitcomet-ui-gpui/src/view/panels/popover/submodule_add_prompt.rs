use super::*;

fn advanced_toggle(
    theme: AppTheme,
    expanded: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    focusable_toggle_row(
        "submodule_add_advanced_toggle",
        "submodule_add_advanced_toggle",
        theme,
        focus_handle,
        cx,
    )
    .flex()
    .child(div().text_sm().child("Advanced"))
    .child(
        div()
            .text_sm()
            .font_family(UI_MONOSPACE_FONT_FAMILY)
            .text_color(theme.colors.text_muted)
            .child(if expanded { "^" } else { "v" }),
    )
}

fn force_toggle(
    theme: AppTheme,
    enabled: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    focusable_toggle_row(
        "submodule_add_force_toggle",
        "submodule_add_force_toggle",
        theme,
        focus_handle,
        cx,
    )
    .flex()
    .child(
        div()
            .text_sm()
            .child("Force reuse / bypass collision checks"),
    )
    .child(
        div()
            .text_sm()
            .text_color(if enabled {
                theme.colors.success
            } else {
                theme.colors.text_muted
            })
            .child(if enabled { "On" } else { "Off" }),
    )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let advanced_expanded = this.submodule_add_advanced_expanded;
    let force_enabled = this.submodule_force_enabled;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title("Add submodule"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(input_label(theme, "URL"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_url_input.clone()),
        )
        .child(input_label(theme, "Path (relative)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_path_input.clone()),
        )
        .child(input_label(theme, "Branch (optional)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_branch_input.clone()),
        )
        .child(
            advanced_toggle(
                theme,
                advanced_expanded,
                &this.submodule_advanced_focus_handle,
                cx,
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                this.submodule_add_advanced_expanded = !this.submodule_add_advanced_expanded;
                cx.notify();
            })),
        )
        .when(advanced_expanded, |this_panel| {
            this_panel
                .child(input_label(theme, "Logical name (optional)"))
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .w_full()
                        .min_w(px(0.0))
                        .child(this.submodule_name_input.clone()),
                )
                .child(
                    force_toggle(
                        theme,
                        force_enabled,
                        &this.submodule_force_focus_handle,
                        cx,
                    )
                    .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                        this.submodule_force_enabled = !this.submodule_force_enabled;
                        cx.notify();
                    })),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(
                            "Force reuses an existing local submodule git dir or bypasses Git's normal collision refusal.",
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
                    cancel_button("submodule_add_cancel", "submodule_add_cancel_hint", theme)
                        .focus_handle(this.submodule_cancel_focus_handle.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("submodule_add_go", "Add")
                        .focus_handle(this.submodule_submit_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "submodule_add_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_submodule_add(cx);
                        }),
                ),
        )
}
