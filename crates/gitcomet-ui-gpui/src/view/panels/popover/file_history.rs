use super::*;

#[derive(Clone, Copy, Hash)]
struct FileHistoryRowPrefs {
    format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    relative_dates: bool,
}

impl FileHistoryRowPrefs {
    fn from_host(this: &PopoverHost) -> Self {
        Self {
            format: this.date_time_format,
            timezone: this.timezone,
            show_timezone: this.show_timezone,
            relative_dates: this.history_relative_dates,
        }
    }
}

fn file_history_item(
    commit: &gitcomet_core::domain::Commit,
    is_current: bool,
    date: String,
) -> components::PickerPromptItem {
    use components::PickerPromptItemPart as Part;
    let sha = commit.id.as_ref();
    let short = sha.get(0..8).unwrap_or(sha).to_owned();
    let marker = if is_current { "▶ " } else { "  " };
    let mut details = vec![
        Part::new(marker)
            .flexible(false)
            .searchable(false)
            .tooltip(false),
        Part::new(short).flexible(false).tooltip(false).mono(),
    ];
    if !commit.author.is_empty() {
        let mut chars = commit.author.chars();
        let mut author: String = chars.by_ref().take(24).collect();
        if chars.next().is_some() {
            author.push('…');
        }
        details.extend([
            Part::separator("  •  "),
            Part::new(author).flexible(false).tooltip(false),
        ]);
    }
    details.extend([
        Part::separator("  •  "),
        Part::new(date)
            .flexible(false)
            .searchable(false)
            .tooltip(false),
    ]);
    components::PickerPromptItem::from_parts([Part::new(SharedString::from(Arc::clone(
        &commit.summary,
    )))])
    .secondary_parts(details)
}

fn rows(
    page: &gitcomet_core::domain::LogPage,
    current_commit: Option<&CommitId>,
    now: std::time::SystemTime,
    prefs: FileHistoryRowPrefs,
) -> (Vec<components::PickerPromptItem>, Vec<CommitId>) {
    let mut date = String::new();
    page.commits
        .iter()
        .map(|commit| {
            date.clear();
            if prefs.relative_dates {
                let secs = match commit.time.duration_since(std::time::UNIX_EPOCH) {
                    Ok(duration) => duration.as_secs() as i64,
                    Err(error) => -(error.duration().as_secs() as i64),
                };
                date = crate::view::date_time::format_relative_time(secs, now);
            } else {
                crate::view::date_time::format_datetime_into(
                    &mut date,
                    commit.time,
                    prefs.format,
                    prefs.timezone,
                    prefs.show_timezone,
                );
            }
            (
                file_history_item(commit, current_commit == Some(&commit.id), date.clone()),
                commit.id.clone(),
            )
        })
        .unzip()
}

/// Height this picker caps its row list at. Shared between the panel that
/// renders the list and the keyboard navigation that scrolls it: the list is
/// windowed once it outgrows a couple of viewports (the 200-commit first page
/// already does), and it is built for exactly this viewport.
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
        crate::view::fingerprint::hash_loadable_arc(&repo.history_state.file_history, hasher);
        if let Loadable::Ready(page) = &repo.history_state.file_history {
            page.commits.len().hash(hasher);
            page.commits.first().map(|commit| &commit.id).hash(hasher);
            page.commits.last().map(|commit| &commit.id).hash(hasher);
            page.next_cursor.is_some().hash(hasher);
        }
        if let Some(PopoverKind::FileHistory { path, .. }) = &this.popover {
            path.hash(hasher);
        }
        let prefs = FileHistoryRowPrefs::from_host(this);
        prefs.hash(hasher);
        if prefs.relative_dates {
            super::rows_cache::date_bucket(std::time::SystemTime::now()).hash(hasher);
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
    )
    .with_order(components::PickerPromptOrder::Source);
    super::rows_cache::get_or_build(&this.file_history_rows_cache, key, |now| {
        let Some((repo, current_commit)) = file_history_repo(this) else {
            return (Vec::new(), Vec::new(), None);
        };
        let Loadable::Ready(page) = &repo.history_state.file_history else {
            return (Vec::new(), Vec::new(), None);
        };
        let Some(PopoverKind::FileHistory { path, .. }) = &this.popover else {
            return (Vec::new(), Vec::new(), None);
        };
        if repo.history_state.file_history_path.as_ref() != Some(path) {
            return (Vec::new(), Vec::new(), None);
        }
        let (items, payloads) = rows(
            page,
            current_commit.as_ref(),
            now,
            FileHistoryRowPrefs::from_host(this),
        );
        (items, payloads, None)
    })
}

#[derive(Clone)]
pub(super) enum NavTarget {
    Commit(CommitId),
    RowAction(usize),
}

pub(super) fn nav_targets(
    this: &PopoverHost,
    query: &str,
) -> super::picker_nav::IndexedNavRows<NavTarget> {
    let rows = cached(this, query);
    super::picker_nav::IndexedNavRows {
        len: rows.filtered_len(),
        resolve: Box::new(move |ix| rows.filtered_payload(ix).cloned().map(NavTarget::Commit)),
    }
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
    let history = repo.map(|repo| {
        if repo.history_state.file_history_path.as_ref() == Some(&path) {
            &repo.history_state.file_history
        } else {
            &Loadable::Loading
        }
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
                        .text_color(theme.colors.foreground.secondary)
                        .line_height(scaled_px(14.0))
                        .child(
                            components::TruncatedText::path(title.clone())
                                .id(("file_history_title_path", repo_id.0))
                                .text_color(theme.colors.foreground.secondary)
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

    let body: AnyElement = match history {
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
                let menu_ids = std::rc::Rc::clone(&built.payloads);
                let menu_path = path.clone();
                components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                    .prebuilt_items(
                        std::rc::Rc::clone(&built.items),
                        std::rc::Rc::clone(&built.layout),
                    )
                    .prebuilt_geometry(built.geometry(ui_scale))
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No commits")
                    .max_height(scaled_px(FILE_HISTORY_LIST_MAX_HEIGHT_PX))
                    .selected_index(
                        this.picker_row_menu
                            .as_ref()
                            .map(|menu| menu.display_index)
                            .or(this.file_history_selected_index),
                    )
                    .on_context_menu(cx.listener(
                        move |this,
                              event: &components::PickerPromptContextMenuEvent,
                              _window,
                              cx| {
                            let Some(commit_id) = menu_ids.get(event.original_index).cloned()
                            else {
                                return;
                            };
                            picker_row_menu::open(
                                this,
                                picker_row_menu::PickerRowMenuTarget::FileHistoryCommit {
                                    repo_id,
                                    commit_id,
                                    path: menu_path.clone(),
                                },
                                event.display_index,
                                event.position,
                                cx,
                            );
                        },
                    ))
                    .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                        let Some(commit_id) = commit_ids.get(ix).cloned() else {
                            return;
                        };
                        open_at_commit(this, repo_id, commit_id, path.clone(), cx);
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

    // The bounded first page opens at once and the rest is appended when it
    // lands; until then a search would silently miss older commits, so say so.
    let loading_older = matches!(
        history,
        Some(Loadable::Ready(page)) if page.next_cursor.is_some()
    );
    let footer = loading_older.then(|| {
        div()
            .px(scaled_px(8.0))
            .py(scaled_px(4.0))
            .border_t_1()
            .border_color(theme.colors.stroke.default)
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .line_height(scaled_px(14.0))
            .child("Loading older commits…")
    });
    #[cfg(test)]
    let footer =
        footer.map(|footer| footer.debug_selector(|| "file_history_loading_older".to_string()));

    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.stroke.default))
            .child(body)
            .children(footer),
    )
}

pub(super) fn open_at_commit(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.store.dispatch(Msg::OpenFileAtCommit {
        repo_id,
        commit_id,
        path,
    });
    this.close_popover(cx);
}

#[cfg(feature = "benchmarks")]
pub(super) fn benchmark_rows(
    page: &gitcomet_core::domain::LogPage,
    now: std::time::SystemTime,
) -> Vec<components::PickerPromptItem> {
    rows(
        page,
        None,
        now,
        FileHistoryRowPrefs {
            format: DateTimeFormat::YmdHm,
            timezone: Timezone::Utc,
            show_timezone: false,
            relative_dates: true,
        },
    )
    .0
}
