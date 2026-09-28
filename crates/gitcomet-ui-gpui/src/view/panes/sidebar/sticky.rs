use super::*;
use crate::view::rows::sidebar::branch_commit_id;
use crate::view::sidebar_sticky::{self, SidebarRowSurface};

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
    pub(super) eligible_rows: Rc<[usize]>,
    priority_rows: Rc<[usize]>,
    sections: Rc<[usize]>,
    selected_row: Option<(usize, SharedString)>,
    anchor: Option<(BranchSidebarRow, f32)>,
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

    /// Save the first uncovered stable row and its position in slot units so
    /// asynchronous data updates and density/scale changes retain the context.
    fn capture_anchor(
        &self,
        scroll_handle: &UniformListScrollHandle,
    ) -> Option<(BranchSidebarRow, f32)> {
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
        let fitted = self.fitted_rows(f32::from(size.item.height), row_height);
        let fitted = fitted.map(|rows| rows.as_ref()).unwrap_or(&[]);
        let stacked = fitted
            .iter()
            .enumerate()
            .take_while(|(rank, ix)| **ix as f32 * row_height - scroll <= *rank as f32 * row_height)
            .count();
        let start = (scroll / row_height).ceil() as usize + stacked;
        self.rows
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, row)| sidebar_sticky::same_row(row, row))
            .map(|(ix, row)| (row.clone(), ix as f32 - scroll / row_height))
    }
}

pub(super) enum NavigationTarget {
    Header(SharedString),
    Branch(BranchMenuTarget),
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
        if surface != SidebarRowSurface::Tree || event.mouse_position().is_none() {
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
        let tip = self.sidebar_selected_tip();
        let Some(repo) = self.active_repo() else {
            return;
        };
        let head = match &repo.head_branch {
            Loadable::Ready(head) if head != "HEAD" => Some(head.as_str()),
            _ => None,
        };
        let selected = self
            .selected_branch_on_surface(SidebarRowSurface::Tree)
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
        // Only the tree's visible branch rows qualify: filtering or a closed
        // ancestor removes the leaf as well as its sticky path. Resolve once
        // per model/selection change, never while scrolling.
        let selected_row = selected.as_ref().and_then(|selected| {
            presentation.rows.iter().position(|row| {
                matches!(row, BranchSidebarRow::Branch { target, .. } if target == selected)
            }).map(|ix| {
                let key = match selected {
                    BranchMenuTarget::Local { name } => format!("sticky:local:{name}"),
                    BranchMenuTarget::Remote { remote, branch } => {
                        format!("sticky:remote:{}:{remote}:{branch}", remote.len())
                    }
                };
                (ix, SharedString::from(key))
            })
        });
        let mut priority_rows = presentation.structure.sections.clone();
        if let Some((ix, _)) = &selected_row {
            priority_rows.push(*ix);
            priority_rows.sort_unstable();
        }
        let mut eligible_rows = priority_rows.clone();
        for target in current.iter().chain(selected.iter()) {
            eligible_rows.extend(presentation.structure.active_path(
                &presentation.rows,
                target,
                &presentation.filter,
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
            && let Some(rows) = next.fitted_rows(
                f32::from(guard.viewport.size.height),
                f32::from(guard.row_height),
            )
            && let Ok(rank) = rows.binary_search(&guard.row)
        {
            let y = sidebar_sticky::row_y(
                guard.row,
                rank,
                rows.len(),
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
        let row_height = f32::from(size.contents.height) / context.rows.len().max(1) as f32;
        context
            .fitted_rows(f32::from(size.item.height), row_height)
            .cloned()
            .unwrap_or_else(|| Rc::from([]))
    }

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
        _cx: &mut gpui::Context<Self>,
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
            NavigationTarget::Header(key) => presentation
                .structure
                .headers
                .get(key)
                .copied()
                .map(|ix| (ix, false)),
            NavigationTarget::Branch(target) => presentation
                .rows
                .iter()
                .position(|row| matches!(row, BranchSidebarRow::Branch { target: candidate, .. } if candidate == target))
                .map(|ix| (ix, true)),
        });
        let offset = if let Some((ix, center)) = target {
            context.anchor = None;
            let fitted = context.fitted_rows(height, row_height);
            let headers = fitted.map(|rows| rows.as_ref()).unwrap_or(&[]);
            Some(sidebar_sticky::navigation_offset(
                ix,
                headers,
                height,
                row_height,
                presentation.rows.len(),
                center,
            ))
        } else {
            context.anchor.take().and_then(|(anchor, y)| {
                presentation
                    .rows
                    .iter()
                    .position(|row| sidebar_sticky::same_row(row, &anchor))
                    .map(|ix| {
                        ((ix as f32 - y) * row_height).clamp(
                            0.0,
                            (presentation.rows.len() as f32 * row_height - height).max(0.0),
                        )
                    })
            })
        };
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

    pub(super) fn render_fixed_pins(
        &mut self,
        presentation: &SidebarPresentation,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if presentation.pins.is_empty() {
            return None;
        }
        let scale = ui_scale::current(cx);
        let row_height = sidebar_list_row_height(theme, scale.percent);
        let list = restrict_scroll_to_vertical_axis(
            uniform_list(
                "sidebar_pins",
                presentation.pins.len(),
                cx.processor(Self::render_pinned_sidebar_rows),
            )
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.pinned_scroll),
        );
        Some(
            div()
                .id("sidebar_pinned_area")
                .debug_selector(|| "sidebar_pinned_area".to_string())
                .flex()
                .flex_col()
                .flex_none()
                .min_h(px(0.0))
                .h(row_height * presentation.pins.len())
                .max_h(gpui::relative(1.0 / 3.0))
                .overflow_hidden()
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .debug_selector(|| "sidebar_pinned_surface".to_string())
                                .h_full()
                                .bg(theme.colors.surface.panel)
                                .child(list),
                        )
                        .child(
                            components::Scrollbar::new(
                                "sidebar_pins_scrollbar",
                                self.pinned_scroll.clone(),
                            )
                            .auto_hide()
                            .render(theme),
                        ),
                )
                .into_any_element(),
        )
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
            let Some(context) = &this.sticky_context else {
                return div().into_any();
            };
            let Some(rows) =
                context.fitted_rows(f32::from(bounds.size.height), f32::from(row_height))
            else {
                return div().into_any_element();
            };
            let compact = rows.len() < context.eligible_rows.len();
            let elements = Self::render_rows(
                this,
                &presentation,
                rows.clone(),
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
        rows: Rc<[usize]>,
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
                rows.len(),
                -f32::from(scroll_offset.y),
                f32::from(bounds.size.height),
                f32::from(row_height),
            );
            (y, y != natural_y)
        };
        let rendered = SidebarPaneView::render_sidebar_rows(
            this,
            rows.iter()
                .enumerate()
                .map(|(rank, &ix)| (ix, position(rank, ix).1)),
            SidebarRowSurface::Sticky { compact },
            presentation.clone(),
            window,
            cx,
        );
        rendered
            .into_iter()
            .zip(rows.iter())
            .enumerate()
            .map(|(rank, (element, &ix))| {
                let (y, stuck) = position(rank, ix);
                let header_key = sidebar_sticky::header_key(&presentation.rows[ix]);
                let key = header_key.cloned().unwrap_or_else(|| {
                    this.sticky_context
                        .as_ref()
                        .unwrap()
                        .selected_row
                        .as_ref()
                        .unwrap()
                        .1
                        .clone()
                });
                let is_header = header_key.is_some();
                let background = crate::view::rows::sidebar::sidebar_row_background(
                    this.theme,
                    SidebarRowSurface::Sticky { compact },
                    &presentation.rows[ix],
                    stuck,
                );
                let mut row = div()
                    .id(key)
                    .absolute()
                    .debug_selector(move || {
                        if is_header {
                            format!("sidebar_sticky_header_{ix}")
                        } else {
                            "sidebar_sticky_selected_branch".to_string()
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
