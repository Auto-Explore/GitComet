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

/// Height this picker caps its row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it: the list is
/// windowed once it outgrows a couple of viewports (200 commits do), and it is
/// built for exactly this viewport.
pub(super) const FILE_HISTORY_LIST_MAX_HEIGHT_PX: f32 = 340.0;

/// The popover's repository and the commit its rows are about, or `None` when
/// the popover is not the file history.
fn file_history_repo(this: &PopoverHost) -> Option<(&RepoState, Option<CommitId>)> {
    let Some(PopoverKind::FileHistory { repo_id, .. }) = &this.popover else {
        return None;
    };
    let repo = this.state.repos.iter().find(|r| r.id == *repo_id)?;
    // The commit the viewer currently shows this file at, so its row can be
    // marked "you are here". `None` for the working-tree view.
    let current_commit = match &repo.diff_state.diff_target {
        Some(DiffTarget::Commit { commit_id, .. }) => Some(commit_id.clone()),
        _ => None,
    };
    Some((repo, current_commit))
}

/// Everything the rows below read.
fn rows_signature(this: &PopoverHost) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        let Some((repo, current_commit)) = file_history_repo(this) else {
            return;
        };
        repo.id.hash(hasher);
        repo.history_state.file_history_path.hash(hasher);
        super::rows_cache::loadable_kind(&repo.history_state.file_history).hash(hasher);
        // No revision counter on the history page, and a commit id is a content
        // hash — so the ids are the page's identity. Hashing 200 of them costs
        // far less than rebuilding 200 rows, which is what this decides.
        if let Loadable::Ready(page) = &repo.history_state.file_history {
            page.commits.len().hash(hasher);
            for commit in &page.commits {
                commit.id.hash(hasher);
            }
        }
        // Decides which row carries the "you are here" marker.
        current_commit.hash(hasher);
    })
}

/// The rows for `query`, built once per change to the history page. The panel
/// and the arrow keys both read them from here.
pub(super) fn cached(
    this: &PopoverHost,
    query: &str,
) -> std::rc::Rc<super::rows_cache::CachedRows<CommitId>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::FileHistory,
        rows_signature(this),
        query,
    );
    super::rows_cache::get_or_build(&this.file_history_rows_cache, key, |_now| {
        let Some((repo, current_commit)) = file_history_repo(this) else {
            return (Vec::new(), Vec::new(), None);
        };
        let Loadable::Ready(page) = &repo.history_state.file_history else {
            return (Vec::new(), Vec::new(), None);
        };
        let (items, payloads) = page
            .commits
            .iter()
            .map(|commit| {
                (
                    file_history_item(commit, current_commit.as_ref() == Some(&commit.id)),
                    commit.id.clone(),
                )
            })
            .unzip();
        (items, payloads, None)
    })
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<CommitId> {
    cached(this, query).filtered_payloads()
}

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
    // Only for the load-state arms below; the rows themselves come from the
    // cache, which resolves the repository and the marked commit itself.
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
        Some(Loadable::Ready(_)) => {
            if let Some(search) = this.file_history_search_input.clone() {
                let query = search.read(cx).text().trim().to_string();
                let built = cached(this, &query);
                let commit_ids = std::rc::Rc::clone(&built.payloads);
                components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                    .prebuilt_items(
                        std::rc::Rc::clone(&built.items),
                        std::rc::Rc::clone(&built.layout),
                    )
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No commits")
                    .max_height(scaled_px(FILE_HISTORY_LIST_MAX_HEIGHT_PX))
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
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(body),
    )
}
