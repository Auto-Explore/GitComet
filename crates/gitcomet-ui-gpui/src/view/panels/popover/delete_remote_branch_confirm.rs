use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    remote: String,
    branch: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let full = format!("{remote}/{branch}");
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(420.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Delete remote branch?"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div().px_2().py_1().text_sm().child(
                div()
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.text_muted)
                    .child(full.clone()),
            ),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("This will permanently delete the branch from the remote."),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child(format!("git push {remote} --delete {branch}")),
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
                    components::Button::new("delete_remote_branch_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("delete_remote_branch_go", "Delete")
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::DeleteRemoteBranch {
                                repo_id,
                                remote: remote.clone(),
                                branch: branch.clone(),
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
