use super::*;

fn checkout_toggle(
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
    .child(div().text_size(theme.ui_text(14.0)).child("Checkout"))
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
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    let source_row = if source_selectable {
        let search = this
            .branch_picker_search_input
            .clone()
            .expect("branch_picker_search_input must be initialized");
        let is_focused = search
            .read_with(cx, |input, _| input.focus_handle())
            .is_focused(window);
        search.update(cx, |input, cx| {
            input.set_chromeless(is_focused, cx);
            input.set_leading_icon(is_focused.then_some("icons/git_branch.svg"), cx);
        });

        if is_focused {
            let query = search.read(cx).text().trim().to_string();
            let built = branch_picker::ref_rows_cached(
                this,
                branch_picker::RefRowsSpec::source_ref(),
                &query,
            );
            let names = std::rc::Rc::clone(&built.payloads);

            div()
                .flex()
                .flex_col()
                .child(super::popover_detail(theme, "Source:"))
                .child(
                    div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                        branch_picker::ref_picker_prompt(
                            search,
                            this.picker_prompt_scroll.clone(),
                            &built,
                            cx,
                        )
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text("No matches")
                        .max_height(scaled_px(branch_picker::REF_PICKER_LIST_MAX_HEIGHT_PX))
                        .selected_index(this.branch_picker_selected_index)
                        .select_on_mouse_down()
                        .render(
                            theme,
                            ui_scale_percent,
                            cx,
                            move |this, ix, _e, window, cx| {
                                let Some(name) = names.get(ix).cloned() else {
                                    return;
                                };
                                let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
                                this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                            },
                        ),
                    ),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .child(super::popover_detail(theme, "Source:"))
                .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(search))
        }
    } else {
        super::popover_detail(theme, format!("Source branch: {target}"))
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title(theme, "Create branch"))
        .child(super::popover_rule(theme))
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
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
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
