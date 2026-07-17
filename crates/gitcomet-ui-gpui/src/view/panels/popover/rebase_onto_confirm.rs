use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    onto: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    // Name the branch being moved rather than the opaque "HEAD".
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let current_branch = repo
        .and_then(|r| match &r.head_branch {
            Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => Some(head.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "HEAD".to_string());
    // The menu entry that opens this popover is gated the same way, but the
    // popover can outlive that check (an operation may start while it is
    // open); git would refuse the rebase, so hold the button too.
    let history_rewrite_busy = repo.is_some_and(|r| r.history_rewrite_busy());

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(380.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Rebase"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(format!("Rebase {current_branch} onto {onto}")),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("This rewrites commit history. Avoid rebasing commits already pushed to a shared branch."),
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
                    super::cancel_button("rebase_onto_cancel", "rebase_onto_cancel_hint", theme)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("rebase_onto_go", "Rebase")
                        .disabled(history_rewrite_busy)
                        .focus_handle(this.rebase_onto_submit_focus_handle.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "rebase_onto_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::Rebase {
                                repo_id,
                                onto: onto.clone(),
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
