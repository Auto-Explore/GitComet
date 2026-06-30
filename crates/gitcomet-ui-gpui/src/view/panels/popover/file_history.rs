use super::*;

fn file_history_item(
    commit: &gitcomet_core::domain::Commit,
    is_current: bool,
) -> components::PickerPromptItem {
    let sha = commit.id.as_ref();
    let short = sha.get(0..8).unwrap_or(sha).to_owned();
    // A fixed-width marker on the row currently shown in the viewer ("you are
    // here"); a blank marker on the others keeps the SHA column aligned.
    let marker = if is_current { "▶ " } else { "  " };
    components::PickerPromptItem::from_parts([
        components::PickerPromptItemPart::new(marker).flexible(false),
        components::PickerPromptItemPart::new(short)
            .profile(components::TextTruncationProfile::End)
            .flexible(false),
        components::PickerPromptItemPart::separator("  "),
        components::PickerPromptItemPart::new(commit.summary.to_string())
            .profile(components::TextTruncationProfile::End),
    ])
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    // The commit the viewer currently shows this file at, so its row can be
    // marked "you are here". `None` for the working-tree view.
    let current_commit = repo.and_then(|r| match &r.diff_state.diff_target {
        Some(DiffTarget::Commit { commit_id, .. }) => Some(commit_id.clone()),
        _ => None,
    });
    let title: SharedString = path.display().to_string().into();

    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("File history"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .line_height(scaled_px(14.0))
                        .child(
                            components::TruncatedText::path(title.clone())
                                .id(("file_history_title_path", repo_id.0))
                                .text_color(theme.colors.text_muted)
                                .full_text_tooltip(this.tooltip_host.clone())
                                .render(cx),
                        ),
                ),
        )
        .child(
            components::Button::new("file_history_close", "Close")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| this.close_popover(cx)),
        );

    let body: AnyElement = match repo.map(|r| &r.history_state.file_history) {
        None => components::context_menu_label(
            theme,
            ui_scale_percent,
            "No repository",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Loading) => components::context_menu_label(
            theme,
            ui_scale_percent,
            "Loading",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Error(e)) => components::context_menu_label(
            theme,
            ui_scale_percent,
            e.clone(),
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::NotLoaded) => components::context_menu_label(
            theme,
            ui_scale_percent,
            "Not loaded",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element(),
        Some(Loadable::Ready(page)) => {
            let commit_ids = page
                .commits
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
            let items = page
                .commits
                .iter()
                .map(|c| file_history_item(c, current_commit.as_ref() == Some(&c.id)))
                .collect::<Vec<_>>();

            if let Some(search) = this.file_history_search_input.clone() {
                components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                    .items(items)
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No commits")
                    .max_height(scaled_px(340.0))
                    .selected_index(this.file_history_selected_index)
                    .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                        let Some(commit_id) = commit_ids.get(ix).cloned() else {
                            return;
                        };
                        // Open the file's *content* at the chosen commit (which
                        // also records the view in the back/forward history),
                        // rather than showing that commit's diff. Routed through
                        // `OpenFileAtCommit` so the path is resolved to the name
                        // the file had at that commit, following renames.
                        this.store.dispatch(Msg::OpenFileAtCommit {
                            repo_id,
                            commit_id,
                            path: path.clone(),
                        });
                        this.close_popover(cx);
                    })
                    .into_any_element()
            } else {
                components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Search input not initialized",
                    Some(this.tooltip_host.clone()),
                    cx,
                )
                .into_any_element()
            }
        }
    };

    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(scaled_px(520.0))
            .max_w(scaled_px(820.0))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(body),
    )
}
