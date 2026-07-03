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
                .child(format!("Rebase HEAD onto {onto}")),
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
                    components::Button::new("rebase_onto_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("rebase_onto_go", "Rebase")
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
