use super::*;
use crate::ui_scale::UiScale;
use gitcomet_core::domain::ReflogEntry;

/// Bottom panel's minimum height while the reflog panel is the sole (or
/// active) content. Matches the terminal panel's own floor so the resize
/// handle behaves identically regardless of which content is showing.
const REFLOG_PANEL_MIN_HEIGHT_PX: f32 = 120.0;
/// Height of the Terminal/Reflog switcher shown above the bottom panel's
/// content once more than one of its panels is open for the active repo.
const BOTTOM_PANEL_TAB_BAR_HEIGHT_PX: f32 = 30.0;
const REFLOG_ROW_HEIGHT_PX: f32 = 28.0;

impl GitCometView {
    /// Whether the reflog panel is open for `repo_id`. Its presence in
    /// `reflog_panels` *is* "open" — closing it drops the entry, and with it
    /// the filter text, scroll position, and row selection.
    pub(super) fn reflog_panel_is_open(&self, repo_id: RepoId) -> bool {
        self.reflog_panels.contains_key(&repo_id)
    }

    /// Opens (or re-focuses) the persistent reflog panel for `repo_id`:
    /// ensures its state exists, kicks off a load, and brings it to the front
    /// of the bottom panel's tab switcher.
    pub(super) fn open_reflog_panel(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.ensure_reflog_panel_state(repo_id, window, cx);
        self.active_bottom_panel
            .insert(repo_id, BottomPanelTab::Reflog);
        self.store.dispatch(Msg::LoadReflog { repo_id });
        cx.notify();
    }

    /// Closes the reflog panel for `repo_id`, dropping its state. The bottom
    /// panel falls back to the terminal automatically once nothing keys
    /// `active_bottom_panel` to `Reflog` for a repo without a reflog panel.
    pub(super) fn close_reflog_panel(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.reflog_panels.remove(&repo_id);
        if self.active_bottom_panel.get(&repo_id) == Some(&BottomPanelTab::Reflog) {
            self.active_bottom_panel
                .insert(repo_id, BottomPanelTab::Terminal);
        }
        cx.notify();
    }

    /// Drops reflog panel state for any repository that is no longer open,
    /// mirroring `sync_terminal_sessions_with_state`'s cleanup of terminal
    /// sessions — otherwise a closed repo's filter text, scroll position, and
    /// selection would linger in memory (and, if the same `RepoId` were ever
    /// reused, resurface for an unrelated repository).
    pub(super) fn sync_reflog_panels_with_state(&mut self) {
        if self.reflog_panels.is_empty() && self.active_bottom_panel.is_empty() {
            return;
        }
        let active_repo_ids: HashSet<RepoId> =
            self.state.repos.iter().map(|repo| repo.id).collect();
        self.reflog_panels
            .retain(|repo_id, _| active_repo_ids.contains(repo_id));
        self.active_bottom_panel
            .retain(|repo_id, _| active_repo_ids.contains(repo_id));
    }

    fn ensure_reflog_panel_state(
        &mut self,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.reflog_panels.contains_key(&repo_id) {
            return;
        }
        let query_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Filter reflog entries".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        // The table is filtered by this input's text, which is read fresh on
        // every render — without this observer, typing would update the
        // input's own subtree but never trigger the parent re-render that
        // recomputes the filtered rows.
        let subscription = cx.observe(&query_input, |_this, _input, cx| {
            cx.notify();
        });
        self.reflog_panels.insert(
            repo_id,
            ReflogPanelState {
                query_input,
                _query_input_subscription: subscription,
                selected: None,
                scroll: ScrollHandle::new(),
            },
        );
    }

    /// Absolute date plus a relative duration in parentheses, e.g.
    /// `2026-08-18 14:32:07 (3 minutes ago)`. Reuses the app's own date
    /// preferences (`date_time_format`/`timezone`/`show_timezone`) so the
    /// reflog panel matches the rest of the app instead of inventing its own
    /// format, and `format_relative_time` for the parenthetical the same way
    /// the (now retired) picker version of this panel did.
    fn reflog_entry_date_display(&self, time: Option<std::time::SystemTime>) -> String {
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
            let relative = crate::view::date_time::format_relative_time(
                duration.as_secs() as i64,
                std::time::SystemTime::now(),
            );
            buf.push_str(" (");
            buf.push_str(&relative);
            buf.push(')');
        }
        buf
    }

    /// The bottom panel: the terminal, the reflog panel, or — when both are
    /// open for the active repo — a small tab switcher above whichever one is
    /// currently selected. Mirrors `render_terminal_panel`'s `None` contract,
    /// so a repo with neither open still renders nothing here.
    ///
    /// When the reflog panel isn't open this returns exactly what
    /// `render_terminal_panel` would have returned on its own: the terminal's
    /// behavior and shape are unchanged from before this panel existed.
    pub(super) fn render_bottom_panel(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let repo_id = self.active_repo_id()?;
        let reflog_open = self.reflog_panel_is_open(repo_id);

        if !reflog_open {
            return self.render_terminal_panel(theme, window, cx);
        }

        let terminal_open = self
            .terminal_sessions
            .get(&repo_id)
            .and_then(|s| s.active_instance())
            .is_some();

        if !terminal_open {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .h(self.terminal_panel_height)
                    .min_h(px(REFLOG_PANEL_MIN_HEIGHT_PX))
                    .child(self.render_reflog_panel(theme, repo_id, window, cx))
                    .into_any(),
            );
        }

        let active_tab = self
            .active_bottom_panel
            .get(&repo_id)
            .copied()
            .unwrap_or(BottomPanelTab::Reflog);

        let tab_bar = self.render_bottom_panel_tab_bar(theme, repo_id, active_tab, cx);
        let content = match active_tab {
            BottomPanelTab::Terminal => self.render_terminal_panel(theme, window, cx)?,
            BottomPanelTab::Reflog => self.render_reflog_panel(theme, repo_id, window, cx),
        };

        Some(
            div()
                .flex()
                .flex_col()
                .h(self.terminal_panel_height + px(BOTTOM_PANEL_TAB_BAR_HEIGHT_PX))
                .min_h(px(
                    REFLOG_PANEL_MIN_HEIGHT_PX + BOTTOM_PANEL_TAB_BAR_HEIGHT_PX
                ))
                .child(tab_bar)
                .child(div().flex_1().min_h(px(0.0)).child(content))
                .into_any(),
        )
    }

    /// Minimal two-way switcher between the terminal and the reflog panel,
    /// styled after the terminal panel's own per-instance tab row (see
    /// `render_terminal_header` in `terminal_panel.rs`) rather than the
    /// browser-style `components::Tab`/`TabBar`, which is sized and shaped for
    /// the top-level repository tab strip and would look out of place this
    /// far down the chrome.
    fn render_bottom_panel_tab_bar(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        active_tab: BottomPanelTab,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let tab = |id: &'static str,
                   icon: &'static str,
                   label: &'static str,
                   this_tab: BottomPanelTab,
                   cx: &mut gpui::Context<Self>| {
            let is_active = this_tab == active_tab;
            let bg = if is_active {
                theme.colors.interaction.selected_background
            } else {
                theme.colors.surface.panel
            };
            let text_color = if is_active {
                theme.colors.interaction.selected_foreground
            } else {
                theme.colors.foreground.secondary
            };
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(theme.radii.row))
                .bg(bg)
                .text_color(text_color)
                .text_size(px(12.0))
                .cursor(CursorStyle::PointingHand)
                .when(!is_active, |d| {
                    d.hover(move |s| s.bg(theme.colors.interaction.hover_background))
                })
                .child(svg_icon(icon, text_color, px(12.0)))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                        this.active_bottom_panel.insert(repo_id, this_tab);
                        cx.notify();
                    }),
                )
        };

        div()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(4.0))
            .py(px(4.0))
            .h(px(BOTTOM_PANEL_TAB_BAR_HEIGHT_PX))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(tab(
                "bottom_panel_tab_terminal",
                "icons/terminal.svg",
                "Terminal",
                BottomPanelTab::Terminal,
                cx,
            ))
            .child(tab(
                "bottom_panel_tab_reflog",
                "icons/history.svg",
                "Reflog",
                BottomPanelTab::Reflog,
                cx,
            ))
            .into_any()
    }

    fn render_reflog_panel(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        self.ensure_reflog_panel_state(repo_id, window, cx);

        let query = self
            .reflog_panels
            .get(&repo_id)
            .map(|state| state.query_input.read(cx).text().trim().to_lowercase())
            .unwrap_or_default();

        let repo = self.state.repos.iter().find(|r| r.id == repo_id);
        let entries: Option<Vec<ReflogEntry>> = repo.and_then(|r| match &r.reflog {
            Loadable::Ready(entries) => Some(entries.clone()),
            _ => None,
        });

        let query_input = self
            .reflog_panels
            .get(&repo_id)
            .map(|state| state.query_input.clone());

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("Reflog"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .child(match &entries {
                                Some(entries) => format!("{} entries", entries.len()),
                                None => "Loading…".to_string(),
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().w(px(220.0)).children(query_input))
                    .child(
                        components::Button::new("reflog_panel_close", "Close")
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, cx| {
                                this.close_reflog_panel(repo_id, cx);
                            }),
                    ),
            );

        let table_header = div()
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
            .child(div().flex_1().min_w(px(0.0)).child("Message"));

        let body: AnyElement = match (repo.map(|r| &r.reflog), entries) {
            (None, _) => reflog_placeholder(theme, "No repository"),
            (Some(Loadable::Error(e)), _) => {
                reflog_error_placeholder(theme, format!("Couldn't load reflog: {e}"))
            }
            (Some(_), None) => reflog_placeholder(theme, "Loading…"),
            (Some(_), Some(entries)) => {
                let ui_scale = UiScale::from_percent(self.ui_scale_percent);
                let selected = self
                    .reflog_panels
                    .get(&repo_id)
                    .and_then(|s| s.selected.clone());
                let scroll = self
                    .reflog_panels
                    .get(&repo_id)
                    .map(|s| s.scroll.clone())
                    .unwrap_or_default();

                let filtered: Vec<ReflogEntry> = entries
                    .into_iter()
                    .filter(|entry| reflog_entry_matches(entry, &query))
                    .collect();

                if filtered.is_empty() {
                    reflog_placeholder(
                        theme,
                        if query.is_empty() {
                            "No reflog entries"
                        } else {
                            "No entries match your filter"
                        },
                    )
                } else {
                    let mut rows = div()
                        .id("reflog_panel_rows")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .track_scroll(&scroll);
                    for entry in &filtered {
                        rows = rows.child(self.render_reflog_row(
                            theme,
                            ui_scale,
                            repo_id,
                            entry,
                            entry.index == 0,
                            selected.as_ref() == Some(&entry.new_id),
                            cx,
                        ));
                    }
                    rows.into_any()
                }
            }
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .bg(theme.colors.surface.canvas)
            .child(header)
            .child(table_header)
            .child(body)
            .into_any()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_reflog_row(
        &self,
        theme: AppTheme,
        ui_scale: UiScale,
        repo_id: RepoId,
        entry: &ReflogEntry,
        is_current: bool,
        is_selected: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let sha = entry.new_id.as_ref();
        let short_sha: SharedString = sha.get(0..8).unwrap_or(sha).to_owned().into();
        let selector: SharedString = entry.selector.to_string().into();
        let message: SharedString = entry.message.to_string().into();
        let author: SharedString = entry.author.to_string().into();
        let date = self.reflog_entry_date_display(entry.time);
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
                    .child(selector.clone()),
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
                    .child(components::author_avatar(theme, ui_scale, author.as_ref())),
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
                    if let Some(state) = this.reflog_panels.get_mut(&repo_id) {
                        state.selected = Some(target.clone());
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
                    if let Some(state) = this.reflog_panels.get_mut(&repo_id) {
                        state.selected = Some(target_for_menu.clone());
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
                }),
            )
            .into_any()
    }
}

/// Case-insensitive substring match against selector, short sha, message, and
/// author — the same fields the retired popover picker matched against, plus
/// the author this panel adds. `query_lowercase` must already be lowercased:
/// the one caller (`render_reflog_panel`) lowercases the search box's text
/// once before filtering every row, rather than re-lowercasing it per entry.
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
    use std::sync::Arc;

    fn entry(index: usize, selector: &str, sha: &str, author: &str, message: &str) -> ReflogEntry {
        ReflogEntry {
            index,
            new_id: gitcomet_core::domain::CommitId(Arc::from(sha)),
            message: Arc::from(message),
            time: None,
            selector: Arc::from(selector),
            author: Arc::from(author),
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

    /// `reflog_entry_matches` takes an already-lowercased query (see its
    /// `query_lowercase` parameter) — the one call site, `render_reflog_panel`,
    /// lowercases the search box's text once before filtering every row rather
    /// than re-lowercasing it per entry. This test exercises that real
    /// end-to-end path: a mixed-case query as the user actually types it,
    /// lowercased exactly once, the same way the caller does it.
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
}
