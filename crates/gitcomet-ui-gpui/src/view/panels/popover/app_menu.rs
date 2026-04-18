use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let close = cx.listener(|this, _e: &ClickEvent, _w, cx| this.close_popover(cx));

    let active_repo_id = this.active_repo().map(|r| r.id);

    let separator = || {
        div()
            .h(px(1.0))
            .w_full()
            .bg(theme.colors.border)
            .my(scaled_px(4.0))
    };

    let section_label = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .px(scaled_px(8.0))
            .pt(scaled_px(6.0))
            .pb(scaled_px(4.0))
            .text_xs()
            .line_height(scaled_px(14.0))
            .text_color(theme.colors.text_muted)
            .child(text)
    };

    let entry = |id: &'static str, label: SharedString, disabled: bool| {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .min_h(components::control_height_md(ui_scale_percent))
            .px(scaled_px(8.0))
            .py(scaled_px(4.0))
            .flex()
            .items_center()
            .text_sm()
            .line_height(scaled_px(18.0))
            .when(!disabled, |d| {
                d.cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(theme.colors.hover))
                    .active(move |s| s.bg(theme.colors.active))
            })
            .when(disabled, |d| {
                d.text_color(theme.colors.text_muted)
                    .cursor(CursorStyle::Arrow)
            })
            .child(label)
    };

    let mut install_desktop = div()
        .id("app_menu_install_desktop")
        .debug_selector(|| "app_menu_install_desktop".to_string())
        .min_h(components::control_height_md(ui_scale_percent))
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .items_center()
        .text_sm()
        .line_height(scaled_px(18.0))
        .child("Install desktop integration");

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        install_desktop = install_desktop.on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            this.install_linux_desktop_integration(cx);
            this.close_popover(cx);
        }));
        install_desktop = install_desktop
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.colors.hover))
            .active(move |s| s.bg(theme.colors.active));
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        install_desktop = install_desktop
            .text_color(theme.colors.text_muted)
            .cursor(CursorStyle::Arrow);
    }

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(200.0))
        .child(section_label("app_menu_app_section", "Application"))
        .child(
            entry("app_menu_settings", "Settings…".into(), false).on_click(cx.listener(
                |this, _e: &ClickEvent, _window, cx| {
                    cx.defer(crate::view::open_settings_window);
                    this.close_popover(cx);
                },
            )),
        )
        .child(separator())
        .child(section_label("app_menu_patches_section", "Patches"))
        .child(
            entry(
                "app_menu_apply_patch",
                "Apply patch…".into(),
                active_repo_id.is_none(),
            )
            .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
                let Some(repo_id) = active_repo_id else {
                    return;
                };
                cx.stop_propagation();
                let view = cx.weak_entity();
                let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Select patch file".into()),
                });
                window
                    .spawn(cx, async move |cx| {
                        let result = rx.await;
                        let paths = match result {
                            Ok(Ok(Some(paths))) => paths,
                            Ok(Ok(None)) => return,
                            Ok(Err(_)) | Err(_) => return,
                        };
                        let Some(patch) = paths.into_iter().next() else {
                            return;
                        };
                        let _ = view.update(cx, |this, cx| {
                            this.store.dispatch(Msg::ApplyPatch { repo_id, patch });
                            cx.notify();
                        });
                    })
                    .detach();
                this.close_popover(cx);
            })),
        )
        .child(separator())
        .child(install_desktop)
        .child(
            div()
                .id("app_menu_quit")
                .debug_selector(|| "app_menu_quit".to_string())
                .min_h(components::control_height_md(ui_scale_percent))
                .px(scaled_px(8.0))
                .py(scaled_px(4.0))
                .flex()
                .items_center()
                .text_sm()
                .line_height(scaled_px(18.0))
                .hover(move |s| s.bg(theme.colors.hover))
                .active(move |s| s.bg(theme.colors.active))
                .child("Quit")
                .on_click(cx.listener(|_this, _e: &ClickEvent, _w, cx| {
                    cx.quit();
                })),
        )
        .child(
            div()
                .id("app_menu_close")
                .debug_selector(|| "app_menu_close".to_string())
                .min_h(components::control_height_md(ui_scale_percent))
                .px(scaled_px(8.0))
                .py(scaled_px(4.0))
                .flex()
                .items_center()
                .text_sm()
                .line_height(scaled_px(18.0))
                .hover(move |s| s.bg(theme.colors.hover))
                .active(move |s| s.bg(theme.colors.active))
                .child("Close")
                .on_click(close),
        )
}
