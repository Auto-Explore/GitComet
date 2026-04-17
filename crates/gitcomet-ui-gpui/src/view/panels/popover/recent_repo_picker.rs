use super::*;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let recent_repos = session::load().recent_repos;
    let labels = recent_repos
        .iter()
        .map(|path| crate::app::recent_repository_label(path).into())
        .collect::<Vec<SharedString>>();

    if let Some(search) = this.recent_repo_picker_search_input.clone() {
        components::context_menu(
            theme,
            components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                .items(labels)
                .empty_text("No recent repositories")
                .max_height(scaled_px(320.0))
                .render_scaled(
                    theme,
                    ui_scale_percent,
                    cx,
                    move |this, ix, _event, _window, cx| {
                        let Some(path) = recent_repos.get(ix).cloned() else {
                            return;
                        };

                        select_recent_repository(this, path, cx);
                    },
                ),
        )
        .w(scaled_px(480.0))
        .max_w(scaled_px(860.0))
    } else {
        let mut menu = div()
            .flex()
            .flex_col()
            .min_w(scaled_px(480.0))
            .max_w(scaled_px(860.0));
        for (ix, label) in labels.into_iter().enumerate() {
            let Some(path) = recent_repos.get(ix).cloned() else {
                continue;
            };
            menu = menu.child(
                components::context_menu_entry(
                    ("recent_repo_item", ix),
                    theme,
                    ui_scale_percent,
                    false,
                    false,
                    None,
                    label,
                    None,
                )
                .on_click(cx.listener(
                    move |this, _event: &ClickEvent, _window, cx| {
                        select_recent_repository(this, path.clone(), cx);
                    },
                )),
            );
        }
        components::context_menu(theme, menu)
    }
}

fn select_recent_repository(
    this: &mut PopoverHost,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.close_popover(cx);
    this.store.dispatch(Msg::OpenRepo(path));
}
