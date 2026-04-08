use super::*;

fn squash_mode_buttons(
    this: &mut PopoverHost,
    theme: AppTheme,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let squash_enabled = this.subtree_squash_enabled;
    div()
        .px_2()
        .pb_2()
        .flex()
        .gap_2()
        .child(
            components::Button::new("subtree_pull_mode_squash", "Squash")
                .style(if squash_enabled {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.subtree_squash_enabled = true;
                    cx.notify();
                }),
        )
        .child(
            components::Button::new("subtree_pull_mode_full", "Full history")
                .style(if squash_enabled {
                    components::ButtonStyle::Outlined
                } else {
                    components::ButtonStyle::Filled
                })
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.subtree_squash_enabled = false;
                    cx.notify();
                }),
        )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    div()
        .flex()
        .flex_col()
        .w(px(640.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Pull subtree"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Path"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(path.display().to_string()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Repository"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.subtree_repository_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Reference"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.subtree_reference_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("History mode"),
        )
        .child(squash_mode_buttons(this, theme, cx))
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("subtree_pull_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("subtree_pull_go", "Pull")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let repository = this
                                .subtree_repository_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let reference = this
                                .subtree_reference_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            if repository.is_empty() || reference.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Subtree repository and reference are required".to_string(),
                                    cx,
                                );
                                return;
                            }
                            this.store.dispatch(Msg::PullSubtree {
                                repo_id,
                                repository,
                                reference,
                                path: path.clone(),
                                squash: this.subtree_squash_enabled,
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
