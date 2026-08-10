<<<<<<< New base: Support explicit commit ranges when cherry-picking onto a new branch (#17)
use super::*;

/// One picker row for the Cherry-pick prompt: a chromeless search input that
/// expands into a branch/ref picker while focused, exactly like the create
/// branch prompt's source row.
fn picker_row(
    this: &mut PopoverHost,
    theme: AppTheme,
    label: &'static str,
    input: &Entity<components::TextInput>,
    on_select: impl Fn(&mut PopoverHost, String, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>)
        + 'static,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let is_focused = input
        .read_with(cx, |input, _| input.focus_handle())
        .is_focused(window);
    input.update(cx, |input, cx| {
        input.set_chromeless(is_focused, cx);
        input.set_leading_icon(is_focused.then_some("icons/git_branch.svg"), cx);
    });

    if is_focused {
        let refs = this.active_branch_ref_picker_items(true, true);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(label),
            )
            .child(
                div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                    components::BranchRefPicker::new(
                        input.clone(),
                        this.picker_prompt_scroll.clone(),
                        refs,
                    )
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No matches")
                    .max_height(scaled_px(240.0))
                    .selected_index(this.branch_picker_selected_index)
                    .select_on_mouse_down()
                    .render(theme, ui_scale_percent, cx, on_select),
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
                    .child(label),
            )
            .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(input.clone()))
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scaled_px = super::popover_scaled_px_fn(cx);
    let can_submit = this.cherry_pick_can_submit(cx);
    let source = this.cherry_pick_source_target.trim().to_string();
    let range = this.cherry_pick_range_target.trim().to_string();
    let same_ref_hint = !source.is_empty() && source == range;

    let source_input = this
        .cherry_pick_source_search_input
        .clone()
        .expect("cherry_pick_source_search_input must be initialized");
    let range_input = this
        .cherry_pick_range_search_input
        .clone()
        .expect("cherry_pick_range_search_input must be initialized");
    let base_input = this
        .cherry_pick_base_search_input
        .clone()
        .expect("cherry_pick_base_search_input must be initialized");

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title("Cherry-pick branch"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(
                    "Creates a new branch C from D, checks it out, and cherry-picks every commit unique to A relative to B (B..A, oldest first, merge commits skipped). B must be an ancestor of A.",
                ),
        )
        .child(picker_row(
            this,
            theme,
            "Source ref (A)",
            &source_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_source_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(picker_row(
            this,
            theme,
            "Range ref (B)",
            &range_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_range_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(picker_row(
            this,
            theme,
            "Base branch (D)",
            &base_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_base_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(input_label(theme, "New branch name (C)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.cherry_pick_name_input.clone()),
        )
        .when(same_ref_hint, |this| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_sm()
                    .text_color(theme.colors.warning)
                    .child("Source and range are the same — there is nothing to cherry-pick."),
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
                    cancel_button(
                        "cherry_pick_range_cancel",
                        "cherry_pick_range_cancel_hint",
                        theme,
                    )
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("cherry_pick_range_go", "Create & cherry-pick")
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_submit)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_cherry_pick_range(window, cx);
                        }),
                ),
        )
}
|||||||
=======
use super::*;

/// One picker row for the Cherry-pick prompt: a chromeless search input that
/// expands into a branch/ref picker while focused, exactly like the create
/// branch prompt's source row.
fn picker_row(
    this: &mut PopoverHost,
    theme: AppTheme,
    label: &'static str,
    input: &Entity<components::TextInput>,
    on_select: impl Fn(&mut PopoverHost, String, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>)
        + 'static,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let is_focused = input
        .read_with(cx, |input, _| input.focus_handle())
        .is_focused(window);
    input.update(cx, |input, cx| {
        input.set_chromeless(is_focused, cx);
        input.set_leading_icon(is_focused.then_some("icons/git_branch.svg"), cx);
    });

    if is_focused {
        let refs = this.active_branch_ref_picker_items(true, true);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(label),
            )
            .child(
                div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                    components::BranchRefPicker::new(
                        input.clone(),
                        this.picker_prompt_scroll.clone(),
                        refs,
                    )
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No matches")
                    .max_height(scaled_px(240.0))
                    .selected_index(this.branch_picker_selected_index)
                    .select_on_mouse_down()
                    .render(theme, ui_scale_percent, cx, on_select),
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
                    .child(label),
            )
            .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(input.clone()))
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scaled_px = super::popover_scaled_px_fn(cx);
    let can_submit = this.cherry_pick_can_submit(cx);
    let source = this.cherry_pick_source_target.trim().to_string();
    let base = this.cherry_pick_base_target.trim().to_string();
    let same_branch_hint = !source.is_empty() && source == base;

    let source_input = this
        .cherry_pick_source_search_input
        .clone()
        .expect("cherry_pick_source_search_input must be initialized");
    let base_input = this
        .cherry_pick_base_search_input
        .clone()
        .expect("cherry_pick_base_search_input must be initialized");

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title("Cherry-pick branch"))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(
                    "Creates a new branch C from B, checks it out, and cherry-picks every commit unique to A (oldest first, merge commits skipped).",
                ),
        )
        .child(picker_row(
            this,
            theme,
            "Source branch (A)",
            &source_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_source_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(picker_row(
            this,
            theme,
            "Base branch (B)",
            &base_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_base_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(input_label(theme, "New branch name (C)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.cherry_pick_name_input.clone()),
        )
        .when(same_branch_hint, |this| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_sm()
                    .text_color(theme.colors.warning)
                    .child("Source and base are the same — there is nothing to cherry-pick."),
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
                    cancel_button(
                        "cherry_pick_range_cancel",
                        "cherry_pick_range_cancel_hint",
                        theme,
                    )
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("cherry_pick_range_go", "Create & cherry-pick")
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_submit)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_cherry_pick_range(window, cx);
                        }),
                ),
        )
}
>>>>>>> Current commit: Add cherry-pick branch A onto B as new branch C from the action bar
