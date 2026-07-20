use super::*;

fn short_sha(id: &gitcomet_core::domain::CommitId) -> &str {
    id.as_ref().get(0..8).unwrap_or(id.as_ref())
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    // Recompute eligibility from live state each render: the selection or the
    // log may have changed while the prompt is open. Prefill of the inputs is
    // handled outside the render path (see `sync_squash_prompt_prefill`).
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let plan = this.squash_plan_for_repo_id(repo_id);
    let preview = repo
        .map(|repo| repo.history_state.squash_preview.clone())
        .unwrap_or(Loadable::NotLoaded);

    let cancel_button = |this: &PopoverHost, cx: &mut gpui::Context<PopoverHost>| {
        super::cancel_button("squash_cancel", "squash_cancel_hint", theme)
            .focus_handle(this.squash_cancel_focus_handle.clone())
            .on_click(theme, cx, |this, _e, window, cx| {
                this.dismiss_prompt_popover(window, cx);
            })
    };

    let Some(plan) = plan else {
        return div()
            .flex()
            .flex_col()
            .w(scaled_px(420.0))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Squash commits"),
            )
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child("The selected commits are no longer squashable."),
            )
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(cancel_button(this, cx)),
            );
    };

    let message_empty = this
        .squash_message_input
        .read_with(cx, |input, _| input.text().trim().is_empty());

    let summary = if plan.head == plan.actual_head {
        format!(
            "{}..{} → one commit on HEAD",
            short_sha(&plan.oldest),
            short_sha(&plan.head)
        )
    } else {
        format!(
            "{}..{} → one commit · rewriting commits above",
            short_sha(&plan.oldest),
            short_sha(&plan.head)
        )
    };
    let message_hint: Option<SharedString> = match &preview {
        Loadable::Loading | Loadable::NotLoaded => Some("Building combined message…".into()),
        Loadable::Error(e) => Some(format!("Could not build the combined message: {e}").into()),
        Loadable::Ready(_) => None,
    };

    let description_scroll = this.squash_description_scroll.clone();
    let count = plan.commit_count;

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
                .child(format!("Squash {count} commits")),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(summary),
        )
        .child(
            div()
                .px_2()
                .pt_1()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("Commit message"),
                )
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .child(this.squash_message_input.clone()),
                ),
        )
        .child(
            div()
                .px_2()
                .pt_1()
                .pb_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("Description"),
                )
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .child(
                            restrict_scroll_to_vertical_axis(
                                div()
                                    .id("squash_description_scroll_surface")
                                    .relative()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .max_h(scaled_px(180.0))
                                    .pr(components::Scrollbar::visible_gutter(
                                        description_scroll.clone(),
                                        components::ScrollbarAxis::Vertical,
                                    ))
                                    .overflow_y_scroll()
                                    .track_scroll(&description_scroll),
                            )
                            .child(this.squash_description_input.clone()),
                        )
                        .child(
                            components::Scrollbar::new(
                                "squash_description_scrollbar",
                                description_scroll,
                            )
                            .render(theme),
                        ),
                ),
        )
        .when_some(message_hint, |el, hint| {
            el.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child(hint),
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
                .child(cancel_button(this, cx))
                .child(
                    components::Button::new("squash_go", "Squash")
                        .focus_handle(this.squash_submit_focus_handle.clone())
                        .style(components::ButtonStyle::Filled)
                        .disabled(message_empty)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.submit_squash(cx);
                        }),
                ),
        )
}
