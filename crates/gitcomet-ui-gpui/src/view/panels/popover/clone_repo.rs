use super::*;

fn hotkey_hint(theme: AppTheme, debug_selector: &'static str, label: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child(label)
}

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let can_clone = this.can_submit_clone_repo(cx);
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

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
                .child("Clone repository"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Repository URL / Path"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.clone_repo_url_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Destination parent folder"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(this.clone_repo_parent_dir_input.clone()),
                )
                .child(
                    components::Button::new("clone_repo_browse", "Browse")
                        .focus_handle(this.clone_repo_browse_focus_handle.clone())
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _e, window, cx| {
                            cx.stop_propagation();
                            let view = cx.weak_entity();
                            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("Clone into folder".into()),
                            });

                            window
                                .spawn(cx, async move |cx| {
                                    let result = rx.await;
                                    let paths = match result {
                                        Ok(Ok(Some(paths))) => paths,
                                        Ok(Ok(None)) => return,
                                        Ok(Err(_)) | Err(_) => return,
                                    };
                                    let Some(path) = paths.into_iter().next() else {
                                        return;
                                    };
                                    let _ = view.update(cx, |this, cx| {
                                        this.clone_repo_parent_dir_input.update(cx, |input, cx| {
                                            input.set_text(path.display().to_string(), cx);
                                        });
                                        cx.notify();
                                    });
                                })
                                .detach();
                        }),
                ),
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
                    components::Button::new("clone_repo_cancel", "Cancel")
                        .focus_handle(this.clone_repo_cancel_focus_handle.clone())
                        .separated_end_slot(hotkey_hint(theme, "clone_repo_cancel_hint", "Esc"))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("clone_repo_go", "Clone")
                        .focus_handle(this.clone_repo_submit_focus_handle.clone())
                        .separated_end_slot(hotkey_hint(theme, "clone_repo_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_clone)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_clone_repo(cx);
                        }),
                ),
        )
}
