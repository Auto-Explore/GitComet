use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    kind: RemoteUrlKind,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let kind_label = match kind {
        RemoteUrlKind::Fetch => "fetch",
        RemoteUrlKind::Push => "push",
    };
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(format!("Edit remote URL ({kind_label})")),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(format!("Remote: {name}")),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_url_edit_input.clone()),
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
                    components::Button::new("edit_remote_url_cancel", "Cancel")
                        .focus_handle(this.remote_edit_cancel_focus_handle.clone())
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("edit_remote_url_go", "Save")
                        .focus_handle(this.remote_edit_submit_focus_handle.clone())
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let url = this
                                .remote_url_edit_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            if url.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Remote URL cannot be empty".to_string(),
                                    cx,
                                );
                                return;
                            }
                            this.store.dispatch(Msg::SetRemoteUrl {
                                repo_id,
                                name: name.clone(),
                                url,
                                kind,
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
