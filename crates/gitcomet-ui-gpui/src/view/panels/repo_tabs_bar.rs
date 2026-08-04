use super::super::path_display;
use super::*;
use rustc_hash::FxHasher;
use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub(in super::super) struct RepoTabsBarView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    open_terminal_repo_ids: HashSet<RepoId>,

    hovered_repo_tab: Option<RepoId>,
    /// Left-pressed tab, tracked so its text fade matches the tab's active fill.
    pressed_repo_tab: Option<RepoId>,
    active_context_menu_invoker: Option<SharedString>,
    repo_tab_spinner_delay: Option<RepoTabSpinnerDelayState>,
    repo_tab_spinner_delay_seq: u64,
    notify_fingerprint: u64,
    repo_tab_drag_visual: Option<RepoTabDragVisual>,
    tab_scroll: components::TabBarScroll,
    /// Active repo the strip last scrolled into view, so a repo the user
    /// switched to is revealed without fighting manual scrolling.
    revealed_repo: Option<RepoId>,
    /// Reveal still owed to a repo, with the frames already spent on it.
    pending_reveal: Option<(RepoId, u8)>,
    /// Timestamp of the last edge-drag auto-scroll step, for pacing the next.
    drag_scroll_tick: Option<Instant>,
}

#[derive(Clone, Debug)]
struct RepoTabDrag {
    repo_id: RepoId,
    cursor_offset_x: Rc<Cell<Pixels>>,
    tab_width: Rc<Cell<Pixels>>,
    last_center_x: Rc<Cell<Pixels>>,
    direction: Rc<Cell<i8>>,
}

impl RepoTabDrag {
    fn center_x(&self, cursor_x: Pixels) -> Pixels {
        let width = self.tab_width.get();
        if width == px(0.0) {
            cursor_x
        } else {
            cursor_x - self.cursor_offset_x.get() + width / 2.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RepoTabDragVisual {
    repo_id: RepoId,
    left: Pixels,
}

struct RepoTabDragCarrier;

impl Render for RepoTabDragCarrier {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        // GPUI's drag machinery requires a preview entity. Keep that carrier
        // invisible: the real tab remains in the strip and is what reorders.
        div().size(px(1.0)).opacity(0.0)
    }
}

const REPO_TAB_SLIDE_DURATION: Duration = Duration::from_millis(100);
const REPO_TAB_TAKEOVER_BIAS: f32 = 0.25;
/// How close to a strip edge a dragged tab has to get before the strip starts
/// scrolling under it.
const REPO_TAB_DRAG_EDGE_PX: f32 = 40.0;
/// Speed of that scroll. Fast enough to cross a full strip in about a second,
/// slow enough to drop the tab on a specific neighbour.
const REPO_TAB_DRAG_SCROLL_PX_PER_SEC: f32 = 700.0;
/// Ceiling on the gap between two auto-scroll steps, so a stalled frame can't
/// launch the strip across several tabs at once.
const REPO_TAB_DRAG_SCROLL_MAX_STEP: Duration = Duration::from_millis(50);
/// Shared label-row geometry. Keeping an explicit line box lets the text,
/// status glyph, and close button all center on the same title-bar axis.
const REPO_TAB_FONT_SIZE_PX: f32 = 15.0;
const REPO_TAB_CONTENT_HEIGHT_PX: f32 = 18.0;
const REPO_TAB_STATUS_SIZE_PX: f32 = components::REPOSITORY_BADGE_SIZE_PX;
const REPO_TAB_LABEL_GAP_PX: f32 = 6.0;
const REPO_TAB_CLOSE_FADE_WIDTH_PX: f32 = 16.0;
const REPO_TAB_SIDE_PADDING_PX: f32 = 10.0;

fn repo_tab_text_width(label: SharedString, font_size: Pixels, window: &mut Window) -> Pixels {
    if label.is_empty() {
        return px(0.0);
    }
    let style = window.text_style();
    let run = style.to_run(label.len());
    window
        .text_system()
        .shape_line(label, font_size, &[run], None)
        .width
}

fn repo_tab_close_button_fill(
    theme: AppTheme,
    background: gpui::Rgba,
    pressed: bool,
) -> gpui::Rgba {
    let amount = match (theme.is_dark, pressed) {
        (true, false) => 0.44,
        (true, true) => 0.60,
        (false, false) => 0.22,
        (false, true) => 0.32,
    };
    let mut fill =
        crate::theme::composite_over(background, with_alpha(theme.colors.shadow, amount));
    // The hover plate must fully cover repository text beneath the overlaid
    // close action rather than depend on stacked alpha compositing.
    fill.a = 1.0;
    fill
}

struct RepoTabSlide {
    id: ElementId,
    child: Option<AnyElement>,
    drag_left: Option<Pixels>,
}

impl RepoTabSlide {
    fn new(id: impl Into<ElementId>, child: impl IntoElement, drag_left: Option<Pixels>) -> Self {
        Self {
            id: id.into(),
            child: Some(child.into_any_element()),
            drag_left,
        }
    }
}

impl IntoElement for RepoTabSlide {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct RepoTabSlideState {
    layout_x: Pixels,
    from_offset_x: Pixels,
    started_at: Instant,
}

impl RepoTabSlideState {
    fn offset_at(&self, now: Instant) -> Pixels {
        let delta = (now.duration_since(self.started_at).as_secs_f32()
            / REPO_TAB_SLIDE_DURATION.as_secs_f32())
        .min(1.0);
        let eased = gpui::ease_out_quint()(delta);
        self.from_offset_x * (1.0 - eased)
    }
}

impl Element for RepoTabSlide {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child = self.child.take().expect("repo tab slide child");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let now = Instant::now();
        let offset_x = window.with_element_state(
            global_id.expect("repo tab slide requires a stable element id"),
            |state: Option<RepoTabSlideState>, window| {
                let mut state = state.unwrap_or(RepoTabSlideState {
                    layout_x: bounds.left(),
                    from_offset_x: px(0.0),
                    started_at: now - REPO_TAB_SLIDE_DURATION,
                });

                let offset_x = if let Some(drag_left) = self.drag_left {
                    let offset_x = drag_left - bounds.left();
                    state.layout_x = bounds.left();
                    state.from_offset_x = offset_x;
                    state.started_at = now;
                    offset_x
                } else if state.layout_x != bounds.left() {
                    let current_offset_x = state.offset_at(now);
                    let previous_visual_x = state.layout_x + current_offset_x;
                    state.layout_x = bounds.left();
                    state.from_offset_x = previous_visual_x - bounds.left();
                    state.started_at = now;
                    state.from_offset_x
                } else {
                    state.offset_at(now)
                };

                if self.drag_left.is_none() && offset_x != px(0.0) {
                    window.request_animation_frame();
                }

                (offset_x, state)
            },
        );

        window.with_element_offset(point(offset_x, px(0.0)), |window| {
            child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
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
        self.pressed_repo_tab = None;
        if let Ok(true) = self.root_view.update(cx, |root, cx| {
            root.request_terminal_shutdown_action(TerminalShutdownAction::CloseRepo { repo_id }, cx)
        }) {
            return;
        }
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
            if this
                .pressed_repo_tab
                .is_some_and(|id| !this.state.repos.iter().any(|r| r.id == id))
            {
                this.pressed_repo_tab = None;
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
            open_terminal_repo_ids: HashSet::default(),
            hovered_repo_tab: None,
            pressed_repo_tab: None,
            active_context_menu_invoker: None,
            repo_tab_spinner_delay: None,
            repo_tab_spinner_delay_seq: 0,
            notify_fingerprint,
            repo_tab_drag_visual: None,
            tab_scroll: components::TabBarScroll::new(),
            revealed_repo: None,
            pending_reveal: None,
            drag_scroll_tick: None,
        };
        this.update_repo_tab_spinner_delay(cx);
        this
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in super::super) fn set_open_terminal_repo_ids(
        &mut self,
        next: HashSet<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.open_terminal_repo_ids == next {
            return;
        }
        self.open_terminal_repo_ids = next;
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    /// Scrolls a newly activated tab into view. GPUI resolves the request
    /// against the previously measured layout, so the reveal is re-requested
    /// until the tab really is on screen — capped so a tab that can never
    /// satisfy it cannot spin frames forever.
    fn reveal_pending_repo_tab(&mut self, window: &mut Window) {
        const MAX_REVEAL_ATTEMPTS: u8 = 3;

        let Some((repo_id, attempts)) = self.pending_reveal else {
            return;
        };
        let Some(ix) = self.state.repos.iter().position(|r| r.id == repo_id) else {
            return;
        };
        if self.tab_scroll.is_measured() {
            if self.tab_scroll.tab_is_visible(ix) || attempts >= MAX_REVEAL_ATTEMPTS {
                self.pending_reveal = None;
                return;
            }
            self.tab_scroll.scroll_to_tab(ix);
        }

        self.pending_reveal = Some((repo_id, attempts + 1));
        window.request_animation_frame();
    }

    /// Scrolls the strip while a dragged tab is held against one of its edges.
    /// The pointer can sit still for as long as it likes, so this runs off the
    /// render loop rather than drag-move events, re-picking the drop target on
    /// every step as tabs slide past underneath.
    fn drive_repo_tab_drag_scroll(&mut self, ui_scale_percent: u32, window: &mut Window) {
        let Some(dragged) = self.repo_tab_drag_visual.map(|drag| drag.repo_id) else {
            self.drag_scroll_tick = None;
            return;
        };

        let viewport = self.tab_scroll.viewport();
        let edge = ui_scale::design_px_from_percent(REPO_TAB_DRAG_EDGE_PX, ui_scale_percent);
        let cursor_x = window.mouse_position().x;
        let direction = if cursor_x <= viewport.left() + edge {
            -1.0
        } else if cursor_x >= viewport.right() - edge {
            1.0
        } else {
            0.0
        };

        if direction == 0.0 || !self.tab_scroll.can_scroll(direction) {
            self.drag_scroll_tick = None;
            return;
        }

        let now = Instant::now();
        let step = self
            .drag_scroll_tick
            .map_or(Duration::ZERO, |tick| now.saturating_duration_since(tick))
            .min(REPO_TAB_DRAG_SCROLL_MAX_STEP);
        self.drag_scroll_tick = Some(now);

        let delta = px(REPO_TAB_DRAG_SCROLL_PX_PER_SEC * step.as_secs_f32() * direction);
        if self.tab_scroll.scroll_by(delta) {
            self.reorder_dragged_repo_tab_at(dragged, cursor_x);
        }
        window.request_animation_frame();
    }

    /// Pen the dragged tab inside the strip it belongs to. The pointer is free
    /// to roam the whole title bar, so without this the tab follows it out past
    /// either end and paints over the window chrome. Held against an edge the
    /// tab now stops there while the strip auto-scrolls underneath.
    fn clamp_repo_tab_drag_left(&self, left: Pixels, repo_id: RepoId, tab_width: Pixels) -> Pixels {
        let viewport = self.tab_scroll.viewport();
        // The drag records its own width on the first move over the dragged tab;
        // until then fall back to the width the strip laid out for it.
        let tab_width = if tab_width > px(0.0) {
            tab_width
        } else {
            self.state
                .repos
                .iter()
                .position(|repo| repo.id == repo_id)
                .and_then(|ix| self.tab_scroll.tab_bounds(ix))
                .map_or(px(0.0), |bounds| bounds.size.width)
        };
        // Travel is bounded by the run of tabs, not the whole strip: while they
        // fit, the space past the last one belongs to the add-repo button and
        // the window-drag filler, and a tab dragged into it would float there
        // detached. Once the tabs overflow, their ends leave the viewport and
        // it becomes the tighter bound.
        let last_ix = self.state.repos.len().saturating_sub(1);
        let min_left = self
            .tab_scroll
            .tab_bounds(0)
            .map_or(viewport.left(), |bounds| bounds.left().max(viewport.left()));
        let tabs_right = self
            .tab_scroll
            .tab_bounds(last_ix)
            .map_or(viewport.right(), |bounds| {
                bounds.right().min(viewport.right())
            });
        // A tab wider than the space it may travel has nowhere to go; pin it to
        // the left edge rather than inverting the clamp range.
        let max_left = (tabs_right - tab_width).max(min_left);
        left.clamp(min_left, max_left)
    }

    /// Re-runs the drop-target decision for a pointer that hasn't moved, using
    /// the tab now sitting under it.
    fn reorder_dragged_repo_tab_at(&mut self, dragged: RepoId, cursor_x: Pixels) {
        let target = self.state.repos.iter().enumerate().find_map(|(ix, repo)| {
            let bounds = self.tab_scroll.tab_bounds(ix)?;
            (cursor_x >= bounds.left() && cursor_x < bounds.right()).then(|| {
                (
                    repo.id,
                    self.state.repos.get(ix + 1).map(|next| next.id),
                    bounds.center().x,
                )
            })
        });

        let Some((target_repo_id, next_repo_id, center_x)) = target else {
            return;
        };
        if target_repo_id == dragged {
            return;
        }

        let insert_before = repo_tab_insert_before_for_drag_cursor(
            target_repo_id,
            next_repo_id,
            f32::from(cursor_x),
            f32::from(center_x),
        );
        self.store.dispatch(Msg::ReorderRepoTabs {
            repo_id: dragged,
            insert_before,
        });
    }

    #[cfg(test)]
    pub(in crate::view) fn tab_scroll_for_tests(&self) -> (Pixels, Pixels) {
        (self.tab_scroll.scrolled(), self.tab_scroll.max_scroll())
    }

    #[cfg(test)]
    pub(in crate::view) fn tab_strip_viewport_for_tests(&self) -> Bounds<Pixels> {
        self.tab_scroll.viewport()
    }

    #[cfg(test)]
    pub(in crate::view) fn pressed_repo_tab_for_tests(&self) -> Option<RepoId> {
        self.pressed_repo_tab
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn clear_repo_tab_drag_visual(&mut self, cx: &mut gpui::Context<Self>) {
        if self.repo_tab_drag_visual.take().is_some() {
            cx.notify();
        }
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
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if self.repo_tab_drag_visual.is_some() && !cx.has_active_drag() {
            self.repo_tab_drag_visual = None;
        }

        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let active = self.active_repo_id();
        let spinner = |id: (&'static str, u64), color: gpui::Rgba| {
            svg_spinner(id, color, scaled_px(REPO_TAB_STATUS_SIZE_PX))
        };
        // Tabs are transparent, so a label fading out has to land on whatever
        // is actually behind it: the bar for an idle tab, the content strip
        // for the active one.
        let strip_bg = crate::view::chrome::title_bar_background(theme, window.is_window_active());
        let hovered_tab_bg =
            crate::theme::composite_over(strip_bg, components::Tab::hover_overlay(theme));

        // Reveal the active tab when the repository changes, then leave the
        // offset alone so manual scrolling sticks.
        if self.revealed_repo != active {
            self.revealed_repo = active;
            self.pending_reveal = active.map(|repo_id| (repo_id, 0));
        }
        self.reveal_pending_repo_tab(window);
        self.drive_repo_tab_drag_scroll(ui_scale_percent, window);

        let tab_horizontal_padding = scaled_px(REPO_TAB_SIDE_PADDING_PX);
        let repo_tab_labels = self
            .state
            .repos
            .iter()
            .map(|repo| path_display::repo_path_name(&repo.spec.workdir))
            .collect::<Vec<_>>();
        let natural_tab_widths = self
            .state
            .repos
            .iter()
            .zip(&repo_tab_labels)
            .map(|(repo, label)| {
                let text_width =
                    repo_tab_text_width(label.clone(), scaled_px(REPO_TAB_FONT_SIZE_PX), window);
                let terminal_width = if self.open_terminal_repo_ids.contains(&repo.id) {
                    scaled_px(REPO_TAB_LABEL_GAP_PX + REPO_TAB_STATUS_SIZE_PX)
                } else {
                    px(0.0)
                };
                let content_width = scaled_px(REPO_TAB_STATUS_SIZE_PX + REPO_TAB_LABEL_GAP_PX)
                    + text_width
                    + terminal_width;
                components::Tab::natural_width(
                    content_width,
                    tab_horizontal_padding,
                    ui_scale_percent,
                )
            })
            .collect::<Vec<_>>();
        let mut bar = components::TabBar::new("repo_tab_bar").scroll(self.tab_scroll.clone());
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
            let context_menu_invoker: SharedString = format!("repo_tab_{}", repo_id.0).into();
            let context_menu_active =
                self.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
            let next_context_menu_active = next_repo_id.is_some_and(|next_repo_id| {
                self.active_context_menu_invoker
                    .as_ref()
                    .is_some_and(|invoker| {
                        invoker.as_ref() == format!("repo_tab_{}", next_repo_id.0)
                    })
            });
            let show_inactive_separator = !is_active
                && !context_menu_active
                && next_repo_id.is_some()
                && next_repo_id != active
                && !next_context_menu_active;
            let context_menu_invoker_for_right_click = context_menu_invoker.clone();
            let is_hovered = self.hovered_repo_tab == Some(repo_id);
            let is_pressed = self.pressed_repo_tab == Some(repo_id);
            let label = repo_tab_labels[ix].clone();
            let initials: SharedString = components::repository_initials(label.as_ref()).into();
            let label_bg = if is_active || context_menu_active {
                theme.colors.sidebar_bg
            } else if is_pressed {
                theme.colors.active
            } else if is_hovered {
                hovered_tab_bg
            } else {
                strip_bg
            };
            let drag_left = self
                .repo_tab_drag_visual
                .filter(|drag| drag.repo_id == repo_id)
                .map(|drag| drag.left);

            let tooltip = Self::repo_tab_tooltip(repo);
            let close_tooltip: SharedString = "Close repository".into();
            let close_hover_bg = repo_tab_close_button_fill(theme, label_bg, false);
            let close_pressed_bg = repo_tab_close_button_fill(theme, label_bg, true);

            let close_button = div()
                .id(("repo_tab_close", repo_id.0))
                .debug_selector(move || format!("repo_tab_close_{}", repo_id.0))
                .flex()
                .items_center()
                .justify_center()
                .size(scaled_px(REPO_TAB_STATUS_SIZE_PX))
                .rounded(px(theme.radii.row))
                // Fully cover any label ink beneath the button; the sibling
                // ramp transitions into this exact background.
                .bg(label_bg)
                .cursor_pointer()
                .hover(move |s| s.bg(close_hover_bg))
                .active(move |s| s.bg(close_pressed_bg))
                .child(svg_icon(
                    "icons/repo_tab_close.svg",
                    theme.colors.danger,
                    scaled_px(REPO_TAB_STATUS_SIZE_PX),
                ))
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    cx.stop_propagation();
                    this.close_repo_tab(repo_id, cx);
                }))
                .gitcomet_tooltip(theme, close_tooltip.clone());
            let close_overlay = div()
                .flex()
                .items_center()
                .h(scaled_px(REPO_TAB_STATUS_SIZE_PX))
                .child(
                    components::trailing_fade(label_bg, scaled_px(REPO_TAB_CLOSE_FADE_WIDTH_PX))
                        .debug_selector(move || format!("repo_tab_close_fade_{}", repo_id.0)),
                )
                .child(close_button);

            let mut tab = components::Tab::new(("repo_tab", repo_id.0))
                .selected(is_active || context_menu_active)
                .horizontal_padding(tab_horizontal_padding)
                .responsive_width(natural_tab_widths[ix]);
            if is_hovered {
                tab = tab.end_slot(close_overlay);
            }

            let show_missing_warning = Self::repo_tab_shows_missing_warning(repo, show_spinner);
            let show_initials = !show_spinner && !show_missing_warning;
            let status_color =
                with_alpha(theme.colors.text, if theme.is_dark { 0.72 } else { 0.62 });
            let badge_color = if is_active {
                theme.colors.accent
            } else {
                status_color
            };
            let tab_label = div()
                .flex()
                .flex_1()
                .items_center()
                .h(scaled_px(REPO_TAB_CONTENT_HEIGHT_PX))
                .gap(scaled_px(REPO_TAB_LABEL_GAP_PX))
                .min_w(px(0.0))
                .child(
                    div()
                        .size(scaled_px(REPO_TAB_STATUS_SIZE_PX))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(show_spinner, |d| {
                            d.debug_selector(move || format!("repo_tab_busy_spinner_{}", repo_id.0))
                                .child(
                                    spinner(("repo_tab_busy_spinner", repo_id.0), badge_color)
                                        .into_any_element(),
                                )
                        })
                        .when(show_missing_warning, |d| {
                            d.child(svg_icon(
                                "icons/warning.svg",
                                theme.colors.warning,
                                scaled_px(12.0),
                            ))
                        })
                        .when(show_initials, |d| {
                            d.child(
                                components::repository_initials_box(
                                    theme,
                                    ui_scale_percent,
                                    initials.clone(),
                                    is_active,
                                )
                                .debug_selector(move || format!("repo_tab_initials_{}", repo_id.0)),
                            )
                        }),
                )
                .child(
                    // A name too long for the tab fades into the tab's own
                    // background rather than being cut mid-glyph.
                    components::FadingText::new(
                        div()
                            .debug_selector(move || format!("repo_tab_label_text_{}", repo_id.0))
                            .text_size(scaled_px(REPO_TAB_FONT_SIZE_PX))
                            .line_height(scaled_px(REPO_TAB_CONTENT_HEIGHT_PX))
                            .child(label),
                        label_bg,
                    )
                    .render(ui_scale_percent)
                    .debug_selector(move || format!("repo_tab_label_{}", repo_id.0))
                    .flex_1(),
                )
                .when(self.open_terminal_repo_ids.contains(&repo_id), |d| {
                    d.child(
                        div()
                            .debug_selector(move || format!("repo_tab_terminal_{}", repo_id.0))
                            .child(svg_icon(
                                "icons/terminal.svg",
                                theme.colors.accent,
                                scaled_px(REPO_TAB_STATUS_SIZE_PX),
                            )),
                    )
                });

            let tab = tab
                .child(tab_label)
                .render(theme, ui_scale_percent)
                .relative()
                .when(show_inactive_separator, |tab| {
                    tab.child(
                        div()
                            .debug_selector(move || {
                                format!("repo_tab_separator_after_{}", repo_id.0)
                            })
                            .absolute()
                            // Each tab has 3px horizontal margins. Paint in
                            // their shared gap so the divider sits between the
                            // two idle tab shapes rather than on either one.
                            .right(scaled_px(-3.0))
                            .top(scaled_px(7.0))
                            .w(px(1.0))
                            .h(scaled_px(16.0))
                            .bg(theme.colors.border_variant),
                    )
                })
                .debug_selector(move || format!("repo_tab_{}", repo_id.0))
                .on_drag(
                    RepoTabDrag {
                        repo_id,
                        cursor_offset_x: Rc::new(Cell::new(px(0.0))),
                        tab_width: Rc::new(Cell::new(px(0.0))),
                        last_center_x: Rc::new(Cell::new(px(0.0))),
                        direction: Rc::new(Cell::new(0)),
                    },
                    move |drag, offset, window, cx| {
                        drag.cursor_offset_x.set(offset.x);
                        drag.last_center_x.set(window.mouse_position().x);
                        cx.new(|_cx| RepoTabDragCarrier)
                    },
                )
                .can_drop(move |dragged, _window, _cx| {
                    dragged.downcast_ref::<RepoTabDrag>().is_some()
                })
                .on_drag_move(cx.listener(
                    move |this, e: &gpui::DragMoveEvent<RepoTabDrag>, _w, cx| {
                        let drag = e.drag(cx);
                        let dragged_repo_id = drag.repo_id;
                        if dragged_repo_id == repo_id {
                            drag.tab_width.set(e.bounds.size.width);
                            return;
                        }

                        let dragged_ix = this
                            .state
                            .repos
                            .iter()
                            .position(|repo| repo.id == dragged_repo_id);
                        match (drag.direction.get(), dragged_ix) {
                            (1, Some(dragged_ix)) if ix <= dragged_ix => return,
                            (-1, Some(dragged_ix)) if ix >= dragged_ix => return,
                            _ => {}
                        }

                        let drag_center_x = drag.center_x(e.event.position.x);
                        let Some(insert_before) = repo_tab_insert_before_for_drop(
                            repo_id,
                            next_repo_id,
                            point(drag_center_x, e.event.position.y),
                            e.bounds,
                            drag.direction.get(),
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
                    this.pressed_repo_tab = None;
                    this.clear_repo_tab_drag_visual(cx);
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, _w, cx| {
                        this.pressed_repo_tab = Some(repo_id);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.hovered_repo_tab = Some(repo_id);
                        let invoker = context_menu_invoker_for_right_click.clone();
                        let anchor = e.position;
                        let _ = this.root_view.update(cx, move |root, cx| {
                            root.set_active_context_menu_invoker(Some(invoker), cx);
                            root.open_popover_at(
                                PopoverKind::RepoTabMenu { repo_id },
                                anchor,
                                window,
                                cx,
                            );
                        });
                        cx.notify();
                    }),
                )
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

            let sliding_tab = RepoTabSlide::new(("repo_tab_slide", repo_id.0), tab, drag_left);
            if drag_left.is_some() {
                bar = bar.tab(gpui::deferred(sliding_tab).with_priority(1));
            } else {
                bar = bar.tab(sliding_tab);
            }
        }

        // A single interactive element: putting the hover style on an inner
        // non-interactive div makes the highlight lag behind the pointer.
        // Browser-style "+" at the end of the strip: one entry point for
        // opening or cloning a repository. It is pinned outside the scroll
        // area so it stays reachable however far the tabs are scrolled, with
        // extra room on its right so it doesn't crowd the title bar badge.
        let root_view = self.root_view.clone();
        let add_repo = div()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            .pl(scaled_px(2.0))
            .pr(scaled_px(8.0))
            .child(
                components::Button::new("add_repo_menu", "")
                    .start_slot(svg_icon(
                        "icons/plus.svg",
                        theme.colors.text_muted,
                        scaled_px(14.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .borderless()
                    .on_click_with_bounds(theme, cx, move |_this, _e, bounds, window, cx| {
                        cx.stop_propagation();
                        let _ = root_view.update(cx, |root, cx| {
                            root.open_popover_for_bounds(
                                PopoverKind::AddRepoMenu,
                                bounds,
                                window,
                                cx,
                            );
                        });
                    })
                    .debug_selector(|| "add_repo_menu".to_string())
                    .block_mouse_except_scroll()
                    .gitcomet_tooltip(theme, "Add repository".into()),
            );

        // Keeps repository-tab drag reordering alive across the empty part of
        // the strip. Ordinary pointer input falls through to the title bar's
        // shared drag surface underneath.
        let tab_strip_drag_listener =
            cx.listener(|this, e: &gpui::DragMoveEvent<RepoTabDrag>, _window, cx| {
                let (repo_id, cursor_offset_x, drag_center_x, dragged_tab_width) = {
                    let drag = e.drag(cx);
                    let drag_center_x = drag.center_x(e.event.position.x);
                    let previous_center_x = drag.last_center_x.replace(drag_center_x);
                    if drag_center_x > previous_center_x {
                        drag.direction.set(1);
                    } else if drag_center_x < previous_center_x {
                        drag.direction.set(-1);
                    }
                    (
                        drag.repo_id,
                        drag.cursor_offset_x.get(),
                        drag_center_x,
                        drag.tab_width.get(),
                    )
                };
                let visual = RepoTabDragVisual {
                    repo_id,
                    left: this.clamp_repo_tab_drag_left(
                        e.event.position.x - cursor_offset_x,
                        repo_id,
                        dragged_tab_width,
                    ),
                };
                if this.repo_tab_drag_visual != Some(visual) {
                    this.repo_tab_drag_visual = Some(visual);
                    cx.notify();
                }

                if drag_center_x < e.bounds.left() {
                    return;
                }

                this.store.dispatch(Msg::ReorderRepoTabs {
                    repo_id,
                    insert_before: None,
                });
            });

        let bar = bar.tab_end(add_repo).render(theme, ui_scale_percent);
        div()
            .size_full()
            .child(bar)
            .id("repo_tabs_responsive_root")
            .on_drag_move(tab_strip_drag_listener)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                    if this.pressed_repo_tab.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                    if this.pressed_repo_tab.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .can_drop(|dragged, _window, _cx| dragged.downcast_ref::<RepoTabDrag>().is_some())
            .on_drop(cx.listener(|this, drag: &RepoTabDrag, _w, cx| {
                // Drop on the bar (but not on a specific tab) -> move to end.
                this.store.dispatch(Msg::ReorderRepoTabs {
                    repo_id: drag.repo_id,
                    insert_before: None,
                });
                this.hovered_repo_tab = None;
                this.pressed_repo_tab = None;
                this.clear_repo_tab_drag_visual(cx);
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
    direction: i8,
) -> Option<Option<RepoId>> {
    // Dragged tabs are constrained to the strip visually, so only the
    // horizontal position participates in reordering. This keeps reordering
    // responsive even if the physical pointer strays above or below the bar.
    // Keep the right edge exclusive so adjacent tabs cannot both match.
    if pos.x < bounds.left() || pos.x >= bounds.right() {
        return None;
    }

    let takeover_x = f32::from(bounds.center().x)
        - f32::from(bounds.size.width) * REPO_TAB_TAKEOVER_BIAS * f32::from(direction);
    Some(repo_tab_insert_before_for_drag_cursor(
        target_repo_id,
        next_repo_id,
        f32::from(pos.x),
        takeover_x,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        RepoTabsBarView, repo_tab_close_button_fill, repo_tab_insert_before_for_drag_cursor,
        repo_tab_insert_before_for_drop,
    };
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_state::model::{RepoId, RepoState};
    use gitcomet_state::msg::Msg;
    use gpui::{Bounds, point, px, size};
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
    fn repo_tab_close_hover_uses_an_opaque_darker_fill() {
        for theme in [
            crate::theme::AppTheme::gitcomet_dark(),
            crate::theme::AppTheme::gitcomet_light(),
        ] {
            let background = theme.colors.sidebar_bg;
            let hover = repo_tab_close_button_fill(theme, background, false);
            let pressed = repo_tab_close_button_fill(theme, background, true);
            let brightness = |color: gpui::Rgba| color.r + color.g + color.b;

            assert_eq!(hover.a, 1.0, "hover background must be solid");
            assert_eq!(pressed.a, 1.0, "pressed background must be solid");
            assert!(brightness(hover) < brightness(background));
            assert!(brightness(pressed) < brightness(hover));
        }
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
    fn repo_tab_rail_drag_ignores_vertical_pointer_position() {
        let bounds = Bounds::new(point(px(20.0), px(40.0)), size(px(100.0), px(34.0)));

        assert_eq!(
            repo_tab_insert_before_for_drop(
                RepoId(5),
                Some(RepoId(6)),
                point(px(30.0), px(-500.0)),
                bounds,
                0,
            ),
            Some(Some(RepoId(5)))
        );
        assert_eq!(
            repo_tab_insert_before_for_drop(
                RepoId(5),
                Some(RepoId(6)),
                point(px(100.0), px(500.0)),
                bounds,
                0,
            ),
            Some(Some(RepoId(6)))
        );
    }

    #[test]
    fn repo_tab_takeover_zone_expands_in_the_drag_direction() {
        let bounds = Bounds::new(point(px(20.0), px(40.0)), size(px(100.0), px(34.0)));

        assert_eq!(
            repo_tab_insert_before_for_drop(
                RepoId(5),
                Some(RepoId(6)),
                point(px(66.0), px(50.0)),
                bounds,
                1,
            ),
            Some(Some(RepoId(6)))
        );
        assert_eq!(
            repo_tab_insert_before_for_drop(
                RepoId(5),
                Some(RepoId(6)),
                point(px(74.0), px(50.0)),
                bounds,
                -1,
            ),
            Some(Some(RepoId(5)))
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
