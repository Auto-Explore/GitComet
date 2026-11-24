use super::super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::ui_scale::UiScale;
use crate::view::date_time::{DateTimeFormat, Timezone};
use crate::view::gitcomet_view_requests::{RootRequest, RootRequests as _};
use gitcomet_core::domain::ReflogEntry;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::time::SystemTime;

mod columns;
use columns::{ReflogColumn, ReflogColumnResizeState, ReflogColumnWidths};

const REFLOG_ROW_HEIGHT_PX: f32 = 28.0;
/// Table columns, in UI-font units — see `reflog_column`.
const REFLOG_MARKER_COLUMN_PX: f32 = 14.0;
const REFLOG_SELECTOR_COLUMN_PX: f32 = 70.0;
const REFLOG_SHA_COLUMN_PX: f32 = 70.0;
const REFLOG_AVATAR_COLUMN_PX: f32 = 22.0;
const REFLOG_DATE_COLUMN_PX: f32 = 260.0;
const REFLOG_FILTER_WIDTH_PX: f32 = 220.0;
const REFLOG_CELL_GAP_PX: f32 = 8.0;
const REFLOG_TABLE_PADDING_PX: f32 = 8.0;
/// A dense table like the log, so rows take the log's comfortable height.
const REFLOG_ROW_COMFORTABLE_HEIGHT_PX: f32 = 36.0;

#[cfg(test)]
thread_local! {
    // (row-builder calls, rows built). Thread-local because parallel tests
    // draw their own reflog panes; a process-wide count picks up their rows.
    static REFLOG_ROWS_BUILT: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

#[inline]
fn record_reflog_rows_built(rows: usize) {
    #[cfg(test)]
    REFLOG_ROWS_BUILT.with(|built| {
        let (calls, total) = built.get();
        built.set((calls + 1, total + rows));
    });
    #[cfg(not(test))]
    let _ = rows;
}

/// A reflog table column: design pixels widened by the UI font, so text cells
/// keep their content as the font grows.
fn reflog_column(ui_scale: UiScale, design_px: f32) -> Pixels {
    ui_scale.px(ui_scale.appearance.ui_text(design_px))
}

/// Per-repository reflog panel state: unlike the popover picker it replaced,
/// this survives being hidden behind the terminal tab, so scroll position, the
/// selected row, and the filter text are exactly where the user left them when
/// they switch back.
pub(in super::super) struct ReflogPanelState {
    query_input: Entity<components::TextInput>,
    _query_input_subscription: gpui::Subscription,
    selected: Option<SharedString>,
    column_widths: ReflogColumnWidths,
    column_resize: Option<ReflogColumnResizeState>,
    table_width: Option<Pixels>,
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
        let ReflogPaneInit { root_view } = init;
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
    /// Reflog revisions capture data changes; backend availability also wakes
    /// an open panel after a parked tab finishes reopening.
    fn notify_fingerprint(&self, state: &AppState) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);
        state.repos.len().hash(&mut hasher);
        for repo in &state.repos {
            repo.id.hash(&mut hasher);
            if self.panels.contains_key(&repo.id) {
                repo.reflog_rev.hash(&mut hasher);
                repo.is_backend_parked().hash(&mut hasher);
                matches!(repo.open, Loadable::Ready(())).hash(&mut hasher);
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
            for repo in &self.state.repos {
                if repo.is_backend_parked()
                    && let Some(panel) = self.panels.get_mut(&repo.id)
                {
                    panel.rows = Vec::new();
                    panel.rows_key = None;
                }
            }
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
            if self.panels.contains_key(&repo.id)
                && !repo.is_backend_parked()
                && matches!(repo.open, Loadable::Ready(()))
                && matches!(repo.reflog, Loadable::NotLoaded)
            {
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
        if self.state.repos.iter().any(|repo| {
            repo.id == repo_id
                && !repo.is_backend_parked()
                && matches!(repo.open, Loadable::Ready(()))
        }) {
            self.store.dispatch(Msg::LoadReflog { repo_id });
        }
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
        let query_input = cx.new(|cx| {
            components::TextInput::new_inert(
                components::TextInputOptions {
                    placeholder: "Filter reflog entries".into(),
                    ..Default::default()
                },
                cx,
            )
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
                column_widths: ReflogColumnWidths::default(),
                column_resize: None,
                table_width: None,
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
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = AppTheme::current(cx);
        let ui_scale = UiScale::from_percent(this.ui_scale_percent).with_appearance(theme.metrics);
        let now = SystemTime::now();

        let Some(repo_id) = this.state.active_repo else {
            record_reflog_rows_built(0);
            return Vec::new();
        };
        let Some(panel) = this.panels.get(&repo_id) else {
            record_reflog_rows_built(0);
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
            let is_selected = selected.as_deref() == Some(entry.selector.as_ref());
            rows.push(this.render_reflog_row(
                theme,
                ui_scale,
                repo_id,
                entry,
                now,
                is_selected,
                window,
                cx,
            ));
        }
        record_reflog_rows_built(rows.len());
        rows
    }

    fn render_reflog_row(
        &self,
        theme: AppTheme,
        ui_scale: UiScale,
        repo_id: RepoId,
        entry: &ReflogEntry,
        now: SystemTime,
        is_selected: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let is_current = entry.index == 0;
        let sha = entry.new_id.as_ref();
        let short_sha: SharedString = sha.get(0..8).unwrap_or(sha).to_owned().into();
        let selector: SharedString = SharedString::from(entry.selector.as_ref());
        let selector_for_selection = selector.clone();
        let date = self.reflog_entry_date_display(entry.time, now);
        let message = selectable_reflog_text(
            "reflog_message",
            repo_id,
            entry,
            entry.message.as_ref().into(),
            window,
            cx,
        );
        let sha = selectable_reflog_text("reflog_sha", repo_id, entry, short_sha, window, cx);
        let date = selectable_reflog_text("reflog_date", repo_id, entry, date.into(), window, cx);
        let panel = &self.panels[&repo_id];
        let widths = panel.column_widths.fitted(panel.table_width, ui_scale);
        let target = entry.new_id.clone();
        let target_for_menu = entry.new_id.clone();
        let selector_for_menu = selector.clone();

        div()
            .id(("reflog_row", entry.index))
            .debug_selector(|| format!("reflog_row-{}", entry.index))
            .flex()
            .items_center()
            .h(ui_scale.row_height(REFLOG_ROW_HEIGHT_PX, REFLOG_ROW_COMFORTABLE_HEIGHT_PX))
            .w_full()
            .flex_none()
            .px(ui_scale.px(REFLOG_TABLE_PADDING_PX))
            .gap(ui_scale.px(REFLOG_CELL_GAP_PX))
            .control_interaction(
                controls::InteractionStyle::new(theme)
                    .resting_background(theme.colors.surface.canvas),
                controls::InteractionState::default()
                    .selected(is_selected, theme.colors.interaction.selected_background),
            )
            .child({
                let marker = div()
                    .id(("reflog_row_marker", entry.index))
                    .w(reflog_column(ui_scale, REFLOG_MARKER_COLUMN_PX))
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
                    .debug_selector(|| format!("reflog_selector-{}", entry.index))
                    .w(reflog_column(ui_scale, widths.get(ReflogColumn::Selector)))
                    .flex_none()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(selector),
            )
            .child(
                div()
                    .id(("reflog_row_sha", entry.index))
                    .debug_selector(|| format!("reflog_sha-{}", entry.index))
                    .w(reflog_column(ui_scale, widths.get(ReflogColumn::Sha)))
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.accent.foreground)
                    .child(sha),
            )
            .child(
                div()
                    .w(reflog_column(ui_scale, REFLOG_AVATAR_COLUMN_PX))
                    .flex_none()
                    .child(components::author_avatar(
                        theme,
                        ui_scale,
                        entry.author.as_ref(),
                    )),
            )
            .child(
                div()
                    .w(reflog_column(ui_scale, widths.get(ReflogColumn::Date)))
                    .debug_selector(|| format!("reflog_date-{}", entry.index))
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(date),
            )
            .child(
                div()
                    .flex_1()
                    .debug_selector(|| format!("reflog_message-{}", entry.index))
                    .min_w(reflog_column(
                        ui_scale,
                        columns::REFLOG_MESSAGE_MIN_WIDTH_PX,
                    ))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(theme.ui_text(12.0))
                    .child(message),
            )
            .on_activate(
                false,
                controls::ControlActivation::Composite,
                cx.listener(move |this, _e: &gpui::ClickEvent, _window, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.selected = Some(selector_for_selection.clone());
                    }
                    this.store.dispatch(Msg::SelectCommit {
                        request_id: None,
                        repo_id,
                        commit_id: target.clone(),
                    });
                    cx.notify();
                }),
            )
            .on_pointer_click(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.selected = Some(selector_for_menu.clone());
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
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ =
                    root_view.request(RootRequest::OpenPopoverAt(kind, anchor), Some(window), cx);
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
        ui_scale: UiScale,
        repo_id: RepoId,
        entry_count: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let query_input = self
            .panels
            .get(&repo_id)
            .map(|panel| panel.query_input.clone());
        let text_color = theme.colors.interaction.selected_foreground;

        let close = components::on_nested_control_click(
            components::panel_tab_close("reflog_panel_tab_close", theme, ui_scale, text_color)
                .gitcomet_tooltip(theme, "Close reflog".into()),
            cx,
            move |this, _e: &gpui::ClickEvent, _window, cx| {
                this.close(repo_id, cx);
            },
        );

        let tab = components::panel_tab(
            "reflog_panel_tab",
            theme,
            ui_scale,
            "icons/history.svg",
            "Reflog",
            true,
        )
        .debug_selector(|| "reflog_panel_tab".to_string())
        .child(close);

        // The tab and the entry count take the leading edge and absorb the slack,
        // which parks the filter box against the trailing edge — the same shape
        // `render_terminal_header` uses to push its action icons right.
        let leading = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(ui_scale.px(6.0))
            .flex_1()
            .min_w(px(0.0))
            .child(tab)
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
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
            .gap(ui_scale.px(6.0))
            .px(ui_scale.px(4.0))
            .py(ui_scale.px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(leading)
            .child(
                div()
                    .id("reflog_panel_filter")
                    .debug_selector(|| "reflog_panel_filter".to_string())
                    .flex_none()
                    .w(reflog_column(ui_scale, REFLOG_FILTER_WIDTH_PX))
                    .children(query_input),
            )
            .into_any()
    }

    fn pane_scale(&self, theme: AppTheme) -> UiScale {
        UiScale::from_percent(self.ui_scale_percent).with_appearance(theme.metrics)
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
        let theme = AppTheme::current(cx);
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
        let ui_scale = self.pane_scale(theme);

        let body = match reflog {
            None => reflog_placeholder(theme, "No repository"),
            Some(Loadable::Error(e)) => {
                reflog_error_placeholder(theme, ui_scale, format!("Couldn't load reflog: {e}"))
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

        let header = self.render_header(theme, ui_scale, repo_id, entry_count, cx);
        let table_header = self.render_table_header(theme, ui_scale, repo_id, cx);
        container.child(header).child(table_header).child(body)
    }
}

fn selectable_reflog_text(
    id: &'static str,
    repo_id: RepoId,
    entry: &ReflogEntry,
    text: SharedString,
    window: &mut Window,
    cx: &mut gpui::Context<ReflogPaneView>,
) -> Entity<components::TextInput> {
    let key = (
        gpui::ElementId::from((id, repo_id.0)),
        SharedString::from(entry.selector.as_ref()),
    );
    let input = window.use_keyed_state(key, cx, |_window, cx| {
        let mut input =
            components::TextInput::new_inert(components::TextInputOptions::selectable(), cx);
        input.set_display_text(cx);
        input.set_display_truncation(Some(components::TextTruncationProfile::End), cx);
        input.set_suppress_right_click(true);
        input
    });
    input.update(cx, |input, cx| input.set_text(text, cx));
    input
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
fn reflog_error_placeholder(
    theme: AppTheme,
    ui_scale: UiScale,
    text: impl Into<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .min_h(px(0.0))
        .px(ui_scale.px(16.0))
        .text_color(theme.colors.status.danger.foreground)
        .child(text.into())
        .into_any()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod view_tests;
