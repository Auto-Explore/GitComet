use super::*;

/// Menu behind the "+" button after the repository tabs: open, clone, or
/// initialize a repository.
pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    // Text-alpha overlays: the canvas-tuned hover token has no contrast on
    // the elevated popover surface.
    let hover_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.07 } else { 0.05 });
    let active_overlay = with_alpha(theme.colors.text, if theme.is_dark { 0.11 } else { 0.08 });
    let entry = |id: &'static str, icon_path: &'static str, label: SharedString| {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .min_h(components::control_height_md(ui_scale_percent))
            .px(scaled_px(8.0))
            .py(scaled_px(4.0))
            .flex()
            .items_center()
            .gap(scaled_px(8.0))
            .text_sm()
            .line_height(scaled_px(18.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(hover_overlay))
            .active(move |s| s.bg(active_overlay))
            .child(
                div()
                    .w(scaled_px(16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(svg_icon(
                        icon_path,
                        theme.colors.text_muted,
                        scaled_px(14.0),
                    )),
            )
            .child(label)
    };

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(180.0))
        .child(
            entry(
                "add_repo_menu_open",
                "icons/disk.svg",
                "Open repository".into(),
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                this.close_popover(cx);
                let _ = this
                    .root_view
                    .update(cx, |root, cx| root.prompt_open_repo(window, cx));
            })),
        )
        .child(
            entry(
                "add_repo_menu_clone",
                "icons/cloud.svg",
                "Clone repository".into(),
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                let Some(anchor) = this.popover_anchor.clone() else {
                    this.close_popover(cx);
                    return;
                };
                this.open_popover(PopoverKind::CloneRepo, anchor, window, cx);
            })),
        )
        .child(
            entry(
                "add_repo_menu_init",
                "icons/git_branch.svg",
                "Initialize repository".into(),
            )
            .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                this.close_popover(cx);
                let _ = this
                    .root_view
                    .update(cx, |root, cx| root.prompt_init_repo(window, cx));
            })),
        )
}
