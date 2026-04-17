use super::*;

fn text_field(
    theme: AppTheme,
    label: &'static str,
    input: Entity<components::TextInput>,
) -> gpui::Div {
    div()
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(label),
        )
        .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(input))
}

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
            components::Button::new("subtree_merge_mode_squash", "Squash")
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
            components::Button::new("subtree_merge_mode_full", "Full history")
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
    initial_path: Option<std::path::PathBuf>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;

    div()
        .flex()
        .flex_col()
        .w(px(680.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Merge subtree"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(text_field(
            theme,
            "Path (repo-relative)",
            this.subtree_path_input.clone(),
        ))
        .child(
            div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(if initial_path.is_some() {
                    "Edit the subtree prefix to merge into. The revision must point to a locally available subtree split ref or commit."
                } else {
                    "Choose the subtree prefix to merge into. The revision must point to a locally available subtree split ref or commit."
                }),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(text_field(
            theme,
            "Revision / ref",
            this.subtree_merge_revision_input.clone(),
        ))
        .child(text_field(
            theme,
            "Merge message (optional)",
            this.subtree_merge_message_input.clone(),
        ))
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
                    components::Button::new("subtree_merge_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("subtree_merge_go", "Merge")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let path_text = this
                                .subtree_path_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let revision = this
                                .subtree_merge_revision_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let message = this
                                .subtree_merge_message_input
                                .read_with(cx, |i, _| i.text().trim().to_string());

                            if path_text.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Subtree path is required".to_string(),
                                    cx,
                                );
                                return;
                            }
                            if revision.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Subtree merge revision is required".to_string(),
                                    cx,
                                );
                                return;
                            }

                            this.store.dispatch(Msg::MergeSubtree {
                                repo_id,
                                path: std::path::PathBuf::from(path_text),
                                revision,
                                squash: this.subtree_squash_enabled,
                                message: (!message.is_empty()).then_some(message),
                            });
                            this.close_popover(cx);
                        }),
                ),
        )
}
