use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: &std::path::Path,
    has_conflict_markers: bool,
    unresolved_blocks: usize,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let path = path.to_path_buf();
    let has_unresolved_blocks = unresolved_blocks > 0;

    let title = match (has_conflict_markers, has_unresolved_blocks) {
        (true, true) => "Unresolved conflict content detected",
        (true, false) => "Unresolved conflict markers detected",
        (false, true) => "Unresolved conflict blocks detected",
        (false, false) => "Confirm staging",
    };

    let mut detail = String::new();
    if has_conflict_markers {
        detail.push_str(
            "The resolved text still contains conflict markers (<<<<<<<, =======, >>>>>>>). ",
        );
    }
    if has_unresolved_blocks {
        let block_word = if unresolved_blocks == 1 {
            "block is"
        } else {
            "blocks are"
        };
        detail.push_str(&format!(
            "{unresolved_blocks} conflict {block_word} still unresolved in the resolver."
        ));
    }
    if detail.is_empty() {
        detail.push_str("The file may still be in an unresolved state.");
    }
    detail.push_str(" Staging this file may leave it in a broken state.");
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .min_w(scaled_px(360.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(detail),
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
                    components::Button::new("conflict_stage_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new("conflict_stage_anyway", "Stage anyway")
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            let text = this
                                .main_pane
                                .update(cx, |main, cx| main.conflict_resolver_save_contents(cx));
                            this.store.dispatch(Msg::SaveWorktreeFile {
                                repo_id,
                                path: path.clone(),
                                contents: text,
                                stage: true,
                            });
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
