use super::super::*;
use crate::ui_scale::UiScale;
use crate::view::date_time::{DateTimeFormat, Timezone};
use crate::view::panels::popover::picker_nav::{
    PickerNavKeys, PickerNavOutcome, handle_picker_nav,
};
use crate::view::panels::popover::rows_cache::{self, CachedRows, RowsCache};
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// Commits per first page; the backend appends the rest on its own.
const FILE_HISTORY_PAGE_LIMIT: usize = 200;
/// Height of the filter row plus its rule, in design pixels. The measured pane
/// body includes it, the row list does not.
const FILTER_ROW_HEIGHT_PX: f32 = 37.0;

#[derive(Clone, Copy, Hash)]
pub(in crate::view) struct FileHistoryRowPrefs {
    pub(in crate::view) format: DateTimeFormat,
    pub(in crate::view) timezone: Timezone,
    pub(in crate::view) show_timezone: bool,
    pub(in crate::view) relative_dates: bool,
}

pub(in crate::view) fn file_history_item(
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

pub(in crate::view) fn rows(
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

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_rows(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum FileHistoryMode {
    /// The change the commit made to the file.
    Diff,
    /// The whole file as of the commit.
    File,
}

pub(in crate::view) struct FileHistoryPaneInit {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) root_view: WeakEntity<GitCometView>,
}

/// The file-history sidebar tab: the commits of one file, whose selection
/// drives the main diff through the existing commit messages.
pub(in crate::view) struct FileHistoryPaneView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    row_prefs: FileHistoryRowPrefs,
    pub(in crate::view) target: Option<(RepoId, std::path::PathBuf)>,
    pub(in crate::view) selected: Option<CommitId>,
    pub(in crate::view) mode: FileHistoryMode,
    search_input: Entity<components::TextInput>,
    /// Trimmed text of `search_input` as of the last change.
    query: String,
    scroll: ScrollHandle,
    list_height: Pixels,
    rows_cache: RowsCache<CommitId>,
    in_flight: FxHashSet<CommitId>,
    latest_request: Option<CommitId>,
    latest_arrived: bool,
    /// Paths that the tab's own requests resolved to (renames), per commit.
    resolved: FxHashSet<(CommitId, std::path::PathBuf)>,
    last_target: Option<(Option<DiffTarget>, bool)>,
    load_sent: bool,
    initial_done: bool,
    #[cfg(test)]
    sent: Vec<String>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl FileHistoryPaneView {
    pub(in crate::view) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: FileHistoryPaneInit,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let FileHistoryPaneInit { theme, root_view } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let state = Arc::clone(&ui_model.read(cx).state);
        let search_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Filter commits".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        let model_subscription = cx.observe(&ui_model, |this, model, cx| {
            this.state = Arc::clone(&model.read(cx).state);
            this.on_state(cx);
            if this.target.is_some() {
                cx.notify();
            }
        });
        let input_subscription =
            cx.observe_in(&search_input, window, |this, input, _window, cx| {
                let keys = input.update(cx, |input, _| PickerNavKeys::take(input));
                this.on_filter_changed(keys, cx);
            });
        Self {
            store,
            state,
            theme,
            root_view,
            row_prefs: FileHistoryRowPrefs {
                format: preferences.appearance.date_time_format,
                timezone: preferences.appearance.timezone,
                show_timezone: preferences.appearance.show_timezone,
                relative_dates: preferences.history.relative_dates,
            },
            target: None,
            selected: None,
            mode: FileHistoryMode::Diff,
            search_input,
            query: String::new(),
            scroll: ScrollHandle::new(),
            list_height: px(340.0),
            rows_cache: RowsCache::default(),
            in_flight: FxHashSet::default(),
            latest_request: None,
            latest_arrived: false,
            resolved: FxHashSet::default(),
            last_target: None,
            load_sent: false,
            initial_done: false,
            #[cfg(test)]
            sent: Vec::new(),
            _subscriptions: vec![model_subscription, input_subscription],
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    pub(in crate::view) fn set_date_settings(
        &mut self,
        format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.row_prefs.format = format;
        self.row_prefs.timezone = timezone;
        self.row_prefs.show_timezone = show_timezone;
        cx.notify();
    }

    pub(in crate::view) fn set_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.row_prefs.relative_dates = enabled;
        cx.notify();
    }

    fn send(&mut self, msg: Msg) {
        #[cfg(test)]
        self.sent.push(format!("{msg:?}"));
        self.store.dispatch(msg);
    }

    fn repo(&self, repo_id: RepoId) -> Option<&RepoState> {
        self.state.repos.iter().find(|repo| repo.id == repo_id)
    }

    /// The loaded page, but only when it belongs to the tab's file: the state
    /// holds one history slot per repository.
    fn history(&self) -> &Loadable<Arc<gitcomet_core::domain::LogPage>> {
        let Some((repo_id, path)) = &self.target else {
            return &Loadable::Loading;
        };
        match self.repo(*repo_id) {
            Some(repo) if repo.history_state.file_history_path.as_ref() == Some(path) => {
                &repo.history_state.file_history
            }
            _ => &Loadable::Loading,
        }
    }

    pub(in crate::view) fn open(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.last_target = self.repo(repo_id).map(|repo| {
            (
                repo.diff_state.diff_target.clone(),
                repo.diff_state.content_preview,
            )
        });
        self.target = Some((repo_id, path.clone()));
        self.selected = None;
        self.mode = FileHistoryMode::Diff;
        self.in_flight.clear();
        self.latest_request = None;
        self.latest_arrived = false;
        self.resolved.clear();
        self.initial_done = false;
        self.query.clear();
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        self.search_input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_text("", cx);
        });
        self.send(Msg::LoadFileHistory {
            repo_id,
            path,
            limit: FILE_HISTORY_PAGE_LIMIT,
        });
        self.load_sent = true;
        let focus_handle = self.search_input.read(cx).focus_handle();
        window.focus(&focus_handle, cx);
        cx.notify();
    }

    pub(in crate::view) fn close(&mut self, cx: &mut gpui::Context<Self>) {
        self.target = None;
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::view) fn filter_input(&self) -> Entity<components::TextInput> {
        self.search_input.clone()
    }

    /// Reloads a cleared history, picks the first selection and follows
    /// changes of the main diff's target.
    fn on_state(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, path)) = self.target.clone() else {
            return;
        };
        if self.state.active_repo != Some(repo_id) {
            return;
        }
        let Some(repo) = self.repo(repo_id) else {
            return;
        };
        let loaded = repo.history_state.file_history_path.as_ref() == Some(&path);
        let current = (
            repo.diff_state.diff_target.clone(),
            repo.diff_state.content_preview,
        );
        if loaded {
            self.load_sent = false;
        } else if !self.load_sent {
            // A reload cleared the slot; the selection stays.
            self.send(Msg::LoadFileHistory {
                repo_id,
                path: path.clone(),
                limit: FILE_HISTORY_PAGE_LIMIT,
            });
            self.load_sent = true;
        }
        self.start_selection(cx);
        if self.last_target.as_ref() == Some(&current) {
            return;
        }
        self.last_target = Some(current.clone());
        let (shown, file) = current;
        if let Some(DiffTarget::Commit {
            commit_id,
            path: shown_path,
            ..
        }) = shown
        {
            if self.in_flight.remove(&commit_id) {
                self.resolved.insert((commit_id.clone(), shown_path));
                if self.latest_request.as_ref() == Some(&commit_id) {
                    self.latest_arrived = true;
                } else if self.latest_arrived
                    && let Some(latest) = self.latest_request.clone()
                {
                    // An older result overwrote the newer one: ask again.
                    self.request(latest);
                }
                return;
            }
            let listed = matches!(
                self.history(),
                Loadable::Ready(page) if page.commits.iter().any(|c| c.id == commit_id)
            );
            if listed && shown_path == path {
                self.selected = Some(commit_id);
                self.mode = if file {
                    FileHistoryMode::File
                } else {
                    FileHistoryMode::Diff
                };
                self.scroll_to_selected(cx);
                return;
            }
        }
        self.selected = None;
    }

    /// Picks the first selection once the tab's history has loaded.
    fn start_selection(&mut self, cx: &mut gpui::Context<Self>) {
        if self.initial_done {
            return;
        }
        let Some((repo_id, path)) = self.target.clone() else {
            return;
        };
        let Loadable::Ready(page) = self.history() else {
            return;
        };
        let page = Arc::clone(page);
        self.initial_done = true;
        let Some(repo) = self.repo(repo_id) else {
            return;
        };
        let shown = repo.diff_state.diff_target.clone();
        match shown {
            Some(DiffTarget::WorkingTree { path: shown, .. }) if shown == path => {}
            Some(DiffTarget::Commit {
                commit_id,
                path: shown,
                ..
            }) if shown == path && page.commits.iter().any(|c| c.id == commit_id) => {
                self.select(commit_id, cx);
            }
            _ => {
                if let Some(first) = page.commits.first() {
                    self.select(first.id.clone(), cx);
                }
            }
        }
    }

    /// Shows `commit_id` in the main diff, unless it is shown already.
    fn request(&mut self, commit_id: CommitId) {
        let Some((repo_id, path)) = self.target.clone() else {
            return;
        };
        let file = self.mode == FileHistoryMode::File;
        let shown = self.repo(repo_id).is_some_and(|repo| {
            matches!(
                &repo.diff_state.diff_target,
                Some(DiffTarget::Commit { commit_id: shown, path: shown_path, .. })
                    if *shown == commit_id
                        && (*shown_path == path
                            || self.resolved.contains(&(commit_id.clone(), shown_path.clone())))
            ) && repo.diff_state.content_preview == file
        });
        if shown {
            if !self.in_flight.is_empty() {
                // The diff will show an older result first: ask again after it.
                self.latest_request = Some(commit_id);
                self.latest_arrived = true;
            }
            return;
        }
        self.in_flight.insert(commit_id.clone());
        self.latest_request = Some(commit_id.clone());
        self.latest_arrived = false;
        self.send(if file {
            Msg::OpenFileAtCommit {
                repo_id,
                commit_id,
                path,
            }
        } else {
            Msg::ShowFileChangesAtCommit {
                repo_id,
                commit_id,
                path,
            }
        });
    }

    fn select(&mut self, commit_id: CommitId, cx: &mut gpui::Context<Self>) {
        self.selected = Some(commit_id.clone());
        self.request(commit_id);
        self.scroll_to_selected(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_mode(
        &mut self,
        mode: FileHistoryMode,
        cx: &mut gpui::Context<Self>,
    ) {
        self.mode = mode;
        if let Some(selected) = self.selected.clone() {
            self.request(selected);
        }
        cx.notify();
    }

    fn rows(&self) -> Rc<CachedRows<CommitId>> {
        use std::hash::Hash;

        let history = self.history();
        let signature = rows_cache::signature(|hasher| {
            self.target.hash(hasher);
            rows_cache::loadable_kind(history).hash(hasher);
            crate::view::fingerprint::hash_loadable_arc(history, hasher);
            if let Loadable::Ready(page) = history {
                page.commits.len().hash(hasher);
                page.commits.first().map(|commit| &commit.id).hash(hasher);
                page.commits.last().map(|commit| &commit.id).hash(hasher);
            }
            self.row_prefs.hash(hasher);
            if self.row_prefs.relative_dates {
                rows_cache::date_bucket(std::time::SystemTime::now()).hash(hasher);
            }
        });
        let key = rows_cache::RowsCacheKey::new(
            rows_cache::RowsCacheOwner::FileHistory,
            signature,
            &self.query,
        )
        .with_order(components::PickerPromptOrder::Source);
        rows_cache::get_or_build(&self.rows_cache, key, |now| {
            let Loadable::Ready(page) = history else {
                return (Vec::new(), Vec::new(), None);
            };
            let (items, payloads) = rows(page, None, now, self.row_prefs);
            (items, payloads, None)
        })
    }

    /// Position of the selected commit among the rows the filter keeps.
    fn display_index(&self, rows: &CachedRows<CommitId>) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        rows.layout
            .item_indices
            .iter()
            .position(|&ix| rows.payloads.get(ix) == Some(selected))
    }

    fn scroll_to_selected(&self, cx: &mut gpui::Context<Self>) {
        let rows = self.rows();
        let Some(ix) = self.display_index(&rows) else {
            return;
        };
        let current = -self.scroll.offset().y;
        let offset =
            rows.geometry(UiScale::current(cx))
                .reveal_offset(ix, self.list_height, current);
        self.scroll.set_offset(point(px(0.0), -offset));
    }

    fn on_filter_changed(&mut self, keys: PickerNavKeys, cx: &mut gpui::Context<Self>) {
        let query = self.search_input.read(cx).text().trim().to_string();
        let changed = query != self.query;
        self.query = query;
        let rows = self.rows();
        let page_rows = if keys.page_up || keys.page_down {
            let row_height = components::picker_row_height(UiScale::current(cx), true);
            (f32::from(self.list_height) / f32::from(row_height))
                .floor()
                .max(1.0) as usize
        } else {
            1
        };
        let mut display = self.display_index(&rows);
        match handle_picker_nav(&keys, &mut display, rows.filtered_len(), page_rows) {
            PickerNavOutcome::Escape => {
                if !self.query.is_empty() {
                    self.search_input
                        .update(cx, |input, cx| input.set_text("", cx));
                }
            }
            PickerNavOutcome::Navigated => {
                if let Some(commit_id) = display.and_then(|ix| rows.filtered_payload(ix).cloned()) {
                    self.select(commit_id, cx);
                }
            }
            PickerNavOutcome::Enter | PickerNavOutcome::Idle => {}
        }
        if changed {
            cx.notify();
        }
    }

    /// Popovers are owned by the root view; hop through `cx.defer` so the
    /// root→pane→root update does not panic (as the reflog panel does).
    fn open_commit_menu(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        path: std::path::PathBuf,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind = PopoverKind::FileHistoryCommitMenu {
            repo_id,
            commit_id,
            path,
        };
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    fn set_list_height(&mut self, body: Pixels, cx: &mut gpui::Context<Self>) {
        let list = (body - UiScale::current(cx).px(FILTER_ROW_HEIGHT_PX)).max(px(0.0));
        if (list - self.list_height).abs() > px(0.5) {
            self.list_height = list;
            cx.notify();
        }
    }
}

impl Render for FileHistoryPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale = UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale.percent());
        let Some((_, path)) = self.target.clone() else {
            return div().into_any_element();
        };
        let title: SharedString = path.display().to_string().into();
        let name: SharedString = path
            .file_name()
            .map_or_else(
                || title.to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
            .into();

        let header = div()
            .flex()
            .items_center()
            .gap(scaled_px(6.0))
            .px(scaled_px(8.0))
            .py(scaled_px(4.0))
            .child(
                div()
                    .id("file_history_title")
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(theme.ui_text(13.0))
                    .font_weight(FontWeight::BOLD)
                    .child(name)
                    .gitcomet_tooltip(theme, title),
            )
            .child(
                components::SegmentedControl::new("file_history_mode")
                    .segment(
                        components::Segment::new("file_history_mode_diff", "Diff")
                            .selected(self.mode == FileHistoryMode::Diff),
                    )
                    .segment(
                        components::Segment::new("file_history_mode_file", "File")
                            .selected(self.mode == FileHistoryMode::File),
                    )
                    .render(theme, ui_scale, cx, |this, index, _window, cx| {
                        let mode = if index == 0 {
                            FileHistoryMode::Diff
                        } else {
                            FileHistoryMode::File
                        };
                        this.set_mode(mode, cx);
                    }),
            )
            .child(
                components::Button::new("file_history_close", "×")
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |this, _e, _window, cx| this.close(cx)),
            );

        let built = self.rows();
        let empty_text: SharedString = match self.history() {
            Loadable::Ready(_) => "No commits".into(),
            Loadable::Error(error) => error.clone().into(),
            Loadable::NotLoaded => "Not loaded".into(),
            Loadable::Loading => "Loading".into(),
        };
        let loading_older = matches!(
            self.history(),
            Loadable::Ready(page) if page.next_cursor.is_some()
        );
        let commit_ids = Rc::clone(&built.payloads);
        let menu_ids = Rc::clone(&built.payloads);
        let menu_target = self.target.clone();
        let list = components::PickerPrompt::new(self.search_input.clone(), self.scroll.clone())
            .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
            .prebuilt_geometry(built.geometry(ui_scale))
            .padded_query_row()
            .empty_text(empty_text)
            .max_height(self.list_height)
            .selected_index(self.display_index(&built))
            .on_context_menu(cx.listener(
                move |this, event: &components::PickerPromptContextMenuEvent, window, cx| {
                    let (Some(commit_id), Some((repo_id, path))) = (
                        menu_ids.get(event.original_index).cloned(),
                        menu_target.clone(),
                    ) else {
                        return;
                    };
                    this.open_commit_menu(repo_id, commit_id, path, event.position, window, cx);
                },
            ))
            .render(theme, ui_scale, cx, move |this, ix, _e, _window, cx| {
                if let Some(commit_id) = commit_ids.get(ix).cloned() {
                    this.select(commit_id, cx);
                }
            });
        let entity = cx.entity().downgrade();
        // The body is measured so the list can be windowed for its real height.
        let body = div()
            .relative()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                gpui::canvas(
                    move |bounds, _window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.set_list_height(bounds.size.height, cx);
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(list);
        let footer = loading_older.then(|| {
            div()
                .px(scaled_px(8.0))
                .py(scaled_px(4.0))
                .border_t_1()
                .border_color(theme.colors.stroke.default)
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child("Loading older commits…")
        });
        #[cfg(test)]
        let footer =
            footer.map(|footer| footer.debug_selector(|| "file_history_loading_older".to_string()));

        div()
            .id("file_history_pane")
            .size_full()
            .flex()
            .flex_col()
            // The sidebar clears its search on Escape; here Escape clears the
            // filter only (the filter handles it), so it must not travel on.
            .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                }
            }))
            // Up/Down reach the list through the filter, so a click keeps them working.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    let focus = this.search_input.read(cx).focus_handle();
                    window.focus(&focus, cx);
                }),
            )
            .child(header)
            .child(body)
            .children(footer)
            .into_any_element()
    }
}
