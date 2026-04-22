use super::super::path_display;
use super::*;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

pub(in super::super) struct RepoTabsBarView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,

    hovered_repo_tab: Option<RepoId>,
    repo_tab_spinner_delay: Option<RepoTabSpinnerDelayState>,
    repo_tab_spinner_delay_seq: u64,
    notify_fingerprint: u64,
}

#[derive(Clone, Debug)]
struct RepoTabDrag {
    repo_id: RepoId,
    label: SharedString,
}

struct RepoTabDragGhost {
    theme: AppTheme,
    label: SharedString,
}

impl Render for RepoTabDragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value| ui_scale::design_px_from_percent(value, ui_scale_percent);
        div()
            .px_2()
            .h(scaled_px(28.0))
            .flex()
            .items_center()
            .rounded(px(self.theme.radii.pill))
            .bg(with_alpha(self.theme.colors.active_section, 0.92))
            .border_1()
            .border_color(with_alpha(self.theme.colors.border, 0.85))
            .text_sm()
            .text_color(self.theme.colors.text)
            .child(self.label.clone())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RepoTabSpinnerDelayState {
    repo_id: RepoId,
    show_spinner: bool,
}

impl RepoTabsBarView {
    fn notify_fingerprint(state: &AppState) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);
        state.repos.len().hash(&mut hasher);
        for repo in &state.repos {
            repo.id.hash(&mut hasher);
            repo.spec.workdir.hash(&mut hasher);
            repo.missing_on_disk.hash(&mut hasher);
        }
        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            match &repo.open {
                Loadable::NotLoaded => 0u8.hash(&mut hasher),
                Loadable::Loading => 1u8.hash(&mut hasher),
                Loadable::Ready(()) => 2u8.hash(&mut hasher),
                Loadable::Error(err) => {
                    3u8.hash(&mut hasher);
                    err.hash(&mut hasher);
                }
            }
            repo.loads_in_flight.any_in_flight().hash(&mut hasher);
            repo.local_actions_in_flight.hash(&mut hasher);
            repo.pull_in_flight.hash(&mut hasher);
            repo.push_in_flight.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn is_repo_busy(repo: &RepoState) -> bool {
        matches!(repo.open, Loadable::Loading)
            || repo.loads_in_flight.any_in_flight()
            || repo.local_actions_in_flight > 0
            || repo.pull_in_flight > 0
            || repo.push_in_flight > 0
    }

    fn repo_tab_tooltip(repo: &RepoState) -> SharedString {
        if repo.missing_on_disk {
            return format!(
                "Repository not found!\n{}",
                path_display::path_display_string(&repo.spec.workdir)
            )
            .into();
        }

        path_display::path_display_shared(&repo.spec.workdir)
    }

    fn repo_tab_shows_missing_warning(repo: &RepoState, show_spinner: bool) -> bool {
        repo.missing_on_disk && !show_spinner
    }

    fn repo_tab_click_message(active_repo: Option<RepoId>, repo_id: RepoId) -> Option<Msg> {
        (active_repo != Some(repo_id)).then_some(Msg::SetActiveRepo { repo_id })
    }

    fn close_repo_tab(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        self.hovered_repo_tab = None;
        self.store.dispatch(Msg::CloseRepo { repo_id });
        cx.notify();
    }

    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let notify_fingerprint = Self::notify_fingerprint(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint(&next);

            this.state = next;
            this.update_repo_tab_spinner_delay(cx);

            if this
                .hovered_repo_tab
                .is_some_and(|id| !this.state.repos.iter().any(|r| r.id == id))
            {
                this.hovered_repo_tab = None;
            }

            if next_fingerprint != this.notify_fingerprint {
                this.notify_fingerprint = next_fingerprint;
                cx.notify();
            }
        });

        let mut this = Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            root_view,
            hovered_repo_tab: None,
            repo_tab_spinner_delay: None,
            repo_tab_spinner_delay_seq: 0,
            notify_fingerprint,
        };
        this.update_repo_tab_spinner_delay(cx);
        this
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn update_repo_tab_spinner_delay(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            self.repo_tab_spinner_delay = None;
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            self.repo_tab_spinner_delay = None;
            return;
        };

        if !Self::is_repo_busy(repo) {
            self.repo_tab_spinner_delay = None;
            return;
        }

        let same_repo = self
            .repo_tab_spinner_delay
            .as_ref()
            .is_some_and(|s| s.repo_id == repo_id);
        if same_repo {
            return;
        }

        self.repo_tab_spinner_delay_seq = self.repo_tab_spinner_delay_seq.wrapping_add(1);
        let seq = self.repo_tab_spinner_delay_seq;
        let uses_spinner_delay = crate::ui_runtime::current().uses_repo_tab_spinner_delay();
        self.repo_tab_spinner_delay = Some(RepoTabSpinnerDelayState {
            repo_id,
            show_spinner: !uses_spinner_delay,
        });

        if !uses_spinner_delay {
            cx.notify();
            return;
        }

        cx.spawn(
            async move |view: WeakEntity<RepoTabsBarView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(100)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.repo_tab_spinner_delay_seq != seq {
                        return;
                    }
                    let Some(active_repo_id) = this.active_repo_id() else {
                        return;
                    };
                    if active_repo_id != repo_id {
                        return;
                    }
                    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
                        return;
                    };
                    if !Self::is_repo_busy(repo) {
                        return;
                    }
                    if let Some(state) = this.repo_tab_spinner_delay.as_mut()
                        && state.repo_id == repo_id
                        && !state.show_spinner
                    {
                        state.show_spinner = true;
                        cx.notify();
                    }
                });
            },
        )
        .detach();
    }
}

impl Render for RepoTabsBarView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let active = self.active_repo_id();
        let repos_len = self.state.repos.len();
        let active_ix = active.and_then(|id| self.state.repos.iter().position(|r| r.id == id));
        let spinner =
            |id: (&'static str, u64), color: gpui::Rgba| svg_spinner(id, color, scaled_px(12.0));

        let mut bar = components::TabBar::new("repo_tab_bar");
        for (ix, repo) in self.state.repos.iter().enumerate() {
            let repo_id = repo.id;
            let next_repo_id = self.state.repos.get(ix + 1).map(|r| r.id);
            let is_active = Some(repo_id) == active;
            let is_busy = Self::is_repo_busy(repo);
            let show_spinner = is_active
                && is_busy
                && self
                    .repo_tab_spinner_delay
                    .as_ref()
                    .is_some_and(|s| s.repo_id == repo_id && s.show_spinner);
            let show_close = self.hovered_repo_tab == Some(repo_id);
            let label = path_display::repo_path_name(&repo.spec.workdir);
            let label_for_drag = label.clone();

            let position = if ix == 0 {
                components::TabPosition::First
            } else if ix + 1 == repos_len {
                components::TabPosition::Last
            } else {
                let ordering = match (is_active, active_ix) {
                    (true, _) => std::cmp::Ordering::Equal,
                    (false, Some(active_ix)) => ix.cmp(&active_ix),
                    (false, None) => std::cmp::Ordering::Equal,
                };
                components::TabPosition::Middle(ordering)
            };

            let tooltip = Self::repo_tab_tooltip(repo);
            let close_tooltip: SharedString = "Close repository".into();

            let close_button = div()
                .id(("repo_tab_close", repo_id.0))
                .flex()
                .items_center()
                .justify_center()
                .size(scaled_px(14.0))
                .rounded(px(theme.radii.row))
                .cursor_pointer()
                .hover(move |s| s.bg(with_alpha(theme.colors.danger, 0.18)))
                .active(move |s| s.bg(with_alpha(theme.colors.danger, 0.26)))
                .child(svg_icon(
                    "icons/repo_tab_close.svg",
                    theme.colors.danger,
                    scaled_px(12.0),
                ))
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    cx.stop_propagation();
                    this.close_repo_tab(repo_id, cx);
                }))
                .gitcomet_tooltip(theme, close_tooltip.clone());

            let mut tab = components::Tab::new(("repo_tab", repo_id.0))
                .selected(is_active)
                .position(position);
            if show_close {
                tab = tab.end_slot(close_button);
            }

            let show_missing_warning = Self::repo_tab_shows_missing_warning(repo, show_spinner);
            let tab_label = div()
                .flex()
                .items_center()
                .gap(scaled_px(6.0))
                .min_w(px(0.0))
                .child(
                    div()
                        .w(scaled_px(12.0))
                        .h(scaled_px(12.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(show_spinner, |d| {
                            d.child(
                                spinner(
                                    ("repo_tab_busy_spinner", repo_id.0),
                                    with_alpha(
                                        theme.colors.text,
                                        if theme.is_dark { 0.72 } else { 0.62 },
                                    ),
                                )
                                .into_any_element(),
                            )
                        })
                        .when(show_missing_warning, |d| {
                            d.child(svg_icon(
                                "icons/warning.svg",
                                theme.colors.warning,
                                scaled_px(12.0),
                            ))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .line_clamp(1)
                        .child(label),
                );

            let tab = tab
                .child(tab_label)
                .render(theme, ui_scale_percent)
                .debug_selector(move || format!("repo_tab_{}", repo_id.0))
                .on_drag(
                    RepoTabDrag {
                        repo_id,
                        label: label_for_drag,
                    },
                    move |drag, _offset, _window, cx| {
                        cx.new(|_cx| RepoTabDragGhost {
                            theme,
                            label: drag.label.clone(),
                        })
                    },
                )
                .can_drop(move |dragged, _window, _cx| {
                    dragged.downcast_ref::<RepoTabDrag>().is_some()
                })
                .drag_over::<RepoTabDrag>(move |s, drag, _window, _cx| {
                    if drag.repo_id == repo_id {
                        return s;
                    }

                    s.bg(with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.14 } else { 0.10 },
                    ))
                    .border_color(theme.colors.accent)
                })
                .on_drag_move(cx.listener(
                    move |this, e: &gpui::DragMoveEvent<RepoTabDrag>, _w, cx| {
                        let dragged_repo_id = e.drag(cx).repo_id;
                        if dragged_repo_id == repo_id {
                            return;
                        }

                        let Some(insert_before) = repo_tab_insert_before_for_drop(
                            repo_id,
                            next_repo_id,
                            e.event.position,
                            e.bounds,
                        ) else {
                            return;
                        };

                        this.store.dispatch(Msg::ReorderRepoTabs {
                            repo_id: dragged_repo_id,
                            insert_before,
                        });
                    },
                ))
                .on_drop(cx.listener(move |this, _drag: &RepoTabDrag, _w, cx| {
                    this.hovered_repo_tab = None;
                    cx.notify();
                }))
                .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                    if *hovering {
                        this.hovered_repo_tab = Some(repo_id);
                    } else if this.hovered_repo_tab == Some(repo_id) {
                        this.hovered_repo_tab = None;
                    }
                    cx.notify();
                }))
                .gitcomet_tooltip(theme, tooltip.clone())
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, _cx| {
                    if let Some(msg) = Self::repo_tab_click_message(this.active_repo_id(), repo_id)
                    {
                        this.store.dispatch(msg);
                    }
                }))
                .on_aux_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if !e.is_middle_click() {
                        return;
                    }

                    cx.stop_propagation();
                    this.close_repo_tab(repo_id, cx);
                }));

            bar = bar.tab(tab);
        }

        let repo_bar_action_button =
            |id: &'static str, icon_path: &'static str, tooltip: SharedString| {
                div()
                    .id(id)
                    .h_full()
                    .w(scaled_px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .size(scaled_px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(theme.radii.pill))
                            .hover(move |s| s.bg(theme.colors.hover))
                            .child(svg_icon(icon_path, theme.colors.accent, scaled_px(14.0))),
                    )
                    .gitcomet_tooltip(theme, tooltip)
            };

        let root_view = self.root_view.clone();
        let open_repo =
            repo_bar_action_button("open_repo", "icons/folder.svg", "Open repository".into())
                .on_click(cx.listener(move |_this, _e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    let _ = root_view.update(cx, |root, cx| root.prompt_open_repo(window, cx));
                }));

        let root_view = self.root_view.clone();
        let clone_repo =
            repo_bar_action_button("clone_repo", "icons/cloud.svg", "Clone repository".into())
                .on_click(cx.listener(move |_this, e: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    let _ = root_view.update(cx, |root, cx| {
                        root.open_popover_at(PopoverKind::CloneRepo, e.position(), window, cx);
                    });
                }));

        bar.end_child(
            div()
                .id("add_repo_container")
                .relative()
                .h_full()
                .flex()
                .items_center()
                .px_1()
                .gap_1()
                .child(open_repo)
                .child(clone_repo),
        )
        .render(theme, ui_scale_percent)
        .can_drop(|dragged, _window, _cx| dragged.downcast_ref::<RepoTabDrag>().is_some())
        .on_drop(cx.listener(|this, drag: &RepoTabDrag, _w, cx| {
            // Drop on the bar (but not on a specific tab) -> move to end.
            this.store.dispatch(Msg::ReorderRepoTabs {
                repo_id: drag.repo_id,
                insert_before: None,
            });
            this.hovered_repo_tab = None;
            cx.notify();
        }))
    }
}

#[inline(always)]
pub(in crate::view) fn repo_tab_insert_before_for_drag_cursor(
    target_repo_id: RepoId,
    next_repo_id: Option<RepoId>,
    cursor_x: f32,
    tab_center_x: f32,
) -> Option<RepoId> {
    if cursor_x <= tab_center_x {
        Some(target_repo_id)
    } else {
        next_repo_id
    }
}

fn repo_tab_insert_before_for_drop(
    target_repo_id: RepoId,
    next_repo_id: Option<RepoId>,
    pos: Point<Pixels>,
    bounds: Bounds<Pixels>,
) -> Option<Option<RepoId>> {
    // Use exclusive right/bottom edges so adjacent tabs don't both match when the cursor is
    // exactly on the boundary.
    if pos.x < bounds.left()
        || pos.x >= bounds.right()
        || pos.y < bounds.top()
        || pos.y >= bounds.bottom()
    {
        return None;
    }

    Some(repo_tab_insert_before_for_drag_cursor(
        target_repo_id,
        next_repo_id,
        f32::from(pos.x),
        f32::from(bounds.center().x),
    ))
}

#[cfg(test)]
mod tests {
    use super::{RepoTabsBarView, repo_tab_insert_before_for_drag_cursor};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::{RepoId, RepoState};
    use gitcomet_state::msg::Msg;
    use std::path::PathBuf;

    fn repo_state(path: &str) -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from(path),
            },
        )
    }

    #[test]
    fn repo_tab_tooltip_defaults_to_repo_path() {
        let repo = repo_state("/tmp/repo");
        assert_eq!(
            RepoTabsBarView::repo_tab_tooltip(&repo).as_ref(),
            "/tmp/repo"
        );
    }

    #[test]
    fn repo_tab_tooltip_reports_missing_repository() {
        let mut repo = repo_state("/tmp/missing-repo");
        repo.missing_on_disk = true;
        assert_eq!(
            RepoTabsBarView::repo_tab_tooltip(&repo).as_ref(),
            "Repository not found!\n/tmp/missing-repo"
        );
    }

    #[test]
    fn missing_repo_warning_icon_yields_to_spinner() {
        let mut repo = repo_state("/tmp/missing-repo");
        repo.missing_on_disk = true;
        assert!(RepoTabsBarView::repo_tab_shows_missing_warning(
            &repo, false
        ));
        assert!(!RepoTabsBarView::repo_tab_shows_missing_warning(
            &repo, true
        ));
    }

    #[test]
    fn repo_tab_drag_cursor_prefers_target_on_left_half() {
        assert_eq!(
            repo_tab_insert_before_for_drag_cursor(RepoId(5), Some(RepoId(6)), 12.0, 60.0),
            Some(RepoId(5))
        );
        assert_eq!(
            repo_tab_insert_before_for_drag_cursor(RepoId(5), Some(RepoId(6)), 60.0, 60.0),
            Some(RepoId(5))
        );
    }

    #[test]
    fn repo_tab_drag_cursor_uses_next_repo_on_right_half() {
        assert_eq!(
            repo_tab_insert_before_for_drag_cursor(RepoId(5), Some(RepoId(6)), 60.5, 60.0),
            Some(RepoId(6))
        );
        assert_eq!(
            repo_tab_insert_before_for_drag_cursor(RepoId(5), None, 80.0, 60.0),
            None
        );
    }

    #[test]
    fn repo_tab_click_message_is_none_for_active_repo() {
        assert!(RepoTabsBarView::repo_tab_click_message(Some(RepoId(5)), RepoId(5)).is_none());
    }

    #[test]
    fn repo_tab_click_message_activates_inactive_repo() {
        assert!(matches!(
            RepoTabsBarView::repo_tab_click_message(Some(RepoId(5)), RepoId(6)),
            Some(Msg::SetActiveRepo { repo_id: RepoId(6) })
        ));
    }
}
