use super::*;

fn advanced_toggle(theme: AppTheme, expanded: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id("submodule_add_advanced_toggle")
        .debug_selector(|| "submodule_add_advanced_toggle".to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .child(div().text_sm().child("Advanced"))
        .child(
            div()
                .text_sm()
                .font_family(UI_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child(if expanded { "^" } else { "v" }),
        )
}

fn force_toggle(theme: AppTheme, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id("submodule_add_force_toggle")
        .debug_selector(|| "submodule_add_force_toggle".to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .child(
            div()
                .text_sm()
                .child("Force reuse / bypass collision checks"),
        )
        .child(
            div()
                .text_sm()
                .text_color(if enabled {
                    theme.colors.success
                } else {
                    theme.colors.text_muted
                })
                .child(if enabled { "On" } else { "Off" }),
        )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let advanced_expanded = this.submodule_add_advanced_expanded;
    let force_enabled = this.submodule_force_enabled;

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
                .child("Add submodule"),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("URL"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_url_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Path (relative)"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_path_input.clone()),
        )
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("Branch (optional)"),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_branch_input.clone()),
        )
        .child(
            advanced_toggle(theme, advanced_expanded).on_click(cx.listener(|this, _e: &ClickEvent, _w, cx| {
                this.submodule_add_advanced_expanded = !this.submodule_add_advanced_expanded;
                cx.notify();
            })),
        )
        .when(advanced_expanded, |this_panel| {
            this_panel
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("Logical name (optional)"),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .w_full()
                        .min_w(px(0.0))
                        .child(this.submodule_name_input.clone()),
                )
                .child(
                    force_toggle(theme, force_enabled).on_click(cx.listener(
                        |this, _e: &ClickEvent, _w, cx| {
                            this.submodule_force_enabled = !this.submodule_force_enabled;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(
                            "Force reuses an existing local submodule git dir or bypasses Git's normal collision refusal.",
                        ),
                )
        })
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("submodule_add_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("submodule_add_go", "Add")
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let url = this
                                .submodule_url_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let path_text = this
                                .submodule_path_input
                                .read_with(cx, |i, _| i.text().trim().to_string());
                            let branch = this.submodule_branch_input.read_with(cx, |i, _| {
                                let text = i.text().trim().to_string();
                                if text.is_empty() { None } else { Some(text) }
                            });
                            let name = this.submodule_name_input.read_with(cx, |i, _| {
                                let text = i.text().trim().to_string();
                                if text.is_empty() { None } else { Some(text) }
                            });
                            let force = this.submodule_force_enabled;
                            if url.is_empty() || path_text.is_empty() {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    "Submodule URL and path are required".to_string(),
                                    cx,
                                );
                                return;
                            }
                            this.store.dispatch(Msg::AddSubmodule {
                                repo_id,
                                url,
                                path: std::path::PathBuf::from(path_text),
                                branch,
                                name,
                                force,
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
