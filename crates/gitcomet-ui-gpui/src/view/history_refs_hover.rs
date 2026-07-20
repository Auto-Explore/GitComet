use super::*;
use std::cell::RefCell;
use std::rc::Rc;

const HISTORY_REFS_HOVER_CLOSE_GRACE_MS: u64 = 120;
const HISTORY_REFS_HOVER_OPEN_DELAY_MS: u64 = 160;
const HISTORY_REFS_HOVER_WIDTH_PX: f32 = 220.0;
const HISTORY_REFS_HOVER_MAX_HEIGHT_PX: f32 = 260.0;
const HISTORY_REFS_HOVER_POINTER_INSET_PX: f32 = 16.0;
pub(in crate::view) const HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX: &str = "history_refs_hover_menu_";

#[derive(Clone, Debug)]
struct HistoryRefsHoverState {
    repo_id: RepoId,
    commit_id: CommitId,
    source_bounds: Bounds<Pixels>,
    source_pointer_x: Pixels,
    items: Arc<[HistoryRefListItem]>,
}

#[derive(Clone, Copy, Debug)]
struct HistoryRefsHoverLayout {
    anchor: Point<Pixels>,
    anchor_corner: Anchor,
    panel_w: Pixels,
    max_panel_h: Pixels,
}

fn same_history_refs_hover_state(lhs: &HistoryRefsHoverState, rhs: &HistoryRefsHoverState) -> bool {
    lhs.repo_id == rhs.repo_id
        && lhs.commit_id == rhs.commit_id
        && lhs.source_bounds == rhs.source_bounds
        && *lhs.items == *rhs.items
}

pub(in crate::view) struct HistoryRefsHoverHost {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    state: Option<HistoryRefsHoverState>,
    pending_show: Option<HistoryRefsHoverState>,
    item_menu_open: bool,
    pinned_item_ix: Option<usize>,
    panel_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    last_mouse_pos: Point<Pixels>,
    show_seq: u64,
    close_seq: u64,
}

impl HistoryRefsHoverHost {
    pub(in crate::view) fn new(theme: AppTheme, root_view: WeakEntity<GitCometView>) -> Self {
        Self {
            theme,
            root_view,
            state: None,
            pending_show: None,
            item_menu_open: false,
            pinned_item_ix: None,
            panel_bounds: Rc::new(RefCell::new(None)),
            last_mouse_pos: point(px(0.0), px(0.0)),
            show_seq: 0,
            close_seq: 0,
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in crate::view) fn show(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        source_bounds: Bounds<Pixels>,
        items: Arc<[HistoryRefListItem]>,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if items.is_empty() {
            self.close(cx);
            return;
        }

        self.last_mouse_pos = pointer;

        if self.item_menu_open {
            return;
        }

        let next = HistoryRefsHoverState {
            repo_id,
            commit_id,
            source_bounds,
            source_pointer_x: pointer.x,
            items,
        };

        if self
            .state
            .as_ref()
            .is_some_and(|state| same_history_refs_hover_state(state, &next))
        {
            self.cancel_pending_show();
            return;
        }

        if self.state.is_some() {
            self.cancel_pending_show();
            self.show_now(next, window, cx);
            return;
        }

        if let Some(pending) = self.pending_show.as_mut()
            && same_history_refs_hover_state(pending, &next)
        {
            pending.source_pointer_x = pointer.x;
            return;
        }

        self.pending_show = Some(next);
        self.show_seq = self.show_seq.wrapping_add(1);
        let seq = self.show_seq;
        cx.spawn(async move |view: WeakEntity<HistoryRefsHoverHost>, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(HISTORY_REFS_HOVER_OPEN_DELAY_MS))
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.show_seq != seq {
                    return;
                }
                let Some(next) = this.pending_show.take() else {
                    return;
                };
                if !next.source_bounds.contains(&this.last_mouse_pos) {
                    return;
                }
                // An overlay may have opened during the open delay (e.g. a
                // right-click context menu); don't pop the hover under it.
                if this
                    .root_view
                    .upgrade()
                    .is_some_and(|root| root.read(cx).is_overlay_open(cx))
                {
                    return;
                }
                this.state = Some(next);
                this.item_menu_open = false;
                this.pinned_item_ix = None;
                *this.panel_bounds.borrow_mut() = None;
                this.close_seq = this.close_seq.wrapping_add(1);
                cx.notify();
            });
        })
        .detach();
    }

    fn show_now(
        &mut self,
        next: HistoryRefsHoverState,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.close_seq = self.close_seq.wrapping_add(1);
        let changed = self
            .state
            .as_ref()
            .is_none_or(|state| !same_history_refs_hover_state(state, &next));

        if changed {
            self.item_menu_open = false;
            self.pinned_item_ix = None;
            *self.panel_bounds.borrow_mut() = None;
            self.state = Some(next);
            cx.notify();
            window.refresh();
        }
    }

    fn cancel_pending_show(&mut self) {
        if self.pending_show.is_none() {
            return;
        }
        self.pending_show = None;
        self.show_seq = self.show_seq.wrapping_add(1);
    }

    fn cancel_pending_show_if_pointer_left(&mut self, position: Point<Pixels>) {
        let pointer_left_pending_source = self
            .pending_show
            .as_ref()
            .is_some_and(|pending| !pending.source_bounds.contains(&position));
        if pointer_left_pending_source {
            self.cancel_pending_show();
        }
    }

    fn keep_open_at(&mut self, position: Point<Pixels>) {
        self.last_mouse_pos = position;
        if self.state.is_some() {
            self.close_seq = self.close_seq.wrapping_add(1);
        }
    }

    pub(in crate::view) fn close(&mut self, cx: &mut gpui::Context<Self>) {
        let had_state = self.state.take().is_some();
        let had_pending = self.pending_show.take().is_some();
        self.item_menu_open = false;
        self.pinned_item_ix = None;
        if !had_state && !had_pending {
            return;
        }

        if had_pending {
            self.show_seq = self.show_seq.wrapping_add(1);
        }
        if had_state {
            self.close_seq = self.close_seq.wrapping_add(1);
            *self.panel_bounds.borrow_mut() = None;
            cx.notify();
        }
    }

    pub(in crate::view) fn on_mouse_moved(
        &mut self,
        position: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.last_mouse_pos = position;
        self.cancel_pending_show_if_pointer_left(position);
        if self.state.is_none() {
            return;
        }
        if self.item_menu_open {
            self.close_seq = self.close_seq.wrapping_add(1);
            return;
        }
        if self.pointer_inside_open_regions(position) {
            self.close_seq = self.close_seq.wrapping_add(1);
            return;
        }
        self.schedule_close(cx);
    }

    #[cfg(test)]
    pub(in crate::view) fn is_open_for_tests(&self) -> bool {
        self.state.is_some()
    }

    #[cfg(test)]
    pub(in crate::view) fn source_bounds_for_tests(&self) -> Option<Bounds<Pixels>> {
        self.state.as_ref().map(|state| state.source_bounds)
    }

    #[cfg(test)]
    pub(in crate::view) fn pinned_item_ix_for_tests(&self) -> Option<usize> {
        self.pinned_item_ix
    }

    #[cfg(test)]
    pub(in crate::view) fn pinned_item_text_for_tests(&self) -> Option<SharedString> {
        let ix = self.pinned_item_ix?;
        Some(self.state.as_ref()?.items.get(ix)?.text.shared().clone())
    }

    fn pointer_inside_open_regions(&self, position: Point<Pixels>) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.source_bounds.contains(&position))
            || self
                .panel_bounds
                .borrow()
                .as_ref()
                .is_some_and(|bounds| bounds.contains(&position))
    }

    fn schedule_close(&mut self, cx: &mut gpui::Context<Self>) {
        self.close_seq = self.close_seq.wrapping_add(1);
        let seq = self.close_seq;
        cx.spawn(async move |view: WeakEntity<HistoryRefsHoverHost>, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(HISTORY_REFS_HOVER_CLOSE_GRACE_MS))
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.close_seq != seq {
                    return;
                }
                if this.item_menu_open {
                    return;
                }
                if this.pointer_inside_open_regions(this.last_mouse_pos) {
                    return;
                }
                this.state = None;
                *this.panel_bounds.borrow_mut() = None;
                this.close_seq = this.close_seq.wrapping_add(1);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::view) fn is_item_menu_open(&self) -> bool {
        self.item_menu_open
    }

    pub(in crate::view) fn set_item_menu_open(&mut self, open: bool, cx: &mut gpui::Context<Self>) {
        if self.item_menu_open == open {
            return;
        }

        self.item_menu_open = open;
        if open {
            self.cancel_pending_show();
        } else {
            self.pinned_item_ix = None;
        }
        self.close_seq = self.close_seq.wrapping_add(1);

        if !open && self.state.is_some() && !self.pointer_inside_open_regions(self.last_mouse_pos) {
            self.schedule_close(cx);
        }
    }

    fn item_popover_kind(
        repo_id: RepoId,
        commit_id: &CommitId,
        item: &HistoryRefListItem,
    ) -> Option<PopoverKind> {
        match &item.kind {
            HistoryRefListItemKind::Tag { name } => Some(PopoverKind::TagRefMenu {
                repo_id,
                commit_id: commit_id.clone(),
                name: name.clone(),
            }),
            HistoryRefListItemKind::LocalBranch { name } => Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: name.clone(),
            }),
            HistoryRefListItemKind::RemoteBranch { name } => Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Remote,
                name: name.clone(),
            }),
            HistoryRefListItemKind::AttachedHead { branch } => Some(PopoverKind::BranchMenu {
                repo_id,
                section: BranchSection::Local,
                name: branch.clone(),
            }),
            HistoryRefListItemKind::DetachedHead => None,
        }
    }

    fn item_debug_selector(item: &HistoryRefListItem) -> String {
        let prefix = match item.kind {
            HistoryRefListItemKind::Tag { .. } => "history_refs_hover_item_tag",
            HistoryRefListItemKind::LocalBranch { .. } => "history_refs_hover_item_local_branch",
            HistoryRefListItemKind::RemoteBranch { .. } => "history_refs_hover_item_remote_branch",
            HistoryRefListItemKind::AttachedHead { .. } => "history_refs_hover_item_attached_head",
            HistoryRefListItemKind::DetachedHead => "history_refs_hover_item_detached_head",
        };
        format!("{}_{}", prefix, slug_for_debug_selector(item.text.as_ref()))
    }

    fn item_icon(item: &HistoryRefListItem) -> &'static str {
        match item.kind {
            HistoryRefListItemKind::Tag { .. } => "icons/tag.svg",
            HistoryRefListItemKind::LocalBranch { .. }
            | HistoryRefListItemKind::RemoteBranch { .. }
            | HistoryRefListItemKind::AttachedHead { .. } => "icons/git_branch.svg",
            HistoryRefListItemKind::DetachedHead => "icons/question.svg",
        }
    }

    fn select_commit(&self, repo_id: RepoId, commit_id: CommitId, cx: &mut gpui::Context<Self>) {
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.store
                .dispatch(Msg::SelectCommit { repo_id, commit_id });
            cx.notify();
        });
    }

    fn open_item_menu(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        item_ix: usize,
        item: &HistoryRefListItem,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(kind) = Self::item_popover_kind(repo_id, &commit_id, item) else {
            return;
        };
        let invoker: SharedString = format!(
            "{}{}_{}_{}",
            HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX,
            repo_id.0,
            commit_id.as_ref(),
            item.text.as_ref()
        )
        .into();
        self.pinned_item_ix = Some(item_ix);
        self.set_item_menu_open(true, cx);
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
            root.popover_host.update(cx, |host, cx| {
                host.open_popover_at(kind, position, window, cx)
            });
            cx.notify();
        });
    }
}

fn history_refs_hover_layout(
    source: Bounds<Pixels>,
    source_pointer_x: Pixels,
    window_size: Size<Pixels>,
    preferred_panel_w: Pixels,
    preferred_max_panel_h: Pixels,
    pointer_inset: Pixels,
    gap: Pixels,
    margin: Pixels,
) -> HistoryRefsHoverLayout {
    let horizontal_margin = margin.min(window_size.width * 0.5);
    let min_x = horizontal_margin;
    let max_right = (window_size.width - horizontal_margin).max(min_x);
    let available_w = (max_right - min_x).max(px(0.0));
    let panel_w = preferred_panel_w.min(available_w);
    let max_x = (max_right - panel_w).max(min_x);
    let pointer_inset = pointer_inset.min(panel_w * 0.5).max(px(0.0));
    let mut preferred_x = source.left();
    if source_pointer_x < preferred_x + pointer_inset {
        preferred_x = source_pointer_x - pointer_inset;
    } else if source_pointer_x > preferred_x + panel_w - pointer_inset {
        preferred_x = source_pointer_x - panel_w + pointer_inset;
    }
    let anchor_x = preferred_x.max(min_x).min(max_x);

    let vertical_margin = margin.min(window_size.height * 0.5);
    let min_y = vertical_margin;
    let max_y = (window_size.height - vertical_margin).max(min_y);
    let below_anchor_y = (source.bottom() + gap).max(min_y).min(max_y);
    let above_anchor_y = (source.top() - gap).max(min_y).min(max_y);
    let below_h = preferred_max_panel_h.min((max_y - below_anchor_y).max(px(0.0)));
    let above_h = preferred_max_panel_h.min((above_anchor_y - min_y).max(px(0.0)));

    if above_h > below_h {
        HistoryRefsHoverLayout {
            anchor: point(anchor_x, above_anchor_y),
            anchor_corner: Anchor::BottomLeft,
            panel_w,
            max_panel_h: above_h,
        }
    } else {
        HistoryRefsHoverLayout {
            anchor: point(anchor_x, below_anchor_y),
            anchor_corner: Anchor::TopLeft,
            panel_w,
            max_panel_h: below_h,
        }
    }
}

impl Render for HistoryRefsHoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.clone() else {
            return div().into_any_element();
        };

        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let panel_w = ui_scale.px(HISTORY_REFS_HOVER_WIDTH_PX);
        let max_panel_h = ui_scale.px(HISTORY_REFS_HOVER_MAX_HEIGHT_PX);
        let pointer_inset = ui_scale.px(HISTORY_REFS_HOVER_POINTER_INSET_PX);
        let gap = px(0.0);
        let margin = ui_scale.px(8.0);
        let window_size = window.viewport_size();
        let source = state.source_bounds;
        let layout = history_refs_hover_layout(
            source,
            state.source_pointer_x,
            window_size,
            panel_w,
            max_panel_h,
            pointer_inset,
            gap,
            margin,
        );
        let panel_bounds_for_prepaint = Rc::clone(&self.panel_bounds);

        let items = state.items.iter().enumerate().map(|(ix, item)| {
            let item_for_right = item.clone();
            let label = item.text.shared().clone();
            let actionable =
                Self::item_popover_kind(state.repo_id, &state.commit_id, item).is_some();
            let frozen = self.item_menu_open;
            let pinned = self.pinned_item_ix == Some(ix);
            let debug_selector = Self::item_debug_selector(item);
            let icon = Self::item_icon(item);
            let icon_color = match item.kind {
                HistoryRefListItemKind::Tag { .. } => theme.colors.accent,
                HistoryRefListItemKind::DetachedHead => theme.colors.text_muted,
                _ => theme.colors.text_muted,
            };
            div()
                .id(("history_refs_hover_item", ix))
                .debug_selector(move || debug_selector.clone())
                .h(ui_scale.px(24.0))
                .w_full()
                .min_w(px(0.0))
                .px_2()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .line_height(ui_scale.px(16.0))
                .text_color(if actionable {
                    theme.colors.text
                } else {
                    theme.colors.text_muted
                })
                .cursor(if actionable && !frozen {
                    CursorStyle::PointingHand
                } else {
                    CursorStyle::Arrow
                })
                .when(pinned, |row| row.bg(theme.colors.active))
                .hover(move |row| {
                    if pinned {
                        row.bg(theme.colors.active)
                    } else if frozen {
                        row
                    } else {
                        // `theme.colors.hover` is nearly identical to the
                        // elevated popover surface; use a text-tinted overlay
                        // that reads clearly.
                        row.bg(with_alpha(
                            theme.colors.text,
                            if theme.is_dark { 0.08 } else { 0.05 },
                        ))
                    }
                })
                .active(move |row| {
                    if pinned || !frozen {
                        row.bg(theme.colors.active)
                    } else {
                        row
                    }
                })
                .child(svg_icon(icon, icon_color, ui_scale.px(12.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(label),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener({
                        let commit_id = state.commit_id.clone();
                        let item_for_left = item.clone();
                        move |this, e: &MouseUpEvent, window, cx| {
                            cx.stop_propagation();
                            if actionable {
                                this.open_item_menu(
                                    state.repo_id,
                                    commit_id.clone(),
                                    ix,
                                    &item_for_left,
                                    e.position,
                                    window,
                                    cx,
                                );
                            } else {
                                if this.item_menu_open {
                                    return;
                                }
                                this.select_commit(state.repo_id, commit_id.clone(), cx);
                                this.close(cx);
                            }
                        }
                    }),
                )
                .when(actionable, |row| {
                    row.on_mouse_up(
                        MouseButton::Right,
                        cx.listener({
                            let commit_id = state.commit_id.clone();
                            move |this, e: &MouseUpEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_item_menu(
                                    state.repo_id,
                                    commit_id.clone(),
                                    ix,
                                    &item_for_right,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }
                        }),
                    )
                })
                .when(!actionable, |row| {
                    row.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                })
                .into_any_element()
        });

        let panel = div()
            .id("history_refs_hover_panel")
            .debug_selector(|| "history_refs_hover_panel".to_string())
            .w(layout.panel_w)
            .max_h(layout.max_panel_h)
            .overflow_y_scroll()
            .p_1()
            .bg(theme.colors.surface_bg_elevated)
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(theme.radii.popover))
            .shadow(crate::theme::shadow_popover(theme))
            .occlude()
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                this.keep_open_at(e.position);
                cx.stop_propagation();
            }))
            .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
            .children(items);

        let measured_panel = div()
            .on_children_prepainted(move |children_bounds, _window, _app| {
                if let Some(bounds) = children_bounds.first() {
                    *panel_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                }
            })
            .child(panel);

        div()
            .id("history_refs_hover_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                anchored()
                    .position(layout.anchor)
                    .anchor(layout.anchor_corner)
                    .offset(point(px(0.0), px(0.0)))
                    .child(measured_panel),
            )
            .into_any_element()
    }
}

fn slug_for_debug_selector(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut previous_was_separator = true;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('_');
            previous_was_separator = true;
        }
    }

    while slug.ends_with('_') {
        slug.pop();
    }

    if slug.is_empty() {
        "ref".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_refs_hover_layout_clamps_right_and_chooses_above_near_bottom() {
        let layout = history_refs_hover_layout(
            Bounds::new(point(px(280.0), px(150.0)), size(px(20.0), px(16.0))),
            px(290.0),
            size(px(300.0), px(180.0)),
            px(HISTORY_REFS_HOVER_WIDTH_PX),
            px(HISTORY_REFS_HOVER_MAX_HEIGHT_PX),
            px(HISTORY_REFS_HOVER_POINTER_INSET_PX),
            px(0.0),
            px(8.0),
        );

        assert!(matches!(layout.anchor_corner, Anchor::BottomLeft));
        assert_eq!(layout.panel_w, px(220.0));
        assert_eq!(layout.anchor.x, px(72.0));
        assert_eq!(layout.anchor.y, px(150.0));
        assert_eq!(layout.max_panel_h, px(142.0));
        assert!(layout.anchor.x + layout.panel_w <= px(292.0));
        assert!(layout.anchor.y - layout.max_panel_h >= px(8.0));
    }

    #[test]
    fn history_refs_hover_layout_shrinks_width_in_narrow_viewport() {
        let layout = history_refs_hover_layout(
            Bounds::new(point(px(140.0), px(40.0)), size(px(16.0), px(16.0))),
            px(148.0),
            size(px(160.0), px(240.0)),
            px(HISTORY_REFS_HOVER_WIDTH_PX),
            px(HISTORY_REFS_HOVER_MAX_HEIGHT_PX),
            px(HISTORY_REFS_HOVER_POINTER_INSET_PX),
            px(0.0),
            px(8.0),
        );

        assert_eq!(layout.panel_w, px(144.0));
        assert_eq!(layout.anchor.x, px(8.0));
        assert!(layout.anchor.x + layout.panel_w <= px(152.0));
    }

    #[test]
    fn history_refs_hover_layout_uses_below_when_more_space_is_below() {
        let layout = history_refs_hover_layout(
            Bounds::new(point(px(24.0), px(20.0)), size(px(40.0), px(16.0))),
            px(44.0),
            size(px(320.0), px(220.0)),
            px(HISTORY_REFS_HOVER_WIDTH_PX),
            px(HISTORY_REFS_HOVER_MAX_HEIGHT_PX),
            px(HISTORY_REFS_HOVER_POINTER_INSET_PX),
            px(0.0),
            px(8.0),
        );

        assert!(matches!(layout.anchor_corner, Anchor::TopLeft));
        assert_eq!(layout.anchor, point(px(24.0), px(36.0)));
        assert_eq!(layout.max_panel_h, px(176.0));
        assert!(layout.anchor.y + layout.max_panel_h <= px(212.0));
    }

    #[test]
    fn history_refs_hover_layout_keeps_source_pointer_inside_panel_top_edge() {
        let layout = history_refs_hover_layout(
            Bounds::new(point(px(24.0), px(20.0)), size(px(360.0), px(16.0))),
            px(340.0),
            size(px(520.0), px(220.0)),
            px(HISTORY_REFS_HOVER_WIDTH_PX),
            px(HISTORY_REFS_HOVER_MAX_HEIGHT_PX),
            px(HISTORY_REFS_HOVER_POINTER_INSET_PX),
            px(0.0),
            px(8.0),
        );

        assert!(layout.anchor.x < px(340.0));
        assert!(layout.anchor.x + layout.panel_w > px(340.0));
    }
}
