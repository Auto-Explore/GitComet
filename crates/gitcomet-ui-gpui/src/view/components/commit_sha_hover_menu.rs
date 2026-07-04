use crate::font_preferences::DEFAULT_UI_FONT_FAMILY;
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use crate::view::GitCometView;
use gitcomet_core::domain::{CommitId, LogScope};
use gitcomet_state::model::RepoId;
use gitcomet_state::msg::Msg;
use gpui::prelude::*;
use gpui::{
    Bounds, ElementId, Entity, FocusHandle, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    SharedString, Task, WeakEntity, Window, anchored, deferred, div, point, px,
};
use std::ops::Range;
use std::sync::Arc;

const COMMIT_SHA_HOVER_MENU_OPEN_DELAY_MS: u64 = 300;
const COMMIT_SHA_HOVER_MENU_CLOSE_DELAY_MS: u64 = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingHoverAction {
    Open(usize),
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitShaLink {
    pub range: Range<usize>,
    pub commit_id: CommitId,
}

pub struct CommitShaHoverMenu {
    input: Entity<crate::kit::TextInput>,
    repo_id: RepoId,
    links: Arc<[CommitShaLink]>,
    /// Whether the menu offers "Navigate" (false for the commit's own SHA, where
    /// navigating to yourself is a no-op).
    allow_navigate: bool,
    theme: AppTheme,
    ui_scale: UiScale,
    id: SharedString,
    root_view: WeakEntity<GitCometView>,
    menu_focus_handle: FocusHandle,
    trigger_hovered: bool,
    menu_hovered: bool,
    menu_has_focus: bool,
    active_link_focused: bool,
    hovered_link_ix: Option<usize>,
    pending_action: Option<PendingHoverAction>,
    open_link_ix: Option<usize>,
    hover_delay_seq: u64,
    hover_task: Option<Task<()>>,
    menu_bounds: Option<Bounds<Pixels>>,
}

impl CommitShaHoverMenu {
    pub fn new(
        input: Entity<crate::kit::TextInput>,
        repo_id: RepoId,
        links: Arc<[CommitShaLink]>,
        theme: AppTheme,
        ui_scale: UiScale,
        id: impl Into<SharedString>,
        root_view: WeakEntity<GitCometView>,
        allow_navigate: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            input,
            repo_id,
            links,
            allow_navigate,
            theme,
            ui_scale,
            id: id.into(),
            root_view,
            menu_focus_handle: cx.focus_handle().tab_index(0).tab_stop(false),
            trigger_hovered: false,
            menu_hovered: false,
            menu_has_focus: false,
            active_link_focused: false,
            hovered_link_ix: None,
            pending_action: None,
            open_link_ix: None,
            hover_delay_seq: 0,
            hover_task: None,
            menu_bounds: None,
        }
    }

    pub fn sync(
        &mut self,
        input: Entity<crate::kit::TextInput>,
        repo_id: RepoId,
        links: Arc<[CommitShaLink]>,
        theme: AppTheme,
        ui_scale: UiScale,
        id: impl Into<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let id = id.into();
        let props_changed = self.input != input || self.repo_id != repo_id || self.links != links;
        self.input = input;
        self.repo_id = repo_id;
        self.links = links;
        self.theme = theme;
        self.ui_scale = ui_scale;
        self.id = id;

        if props_changed {
            self.cancel_hover_delay();
            self.trigger_hovered = false;
            self.menu_hovered = false;
            self.menu_has_focus = false;
            self.active_link_focused = false;
            self.hovered_link_ix = None;
            self.pending_action = None;
            self.open_link_ix = None;
            self.menu_bounds = None;
        }

        cx.notify();
    }

    fn next_hover_delay_seq(&mut self) -> u64 {
        self.hover_delay_seq = self.hover_delay_seq.wrapping_add(1).max(1);
        self.hover_delay_seq
    }

    fn cancel_hover_delay(&mut self) {
        self.hover_task.take();
        self.pending_action = None;
        self.next_hover_delay_seq();
    }

    fn close_menu(&mut self, cx: &mut gpui::Context<Self>) {
        if self.open_link_ix.is_none()
            && self.pending_action.is_none()
            && self.menu_bounds.is_none()
        {
            return;
        }

        self.cancel_hover_delay();
        self.open_link_ix = None;
        self.menu_bounds = None;
        cx.notify();
    }

    fn maybe_close_menu(&mut self, cx: &mut gpui::Context<Self>) {
        if self.trigger_hovered
            || self.menu_hovered
            || self.menu_has_focus
            || self.active_link_focused
        {
            return;
        }

        if self.open_link_ix.is_some() {
            self.schedule_close_menu(cx);
            return;
        }

        self.close_menu(cx);
    }

    fn schedule_open_for_link(&mut self, link_ix: usize, cx: &mut gpui::Context<Self>) {
        if self.open_link_ix == Some(link_ix)
            || self.pending_action == Some(PendingHoverAction::Open(link_ix))
        {
            return;
        }

        self.cancel_hover_delay();
        self.pending_action = Some(PendingHoverAction::Open(link_ix));
        let seq = self.next_hover_delay_seq();
        let task = cx.spawn(
            async move |view: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(
                        COMMIT_SHA_HOVER_MENU_OPEN_DELAY_MS,
                    ))
                    .await;
                let _ = view.update(cx, move |this, cx| {
                    if this.hover_delay_seq != seq
                        || this.pending_action != Some(PendingHoverAction::Open(link_ix))
                        || this.hovered_link_ix != Some(link_ix)
                        || !this.trigger_hovered
                    {
                        return;
                    }

                    this.hover_task = None;
                    this.pending_action = None;
                    this.open_link_ix = Some(link_ix);
                    cx.notify();
                });
            },
        );
        self.hover_task = Some(task);
    }

    fn schedule_close_menu(&mut self, cx: &mut gpui::Context<Self>) {
        if self.open_link_ix.is_none() || self.pending_action == Some(PendingHoverAction::Close) {
            return;
        }

        self.cancel_hover_delay();
        self.pending_action = Some(PendingHoverAction::Close);
        let seq = self.next_hover_delay_seq();
        let task = cx.spawn(
            async move |view: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(
                        COMMIT_SHA_HOVER_MENU_CLOSE_DELAY_MS,
                    ))
                    .await;
                let _ = view.update(cx, move |this, cx| {
                    if this.hover_delay_seq != seq
                        || this.pending_action != Some(PendingHoverAction::Close)
                        || this.trigger_hovered
                        || this.menu_hovered
                        || this.menu_has_focus
                        || this.active_link_focused
                    {
                        return;
                    }

                    this.hover_task = None;
                    this.pending_action = None;
                    this.open_link_ix = None;
                    this.menu_bounds = None;
                    cx.notify();
                });
            },
        );
        self.hover_task = Some(task);
    }

    fn active_link(&self) -> Option<&CommitShaLink> {
        self.open_link_ix.and_then(|ix| self.links.get(ix))
    }

    fn active_link_has_input_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        let Some(link) = self.active_link() else {
            return false;
        };

        self.input.read_with(cx, |input, _| {
            if !input.focus_handle().is_focused(window) {
                return false;
            }

            let selected = input.selected_range();
            if selected.is_empty() {
                let caret = selected.start;
                caret >= link.range.start && caret <= link.range.end
            } else {
                selected.start < link.range.end && selected.end > link.range.start
            }
        })
    }

    fn anchor_for_open_link(&self, cx: &gpui::App) -> Option<gpui::Point<Pixels>> {
        let link = self.active_link()?;
        self.input
            .read_with(cx, |input, _| input.hotspot_bounds(&link.range))
            .map(|bounds| point(bounds.left(), bounds.bottom()))
    }

    fn update_pointer_target(
        &mut self,
        position: gpui::Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        let hotspot_ranges = self
            .links
            .iter()
            .map(|link| link.range.clone())
            .collect::<Vec<_>>();
        let hovered_link_ix = self.input.read_with(cx, |input, _| {
            input.hotspot_range_index_at_position(position, &hotspot_ranges)
        });

        if let Some(link_ix) = hovered_link_ix {
            self.trigger_hovered = true;
            self.hovered_link_ix = Some(link_ix);
            if self.pending_action == Some(PendingHoverAction::Close) {
                self.cancel_hover_delay();
            }
            if self.open_link_ix.is_some() {
                if self.open_link_ix != Some(link_ix) {
                    self.cancel_hover_delay();
                    self.open_link_ix = Some(link_ix);
                    cx.notify();
                }
            } else {
                self.schedule_open_for_link(link_ix, cx);
            }
            return;
        }

        self.trigger_hovered = false;
        self.hovered_link_ix = None;
        if matches!(self.pending_action, Some(PendingHoverAction::Open(_))) {
            self.cancel_hover_delay();
        }
        if self
            .menu_bounds
            .is_some_and(|bounds| bounds.contains(&position))
        {
            return;
        }
        self.maybe_close_menu(cx);
    }

    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.update_pointer_target(event.position, cx);
    }

    fn on_root_hover(
        &mut self,
        hovering: &bool,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if *hovering {
            return;
        }

        self.trigger_hovered = false;
        self.hovered_link_ix = None;
        if matches!(self.pending_action, Some(PendingHoverAction::Open(_))) {
            self.cancel_hover_delay();
        }
        self.maybe_close_menu(cx);
    }

    fn on_menu_hover(
        &mut self,
        hovering: &bool,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.menu_hovered = *hovering;
        if !*hovering {
            self.maybe_close_menu(cx);
        } else {
            if self.pending_action == Some(PendingHoverAction::Close) {
                self.cancel_hover_delay();
            }
            cx.notify();
        }
    }

    fn on_menu_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        cx.stop_propagation();
        window.focus(&self.menu_focus_handle, cx);
        self.menu_has_focus = true;
        cx.notify();
    }

    fn on_menu_entry_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(link) = self.active_link().cloned() else {
            return;
        };

        self.close_menu(cx);
        let repo_id = self.repo_id;
        let _ = self.root_view.update(cx, move |root, cx| {
            root.main_pane.update(cx, |main, cx| {
                main.reveal_history_commit(
                    repo_id,
                    link.commit_id,
                    Some(LogScope::AllBranches),
                    cx,
                );
            });
        });
        window.refresh();
    }

    fn on_browse_entry_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(link) = self.active_link().cloned() else {
            return;
        };
        self.close_menu(cx);
        let repo_id = self.repo_id;
        let _ = self.root_view.update(cx, move |root, _cx| {
            root.store.dispatch(Msg::BrowseRepositoryAtCommit {
                repo_id,
                commit_id: link.commit_id,
            });
        });
        window.refresh();
    }

    fn render_menu(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let mut entries = div().flex().flex_col();
        if self.allow_navigate {
            let navigate_selector = format!("{}_navigate", self.id);
            entries = entries.child(
                super::context_menu_entry(
                    (
                        ElementId::from("commit_sha_hover_menu_navigate"),
                        self.id.clone(),
                    ),
                    self.theme,
                    self.ui_scale,
                    false,
                    false,
                    Some("icons/link.svg".into()),
                    "Navigate",
                    None,
                )
                .debug_selector(move || navigate_selector.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_menu_entry_mouse_down),
                ),
            );
        }
        let browse_selector = format!("{}_browse", self.id);
        entries = entries.child(
            super::context_menu_entry(
                (
                    ElementId::from("commit_sha_hover_menu_browse"),
                    self.id.clone(),
                ),
                self.theme,
                self.ui_scale,
                false,
                false,
                Some("icons/history.svg".into()),
                "Browse repository at this point",
                None,
            )
            .debug_selector(move || browse_selector.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::on_browse_entry_mouse_down),
            ),
        );

        let menu_surface = super::context_menu(self.theme, entries)
            .id((ElementId::from("commit_sha_hover_menu"), self.id.clone()))
            .debug_selector({
                let id = format!("{}_menu", self.id);
                move || id.clone()
            })
            .w(self.ui_scale.px(300.0))
            .p_1()
            .font_family(DEFAULT_UI_FONT_FAMILY)
            .bg(self.theme.colors.surface_bg_elevated)
            .border_1()
            .border_color(self.theme.colors.border)
            .rounded(px(self.theme.radii.popover))
            .shadow(crate::theme::shadow_popover(self.theme))
            .track_focus(&self.menu_focus_handle)
            .on_hover(cx.listener(Self::on_menu_hover))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_menu_mouse_down));

        let view = cx.entity().downgrade();
        div()
            .on_children_prepainted(move |children_bounds, _window, cx| {
                let bounds = children_bounds.first().copied();
                let _ = view.update(cx, |this, _cx| {
                    this.menu_bounds = bounds;
                });
            })
            .child(menu_surface)
    }
}

impl Render for CommitShaHoverMenu {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let active_link_focused = self.active_link_has_input_focus(window, cx);
        if self.active_link_focused != active_link_focused {
            self.active_link_focused = active_link_focused;
            if !active_link_focused {
                self.maybe_close_menu(cx);
            }
        }

        let menu_has_focus = self.menu_focus_handle.is_focused(window);
        if self.menu_has_focus != menu_has_focus {
            self.menu_has_focus = menu_has_focus;
            if !menu_has_focus {
                self.maybe_close_menu(cx);
            }
        }

        let debug_selector = self.id.to_string();
        let mut root = div()
            .id((
                ElementId::from("commit_sha_hover_menu_root"),
                self.id.clone(),
            ))
            .debug_selector(move || debug_selector.clone())
            .relative()
            .w_full()
            .min_w(px(0.0))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_hover(cx.listener(Self::on_root_hover))
            .child(self.input.clone());

        if let Some(anchor) = self.anchor_for_open_link(cx) {
            root = root.child(
                deferred(
                    anchored()
                        .position(anchor)
                        .offset(point(px(0.0), px(0.0)))
                        .child(self.render_menu(cx)),
                )
                .priority(10_000),
            );
        } else {
            self.menu_bounds = None;
        }

        root
    }
}
