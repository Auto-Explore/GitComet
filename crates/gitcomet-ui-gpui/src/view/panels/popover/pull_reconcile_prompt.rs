use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(440.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Pull: choose strategy"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child("Fast-forward isn't possible. Choose whether to merge or rebase."),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child("Merge: git pull --no-rebase"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child("Rebase: git pull --rebase"),
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
                    components::Button::new("pull_reconcile_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            components::Button::new("pull_reconcile_merge", "Merge")
                                .style(components::ButtonStyle::Filled)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    this.store.dispatch(Msg::Pull {
                                        repo_id,
                                        mode: PullMode::Merge,
                                    });
                                    this.popover = None;
                                    this.popover_anchor = None;
                                    cx.notify();
                                }),
                        )
                        .child(
                            components::Button::new("pull_reconcile_rebase", "Rebase")
                                .style(components::ButtonStyle::Outlined)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    this.store.dispatch(Msg::Pull {
                                        repo_id,
                                        mode: PullMode::Rebase,
                                    });
                                    this.popover = None;
                                    this.popover_anchor = None;
                                    cx.notify();
                                }),
                        ),
                ),
        )
}
