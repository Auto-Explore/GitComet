use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let width = super::LARGE_PICKER_WIDTH;
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let branches = repo
        .map(|r| &r.virtual_branches)
        .cloned()
        .unwrap_or_default();

    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Assign to virtual branch"),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .line_height(scaled_px(14.0))
                .child(
                    components::TruncatedText::path(path.display().to_string())
                        .id(("vb_picker_path", repo_id.0))
                        .text_color(theme.colors.text_muted)
                        .full_text_tooltip(this.tooltip_host.clone())
                        .render(cx),
                ),
        );

    let body: AnyElement = if branches.is_empty() {
        components::context_menu_label(
            theme,
            ui_scale_percent,
            "No virtual branches. Open the Virtual Branches panel to create one.",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element()
    } else {
        let mut list = div().flex().flex_col().w_full();
        for branch in branches.iter() {
            let branch_id = branch.id;
            let name = branch.name.to_string();
            let count = branch.paths.len();
            let path_for_row = path.clone();
            let row = div()
                .id(("vb_picker_row", branch_id))
                .debug_selector(move || format!("vb_picker_row_{branch_id}"))
                .h(scaled_px(28.0))
                .w_full()
                .flex()
                .items_center()
                .gap(scaled_px(8.0))
                .px(scaled_px(8.0))
                .rounded(px(theme.radii.row))
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(theme.hover_overlay()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .whitespace_nowrap()
                        .line_clamp(1)
                        .text_color(theme.colors.text)
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(format!("{count} file(s)")),
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    this.store.dispatch(Msg::AssignPathToVirtualBranch {
                        repo_id,
                        branch_id,
                        path: path_for_row.clone(),
                    });
                    this.close_popover(cx);
                }));
            list = list.child(row);
        }
        list.into_any_element()
    };

    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(body),
    )
}
