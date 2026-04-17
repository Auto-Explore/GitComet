use super::super::super::path_display;
use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    if let Some(search) = this.repo_picker_search_input.clone() {
        let repo_ids = this.state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
        let items = this
            .state
            .repos
            .iter()
            .map(|r| path_display::path_display_shared(&r.spec.workdir))
            .collect::<Vec<SharedString>>();

        components::context_menu(
            theme,
            components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                .items(items)
                .empty_text("No repositories")
                .max_height(scaled_px(260.0))
                .render_scaled(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                    if let Some(&repo_id) = repo_ids.get(ix) {
                        this.store.dispatch(Msg::SetActiveRepo { repo_id });
                    }
                    this.popover = None;
                    this.popover_anchor = None;
                    cx.notify();
                }),
        )
        .w(scaled_px(420.0))
        .max_w(scaled_px(820.0))
    } else {
        let mut menu = div()
            .flex()
            .flex_col()
            .min_w(scaled_px(420.0))
            .max_w(scaled_px(820.0));
        for repo in this.state.repos.iter() {
            let id = repo.id;
            let label = path_display::path_display_shared(&repo.spec.workdir);
            menu = menu.child(
                components::context_menu_entry(
                    ("repo_item", id.0),
                    theme,
                    ui_scale_percent,
                    false,
                    false,
                    None,
                    label.clone(),
                    None,
                )
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
