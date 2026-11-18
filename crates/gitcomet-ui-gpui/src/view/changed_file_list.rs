//! The virtualized body used by built-in and extension-owned changed-file lists.
//!
//! A source adapter supplies row actions and presentation; this entity keeps the
//! controller, scroll and list identity together across parent renders. The
//! adapter is referenced weakly by its row callback, so unmounting releases it.
use super::file_list_controller::FileListController;
use super::*;
use std::{cell::RefCell, ops::Range, rc::Rc};

pub(in crate::view) type SharedFileListController = Rc<RefCell<FileListController>>;
type RenderRows = Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement>>;
type StickyHeaders = crate::view::rows::StickyGroupHeaders;

pub(in crate::view) struct ChangedFileListView {
    controller: SharedFileListController,
    id: ElementId,
    /// Built-in status lists project richer status entries without copying them.
    row_count: Option<usize>,
    scroll: UniformListScrollHandle,
    render_rows: RenderRows,
    sticky_headers: Option<StickyHeaders>,
}

impl ChangedFileListView {
    pub(in crate::view) fn new(
        controller: SharedFileListController,
        id: impl Into<ElementId>,
        scroll: UniformListScrollHandle,
        render_rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement> + 'static,
    ) -> Self {
        Self {
            controller,
            id: id.into(),
            row_count: None,
            scroll,
            render_rows: Rc::new(render_rows),
            sticky_headers: None,
        }
    }

    /// Called from the parent's render. The list is mounted uncached, so it
    /// renders again right after, with the parent's selection, marks and
    /// theme. No notify: one raised during the draw dirtied the parent again
    /// on the next frame, so the parent re-rendered on every window frame.
    pub(in crate::view) fn refresh(
        &mut self,
        row_count: Option<usize>,
        sticky_headers: Option<StickyHeaders>,
    ) {
        self.row_count = row_count;
        self.sticky_headers = sticky_headers;
    }
}

impl Render for ChangedFileListView {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let rows = self
            .row_count
            .unwrap_or_else(|| self.controller.borrow_mut().row_count());
        let render = Rc::clone(&self.render_rows);
        let headers = self
            .sticky_headers
            .as_ref()
            .map(|sticky| sticky.headers.clone());
        let list = uniform_list(self.id.clone(), rows, move |range, window, cx| {
            let start = range.start;
            render(range, window, cx)
                .into_iter()
                .enumerate()
                .map(|(offset, row)| {
                    if headers
                        .as_ref()
                        .is_some_and(|headers| headers.binary_search(&(start + offset)).is_ok())
                    {
                        // Retain the measured row slot. Its header is painted
                        // once in the sibling overlay with sticky positioning.
                        div().w_full().invisible().child(row).into_any_element()
                    } else {
                        row
                    }
                })
                .collect()
        })
        .size_full()
        .flex_1()
        .min_h(px(0.0))
        .track_scroll(&self.scroll);
        let (list, overlay) = match &self.sticky_headers {
            Some(headers) => {
                let (decoration, overlay) = headers.clone().with_overlay();
                (list.with_decoration(decoration), Some(overlay))
            }
            None => (list, None),
        };
        let list = crate::kit::elastic_scroll::ElasticScroll::vertical(
            "changed_files_elastic",
            self.scroll.clone(),
            list,
        );
        div()
            .relative()
            .size_full()
            .flex_1()
            .min_h(px(0.0))
            .child(list)
            .children(overlay)
    }
}

pub(in crate::view) struct BuiltinFileList {
    pub controller: SharedFileListController,
    pub view: Option<Entity<ChangedFileListView>>,
}

impl Default for BuiltinFileList {
    fn default() -> Self {
        Self {
            controller: Rc::new(RefCell::new(FileListController::new(
                gitcomet_extension_api::FileListMode::Tree,
            ))),
            view: None,
        }
    }
}

type BuiltinRowRenderer = fn(
    &mut DetailsPaneView,
    Range<usize>,
    &mut Window,
    &mut gpui::Context<DetailsPaneView>,
) -> Vec<AnyElement>;

impl DetailsPaneView {
    /// Number of whole rows visible in a file list, or `None` before its first layout.
    pub(in crate::view) fn file_list_page_rows(
        &self,
        _repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> Option<usize> {
        use crate::view::rows::FileListId;
        let (scroll, row_height) = match list {
            FileListId::CommitFiles => (&self.commit_files_scroll, self.sidebar_row_height()),
            FileListId::RangeFiles => (&self.range_files_scroll, self.sidebar_row_height()),
            FileListId::WorktreeFiles => (&self.worktree_files_scroll, self.sidebar_row_height()),
            FileListId::Status(section) => {
                let scroll = match section {
                    StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
                        &self.unstaged_scroll
                    }
                    StatusSection::Untracked => &self.untracked_scroll,
                    StatusSection::Staged => &self.staged_scroll,
                };
                (
                    scroll,
                    crate::view::rows::status::status_row_height(self.ui_scale()),
                )
            }
        };
        let viewport = f32::from(scroll.0.borrow().last_item_size?.item.height);
        let row_height = f32::from(row_height);
        (viewport.is_finite() && row_height.is_finite() && viewport > 0.0 && row_height > 0.0)
            .then(|| (viewport / row_height).floor().max(1.0) as usize)
    }

    fn sidebar_row_height(&self) -> Pixels {
        crate::view::rows::sidebar::sidebar_list_row_height(self.theme, self.ui_scale_percent)
    }

    pub(in crate::view) fn changed_file_list(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<ChangedFileListView> {
        use crate::view::rows::FileListId;
        let (id, scroll, render_rows): (ElementId, _, BuiltinRowRenderer) = match list {
            FileListId::CommitFiles => (
                ("commit_details_files_list", repo_id.0).into(),
                &self.commit_files_scroll,
                Self::render_commit_file_rows,
            ),
            FileListId::RangeFiles => (
                ("range_files_list", repo_id.0).into(),
                &self.range_files_scroll,
                Self::render_range_file_rows,
            ),
            FileListId::WorktreeFiles => (
                ("worktree_files_list", repo_id.0).into(),
                &self.worktree_files_scroll,
                Self::render_worktree_file_rows,
            ),
            FileListId::Status(StatusSection::CombinedUnstaged) => (
                "unstaged".into(),
                &self.unstaged_scroll,
                Self::render_unstaged_rows,
            ),
            FileListId::Status(StatusSection::Untracked) => (
                "untracked".into(),
                &self.untracked_scroll,
                Self::render_untracked_rows,
            ),
            FileListId::Status(StatusSection::Unstaged) => (
                "split_unstaged".into(),
                &self.unstaged_scroll,
                Self::render_split_unstaged_rows,
            ),
            FileListId::Status(StatusSection::Staged) => (
                "staged".into(),
                &self.staged_scroll,
                Self::render_staged_rows,
            ),
        };
        let sticky = self.sticky_group_headers(repo_id, list, cx);
        let mut lists = self.file_controllers.borrow_mut();
        let entry = lists.entry((repo_id, list)).or_default();
        let view = entry.view.get_or_insert_with(|| {
            let parent = cx.weak_entity();
            let controller = Rc::clone(&entry.controller);
            cx.new(|_| {
                ChangedFileListView::new(
                    controller,
                    id,
                    scroll.clone(),
                    move |range, window, cx| {
                        parent
                            .update(cx, |pane, cx| {
                                let start = range.start;
                                let rows = render_rows(pane, range, window, cx);
                                let plan = pane
                                    .file_controllers
                                    .borrow()
                                    .get(&(repo_id, list))
                                    .and_then(|entry| {
                                        entry.controller.borrow().plan_cache.current()
                                    });
                                rows.into_iter()
                                    .enumerate()
                                    .map(|(ix, element)| {
                                        let focused = plan
                                            .as_ref()
                                            .and_then(|plan| {
                                                plan.row_at(crate::view::rows::RowIx(start + ix))
                                            })
                                            .is_some_and(|row| {
                                                pane.review_row_is_focused(
                                                    repo_id, list, &row, window,
                                                )
                                            });
                                        if focused {
                                            div()
                                                .relative()
                                                .child(element)
                                                .child(
                                                    div()
                                                        .debug_selector(|| {
                                                            "review_file_focus".to_owned()
                                                        })
                                                        .absolute()
                                                        .inset_0()
                                                        .border_1()
                                                        .border_color(
                                                            pane.theme
                                                                .colors
                                                                .interaction
                                                                .focus_ring,
                                                        ),
                                                )
                                                .into_any_element()
                                        } else {
                                            element
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    },
                )
            })
        });
        view.update(cx, |view, _| view.refresh(Some(count), sticky));
        view.clone()
    }

    /// The pinned group header of `list` while its plan is grouped. The
    /// caller has just planned the list, so the cached plan is the drawn one.
    fn sticky_group_headers(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<crate::view::rows::StickyGroupHeaders> {
        use crate::view::rows::{FileListId, FileListRow, RowIx};
        let plan = self
            .file_controllers
            .borrow()
            .get(&(repo_id, list))?
            .controller
            .borrow()
            .plan_cache
            .current()?;
        let headers = plan.headers()?;
        let prefix = match list {
            FileListId::CommitFiles => format!("commit_file_group_{}", repo_id.0),
            FileListId::WorktreeFiles => format!("worktree_file_group_{}", repo_id.0),
            FileListId::RangeFiles => format!("range_file_group_{}", repo_id.0),
            FileListId::Status(section) => {
                format!("status_group_{}_{}", repo_id.0, section.id_label())
            }
        };
        let pane = cx.weak_entity();
        let theme = self.theme;
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        Some(crate::view::rows::StickyGroupHeaders {
            headers,
            header: Rc::new(move |row, row_height, pinned, _cx| {
                let FileListRow::Group {
                    group,
                    label,
                    count,
                    collapsed,
                } = plan.row_at(RowIx(row))?
                else {
                    return None;
                };
                let pane = pane.clone();
                let id = match list {
                    FileListId::CommitFiles => "commit_file_group",
                    FileListId::WorktreeFiles => "worktree_file_group",
                    FileListId::RangeFiles => "range_file_group",
                    FileListId::Status(_) => "status_group",
                };
                Some(crate::view::rows::group_header_row(
                    crate::view::rows::GroupHeaderProps {
                        id: (if pinned { "file_list_sticky_group" } else { id }, group),
                        selector: if pinned {
                            format!("{prefix}_sticky_{label}")
                        } else {
                            format!("{prefix}_{label}")
                        },
                        label,
                        count,
                        collapsed,
                    },
                    theme,
                    ui_scale,
                    row_height,
                    move |cx| {
                        let _ = pane.update(cx, |pane, cx| {
                            pane.toggle_file_list_group(repo_id, list, group, cx)
                        });
                    },
                ))
            }),
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
#[gpui::test]
fn elastic_grouped_files_keep_headers_fixed_and_forward_header_wheel_input(
    cx: &mut gpui::TestAppContext,
) {
    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, app| {
            window.simulate_next_frame(app);
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    fn gesture(
        cx: &mut gpui::VisualTestContext,
        position: Point<Pixels>,
        delta: f32,
        phase: gpui::TouchPhase,
    ) {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(delta))),
            modifiers: Default::default(),
            touch_phase: phase,
        });
        draw(cx);
    }

    let scroll = UniformListScrollHandle::default();
    let activations = Rc::new(std::cell::Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, _| {
        ChangedFileListView::new(
            Rc::new(RefCell::new(FileListController::new(
                gitcomet_extension_api::FileListMode::Grouped,
            ))),
            "elastic_grouped_files_test",
            scroll.clone(),
            |range, _, _| {
                range
                    .map(|row| {
                        div()
                            .h(px(24.0))
                            .w_full()
                            .debug_selector(move || format!("elastic_grouped_body_{row}"))
                            .child(format!("Row {row}"))
                            .into_any_element()
                    })
                    .collect()
            },
        )
    });
    let clicked = activations.clone();
    cx.update(|_, app| {
        view.update(app, |list, _| {
            list.refresh(
                Some(100),
                Some(StickyHeaders {
                    headers: Arc::from([0, 40]),
                    header: Rc::new(move |row, height, pinned, app| {
                        let clicked = clicked.clone();
                        Some(crate::view::rows::group_header_row(
                            crate::view::rows::GroupHeaderProps {
                                id: (
                                    if pinned {
                                        "elastic_grouped_pin"
                                    } else {
                                        "elastic_grouped_header"
                                    },
                                    row,
                                ),
                                selector: format!(
                                    "elastic_grouped_{}_{row}",
                                    if pinned { "sticky" } else { "natural" }
                                ),
                                label: format!("Group {row}").into(),
                                count: 39,
                                collapsed: false,
                            },
                            AppTheme::from_key("gitcomet_light").unwrap(),
                            crate::ui_scale::UiScale::current(app),
                            height,
                            move |_| clicked.set(clicked.get() + 1),
                        ))
                    }),
                }),
            );
        });
    });
    cx.simulate_resize(gpui::size(px(320.0), px(240.0)));
    draw(cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        cx.update(|window, _| {
            window.activate();
            window.refresh();
        });
        draw(cx);
        let header = cx.debug_bounds("elastic_grouped_natural_0").unwrap();
        let body = cx.debug_bounds("elastic_grouped_body_1").unwrap();
        gesture(cx, header.center(), 100.0, gpui::TouchPhase::Started);
        assert_eq!(
            cx.debug_bounds("elastic_grouped_natural_0").unwrap(),
            header
        );
        assert!(cx.debug_bounds("elastic_grouped_body_1").unwrap().top() > body.top() + px(20.0));
        assert_eq!(scroll.0.borrow().base_handle.offset().y, px(0.0));
        gesture(cx, header.center(), 0.0, gpui::TouchPhase::Ended);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        draw(cx);
        assert_eq!(cx.debug_bounds("elastic_grouped_body_1").unwrap(), body);
        cx.simulate_click(header.center(), gpui::Modifiers::default());
        draw(cx);
        assert_eq!(
            activations.get(),
            1,
            "the fixed natural header remains interactive"
        );

        gesture(cx, header.center(), -96.0, gpui::TouchPhase::Started);
        assert_eq!(scroll.0.borrow().base_handle.offset().y, px(-96.0));
        assert_eq!(cx.debug_bounds("elastic_grouped_sticky_0").unwrap(), header);
        gesture(cx, header.center(), 0.0, gpui::TouchPhase::Ended);
        scroll.scroll_to_item_strict(99, gpui::ScrollStrategy::Bottom);
        cx.update(|window, _| window.refresh());
        draw(cx);
        let header = cx.debug_bounds("elastic_grouped_sticky_40").unwrap();
        let body = cx.debug_bounds("elastic_grouped_body_99").unwrap();
        let bottom_offset = scroll.0.borrow().base_handle.offset();
        for (delta, phase) in [
            (-100.0, gpui::TouchPhase::Started),
            (0.0, gpui::TouchPhase::Ended),
        ] {
            gesture(cx, header.center(), delta, phase);
            assert_eq!(
                cx.debug_bounds("elastic_grouped_sticky_40").unwrap(),
                header
            );
            assert!(
                cx.debug_bounds("elastic_grouped_body_99").unwrap().top() < body.top() - px(20.0)
            );
            assert_eq!(scroll.0.borrow().base_handle.offset(), bottom_offset);
        }
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        draw(cx);
        assert_eq!(cx.debug_bounds("elastic_grouped_body_99").unwrap(), body);
        cx.simulate_click(header.center(), gpui::Modifiers::default());
        draw(cx);
        assert_eq!(
            activations.get(),
            2,
            "the fixed pinned header remains interactive"
        );
    });
}

#[cfg(all(test, target_os = "macos"))]
#[gpui::test]
fn elastic_grouped_files_keep_interior_headers_with_rows_and_reverse_top_push_off(
    cx: &mut gpui::TestAppContext,
) {
    fn draw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, app| {
            window.simulate_next_frame(app);
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    fn gesture(
        cx: &mut gpui::VisualTestContext,
        position: Point<Pixels>,
        delta: f32,
        phase: gpui::TouchPhase,
    ) {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(delta))),
            modifiers: Default::default(),
            touch_phase: phase,
        });
        draw(cx);
    }

    let scroll = UniformListScrollHandle::default();
    let (view, cx) = cx.add_window_view(|_, _| {
        ChangedFileListView::new(
            Rc::new(RefCell::new(FileListController::new(
                gitcomet_extension_api::FileListMode::Grouped,
            ))),
            "elastic_interior_groups_test",
            scroll.clone(),
            |range, _, _| {
                range
                    .map(|row| {
                        div()
                            .h(px(24.0))
                            .w_full()
                            .debug_selector(move || format!("interior_group_body_{row}"))
                            .child(format!("Row {row}"))
                            .into_any_element()
                    })
                    .collect()
            },
        )
    });
    cx.update(|_, app| {
        view.update(app, |list, _| {
            list.refresh(
                Some(100),
                Some(StickyHeaders {
                    headers: Arc::from([0, 4, 40, 91, 96]),
                    header: Rc::new(|row, height, pinned, _| {
                        Some(
                            div()
                                .h(height)
                                .w_full()
                                .debug_selector(move || {
                                    format!(
                                        "interior_group_{}_{row}",
                                        if pinned { "pinned" } else { "natural" }
                                    )
                                })
                                .child(format!("Group {row}"))
                                .into_any_element(),
                        )
                    }),
                }),
            );
        });
    });
    cx.simulate_resize(gpui::size(px(320.0), px(240.0)));
    draw(cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        cx.update(|window, _| {
            window.activate();
            window.refresh();
        });
        draw(cx);
        let first = cx.debug_bounds("interior_group_natural_0").unwrap();
        let interior = cx.debug_bounds("interior_group_natural_4").unwrap();
        let body = cx.debug_bounds("interior_group_body_5").unwrap();
        gesture(cx, interior.center(), 100.0, gpui::TouchPhase::Started);
        let moved = cx.debug_bounds("interior_group_body_5").unwrap();
        assert!(moved.top() > body.top() + px(20.0));
        assert_eq!(cx.debug_bounds("interior_group_natural_0").unwrap(), first);
        assert_eq!(
            cx.debug_bounds("interior_group_natural_4").unwrap().top() - interior.top(),
            moved.top() - body.top(),
            "the interior group must follow its file rows during a top bounce"
        );
        gesture(cx, interior.center(), 0.0, gpui::TouchPhase::Ended);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        draw(cx);
        assert_eq!(
            cx.debug_bounds("interior_group_natural_4").unwrap(),
            interior
        );

        scroll.scroll_to_item_strict(99, gpui::ScrollStrategy::Bottom);
        cx.update(|window, _| window.refresh());
        draw(cx);
        let pinned = cx.debug_bounds("interior_group_pinned_40").unwrap();
        let near_top = cx.debug_bounds("interior_group_natural_91").unwrap();
        let interior = cx.debug_bounds("interior_group_natural_96").unwrap();
        let body = cx.debug_bounds("interior_group_body_97").unwrap();
        let bottom_offset = scroll.0.borrow().base_handle.offset();
        assert_eq!(near_top.top(), pinned.bottom());
        gesture(cx, interior.center(), -200.0, gpui::TouchPhase::Started);
        let moved = cx.debug_bounds("interior_group_body_97").unwrap();
        let displacement = moved.top() - body.top();
        assert!(displacement < -px(24.0));
        assert_eq!(
            cx.debug_bounds("interior_group_natural_96").unwrap().top() - interior.top(),
            displacement,
            "short bottom groups must keep their headers with the file rows"
        );
        assert_eq!(
            cx.debug_bounds("interior_group_natural_91").unwrap().top(),
            pinned.top(),
            "an interior header reaching the top must take over the anchor"
        );
        assert!(cx.debug_bounds("interior_group_pinned_40").is_none());
        assert!(cx.debug_bounds("interior_group_pinned_91").is_none());
        assert_eq!(scroll.0.borrow().base_handle.offset(), bottom_offset);

        gesture(cx, interior.center(), 160.0, gpui::TouchPhase::Moved);
        let moved = cx.debug_bounds("interior_group_body_97").unwrap();
        let displacement = moved.top() - body.top();
        assert!(displacement > -px(24.0) && displacement < px(0.0));
        let returning = cx.debug_bounds("interior_group_natural_91").unwrap();
        assert_eq!(returning.top() - near_top.top(), displacement);
        assert_eq!(
            cx.debug_bounds("interior_group_pinned_40")
                .unwrap()
                .bottom(),
            returning.top(),
            "reversing the bounce must reverse the next-header push-off continuously"
        );
        assert_eq!(scroll.0.borrow().base_handle.offset(), bottom_offset);
        gesture(cx, interior.center(), 0.0, gpui::TouchPhase::Ended);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        draw(cx);
        assert_eq!(cx.debug_bounds("interior_group_pinned_40").unwrap(), pinned);
        assert_eq!(
            cx.debug_bounds("interior_group_natural_91").unwrap(),
            near_top
        );
        assert_eq!(
            cx.debug_bounds("interior_group_natural_96").unwrap(),
            interior
        );
        assert_eq!(cx.debug_bounds("interior_group_body_97").unwrap(), body);
    });
}
