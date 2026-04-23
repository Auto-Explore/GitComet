use super::*;

fn file_history_item(commit: &gitcomet_core::domain::Commit) -> components::PickerPromptItem {
    let sha = commit.id.as_ref();
    let short = sha.get(0..8).unwrap_or(sha).to_owned();
    components::PickerPromptItem::from_parts([
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
                            components::TruncatedText::new(title.clone())
                                .profile(components::TextTruncationProfile::Path)
                                .tooltip_host(this.tooltip_host.clone())
                                .tooltip_mode(
                                    components::TruncatedTextTooltipMode::FullTextIfTruncated,
                                )
                                .render(cx),
                        ),
                ),
        )
        .child(
            components::Button::new("file_history_close", "Close")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.popover = None;
                    this.popover_anchor = None;
                    cx.notify();
                }),
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
                .map(file_history_item)
                .collect::<Vec<_>>();

            if let Some(search) = this.file_history_search_input.clone() {
                components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                    .items(items)
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No commits")
                    .max_height(scaled_px(340.0))
                    .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                        let Some(commit_id) = commit_ids.get(ix).cloned() else {
                            return;
                        };
                        this.store.dispatch(Msg::SelectCommit {
                            repo_id,
                            commit_id: commit_id.clone(),
                        });
                        this.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::Commit {
                                commit_id,
                                path: Some(path.clone()),
                            },
                        });
                        this.popover = None;
                        this.popover_anchor = None;
                        cx.notify();
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
