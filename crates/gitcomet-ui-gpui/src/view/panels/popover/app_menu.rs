use super::*;

use crate::view::shortcut_labels::secondary_shortcut;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let close = cx.listener(|this, _e: &ClickEvent, _w, cx| this.close_popover(cx));
    // Text-alpha overlays: the canvas-tuned hover token has no contrast on
    // the elevated popover surface.
    let hover_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.07 } else { 0.05 });
    let active_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.11 } else { 0.08 });

    let active_repo_id = this.active_repo().map(|r| r.id);
    let active_repo_workdir = this.active_repo().map(|r| r.spec.workdir.clone());
    let external_editor_configured = crate::external_editor::configured_setting().is_some();

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

    let entry =
        |id: &'static str, label: SharedString, shortcut: Option<SharedString>, disabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .min_h(components::control_height_md(ui_scale_percent))
                .px(scaled_px(8.0))
                .py(scaled_px(4.0))
                .flex()
                .items_center()
                .justify_between()
                .text_sm()
                .line_height(scaled_px(18.0))
                .when(!disabled, |d| {
                    d.cursor(CursorStyle::PointingHand)
                        .hover(move |s| s.bg(hover_overlay))
                        .active(move |s| s.bg(active_overlay))
                })
                .when(disabled, |d| {
                    d.text_color(theme.colors.text_muted)
                        .cursor(CursorStyle::Arrow)
                })
                .child(label)
                .when_some(shortcut, |d, s| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child(s),
                    )
                })
        };

    let desktop_integration_supported = cfg!(any(target_os = "linux", target_os = "freebsd"));
    #[allow(unused_mut)]
    let mut install_desktop = entry(
        "app_menu_install_desktop",
        "Install desktop integration".into(),
        None,
        !desktop_integration_supported,
    );

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        install_desktop = install_desktop.on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
            this.install_linux_desktop_integration(cx);
            this.close_popover(cx);
        }));
    }

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(200.0))
        .child(section_label("app_menu_app_section", "Application"))
        .child(
            entry(
                "app_menu_command_palette",
                "Command Palette".into(),
                Some(secondary_shortcut("P").into()),
                false,
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                this.close_popover(cx);
                window.dispatch_action(Box::new(ToggleCommandPalette), cx);
            })),
        )
        .child(
            entry(
                "app_menu_settings",
                "Settings…".into(),
                Some(secondary_shortcut(",").into()),
                false,
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, _window, cx| {
                cx.defer(crate::view::open_settings_window);
                this.close_popover(cx);
            })),
        )
        .when(external_editor_configured, |menu| {
            menu.child(
                entry(
                    "app_menu_open_in_code_editor",
                    "Open in code editor".into(),
                    Some(secondary_shortcut("Shift+E").into()),
                    active_repo_workdir.is_none(),
                )
                .on_click(cx.listener(
                    move |this, _e: &ClickEvent, _window, cx| {
                        let Some(path) = active_repo_workdir.clone() else {
                            return;
                        };
                        let _ = this.root_view.update(cx, |root, cx| {
                            root.open_path_in_external_code_editor(path, cx);
                        });
                        this.close_popover(cx);
                    },
                )),
            )
        })
        .child(separator())
        .child(section_label("app_menu_patches_section", "Patches"))
        .child(
            entry(
                "app_menu_apply_patch",
                "Apply patch…".into(),
                None,
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
            entry(
                "app_menu_quit",
                "Quit".into(),
                Some(secondary_shortcut("Q").into()),
                false,
            )
            .on_click(cx.listener(|_this, _e: &ClickEvent, _w, cx| {
                crate::app::quit_app_or_warn(cx);
            })),
        )
        .child(entry("app_menu_close", "Close".into(), None, false).on_click(close))
}
