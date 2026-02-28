use super::*;

impl MainPaneView {
    pub(in crate::view) fn conflict_resolver_strategy(
        conflict: Option<gitgpui_core::domain::FileConflictKind>,
    ) -> Option<gitgpui_core::conflict_session::ConflictResolverStrategy> {
        conflict.map(|kind| {
            gitgpui_core::conflict_session::ConflictResolverStrategy::for_conflict(
                kind,
                // Binary conflicts are handled separately once byte-backed payloads
                // are threaded through UI state.
                false,
            )
        })
    }

    pub(super) fn render_selected_file_diff(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let (wants_image, is_svg) = self
            .active_repo()
            .map(|repo| {
                let is_svg = match repo.diff_target.as_ref() {
                    Some(DiffTarget::WorkingTree { path, .. }) => crate::view::is_svg_path(path),
                    Some(DiffTarget::Commit {
                        path: Some(path), ..
                    }) => crate::view::is_svg_path(path),
                    _ => false,
                };
                let has_image = !matches!(repo.diff_file_image, Loadable::NotLoaded);
                let wants_image =
                    has_image && (!is_svg || self.svg_diff_view_mode == SvgDiffViewMode::Image);
                (wants_image, is_svg)
            })
            .unwrap_or((false, false));

        if wants_image {
            enum DiffFileImageState {
                NotLoaded,
                Loading,
                Error(String),
                Ready { has_file: bool },
            }

            let diff_file_state = match self.active_repo().map(|repo| &repo.diff_file_image) {
                None => {
                    return zed::empty_state(theme, "Diff", "No repository.").into_any_element();
                }
                Some(Loadable::NotLoaded) => DiffFileImageState::NotLoaded,
                Some(Loadable::Loading) => DiffFileImageState::Loading,
                Some(Loadable::Error(e)) => DiffFileImageState::Error(e.clone()),
                Some(Loadable::Ready(file)) => DiffFileImageState::Ready {
                    has_file: file.is_some(),
                },
            };

            self.ensure_file_image_diff_cache();
            match diff_file_state {
                DiffFileImageState::NotLoaded => {
                    zed::empty_state(theme, "Diff", "Select a file.").into_any_element()
                }
                DiffFileImageState::Loading => {
                    zed::empty_state(theme, "Diff", "Loading").into_any_element()
                }
                DiffFileImageState::Error(e) => {
                    self.diff_raw_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(e, cx);
                        input.set_read_only(true, cx);
                    });
                    div()
                        .id("diff_file_image_error_scroll")
                        .bg(theme.colors.window_bg)
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .child(self.diff_raw_input.clone())
                        .into_any_element()
                }
                DiffFileImageState::Ready { has_file } => {
                    if !has_file || !self.is_file_image_diff_view_active() {
                        zed::empty_state(theme, "Diff", "No image contents available.")
                            .into_any_element()
                    } else {
                        let old = self.file_image_diff_cache_old.clone();
                        let new = self.file_image_diff_cache_new.clone();

                        let cell = |id: &'static str, image: Option<Arc<gpui::Image>>| {
                            div()
                                .id(id)
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(match image {
                                    Some(img_data) => gpui::img(img_data)
                                        .w_full()
                                        .h_full()
                                        .object_fit(gpui::ObjectFit::Contain)
                                        .into_any_element(),
                                    None => div()
                                        .text_sm()
                                        .text_color(theme.colors.text_muted)
                                        .child("No image")
                                        .into_any_element(),
                                })
                        };

                        let columns_header =
                            zed::split_columns_header(theme, "A (before)", "B (after)");

                        div()
                            .id("diff_image_container")
                            .relative()
                            .h_full()
                            .min_h(px(0.0))
                            .flex()
                            .flex_col()
                            .bg(theme.colors.window_bg)
                            .child(columns_header)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .flex()
                                    .child(cell("diff_image_left", old))
                                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border))
                                    .child(cell("diff_image_right", new)),
                            )
                            .into_any_element()
                    }
                }
            }
        } else {
            enum DiffFileState {
                NotLoaded,
                Loading,
                Error(String),
                Ready { has_file: bool },
            }

            let diff_file_state = match self.active_repo().map(|repo| &repo.diff_file) {
                None => {
                    return zed::empty_state(theme, "Diff", "No repository.").into_any_element();
                }
                Some(Loadable::NotLoaded) => DiffFileState::NotLoaded,
                Some(Loadable::Loading) => DiffFileState::Loading,
                Some(Loadable::Error(e)) => DiffFileState::Error(e.clone()),
                Some(Loadable::Ready(file)) => DiffFileState::Ready {
                    has_file: file.is_some(),
                },
            };

            if is_svg && matches!(diff_file_state, DiffFileState::NotLoaded) {
                return zed::empty_state(theme, "Diff", "SVG code view is not available.")
                    .into_any_element();
            }

            self.ensure_file_diff_cache(cx);
            match diff_file_state {
                DiffFileState::NotLoaded => {
                    zed::empty_state(theme, "Diff", "Select a file.").into_any_element()
                }
                DiffFileState::Loading => {
                    zed::empty_state(theme, "Diff", "Loading").into_any_element()
                }
                DiffFileState::Error(e) => {
                    self.diff_raw_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(e, cx);
                        input.set_read_only(true, cx);
                    });
                    div()
                        .id("diff_file_error_scroll")
                        .bg(theme.colors.window_bg)
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .child(self.diff_raw_input.clone())
                        .into_any_element()
                }
                DiffFileState::Ready { has_file } => {
                    if !has_file || !self.is_file_diff_view_active() {
                        zed::empty_state(theme, "Diff", "No file contents available.")
                            .into_any_element()
                    } else if self.file_diff_cache_inflight.is_some() {
                        zed::empty_state(theme, "Diff", "Processing file…").into_any_element()
                    } else {
                        self.ensure_diff_visible_indices();
                        self.maybe_autoscroll_diff_to_first_change();

                        if self.diff_word_wrap {
                            let approx_len: usize = self
                                .file_diff_inline_cache
                                .iter()
                                .map(|l| l.text.len().saturating_add(1))
                                .sum();
                            let mut raw = String::with_capacity(approx_len);
                            for line in &self.file_diff_inline_cache {
                                raw.push_str(line.text.as_ref());
                                raw.push('\n');
                            }
                            self.diff_raw_input.update(cx, |input, cx| {
                                input.set_theme(theme, cx);
                                input.set_soft_wrap(true, cx);
                                input.set_text(raw, cx);
                                input.set_read_only(true, cx);
                            });

                            return div()
                                .id("diff_word_wrap_scroll")
                                .bg(theme.colors.window_bg)
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h(px(0.0))
                                .overflow_y_scroll()
                                .child(self.diff_raw_input.clone())
                                .into_any_element();
                        }

                        let total_len = match self.diff_view {
                            DiffViewMode::Inline => self.file_diff_inline_cache.len(),
                            DiffViewMode::Split => self.file_diff_cache_rows.len(),
                        };
                        if total_len == 0 {
                            zed::empty_state(theme, "Diff", "Empty file.").into_any_element()
                        } else if self.diff_visible_indices.is_empty() {
                            zed::empty_state(theme, "Diff", "Nothing to render.").into_any_element()
                        } else {
                            let scroll_handle = self.diff_scroll.0.borrow().base_handle.clone();
                            let markers = self.diff_scrollbar_markers_cache.clone();
                            match self.diff_view {
                                DiffViewMode::Inline => {
                                    let list = uniform_list(
                                        "diff",
                                        self.diff_visible_indices.len(),
                                        cx.processor(Self::render_diff_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .track_scroll(self.diff_scroll.clone())
                                    .with_horizontal_sizing_behavior(
                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                    );
                                    div()
                                        .id("diff_scroll_container")
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .bg(theme.colors.window_bg)
                                        .child(list)
                                        .child(
                                            zed::Scrollbar::new(
                                                "diff_scrollbar",
                                                scroll_handle.clone(),
                                            )
                                            .markers(markers)
                                            .always_visible()
                                            .render(theme),
                                        )
                                        .child(
                                            zed::Scrollbar::horizontal(
                                                "diff_hscrollbar",
                                                scroll_handle,
                                            )
                                            .always_visible()
                                            .render(theme),
                                        )
                                        .into_any_element()
                                }
                                DiffViewMode::Split => {
                                    self.sync_diff_split_vertical_scroll();
                                    let right_scroll_handle =
                                        self.diff_split_right_scroll.0.borrow().base_handle.clone();
                                    let count = self.diff_visible_indices.len();
                                    let left = uniform_list(
                                        "diff_split_left",
                                        count,
                                        cx.processor(Self::render_diff_split_left_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .track_scroll(self.diff_scroll.clone())
                                    .with_horizontal_sizing_behavior(
                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                    );
                                    let right = uniform_list(
                                        "diff_split_right",
                                        count,
                                        cx.processor(Self::render_diff_split_right_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .track_scroll(self.diff_split_right_scroll.clone())
                                    .with_horizontal_sizing_behavior(
                                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                                    );

                                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                                    let min_col_w = px(DIFF_SPLIT_COL_MIN_PX);
                                    let main_w = self.main_pane_content_width(cx);
                                    let available = (main_w - handle_w).max(px(0.0));
                                    let left_w = if available <= min_col_w * 2.0 {
                                        available * 0.5
                                    } else {
                                        (available * self.diff_split_ratio)
                                            .max(min_col_w)
                                            .min(available - min_col_w)
                                    };
                                    let right_w = available - left_w;

                                    let resize_handle = |id: &'static str| {
                                        div()
                                            .id(id)
                                            .w(handle_w)
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor(CursorStyle::ResizeLeftRight)
                                            .hover(move |s| {
                                                s.bg(with_alpha(theme.colors.hover, 0.65))
                                            })
                                            .active(move |s| s.bg(theme.colors.active))
                                            .child(
                                                div()
                                                    .w(px(1.0))
                                                    .h_full()
                                                    .bg(theme.colors.border),
                                            )
                                            .on_drag(
                                                DiffSplitResizeHandle::Divider,
                                                |_handle, _offset, _window, cx| {
                                                    cx.new(|_cx| DiffSplitResizeDragGhost)
                                                },
                                            )
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(
                                                    move |this, e: &MouseDownEvent, _w, cx| {
                                                        cx.stop_propagation();
                                                        this.diff_split_resize =
                                                            Some(DiffSplitResizeState {
                                                                handle:
                                                                    DiffSplitResizeHandle::Divider,
                                                                start_x: e.position.x,
                                                                start_ratio: this.diff_split_ratio,
                                                            });
                                                        cx.notify();
                                                    },
                                                ),
                                            )
                                            .on_drag_move(cx.listener(
                                                move |this,
                                                      e: &gpui::DragMoveEvent<
                                                    DiffSplitResizeHandle,
                                                >,
                                                      _w,
                                                      cx| {
                                                    let Some(state) = this.diff_split_resize else {
                                                        return;
                                                    };
                                                    if state.handle != *e.drag(cx) {
                                                        return;
                                                    }

                                                    let main_w =
                                                        this.main_pane_content_width(cx);
                                                    let available =
                                                        (main_w - handle_w).max(px(0.0));
                                                    if available <= min_col_w * 2.0 {
                                                        this.diff_split_ratio = 0.5;
                                                        cx.notify();
                                                        return;
                                                    }

                                                    let dx =
                                                        e.event.position.x - state.start_x;
                                                    let max_left = available - min_col_w;
                                                    let mut next_left =
                                                        (available * state.start_ratio) + dx;
                                                    next_left =
                                                        next_left.max(min_col_w).min(max_left);

                                                    this.diff_split_ratio =
                                                        (next_left / available).clamp(0.0, 1.0);
                                                    cx.notify();
                                                },
                                            ))
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|this, _e, _w, cx| {
                                                    this.diff_split_resize = None;
                                                    cx.notify();
                                                }),
                                            )
                                            .on_mouse_up_out(
                                                MouseButton::Left,
                                                cx.listener(|this, _e, _w, cx| {
                                                    this.diff_split_resize = None;
                                                    cx.notify();
                                                }),
                                            )
                                    };

                                    let columns_header = div()
                                        .id("diff_split_columns_header")
                                        .h(px(zed::CONTROL_HEIGHT_PX))
                                        .flex()
                                        .items_center()
                                        .text_xs()
                                        .text_color(theme.colors.text_muted)
                                        .bg(theme.colors.surface_bg_elevated)
                                        .border_b_1()
                                        .border_color(theme.colors.border)
                                        .child(
                                            div()
                                                .w(left_w)
                                                .min_w(px(0.0))
                                                .px_2()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .child("A (local / before)"),
                                        )
                                        .child(resize_handle("diff_split_resize_handle_header"))
                                        .child(
                                            div()
                                                .w(right_w)
                                                .min_w(px(0.0))
                                                .px_2()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .child("B (remote / after)"),
                                        );

                                    div()
                                        .id("diff_split_scroll_container")
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .flex()
                                        .flex_col()
                                        .bg(theme.colors.window_bg)
                                        .child(columns_header)
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h(px(0.0))
                                                .flex()
                                                .child(
                                                    div()
                                                        .relative()
                                                        .w(left_w)
                                                        .min_w(px(0.0))
                                                        .h_full()
                                                        .child(left)
                                                        .child(
                                                            zed::Scrollbar::horizontal(
                                                                "diff_split_left_hscrollbar",
                                                                scroll_handle.clone(),
                                                            )
                                                            .always_visible()
                                                            .render(theme),
                                                        ),
                                                )
                                                .child(resize_handle(
                                                    "diff_split_resize_handle_body",
                                                ))
                                                .child(
                                                    div()
                                                        .relative()
                                                        .w(right_w)
                                                        .min_w(px(0.0))
                                                        .h_full()
                                                        .child(right)
                                                        .child(
                                                            zed::Scrollbar::horizontal(
                                                                "diff_split_right_hscrollbar",
                                                                right_scroll_handle,
                                                            )
                                                            .always_visible()
                                                            .render(theme),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            zed::Scrollbar::new(
                                                "diff_scrollbar",
                                                scroll_handle.clone(),
                                            )
                                            .markers(markers)
                                            .always_visible()
                                            .render(theme),
                                        )
                                        .into_any_element()
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
