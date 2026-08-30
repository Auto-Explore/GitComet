use super::super::*;
use crate::ui_scale::UiScale;
use crate::view::date_time::{DateTimeFormat, Timezone};
use crate::view::perf::{self, ViewPerfRenderLane};
use gitcomet_core::domain::ReflogEntry;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::time::SystemTime;

const REFLOG_ROW_HEIGHT_PX: f32 = 28.0;

/// Per-repository reflog panel state: unlike the popover picker it replaced,
/// this survives being hidden behind the terminal tab, so scroll position, the
/// selected row, and the filter text are exactly where the user left them when
/// they switch back.
pub(in super::super) struct ReflogPanelState {
    query_input: Entity<components::TextInput>,
    _query_input_subscription: gpui::Subscription,
    selected: Option<CommitId>,
    scroll: UniformListScrollHandle,
    /// Trimmed, lowercased mirror of `query_input`'s text, updated only when it
    /// actually changes. `reflog_entry_matches` takes an already-lowercased
    /// query, so the filter box's text is lowercased once here rather than once
    /// per entry per keystroke.
    query: String,
    /// The filtered entries the list renders, in display order.
    rows: Vec<ReflogEntry>,
    /// What `rows` was built from: `(reflog_rev, hash(query))`. `None` until the
    /// first build. The rows are rebuilt only when this changes — everything
    /// else that repaints the panel (hover, selection, theme) reuses them.
    rows_key: Option<(u64, u64)>,
}

/// Everything the pane needs that isn't the store or the state model. Grouped
/// like [`DetailsPaneInit`] so the constructor keeps a readable signature.
pub(in super::super) struct ReflogPaneInit {
    pub(in super::super) theme: AppTheme,
    pub(in super::super) root_view: WeakEntity<GitCometView>,
}

/// The reflog panel, as its own view.
///
/// It is a separate entity for a performance reason, not an organizational one:
/// gpui notifies `current_view` when a hover state flips, so rows built into the
/// root view's element tree make every hover rebuild the entire application UI.
/// Owning its own entity scopes those repaints to this pane.
pub(in super::super) struct ReflogPaneView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    ui_scale_percent: u32,
    date_time_format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
    root_view: WeakEntity<GitCometView>,
    /// A repo's presence here *is* "the panel is open" — closing drops the
    /// entry, and with it the filter text, scroll position, and selection.
    panels: FxHashMap<RepoId, ReflogPanelState>,
    notify_fingerprint: u64,
    _ui_model_subscription: gpui::Subscription,
}

impl ReflogPaneView {
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        init: ReflogPaneInit,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let ReflogPaneInit { theme, root_view } = init;
        let preferences = ui_model.read(cx).preferences.clone();
        let ui_scale_percent = preferences.appearance.ui_scale_percent;
        let date_time_format = preferences.appearance.date_time_format;
        let timezone = preferences.appearance.timezone;
        let show_timezone = preferences.appearance.show_timezone;
        let state = Arc::clone(&ui_model.read(cx).state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = this.notify_fingerprint(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let mut this = Self {
            store,
            state,
            theme,
            ui_scale_percent,
            date_time_format,
            timezone,
            show_timezone,
            root_view,
            panels: FxHashMap::default(),
            notify_fingerprint: 0,
            _ui_model_subscription: subscription,
        };
        this.notify_fingerprint = this.notify_fingerprint(&Arc::clone(&this.state));
        this
    }

    /// Hash of exactly the state this pane reads. Repos that have no panel open
    /// contribute only their identity, so an unrelated repository's reflog churn
    /// cannot repaint the panel — while a repo *closing* still lands here, so
    /// its panel state is dropped with it.
    ///
    /// `reflog_rev` alone captures every reflog transition (loading, ready,
    /// error, cleared) because every write goes through `RepoState::set_reflog`.
    fn notify_fingerprint(&self, state: &AppState) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);
        state.repos.len().hash(&mut hasher);
        for repo in &state.repos {
            repo.id.hash(&mut hasher);
            if self.panels.contains_key(&repo.id) {
                repo.reflog_rev.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn apply_state_snapshot(&mut self, next: Arc<AppState>, _cx: &mut gpui::Context<Self>) {
        self.state = next;
        if !self.panels.is_empty() {
            // Mirrors `sync_terminal_sessions_with_state`'s cleanup: a closed
            // repo's filter text, scroll position, and selection must not linger
            // (and, if the same `RepoId` were ever reused, resurface for an
            // unrelated repository).
            let open: FxHashSet<RepoId> = self.state.repos.iter().map(|repo| repo.id).collect();
            self.panels.retain(|repo_id, _| open.contains(repo_id));
        }
        let _ = self.request_missing_reflogs();
    }

    /// Re-requests the reflog for any open panel whose data was dropped.
    ///
    /// `reload_repo` resets `reflog` to `NotLoaded` on every repo reload (a
    /// commit, a checkout, an external change), and the only other dispatch is
    /// on open — so without this an open panel sits on "Loading…" forever after
    /// the first such reload. Safe to run on every snapshot: `load_reflog`
    /// latches the state to `Loading` immediately and `loads_in_flight`
    /// coalesces duplicates, so this cannot loop. `Error` is deliberately not
    /// retried — a repo with an unborn HEAD would spin.
    ///
    /// Returns the repositories it dispatched for, so the rule can be asserted
    /// without reaching into the store.
    fn request_missing_reflogs(&self) -> Vec<RepoId> {
        let mut requested = Vec::new();
        for repo in &self.state.repos {
            if self.panels.contains_key(&repo.id) && matches!(repo.reflog, Loadable::NotLoaded) {
                self.store.dispatch(Msg::LoadReflog { repo_id: repo.id });
                requested.push(repo.id);
            }
        }
        requested
    }

    /// Whether the panel is open for `repo_id`.
    pub(in super::super) fn is_open(&self, repo_id: RepoId) -> bool {
        self.panels.contains_key(&repo_id)
    }

    /// Opens (or re-focuses) the panel for `repo_id` and kicks off a load.
    pub(in super::super) fn open(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.ensure_panel_state(repo_id, cx);
        self.store.dispatch(Msg::LoadReflog { repo_id });
        cx.notify();
    }

    /// Closes the panel for `repo_id`, dropping its state, and tells the root
    /// view to fall the bottom panel back to the terminal.
    pub(in super::super) fn close(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.panels.remove(&repo_id);
        let root_view = self.root_view.clone();
        // Deferred: this runs from inside a listener on the root's element tree,
        // and a direct `root_view.update()` on the root→pane→root path panics.
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.on_reflog_panel_closed(repo_id, cx);
            });
        });
        cx.notify();
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        for panel in self.panels.values() {
            panel
                .query_input
                .update(cx, |input, cx| input.set_theme(theme, cx));
        }
        cx.notify();
    }

    pub(in super::super) fn set_date_settings(
        &mut self,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == date_time_format
            && self.timezone == timezone
            && self.show_timezone == show_timezone
        {
            return;
        }
        self.date_time_format = date_time_format;
        self.timezone = timezone;
        self.show_timezone = show_timezone;
        cx.notify();
    }

    pub(in super::super) fn set_ui_scale_percent(
        &mut self,
        percent: u32,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.ui_scale_percent == percent {
            return;
        }
        self.ui_scale_percent = percent;
        cx.notify();
    }

    fn ensure_panel_state(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        if self.panels.contains_key(&repo_id) {
            return;
        }
        let theme = self.theme;
        let query_input = cx.new(|cx| {
            let mut input = components::TextInput::new_inert(
                components::TextInputOptions {
                    placeholder: "Filter reflog entries".into(),
                    ..Default::default()
                },
                cx,
            );
            input.set_theme(theme, cx);
            input
        });
        // The table is filtered by this input's text, which the row builder reads
        // from `ReflogPanelState::query`. The input owns its text (uncontrolled);
        // we mirror it here and never write back, which would reset the cursor.
        //
        // The `query == text` guard is load-bearing, not a micro-optimization:
        // a focused `TextInput` notifies every 800 ms from its caret-blink task,
        // and notifying on those would repaint the panel 1.25 times a second for
        // as long as the filter box holds focus.
        let subscription = cx.observe(&query_input, move |this, input, cx| {
            let text = input.read(cx).text().trim().to_lowercase();
            let Some(panel) = this.panels.get_mut(&repo_id) else {
                return;
            };
            if panel.query == text {
                return;
            }
            panel.query = text;
            panel.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
            cx.notify();
        });
        self.panels.insert(
            repo_id,
            ReflogPanelState {
                query_input,
                _query_input_subscription: subscription,
                selected: None,
                scroll: UniformListScrollHandle::default(),
                query: String::new(),
                rows: Vec::new(),
                rows_key: None,
            },
        );
    }

    /// Rebuilds the filtered row list for `repo_id` if the reflog or the filter
    /// changed since it was last built, and reports how many rows it holds.
    ///
    /// The entries are `Arc`-backed, so the filtered `Vec` costs refcount bumps
    /// rather than copies — but it is still only rebuilt on a real change, so a
    /// repaint from hover or selection reuses it untouched.
    fn sync_rows(&mut self, repo_id: RepoId) -> usize {
        let state = Arc::clone(&self.state);
        let repo = state.repos.iter().find(|r| r.id == repo_id);
        let Some(panel) = self.panels.get_mut(&repo_id) else {
            return 0;
        };
        let Some((entries, rev)) = repo.and_then(|repo| match &repo.reflog {
            Loadable::Ready(entries) => Some((entries, repo.reflog_rev)),
            _ => None,
        }) else {
            panel.rows.clear();
            panel.rows_key = None;
            return 0;
        };

        let key = (rev, hash_query(&panel.query));
        if panel.rows_key == Some(key) {
            return panel.rows.len();
        }

        panel.rows_key = Some(key);
        panel.rows.clear();
        panel.rows.extend(
            entries
                .iter()
                .filter(|entry| reflog_entry_matches(entry, &panel.query))
                .cloned(),
        );
        panel.rows.len()
    }

    /// Absolute date plus a relative duration in parentheses, e.g.
    /// `2026-08-18 14:32:07 (3 minutes ago)`. Reuses the app's own date
    /// preferences so the reflog panel matches the rest of the app instead of
    /// inventing its own format. `now` is passed in so a batch of rows shares
    /// one clock reading instead of taking one each.
    fn reflog_entry_date_display(&self, time: Option<SystemTime>, now: SystemTime) -> String {
        let Some(time) = time else {
            return String::new();
        };
        let mut buf = String::with_capacity(40);
        crate::view::date_time::format_datetime_into(
            &mut buf,
            time,
            self.date_time_format,
            self.timezone,
            self.show_timezone,
        );
        if let Ok(duration) = time.duration_since(std::time::UNIX_EPOCH) {
            let relative =
                crate::view::date_time::format_relative_time(duration.as_secs() as i64, now);
            buf.push_str(" (");
            buf.push_str(&relative);
            buf.push(')');
        }
        buf
    }

    /// `uniform_list` row builder: builds only the rows in `range`, which is the
    /// visible window plus a small overscan — never the whole reflog.
    fn render_reflog_rows(
        this: &mut Self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let requested = range.len();
        let theme = this.theme;
        let ui_scale = UiScale::from_percent(this.ui_scale_percent);
        let now = SystemTime::now();

        let Some(repo_id) = this.state.active_repo else {
            perf::record_row_batch(ViewPerfRenderLane::Reflog, requested, 0);
            return Vec::new();
        };
        let Some(panel) = this.panels.get(&repo_id) else {
            perf::record_row_batch(ViewPerfRenderLane::Reflog, requested, 0);
            return Vec::new();
        };
        let selected = panel.selected.clone();
        // Lift the slice out so `this` is free for the per-row `cx.listener`s.
        // This is the visible window, not the reflog.
        let entries: Vec<ReflogEntry> = panel
            .rows
            .get(range)
            .map(<[ReflogEntry]>::to_vec)
            .unwrap_or_default();

        let mut rows = Vec::with_capacity(entries.len());
        for entry in &entries {
            let is_selected = selected.as_ref() == Some(&entry.new_id);
            rows.push(this.render_reflog_row(
                theme,
                ui_scale,
                repo_id,
                entry,
                now,
                entry.index == 0,
                is_selected,
                cx,
            ));
        }
        perf::record_row_batch(ViewPerfRenderLane::Reflog, requested, rows.len());
        rows
    }

    #[allow(clippy::too_many_arguments)]
    fn render_reflog_row(
        &self,
        theme: AppTheme,
        ui_scale: UiScale,
        repo_id: RepoId,
        entry: &ReflogEntry,
        now: SystemTime,
        is_current: bool,
        is_selected: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let sha = entry.new_id.as_ref();
        let short_sha: SharedString = sha.get(0..8).unwrap_or(sha).to_owned().into();
        let selector: SharedString = SharedString::from(entry.selector.as_ref());
        let message: SharedString = SharedString::from(entry.message.as_ref());
        let date = self.reflog_entry_date_display(entry.time, now);
        let target = entry.new_id.clone();
        let target_for_menu = entry.new_id.clone();
        let selector_for_menu = selector.clone();

        let row_bg = if is_selected {
            theme.colors.interaction.selected_background
        } else {
            theme.colors.surface.canvas
        };

        div()
            .id(("reflog_row", entry.index))
            .flex()
            .items_center()
            .h(px(REFLOG_ROW_HEIGHT_PX))
            .flex_none()
            .px(px(8.0))
            .gap(px(8.0))
            .bg(row_bg)
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| {
                if is_selected {
                    s
                } else {
                    s.bg(theme.colors.interaction.hover_background)
                }
            })
            .child({
                let marker = div()
                    .id(("reflog_row_marker", entry.index))
                    .w(px(14.0))
                    .flex_none()
                    .text_color(theme.colors.accent.foreground)
                    .child(if is_current { "▶" } else { "" });
                if is_current {
                    marker.gitcomet_tooltip(theme, "Current HEAD position".into())
                } else {
                    marker
                }
            })
            .child(
                div()
                    .w(px(70.0))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(selector),
            )
            .child(
                div()
                    .id(("reflog_row_sha", entry.index))
                    .w(px(70.0))
                    .flex_none()
                    .text_xs()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.accent.foreground)
                    .child(short_sha)
                    .gitcomet_tooltip(theme, "View this commit in the history log".into()),
            )
            .child(
                div()
                    .w(px(22.0))
                    .flex_none()
                    .child(components::author_avatar(
                        theme,
                        ui_scale,
                        entry.author.as_ref(),
                    )),
            )
            .child(
                div()
                    .w(px(260.0))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(date),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_xs()
                    .child(message),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.selected = Some(target.clone());
                    }
                    this.store.dispatch(Msg::SelectCommit {
                        repo_id,
                        commit_id: target.clone(),
                    });
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.selected = Some(target_for_menu.clone());
                    }
                    this.open_popover_at(
                        PopoverKind::ReflogEntryMenu {
                            repo_id,
                            target: target_for_menu.clone(),
                            selector: selector_for_menu.clone(),
                        },
                        e.position,
                        window,
                        cx,
                    );
                    cx.notify();
                }),
            )
            .into_any()
    }

    /// Popovers are owned by the root view; a direct `root_view.update()` from
    /// here panics on the root→pane→root path, so hop through `cx.defer`.
    fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
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

    /// The panel's own header row, shaped like the terminal panel's
    /// (`render_terminal_header`): an active tab carrying the panel's name and
    /// its own `×`, then the controls, all flowing from the left. The tab *is*
    /// the close affordance — there is no separate "Close" button.
    fn render_header(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        entry_count: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let query_input = self
            .panels
            .get(&repo_id)
            .map(|panel| panel.query_input.clone());
        let text_color = theme.colors.interaction.selected_foreground;

        let close = div()
            .id("reflog_panel_tab_close")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(14.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(with_alpha(theme.colors.status.danger.foreground, 0.18)))
            .child(svg_icon("icons/generic_close.svg", text_color, px(10.0)))
            .gitcomet_tooltip(theme, "Close reflog".into())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.close(repo_id, cx);
                }),
            );

        let tab = div()
            .id("reflog_panel_tab")
            .debug_selector(|| "reflog_panel_tab".to_string())
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(theme.radii.row))
            .bg(theme.colors.interaction.selected_background)
            .text_color(text_color)
            .text_size(px(12.0))
            .child(svg_icon("icons/history.svg", text_color, px(12.0)))
            .child("Reflog")
            .child(close);

        // The tab and the entry count take the leading edge and absorb the slack,
        // which parks the filter box against the trailing edge — the same shape
        // `render_terminal_header` uses to push its action icons right.
        let leading = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .flex_1()
            .min_w(px(0.0))
            .child(tab)
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(match entry_count {
                        Some(count) => format!("{count} entries"),
                        None => "Loading…".to_string(),
                    }),
            );

        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(4.0))
            .py(px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(leading)
            .child(
                div()
                    .id("reflog_panel_filter")
                    .debug_selector(|| "reflog_panel_filter".to_string())
                    .flex_none()
                    .w(px(220.0))
                    .children(query_input),
            )
            .into_any()
    }

    fn render_table_header(&self, theme: AppTheme) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .px(px(8.0))
            .py(px(3.0))
            .gap(px(8.0))
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(div().w(px(14.0)).flex_none())
            .child(div().w(px(70.0)).flex_none().child("Selector"))
            .child(div().w(px(70.0)).flex_none().child("SHA"))
            .child(div().w(px(22.0)).flex_none())
            .child(div().w(px(260.0)).flex_none().child("Date"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .pr(components::Scrollbar::gutter(
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child("Message"),
            )
            .into_any()
    }

    fn render_rows(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        row_count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let Some(scroll) = self.panels.get(&repo_id).map(|panel| panel.scroll.clone()) else {
            return reflog_placeholder(theme, "No reflog entries");
        };

        // Keyed by repository: switching tabs must not hand the new repo's list
        // the element state (and scroll offset) of the one it replaced.
        let list = uniform_list(
            ("reflog_panel_rows", repo_id.0 as usize),
            row_count,
            cx.processor(Self::render_reflog_rows),
        )
        .h_full()
        .min_h(px(0.0))
        .track_scroll(&scroll);
        let list = restrict_scroll_to_vertical_axis(list);

        div()
            .id(("reflog_panel_rows_container", repo_id.0 as usize))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .pr(components::Scrollbar::visible_gutter(
                        scroll.clone(),
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(list),
            )
            .child(components::Scrollbar::new("reflog_panel_scrollbar", scroll).render(theme))
            .into_any()
    }
}

impl Render for ReflogPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let container = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .bg(theme.colors.surface.canvas);

        // The root only mounts this pane when a panel is open for the active
        // repo; anything else renders nothing rather than guessing a repo.
        let Some(repo_id) = self.state.active_repo else {
            return container;
        };
        if !self.panels.contains_key(&repo_id) {
            return container;
        }

        let row_count = self.sync_rows(repo_id);
        let reflog = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| &repo.reflog);
        let entry_count = match reflog {
            Some(Loadable::Ready(entries)) => Some(entries.len()),
            _ => None,
        };
        let query_is_empty = self
            .panels
            .get(&repo_id)
            .is_none_or(|panel| panel.query.is_empty());

        let body = match reflog {
            None => reflog_placeholder(theme, "No repository"),
            Some(Loadable::Error(e)) => {
                reflog_error_placeholder(theme, format!("Couldn't load reflog: {e}"))
            }
            Some(Loadable::NotLoaded | Loadable::Loading) => reflog_placeholder(theme, "Loading…"),
            Some(Loadable::Ready(_)) if row_count == 0 => reflog_placeholder(
                theme,
                if query_is_empty {
                    "No reflog entries"
                } else {
                    "No entries match your filter"
                },
            ),
            Some(Loadable::Ready(_)) => self.render_rows(theme, repo_id, row_count, cx),
        };

        let header = self.render_header(theme, repo_id, entry_count, cx);
        let table_header = self.render_table_header(theme);
        container.child(header).child(table_header).child(body)
    }
}

/// Cheap identity for a filter query, so the row cache can be keyed on it
/// without holding a second copy of the string.
fn hash_query(query: &str) -> u64 {
    let mut hasher = FxHasher::default();
    query.hash(&mut hasher);
    hasher.finish()
}

/// Case-insensitive substring match against selector, short sha, message, and
/// author. `query_lowercase` must already be lowercased: the filter box's text
/// is lowercased once, when it changes, rather than once per entry.
fn reflog_entry_matches(entry: &ReflogEntry, query_lowercase: &str) -> bool {
    if query_lowercase.is_empty() {
        return true;
    }
    let sha = entry.new_id.as_ref();
    let short_sha = sha.get(0..8).unwrap_or(sha);
    entry.selector.to_lowercase().contains(query_lowercase)
        || short_sha.to_lowercase().contains(query_lowercase)
        || entry.message.to_lowercase().contains(query_lowercase)
        || entry.author.to_lowercase().contains(query_lowercase)
}

fn reflog_placeholder(theme: AppTheme, text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(0.0))
        .text_color(theme.colors.foreground.secondary)
        .child(text.into())
        .into_any()
}

/// Same layout as [`reflog_placeholder`], but colored to read as a failure —
/// git can refuse to read the reflog for a repo that is present but has no
/// commits yet ("unborn HEAD"), and that message deserves to look distinct
/// from "still loading" or "nothing to show".
fn reflog_error_placeholder(theme: AppTheme, text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(0.0))
        .px(px(16.0))
        .text_color(theme.colors.status.danger.foreground)
        .child(text.into())
        .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    fn entry(index: usize, selector: &str, sha: &str, author: &str, message: &str) -> ReflogEntry {
        ReflogEntry {
            index,
            new_id: CommitId(StdArc::from(sha)),
            message: StdArc::from(message),
            time: None,
            selector: StdArc::from(selector),
            author: StdArc::from(author),
        }
    }

    #[test]
    fn empty_query_matches_every_entry() {
        let e = entry(0, "HEAD@{0}", "deadbeefcafe", "Jane Doe", "commit: initial");
        assert!(reflog_entry_matches(&e, ""));
    }

    #[test]
    fn query_matches_selector_sha_message_and_author() {
        let e = entry(
            1,
            "HEAD@{1}",
            "deadbeefcafe",
            "Jane Doe",
            "checkout: moving to feature",
        );

        assert!(reflog_entry_matches(&e, "head@{1}"));
        assert!(reflog_entry_matches(&e, "deadbeef"));
        assert!(reflog_entry_matches(&e, "checkout"));
        assert!(reflog_entry_matches(&e, "jane"));
        assert!(!reflog_entry_matches(&e, "nonexistent"));
    }

    /// `reflog_entry_matches` takes an already-lowercased query: the filter
    /// observer lowercases the search box's text once, when it changes, rather
    /// than re-lowercasing it per entry. This exercises that real path — a
    /// mixed-case query as the user types it, lowercased exactly once.
    #[test]
    fn a_mixed_case_query_matches_once_lowercased_by_the_caller() {
        let e = entry(
            1,
            "HEAD@{1}",
            "deadbeefcafe",
            "Jane Doe",
            "checkout: moving to feature",
        );
        let query = "CHECKOUT".to_lowercase();
        assert!(reflog_entry_matches(&e, &query));
    }

    #[test]
    fn query_does_not_match_beyond_the_short_sha_prefix() {
        let e = entry(
            2,
            "HEAD@{2}",
            "deadbeefcafe0000",
            "Jane Doe",
            "reset: moving to HEAD@{1}",
        );
        // Only the first 8 chars are shown/searched as the "sha" field, mirroring
        // what the table displays.
        assert!(!reflog_entry_matches(&e, "cafe0000"));
    }

    #[test]
    fn a_query_change_changes_the_row_cache_key() {
        assert_ne!(hash_query(""), hash_query("checkout"));
        assert_eq!(hash_query("checkout"), hash_query("checkout"));
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::view::test_support::NoopBackend;
    use gitcomet_state::model::RepoState;
    use std::cell::Cell;
    use std::rc::Rc;

    const ENTRY_COUNT: usize = 200;
    /// Tall enough to show a handful of 28px rows, short enough that the list
    /// cannot possibly want all `ENTRY_COUNT` of them.
    const PANE_HEIGHT_PX: f32 = 300.0;

    /// Hosts the pane at a fixed height, the way the bottom panel does — a
    /// `uniform_list` with no bounded height would have no visible window to
    /// virtualize against.
    struct Host {
        pane: Entity<ReflogPaneView>,
    }

    impl Render for Host {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().size_full().flex().flex_col().child(
                div()
                    .h(px(PANE_HEIGHT_PX))
                    .flex()
                    .flex_col()
                    .child(self.pane.clone()),
            )
        }
    }

    fn reflog_entries(count: usize) -> Vec<ReflogEntry> {
        (0..count)
            .map(|index| ReflogEntry {
                index,
                new_id: CommitId(Arc::from(format!("{index:040x}").as_str())),
                message: Arc::from(format!("commit: entry {index}").as_str()),
                time: None,
                selector: Arc::from(format!("HEAD@{{{index}}}").as_str()),
                author: Arc::from("Jane Doe"),
            })
            .collect()
    }

    fn seeded_state(repo_id: RepoId, entries: Vec<ReflogEntry>) -> Arc<AppState> {
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/reflog-pane-test"),
            },
        );
        repo.reflog = Loadable::Ready(Arc::new(entries));
        repo.reflog_rev = 1;
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..AppState::default()
        })
    }

    fn open_pane(
        cx: &mut gpui::TestAppContext,
        entries: Vec<ReflogEntry>,
    ) -> (Entity<ReflogPaneView>, RepoId, &mut gpui::VisualTestContext) {
        let repo_id = RepoId(1);
        let (store, _events) = AppStore::new(Arc::new(NoopBackend));
        let store = Arc::new(store);
        let state = seeded_state(repo_id, entries);

        let (host, cx) = cx.add_window_view(|_window, cx| {
            let ui_model = cx.new(|_cx| {
                AppUiModel::new_with_preferences(
                    Arc::clone(&state),
                    UiPreferences {
                        appearance: crate::view::preferences::AppearancePreferences {
                            date_time_format: DateTimeFormat::YmdHms,
                            timezone: Timezone::Utc,
                            show_timezone: false,
                            ..crate::view::preferences::AppearancePreferences::default()
                        },
                        ..UiPreferences::default()
                    },
                )
            });
            let pane = cx.new(|cx| {
                ReflogPaneView::new(
                    store,
                    ui_model,
                    ReflogPaneInit {
                        theme: AppTheme::gitcomet_dark(),
                        root_view: gpui::WeakEntity::new_invalid(),
                    },
                    cx,
                )
            });
            pane.update(cx, |pane, cx| pane.open(repo_id, cx));
            Host { pane }
        });

        let pane = host.read_with(&*cx, |host, _cx| host.pane.clone());
        (pane, repo_id, cx)
    }

    /// The regression guard for what made this panel slow: it used to build a
    /// row element for every loaded reflog entry on every render. The list must
    /// ask for — and paint — only the rows that fit the panel.
    #[gpui::test]
    fn the_list_builds_only_the_visible_window_not_the_whole_reflog(cx: &mut gpui::TestAppContext) {
        let (pane, repo_id, cx) = open_pane(cx, reflog_entries(ENTRY_COUNT));

        let before = perf::snapshot().reflog_rows_batch;
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let after = perf::snapshot().reflog_rows_batch;

        let calls = after.calls - before.calls;
        let painted = after.painted_rows - before.painted_rows;
        assert!(calls > 0, "the row builder should have run");
        assert!(painted > 0, "the visible rows should have been built");

        // A screenful at 28px per row plus overscan — nowhere near all 200.
        let visible_upper_bound = (PANE_HEIGHT_PX / REFLOG_ROW_HEIGHT_PX).ceil() as u64 * 3;
        assert!(
            painted <= visible_upper_bound,
            "expected at most {visible_upper_bound} rows for a {PANE_HEIGHT_PX}px panel, \
             built {painted} of {ENTRY_COUNT}"
        );

        // All 200 are still loaded and filtered — only the *rendering* is windowed.
        pane.read_with(&*cx, |pane, _cx| {
            assert_eq!(pane.panels[&repo_id].rows.len(), ENTRY_COUNT);
        });
    }

    /// The header reads left-to-right as name, then count, then controls: the
    /// filter box is parked against the trailing edge, not next to the tab.
    #[gpui::test]
    fn the_filter_box_sits_at_the_trailing_edge_of_the_header(cx: &mut gpui::TestAppContext) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (_pane, _repo_id, cx) = open_pane(cx, reflog_entries(8));
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let tab = cx
            .debug_bounds("reflog_panel_tab")
            .expect("the header should carry the Reflog tab");
        let filter = cx
            .debug_bounds("reflog_panel_filter")
            .expect("the header should carry the filter box");

        assert!(
            filter.left() >= tab.right(),
            "the filter box should follow the tab, not precede it"
        );
        // Flush with the trailing edge, bar the header's own 4px padding —
        // a filter merely sitting *after* the tab would satisfy the check above.
        let viewport = cx.update(|window, _app| window.viewport_size());
        let trailing_gap = viewport.width - filter.right();
        assert!(
            trailing_gap <= px(5.0),
            "the filter box should be flush with the header's trailing edge \
             (gap was {trailing_gap:?})"
        );
    }

    /// A focused `TextInput` notifies every 800ms from its caret-blink task.
    /// The filter observer must ignore those: notifying on them repainted the
    /// panel 1.25 times a second for as long as the box held focus.
    #[gpui::test]
    fn a_blink_notify_with_unchanged_text_does_not_repaint_the_panel(
        cx: &mut gpui::TestAppContext,
    ) {
        let (pane, repo_id, cx) = open_pane(cx, reflog_entries(8));
        let input = pane.read_with(&*cx, |pane, _cx| pane.panels[&repo_id].query_input.clone());

        let notified = Rc::new(Cell::new(0usize));
        let counter = Rc::clone(&notified);
        let subscription = cx.update(|_window, app| {
            app.observe(&pane, move |_pane, _app| {
                counter.set(counter.get() + 1);
            })
        });

        // Exactly what the blink task does: notify without touching the text.
        input.update(cx, |_input, cx| cx.notify());
        assert_eq!(
            notified.get(),
            0,
            "a caret blink must not repaint the reflog panel"
        );

        // A real edit still gets through.
        input.update(cx, |input, cx| input.set_text("entry 3", cx));
        assert!(
            notified.get() > 0,
            "typing in the filter box must repaint the panel"
        );
        pane.read_with(&*cx, |pane, _cx| {
            assert_eq!(pane.panels[&repo_id].query, "entry 3");
        });

        drop(subscription);
    }

    /// `reload_repo` drops the reflog back to `NotLoaded` on every repo reload;
    /// with the panel open that used to leave it on "Loading…" forever, because
    /// the only load dispatch was on open.
    #[gpui::test]
    fn a_reflog_cleared_under_an_open_panel_is_requested_again(cx: &mut gpui::TestAppContext) {
        let (pane, repo_id, cx) = open_pane(cx, reflog_entries(4));

        pane.read_with(&*cx, |pane, _cx| {
            assert!(pane.is_open(repo_id), "the panel should be open");
        });

        // Loaded data needs nothing.
        pane.read_with(&*cx, |pane, _cx| {
            assert!(pane.request_missing_reflogs().is_empty());
        });

        // Stand in for what `reload_repo` does: drop the entries on the floor.
        let cleared = pane.read_with(&*cx, |pane, _cx| {
            let mut state = (*pane.state).clone();
            state.repos[0].reflog = Loadable::NotLoaded;
            state.repos[0].reflog_rev = 2;
            Arc::new(state)
        });
        let requested = pane.update(cx, |pane, cx| {
            pane.apply_state_snapshot(Arc::clone(&cleared), cx);
            pane.request_missing_reflogs()
        });
        assert_eq!(
            requested,
            vec![repo_id],
            "an open panel whose reflog was cleared must ask for it again"
        );

        // An error is not retried: a repo with an unborn HEAD would spin.
        let failed = pane.read_with(&*cx, |pane, _cx| {
            let mut state = (*pane.state).clone();
            state.repos[0].reflog = Loadable::Error("unborn HEAD".to_string());
            state.repos[0].reflog_rev = 3;
            Arc::new(state)
        });
        let requested = pane.update(cx, |pane, cx| {
            pane.apply_state_snapshot(failed, cx);
            pane.request_missing_reflogs()
        });
        assert!(requested.is_empty(), "a failed load must not be retried");

        // Neither is a repo whose panel the user closed.
        let requested = pane.update(cx, |pane, cx| {
            pane.close(repo_id, cx);
            pane.apply_state_snapshot(cleared, cx);
            pane.request_missing_reflogs()
        });
        assert!(
            requested.is_empty(),
            "a closed panel must not keep loading reflogs"
        );
    }
}
