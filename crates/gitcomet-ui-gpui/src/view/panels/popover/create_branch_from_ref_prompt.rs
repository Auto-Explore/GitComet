use super::*;

fn checkout_toggle(
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
        "create_branch_checkout_toggle",
        "create_branch_checkout_toggle",
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
    .child(div().text_sm().child("Checkout"))
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    target: String,
    source_selectable: bool,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_create = this.can_submit_create_branch(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let source_row = if source_selectable {
        let search = this
            .branch_picker_search_input
            .clone()
            .expect("branch_picker_search_input must be initialized");
        let is_focused = search
            .read_with(cx, |input, _| input.focus_handle())
            .is_focused(window);

        if is_focused {
            let branches: Vec<String> = this
                .active_repo()
                .map(|repo| {
                    let mut names: Vec<String> = vec!["HEAD".to_string()];
                    if let Loadable::Ready(branches) = &repo.branches {
                        names.extend(branches.iter().map(|b| b.name.clone()));
                    }
                    if let Loadable::Ready(tags) = &repo.tags {
                        names.extend(tags.iter().map(|t| t.name.clone()));
                    }
                    names
                })
                .unwrap_or_default();
            let items: Vec<SharedString> = branches.iter().map(|n| n.clone().into()).collect();

            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child("Source:"),
                )
                .child(
                    div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .tooltip_host(this.tooltip_host.clone())
                            .empty_text("No matches")
                            .max_height(scaled_px(240.0))
                            .selected_index(this.branch_picker_selected_index)
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                if let Some(name) = branches.get(ix).cloned() {
                                    let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
                                    this.handle_inline_branch_picker_select(name, repo_id, cx);
                                }
                            }),
                    ),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child("Source:"),
                )
                .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(search))
        }
    } else {
        div()
            .px_2()
            .py_1()
            .text_sm()
            .text_color(theme.colors.text_muted)
            .child(format!("Source branch: {target}"))
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title("Create branch"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(source_row)
        .child(input_label(theme, "New branch name"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(
            checkout_toggle(
                theme,
                this.create_branch_checkout_enabled,
                &this.create_branch_from_ref_checkout_focus_handle,
                cx,
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                this.create_branch_checkout_enabled = !this.create_branch_checkout_enabled;
                cx.notify();
            })),
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
                    cancel_button(
                        "create_branch_from_ref_cancel",
                        "create_branch_from_ref_cancel_hint",
                        theme,
                    )
                    .focus_handle(this.create_branch_from_ref_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("create_branch_from_ref_go", "Create")
                        .focus_handle(this.create_branch_from_ref_focus.submit.clone())
                        .separated_end_slot(hotkey_hint(
                            theme,
                            "create_branch_from_ref_go_hint",
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
