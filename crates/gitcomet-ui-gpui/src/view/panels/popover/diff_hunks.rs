use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let close = cx.listener(|this, _e: &ClickEvent, _w, cx| this.close_popover(cx));

    let pane = this.main_pane.read(cx);
    let visible_len = pane.diff_visible_len();
    let mut items: Vec<SharedString> = Vec::with_capacity(visible_len);
    let mut targets: Vec<usize> = Vec::with_capacity(visible_len);
    let mut current_file: Option<String> = None;

    if !pane.is_file_diff_view_active() {
        for visible_ix in 0..visible_len {
            let Some(ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                continue;
            };
            let (src_ix, click_kind) = match pane.diff_view {
                DiffViewMode::Inline => {
                    let click_kind = pane
                        .diff_click_kinds
                        .get(ix)
                        .copied()
                        .unwrap_or(DiffClickKind::Line);
                    (ix, click_kind)
                }
                DiffViewMode::Split => {
                    let Some(row) = pane.patch_diff_split_row(ix) else {
                        continue;
                    };
                    let PatchSplitRow::Raw { src_ix, click_kind } = row else {
                        continue;
                    };
                    (src_ix, click_kind)
                }
            };

            let Some(line) = pane.patch_diff_row(src_ix) else {
                continue;
            };

            if matches!(click_kind, DiffClickKind::FileHeader) {
                current_file = parse_diff_git_header_path(line.text.as_ref());
            }

            if !matches!(click_kind, DiffClickKind::HunkHeader) {
                continue;
            }

            let label =
                if let Some(parsed) = parse_unified_hunk_header_for_display(line.text.as_ref()) {
                    let file = current_file.as_deref().unwrap_or("<file>").to_string();
                    let heading = parsed.heading.unwrap_or_default();
                    if heading.is_empty() {
                        format!("{file}: {} {}", parsed.old, parsed.new)
                    } else {
                        format!("{file}: {} {} {heading}", parsed.old, parsed.new)
                    }
                } else {
                    current_file.as_deref().unwrap_or("<file>").to_string()
                };

            items.push(label.into());
            targets.push(visible_ix);
        }
    }

    if let Some(search) = this.diff_hunk_picker_search_input.clone() {
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .items(items)
            .tooltip_host(this.tooltip_host.clone())
            .empty_text("No hunks")
            .max_height(scaled_px(260.0))
            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                let Some(&target) = targets.get(ix) else {
                    return;
                };
                this.main_pane.update(cx, |pane, cx| {
                    pane.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
                    pane.diff_selection_anchor = Some(target);
                    pane.diff_selection_range = Some((target, target));
                    cx.notify();
                });
                this.close_popover(cx);
            })
            .w(scaled_px(520.0))
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(
                div()
                    .id("diff_hunks_close")
                    .min_h(components::control_height_md(ui_scale_percent))
                    .px(scaled_px(8.0))
                    .py(scaled_px(4.0))
                    .hover(move |s| s.bg(theme.colors.hover))
                    .child("Close")
                    .on_click(close),
            )
    } else {
        let mut menu = div().flex().flex_col().min_w(scaled_px(520.0));
        for (ix, label) in items.into_iter().enumerate() {
            let target = targets.get(ix).copied().unwrap_or(0);
            menu = menu.child(
                div()
                    .id(("diff_hunk_item", ix))
                    .min_h(components::control_height_md(ui_scale_percent))
                    .px(scaled_px(8.0))
                    .py(scaled_px(4.0))
                    .flex()
                    .items_center()
                    .hover(move |s| s.bg(theme.colors.hover))
                    .child(
                        div()
                            .text_sm()
                            .line_height(scaled_px(18.0))
                            .line_clamp(1)
                            .child(label),
                    )
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.main_pane.update(cx, |pane, cx| {
                            pane.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
                            pane.diff_selection_anchor = Some(target);
                            pane.diff_selection_range = Some((target, target));
                            cx.notify();
                        });
                        this.close_popover(cx);
                    })),
            );
        }
        menu.child(
            div()
                .id("diff_hunks_close")
                .min_h(components::control_height_md(ui_scale_percent))
                .px(scaled_px(8.0))
                .py(scaled_px(4.0))
                .flex()
                .items_center()
                .hover(move |s| s.bg(theme.colors.hover))
                .child("Close")
                .on_click(close),
        )
    }
}
