use super::super::super::path_display;
use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::PICKER_WIDTH;

    if let Some(search) = this.repo_picker_search_input.clone() {
        let repo_ids = this.state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
        let items = this
            .state
            .repos
            .iter()
            .map(|r| {
                components::PickerPromptItem::single(
                    path_display::path_display_shared(&r.spec.workdir),
                    components::TextTruncationProfile::Path,
                )
            })
            .collect::<Vec<_>>();
        let active_index = this
            .state
            .active_repo
            .and_then(|active| repo_ids.iter().position(|id| *id == active));

        components::context_menu(
            theme,
            components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                .items(items)
                .tooltip_host(this.tooltip_host.clone())
                .empty_text("No repositories")
                .max_height(scaled_px(260.0))
                .selected_index(this.repo_picker_selected_index)
                .marked_index(active_index)
                .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                    if let Some(&repo_id) = repo_ids.get(ix) {
                        this.store.dispatch(Msg::SetActiveRepo { repo_id });
                    }
                    this.close_popover(cx);
                }),
        )
        // Fixed width: PickerPrompt rows size with `w_full`, which does not
        // stretch under fit-content parents.
        .w(width.preferred_px(ui_scale))
    } else {
        let mut menu = div()
            .flex()
            .flex_col()
            .min_w(width.min_px(ui_scale))
            .max_w(width.max_px(ui_scale));
        for repo in this.state.repos.iter() {
            let id = repo.id;
            let label = path_display::path_display_shared(&repo.spec.workdir);
            menu = menu.child(
                components::ContextMenuEntry::new(
                    ("repo_item", id.0),
                    components::ContextMenuText::path_single_line(label.clone()),
                )
                .tooltip_host(this.tooltip_host.clone())
                .render(theme, ui_scale_percent, cx)
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    this.store.dispatch(Msg::SetActiveRepo { repo_id: id });
                    this.popover = None;
                    this.popover_anchor = None;
                    cx.notify();
                })),
            );
        }
        components::context_menu(theme, menu)
    }
}
