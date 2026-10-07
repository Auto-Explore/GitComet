use super::*;
use crate::view::rows::sidebar::branch_commit_id;
use crate::view::sidebar_sticky::{self, SidebarRowSurface};

type StickyLayoutCache =
    std::cell::RefCell<Option<((f32, f32, f32), sidebar_sticky::StickyLayout)>>;

pub(super) struct SelectionCache {
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    selected: Option<SelectedBranch>,
    tip: Option<CommitId>,
}

pub(super) struct StickyContext {
    rows: Rc<[BranchSidebarRow]>,
    head: Option<String>,
    selected: Option<BranchMenuTarget>,
    selected_pin_key: Option<SharedString>,
    pin_roots: Rc<[usize]>,
    row_keys: Rc<[SharedString]>,
    layout_cache: StickyLayoutCache,
    pub(super) eligible_rows: Rc<[usize]>,
    priority_rows: Rc<[usize]>,
    sections: Rc<[usize]>,
    selected_row: Option<(usize, SharedString)>,
    anchor: Option<(SharedString, f32)>,
    row_height: Option<f32>,
    click_guard: Option<StickyClickGuard>,
}

#[derive(Clone, Copy)]
struct StickyClickGuard {
    row: usize,
    bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    scroll_offset: Point<Pixels>,
    row_height: Pixels,
    held: bool,
}

impl StickyContext {
    fn fitted_rows(&self, height: f32, row_height: f32) -> Option<&Rc<[usize]>> {
        let fitted = sidebar_sticky::fitted_rows(
            &self.eligible_rows,
            &self.priority_rows,
            &self.sections,
            height,
            row_height,
        );
        if fitted.is_empty() {
            None
        } else if fitted.len() == self.eligible_rows.len() {
            Some(&self.eligible_rows)
        } else if fitted.len() == self.priority_rows.len() {
            Some(&self.priority_rows)
        } else {
            Some(&self.sections)
        }
    }

    fn layout(&self, height: f32, row_height: f32, scroll: f32) -> sidebar_sticky::StickyLayout {
        let key = (height, row_height, scroll);
        if let Some((cached, layout)) = self.layout_cache.borrow().as_ref()
            && *cached == key
        {
            return layout.clone();
        }
        let base = self
            .fitted_rows(height, row_height)
            .map_or(&[][..], |rows| rows.as_ref());
        let layout = sidebar_sticky::fit_pins(
            base,
            &self.pin_roots,
            self.selected_row
                .as_ref()
                .filter(|_| self.selected_pin_key.is_some())
                .map(|(ix, _)| *ix),
            scroll,
            height,
            row_height,
        );
        *self.layout_cache.borrow_mut() = Some((key, layout.clone()));
        layout
    }

    /// Save the first uncovered stable row and its position in slot units so
    /// asynchronous data updates and density/scale changes retain the context.
    fn capture_anchor(
        &self,
        scroll_handle: &UniformListScrollHandle,
    ) -> Option<(SharedString, f32)> {
        let handle = scroll_handle.0.borrow();
        let size = handle.last_item_size?;
        let row_height = self.row_height?;
        if row_height <= 0.0 {
            return None;
        }
        let scroll = -f32::from(handle.base_handle.offset().y);
        // At the start, reveal newly loaded/inserted rows instead of anchoring
        // a later section and scrolling past the new content.
        if scroll <= 0.0 {
            return None;
        }
        let layout = self.layout(f32::from(size.item.height), row_height, scroll);
        let stacked = layout
            .slots
            .iter()
            .enumerate()
            .take_while(|(rank, slot)| {
                slot.row() as f32 * row_height - scroll <= *rank as f32 * row_height
            })
            .count();
        let start = (scroll / row_height).ceil() as usize + stacked;
        self.rows
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, row)| sidebar_sticky::same_row(row, row))
            .map(|(ix, _)| (self.row_keys[ix].clone(), ix as f32 - scroll / row_height))
    }
}

const HEADER_SCROLL_DURATION: std::time::Duration = std::time::Duration::from_millis(150);

pub(super) struct ScrollAnimation {
    repo_id: RepoId,
    key: SharedString,
    row_keys: Rc<[SharedString]>,
    start: f32,
    target: f32,
    last_offset: f32,
    started: std::time::Instant,
    bounds: Bounds<Pixels>,
    row_height: f32,
}

fn animated_scroll_offset(start: f32, target: f32, elapsed: std::time::Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / HEADER_SCROLL_DURATION.as_secs_f32()).clamp(0.0, 1.0);
    start + (target - start) * (1.0 - (1.0 - t).powi(3))
}

fn navigation_destination(
    context: &StickyContext,
    ix: usize,
    center: bool,
    height: f32,
    row_height: f32,
) -> f32 {
    let mut offset = (ix as f32 * row_height).max(0.0);
    for _ in 0..3 {
        let layout = context.layout(height, row_height, offset);
        let headers = layout
            .slots
            .iter()
            .map(sidebar_sticky::StickySlot::row)
            .collect::<Vec<_>>();
        offset = sidebar_sticky::navigation_offset(
            ix,
            &headers,
            height,
            row_height,
            context.rows.len(),
            center,
        );
    }
    offset
}

#[derive(Clone)]
pub(super) enum NavigationTarget {
    #[cfg(test)]
    Header(SharedString),
    Branch(BranchMenuTarget),
    RowKey(SharedString),
}

impl SidebarPaneView {
    /// A first click may add offscreen ancestors above this row. Keep its hit
    /// target in place until the pointer leaves or the user changes the view,
    /// so a native double click still reaches the same branch on every OS.
    pub(in crate::view) fn retain_sidebar_click_target(
        &mut self,
        row: usize,
        surface: SidebarRowSurface,
        event: &ClickEvent,
    ) {
        if !matches!(surface, SidebarRowSurface::Tree | SidebarRowSurface::Pins)
            || event.mouse_position().is_none()
        {
            return;
        }
        let Some(context) = &mut self.sticky_context else {
            return;
        };
        let Some(row_height) = context.row_height.map(px) else {
            return;
        };
        let handle = self.branches_scroll.0.borrow();
        let viewport = handle.base_handle.bounds();
        let scroll_offset = handle.base_handle.offset();
        let bounds = Bounds::new(
            point(
                viewport.left(),
                viewport.top() + scroll_offset.y + row_height * row,
            ),
            gpui::size(viewport.size.width, row_height),
        );
        if bounds.contains(&event.position()) {
            context.click_guard = Some(StickyClickGuard {
                row,
                bounds,
                viewport,
                scroll_offset,
                row_height,
                held: false,
            });
        }
    }

    pub(super) fn clear_sidebar_click_target(&mut self) -> bool {
        self.sticky_context
            .as_mut()
            .and_then(|context| context.click_guard.take())
            .is_some()
    }

    pub(super) fn sidebar_click_target_bounds(&self) -> Option<Bounds<Pixels>> {
        self.sticky_context
            .as_ref()?
            .click_guard
            .map(|guard| guard.bounds)
    }

    pub(in crate::view) fn sidebar_selected_tip(&mut self) -> Option<CommitId> {
        let repo = self.active_repo()?;
        let fingerprint = BranchSidebarFingerprint::from_repo(repo);
        if let Some(cache) = &self.sidebar_selection_cache
            && cache.repo_id == repo.id
            && cache.fingerprint == fingerprint
            && cache.selected == self.selected_branch
        {
            return cache.tip.clone();
        }
        let tip = self
            .selected_branch
            .as_ref()
            .filter(|s| s.repo_id == repo.id)
            .and_then(|s| branch_commit_id(repo, &s.target));
        self.sidebar_selection_cache = Some(SelectionCache {
            repo_id: repo.id,
            fingerprint,
            selected: self.selected_branch.clone(),
            tip: tip.clone(),
        });
        tip
    }

    pub(super) fn update_sticky_context(&mut self, presentation: &SidebarPresentation) {
        // A selected pin can disappear after unpinning, collapsing or filtering.
        // Reconcile once per presentation change, not on every scroll frame.
        if let Some(key) = &self.selected_branch_pin_key
            && !self
                .sticky_context
                .as_ref()
                .is_some_and(|context| Rc::ptr_eq(&context.rows, &presentation.rows))
            && !presentation.row_keys[..presentation.pins.len()].contains(key)
        {
            self.selected_branch_pin_key = None;
        }
        let tip = self.sidebar_selected_tip();
        let Some(repo) = self.active_repo() else {
            return;
        };
        let head = match &repo.head_branch {
            Loadable::Ready(head) if head != "HEAD" => Some(head.as_str()),
            _ => None,
        };
        let selected = self
            .selected_branch()
            .filter(|selected| {
                selected.repo_id == repo.id
                    && tip.is_some()
                    && tip == repo.history_state.selected_commit
            })
            .map(|selected| &selected.target);
        let click_guard = self.sticky_context.as_ref().and_then(|context| {
            context.click_guard.filter(|guard| {
                let handle = self.branches_scroll.0.borrow();
                Rc::ptr_eq(&context.rows, &presentation.rows)
                    && context.head.as_deref() == head
                    && self.pending_sidebar_navigation.is_none()
                    && guard.viewport == handle.base_handle.bounds()
                    && guard.scroll_offset == handle.base_handle.offset()
            })
        });
        if click_guard.is_some_and(|guard| guard.held) {
            return;
        }
        if let Some(context) = &self.sticky_context
            && Rc::ptr_eq(&context.rows, &presentation.rows)
            && context.head.as_deref() == head
            && context.selected.as_ref() == selected
            && context.selected_pin_key == self.selected_branch_pin_key
        {
            return;
        }
        let current = head
            .filter(|head| {
                matches!(
                    &repo.branches,
                    Loadable::Ready(branches) if branches.iter().any(|branch| branch.name == *head)
                )
            })
            .map(BranchMenuTarget::local);
        let head = head.map(str::to_owned);
        let selected = selected.cloned();
        // Only visible branch rows qualify: filtering or a closed ancestor
        // removes the leaf as well as its sticky path. Pinned copies of the
        // same branch are distinguished by their root-specific row keys.
        // Resolve once per model/selection change, never while scrolling.
        let selected_row = selected.as_ref().and_then(|selected| {
            presentation
                .rows
                .iter()
                .enumerate()
                .position(|(ix, row)| {
                    let pin_key =
                        (ix < presentation.pins.len()).then(|| &presentation.row_keys[ix]);
                    pin_key == self.selected_branch_pin_key.as_ref()
                        && matches!(row, BranchSidebarRow::Branch { target, .. } if target == selected)
                })
                .map(|ix| (ix, presentation.row_keys[ix].clone()))
        });
        let mut priority_rows = presentation.structure.sections.clone();
        if let Some((ix, _)) = &selected_row
            && presentation.structure.pin_roots.binary_search(ix).is_err()
        {
            priority_rows.push(*ix);
            priority_rows.sort_unstable();
        }
        let mut eligible_rows = priority_rows.clone();
        for target in current.iter().chain(
            selected
                .iter()
                .filter(|_| self.selected_branch_pin_key.is_none()),
        ) {
            eligible_rows.extend(presentation.structure.active_path(
                &presentation.rows,
                target,
                &presentation.search,
            ));
        }
        eligible_rows.sort_unstable();
        eligible_rows.dedup();
        let anchor = self.sticky_context.as_ref().and_then(|old| {
            if old.anchor.is_some() || Rc::ptr_eq(&old.rows, &presentation.rows) {
                return old.anchor.clone();
            }
            old.capture_anchor(&self.branches_scroll)
        });
        let row_height = self.sticky_context.as_ref().and_then(|old| old.row_height);
        let next = StickyContext {
            rows: Rc::clone(&presentation.rows),
            head,
            selected,
            selected_pin_key: self.selected_branch_pin_key.clone(),
            pin_roots: presentation.structure.pin_roots.clone().into(),
            row_keys: Rc::clone(&presentation.row_keys),
            layout_cache: Default::default(),
            eligible_rows: eligible_rows.into(),
            priority_rows: priority_rows.into(),
            sections: presentation.structure.sections.clone().into(),
            selected_row,
            anchor,
            row_height,
            click_guard,
        };
        if let Some(guard) = click_guard
            && next
                .selected_row
                .as_ref()
                .is_some_and(|(ix, _)| *ix == guard.row)
            && let layout = next.layout(
                f32::from(guard.viewport.size.height),
                f32::from(guard.row_height),
                -f32::from(guard.scroll_offset.y),
            )
            && let Some(rank) = layout.slots.iter().position(
                |slot| matches!(slot, sidebar_sticky::StickySlot::Row(ix) if *ix == guard.row),
            )
        {
            let y = sidebar_sticky::row_y(
                guard.row,
                rank,
                layout.slots.len(),
                -f32::from(guard.scroll_offset.y),
                f32::from(guard.viewport.size.height),
                f32::from(guard.row_height),
            );
            if (px(y) + guard.viewport.top() - guard.bounds.top()).abs() > px(0.5) {
                // Cache this decision: waiting for another input must not
                // rebuild the branch paths on each hover/redraw frame.
                self.sticky_context
                    .as_mut()
                    .unwrap()
                    .click_guard
                    .as_mut()
                    .unwrap()
                    .held = true;
                return;
            }
        }
        self.sticky_context = Some(next);
    }

    pub(in crate::view) fn decorated_sidebar_rows(&self) -> Rc<[usize]> {
        let Some(context) = &self.sticky_context else {
            return Rc::from([]);
        };
        let handle = self.branches_scroll.0.borrow();
        let Some(size) = handle.last_item_size else {
            return Rc::from([]);
        };
        let Some(row_height) = context.row_height else {
            return Rc::from([]);
        };
        context
            .layout(
                f32::from(size.item.height),
                row_height,
                -f32::from(handle.base_handle.offset().y),
            )
            .rows
    }

    pub(in crate::view) fn navigate_sidebar_row(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(key) = self
            .sticky_context
            .as_ref()
            .and_then(|context| context.row_keys.get(ix))
            .cloned()
        {
            self.navigate_sidebar_row_key(key, cx);
        }
    }

    pub(in crate::view) fn navigate_sidebar_row_key(
        &mut self,
        key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_sidebar_navigation = Some(NavigationTarget::RowKey(key));
        self.clear_sidebar_click_target();
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::view) fn navigate_sidebar_header(
        &mut self,
        key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_sidebar_navigation = Some(NavigationTarget::Header(key));
        cx.notify();
    }

    pub(super) fn prepare_sidebar_scroll(
        &mut self,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.sticky_context.as_ref().is_some_and(|context| {
            context
                .click_guard
                .is_some_and(|guard| guard.viewport != bounds || guard.row_height != row_height)
        }) {
            self.clear_sidebar_click_target();
        }
        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return;
        };
        let Some(context) = self.sticky_context.as_mut() else {
            return;
        };
        let height = f32::from(bounds.size.height);
        let row_height = f32::from(row_height);
        if context.anchor.is_none() && context.row_height.is_some_and(|old| old != row_height) {
            context.anchor = context.capture_anchor(&self.branches_scroll);
        }
        context.row_height = Some(row_height);
        let navigation = self.pending_sidebar_navigation.take();
        let target = navigation.as_ref().and_then(|target| match target {
            #[cfg(test)]
            NavigationTarget::Header(key) => presentation
                .structure
                .headers
                .get(key)
                .copied()
                .map(|ix| (ix, false)),
            NavigationTarget::RowKey(key) => presentation.row_keys.iter().position(|candidate| candidate == key).map(|ix| (ix, false)),
            NavigationTarget::Branch(target) => presentation
                .rows
                .iter()
                .rposition(|row| matches!(row, BranchSidebarRow::Branch { target: candidate, .. } if candidate == target))
                .map(|ix| (ix, true)),
        });
        let offset = if let Some((ix, center)) = target {
            context.anchor = None;
            let offset = navigation_destination(context, ix, center, height, row_height);
            let current = -f32::from(self.branches_scroll.0.borrow().base_handle.offset().y);
            self.sidebar_scroll_animation = None;
            if !center
                && crate::ui_runtime::current().uses_pane_animations()
                && !cx.reduce_motion()
                && self.collapsed_popover_section.is_none()
                && (current - offset).abs() > 0.5
                && let Some(repo_id) = self.state.active_repo
            {
                self.sidebar_scroll_animation = Some(ScrollAnimation {
                    repo_id,
                    key: presentation.row_keys[ix].clone(),
                    row_keys: Rc::clone(&presentation.row_keys),
                    start: current,
                    target: offset,
                    last_offset: current,
                    started: std::time::Instant::now(),
                    bounds,
                    row_height,
                });
            }
            self.sidebar_scroll_animation.is_none().then_some(offset)
        } else {
            if self.sidebar_scroll_animation.is_some() {
                context.anchor = None;
            }
            context.anchor.take().and_then(|(anchor, y)| {
                presentation
                    .row_keys
                    .iter()
                    .position(|key| key == &anchor)
                    .map(|ix| {
                        ((ix as f32 - y) * row_height).clamp(
                            0.0,
                            (presentation.rows.len() as f32 * row_height - height).max(0.0),
                        )
                    })
            })
        };
        let mut offset = offset;
        if let Some(mut animation) = self.sidebar_scroll_animation.take() {
            let current = -f32::from(self.branches_scroll.0.borrow().base_handle.offset().y);
            let valid = self.state.active_repo == Some(animation.repo_id)
                && self.state.sidebar_mode == SidebarMode::Branches
                && bounds == animation.bounds
                && row_height == animation.row_height
                && (current - animation.last_offset).abs() <= 0.5;
            if valid {
                let mut target_exists = true;
                if !Rc::ptr_eq(&animation.row_keys, &presentation.row_keys) {
                    if let Some(ix) = presentation
                        .row_keys
                        .iter()
                        .position(|key| *key == animation.key)
                    {
                        animation.target =
                            navigation_destination(context, ix, false, height, row_height);
                        animation.row_keys = Rc::clone(&presentation.row_keys);
                    } else {
                        target_exists = false;
                    }
                }
                if target_exists {
                    let elapsed = animation.started.elapsed();
                    let next = animated_scroll_offset(animation.start, animation.target, elapsed)
                        .clamp(
                            0.0,
                            (presentation.rows.len() as f32 * row_height - height).max(0.0),
                        );
                    offset = Some(next);
                    animation.last_offset = next;
                    if elapsed < HEADER_SCROLL_DURATION {
                        self.sidebar_scroll_animation = Some(animation);
                        cx.on_next_frame(window, |_, _, cx| cx.notify());
                    }
                }
            }
        }
        if let Some(offset) = offset {
            let mut handle = self.branches_scroll.0.borrow_mut();
            handle.deferred_scroll_to_item = None;
            handle.base_handle.set_offset(point(px(0.0), px(-offset)));
        }
    }

    pub(in crate::view) fn sidebar_ancestor_menu_entries(
        &self,
        section: BranchSection,
    ) -> Vec<(SharedString, SharedString)> {
        let Some(context) = &self.sticky_context else {
            return Vec::new();
        };
        context
            .eligible_rows
            .iter()
            .filter_map(|ix| {
                let row = &context.rows[*ix];
                if sidebar_sticky::owning_section(row) != Some(section) {
                    return None;
                }
                let label = match row {
                    BranchSidebarRow::RemoteHeader { name, .. } => name.clone(),
                    BranchSidebarRow::GroupHeader { path, remote, .. } => match remote {
                        Some(remote) => format!("{remote}/{path}/").into(),
                        None => format!("{path}/").into(),
                    },
                    _ => return None,
                };
                Some((label, sidebar_sticky::header_key(row)?.clone()))
            })
            .collect()
    }

    pub(in crate::view) fn sidebar_pin_overflow_entries(
        &self,
        bottom: bool,
    ) -> Vec<(SharedString, SharedString)> {
        let Some(context) = &self.sticky_context else {
            return Vec::new();
        };
        let cache = context.layout_cache.borrow();
        let Some((_, layout)) = cache.as_ref() else {
            return Vec::new();
        };
        let Some(range) = layout.slots.iter().find_map(|slot| match slot {
            sidebar_sticky::StickySlot::Overflow {
                bottom: edge,
                roots,
                ..
            } if *edge == bottom => Some(roots.clone()),
            _ => None,
        }) else {
            return Vec::new();
        };
        context.pin_roots[range]
            .iter()
            .filter(|ix| layout.rows.binary_search(ix).is_err())
            .filter_map(|ix| {
                let label = match &context.rows[*ix] {
                    BranchSidebarRow::Branch { name, .. } => name.clone(),
                    BranchSidebarRow::GroupHeader { label, .. } => label.clone(),
                    _ => return None,
                };
                Some((label, context.row_keys[*ix].clone()))
            })
            .collect()
    }
}

pub(super) struct StickyRows {
    pub(super) view: Entity<SidebarPaneView>,
}

impl gpui::UniformListDecoration for StickyRows {
    fn compute(
        &self,
        _range: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        row_height: Pixels,
        _count: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.view.update(cx, |this, cx| {
            let Some(presentation) = this.branch_sidebar_presentation_cached() else {
                return div().into_any();
            };
            let Some(context) = &mut this.sticky_context else {
                return div().into_any();
            };
            // Keep the actual uniform-list measurement. Multiplying it by the
            // row count and dividing again can lose precision at fractional DPI.
            context.row_height = Some(f32::from(row_height));
            let layout = context.layout(
                f32::from(bounds.size.height),
                f32::from(row_height),
                -f32::from(scroll_offset.y),
            );
            let compact = context
                .fitted_rows(f32::from(bounds.size.height), f32::from(row_height))
                .is_some_and(|rows| rows.len() < context.eligible_rows.len());
            let elements = Self::render_rows(
                this,
                &presentation,
                layout,
                compact,
                bounds,
                scroll_offset,
                row_height,
                window,
                cx,
            );
            div()
                .relative()
                .size_full()
                .children(elements)
                .into_any_element()
        })
    }
}

impl StickyRows {
    #[allow(clippy::too_many_arguments)]
    fn render_rows(
        this: &mut SidebarPaneView,
        presentation: &SidebarPresentation,
        layout: sidebar_sticky::StickyLayout,
        compact: bool,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        row_height: Pixels,
        window: &mut Window,
        cx: &mut gpui::Context<SidebarPaneView>,
    ) -> Vec<AnyElement> {
        let content_inset = ui_scale::design_px_from_percent(
            components::ROW_HIGHLIGHT_INSET_PX,
            ui_scale::current(cx).percent,
        );
        let position = |rank, ix| {
            let natural_y = ix as f32 * f32::from(row_height) + f32::from(scroll_offset.y);
            let y = sidebar_sticky::row_y(
                ix,
                rank,
                layout.slots.len(),
                -f32::from(scroll_offset.y),
                f32::from(bounds.size.height),
                f32::from(row_height),
            );
            (y, y != natural_y)
        };
        let rendered = SidebarPaneView::render_sidebar_rows(
            this,
            layout
                .slots
                .iter()
                .enumerate()
                .filter_map(|(rank, slot)| match slot {
                    sidebar_sticky::StickySlot::Row(ix) => Some((*ix, position(rank, *ix).1)),
                    _ => None,
                }),
            SidebarRowSurface::Sticky { compact },
            presentation.clone(),
            window,
            cx,
        );
        let mut rendered = rendered.into_iter();
        layout
            .slots
            .iter()
            .enumerate()
            .map(|(rank, slot)| {
                let ix = slot.row();
                let (y, stuck) = position(rank, ix);
                if let sidebar_sticky::StickySlot::Overflow { bottom, roots, .. } = slot {
                    let bottom = *bottom;
                    let count = roots.len()
                        - layout
                            .rows
                            .iter()
                            .filter(|ix| {
                                presentation.structure.pin_roots[roots.clone()]
                                    .binary_search(ix)
                                    .is_ok()
                            })
                            .count();
                    return div()
                        .id(if bottom {
                            "sidebar_more_pins_bottom"
                        } else {
                            "sidebar_more_pins_top"
                        })
                        .debug_selector(move || {
                            if bottom {
                                "sidebar_more_pins_bottom"
                            } else {
                                "sidebar_more_pins_top"
                            }
                            .to_owned()
                        })
                        .absolute()
                        .left(-scroll_offset.x)
                        .top(px(y) - scroll_offset.y)
                        .w(bounds.size.width)
                        .h(row_height)
                        .bg(this.theme.colors.surface.panel)
                        .block_mouse_except_scroll()
                        .flex()
                        .items_center()
                        .px(content_inset)
                        .text_size(this.theme.ui_text(12.0))
                        .text_color(this.theme.colors.foreground.primary)
                        .child(format!("More pinned items ({count})"))
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(move |this, e: &ClickEvent, window, cx| {
                                let root = this.root_view.clone();
                                let position = e.position();
                                if let Some(repo_id) = this.active_repo_id() {
                                    window.defer(cx, move |window, cx| {
                                        let _ = root.update(cx, |root, cx| {
                                            root.open_popover_at(
                                                PopoverKind::SidebarPinnedOverflow {
                                                    repo_id,
                                                    bottom,
                                                },
                                                position,
                                                window,
                                                cx,
                                            )
                                        });
                                    });
                                }
                            }),
                        )
                        .into_any_element();
                }
                let element = rendered.next().unwrap();
                let header_key = sidebar_sticky::header_key(&presentation.rows[ix]);
                let key = presentation.row_keys[ix].clone();
                let is_header = header_key.is_some();
                let is_selected = this
                    .sticky_context
                    .as_ref()
                    .and_then(|context| context.selected_row.as_ref())
                    .is_some_and(|(row, _)| *row == ix);
                let background = crate::view::rows::sidebar::sidebar_row_background(
                    this.theme,
                    if ix < presentation.pins.len() {
                        SidebarRowSurface::Pins
                    } else {
                        SidebarRowSurface::Sticky { compact }
                    },
                    &presentation.rows[ix],
                    stuck,
                );
                let mut row = div()
                    .id(key)
                    .absolute()
                    .debug_selector(move || {
                        if is_header {
                            format!("sidebar_sticky_header_{ix}")
                        } else if is_selected {
                            "sidebar_sticky_selected_branch".to_string()
                        } else {
                            format!("sidebar_sticky_pin_{ix}")
                        }
                    })
                    .left(-scroll_offset.x)
                    .top(px(y) - scroll_offset.y)
                    .w(bounds.size.width)
                    .h(row_height)
                    .bg(background)
                    .block_mouse_except_scroll()
                    .child(element);
                if compact
                    && let BranchSidebarRow::SectionHeader { section, .. } = &presentation.rows[ix]
                    && this.sticky_context.as_ref().is_some_and(|context| {
                        context.eligible_rows.iter().any(|ix| {
                            sidebar_sticky::owning_section(&context.rows[*ix]) == Some(*section)
                        })
                    })
                    && let Some(repo_id) = this.active_repo_id()
                {
                    let section = *section;
                    row = row.child(
                        div()
                            .id(("sidebar_ancestor_menu", ix))
                            .debug_selector(move || format!("sidebar_ancestor_menu_{ix}"))
                            .absolute()
                            // Keep the control clear of the overlay scrollbar,
                            // even though the row background reaches the edge.
                            .right(content_inset)
                            .top_0()
                            .w(row_height)
                            .h(row_height)
                            .bg(background)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("…")
                            .on_activate(
                                false,
                                controls::ControlActivation::Nested,
                                cx.listener(move |this, e: &ClickEvent, window, cx| {
                                    // The menu reads this pane's active paths when
                                    // opening. Release its click-handler update first.
                                    let root = this.root_view.clone();
                                    let position = e.position();
                                    window.defer(cx, move |window, cx| {
                                        let _ = root.update(cx, |root, cx| {
                                            root.open_popover_at(
                                                PopoverKind::SidebarAncestorMenu {
                                                    repo_id,
                                                    section,
                                                },
                                                position,
                                                window,
                                                cx,
                                            );
                                        });
                                    });
                                }),
                            ),
                    );
                }
                if is_header && stuck && !this.theme.is_dark {
                    let natural_y = ix as f32 * f32::from(row_height) + f32::from(scroll_offset.y);
                    row = row.child(
                        div()
                            .debug_selector(move || format!("sidebar_sticky_divider_{ix}"))
                            .absolute()
                            .left_0()
                            .right_0()
                            .h(px(1.0))
                            .when(y > natural_y, |line| line.bottom_0())
                            .when(y < natural_y, |line| line.top_0())
                            .bg(this.theme.colors.stroke.subtle),
                    );
                }
                row.into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
#[gpui::test]
fn review_fractional_sidebar_rows_share_the_decoration_geometry(cx: &mut gpui::TestAppContext) {
    use crate::view::test_support::{self, TestBackend};
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let state = super::long_list_tests::branch_fixture(18);
    let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    for (scale, display_scale) in [(100, 1.0), (139, 1.25), (139, 1.5), (133, 1.25)] {
        cx.update(|window, app| {
            window.set_scale_factor(display_scale);
            ui_scale::set_default(app, scale);
            view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            let pane = pane.read(app);
            let context = pane.sticky_context.as_ref().unwrap();
            let before = context.layout_cache.borrow().as_ref().unwrap().0;
            pane.decorated_sidebar_rows();
            let after = context.layout_cache.borrow().as_ref().unwrap().0;
            assert_eq!(
                before, after,
                "row rendering and decoration disagree at {scale}% / {display_scale}x"
            );
        });
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;

    #[gpui::test]
    fn header_animation_retargets_and_manual_scrolling_cancels(cx: &mut gpui::TestAppContext) {
        use crate::view::test_support::{self, TestBackend};
        let _guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = super::super::long_list_tests::branch_fixture(10_000);
        Arc::make_mut(&mut state).repos[0].stashes =
            super::super::long_list_tests::fixture(100, CollapsedSidebarSection::Stashes).repos[0]
                .stashes
                .clone();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state, cx);
                view.set_sidebar_collapsed(false, cx);
            })
        });
        test_support::redraw(cx);
        let pane = cx.update(|_, app| view.read(app).sidebar_pane.clone());
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(300.0), px(500.0)));
        cx.update(|window, app| {
            pane.update(app, |pane, cx| {
                crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                    pane.navigate_sidebar_header(
                        branch_sidebar::stash_section_storage_key().into(),
                        cx,
                    );
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    let animation = pane
                        .sidebar_scroll_animation
                        .as_mut()
                        .expect("live header navigation animates");
                    assert!(animation.target > animation.start + 10_000.0);
                    animation.started =
                        std::time::Instant::now() - std::time::Duration::from_millis(50);
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    let offset = -f32::from(pane.branches_scroll.0.borrow().base_handle.offset().y);
                    let animation = pane.sidebar_scroll_animation.as_ref().unwrap();
                    assert!(offset > animation.start && offset < animation.target);

                    pane.navigate_sidebar_header(
                        branch_sidebar::local_section_storage_key().into(),
                        cx,
                    );
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    let animation = pane
                        .sidebar_scroll_animation
                        .as_ref()
                        .expect("second click retargets");
                    assert!((animation.start - offset).abs() < 0.5);
                    assert!(animation.target < animation.start);

                    // A scrollbar changes the actual offset between frames.
                    pane.branches_scroll
                        .0
                        .borrow()
                        .base_handle
                        .set_offset(point(px(0.0), px(-125.0)));
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    assert!(pane.sidebar_scroll_animation.is_none());
                    assert_eq!(
                        pane.branches_scroll.0.borrow().base_handle.offset().y,
                        px(-125.0)
                    );

                    pane.navigate_sidebar_header(
                        branch_sidebar::stash_section_storage_key().into(),
                        cx,
                    );
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    let target = pane.sidebar_scroll_animation.as_ref().unwrap().target;
                    pane.sidebar_scroll_animation.as_mut().unwrap().started =
                        std::time::Instant::now() - std::time::Duration::from_millis(150);
                    pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                    assert!(pane.sidebar_scroll_animation.is_none());
                    assert_eq!(
                        -f32::from(pane.branches_scroll.0.borrow().base_handle.offset().y),
                        target
                    );
                });
                // Deterministic runtime preserves existing exact-offset tests.
                pane.navigate_sidebar_header(
                    branch_sidebar::local_section_storage_key().into(),
                    cx,
                );
                pane.prepare_sidebar_scroll(bounds, px(24.0), window, cx);
                assert!(pane.sidebar_scroll_animation.is_none());
                assert_eq!(
                    pane.branches_scroll.0.borrow().base_handle.offset().y,
                    px(0.0)
                );
            })
        });
    }
    #[test]
    fn header_scroll_is_fast_monotonic_and_finishes_at_the_exact_destination() {
        for (start, target) in [(0.0, 24000.0), (24000.0, 0.0)] {
            assert_eq!(
                animated_scroll_offset(start, target, std::time::Duration::ZERO),
                start
            );
            let halfway =
                animated_scroll_offset(start, target, std::time::Duration::from_millis(75));
            assert!(halfway > start.min(target) && halfway < start.max(target));
            assert_eq!(
                animated_scroll_offset(start, target, HEADER_SCROLL_DURATION),
                target
            );
            assert_eq!(
                animated_scroll_offset(start, target, std::time::Duration::from_secs(1)),
                target
            );
        }
    }
}
