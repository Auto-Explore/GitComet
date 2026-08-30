use super::*;
use crate::view::panes::main::DiffHorizontalScrollColumn;

fn file_diff_ready_shows_processing(
    has_file: bool,
    cache_active: bool,
    cache_inflight: bool,
    has_rendered_rows: bool,
) -> bool {
    // Rows already built for this file stay on screen while a refresh rebuilds
    // them, so a reload does not blink through a placeholder. The placeholder is
    // only for having nothing to show.
    has_file && (!cache_active || cache_inflight) && !has_rendered_rows
}

fn image_diff_ready_shows_processing(has_file: bool, cache_active: bool) -> bool {
    has_file && !cache_active
}

/// Inset between an image/SVG preview column and its artwork.
const IMAGE_PREVIEW_CELL_PADDING_PX: f32 = 16.0;

/// Gap above the first and below the last row of a rendered markdown preview,
/// so the document does not start and end flush against the pane edges.
pub(in crate::view) const MARKDOWN_PREVIEW_DOCUMENT_EDGE_GAP_PX: f32 = 12.0;

impl MainPaneView {
    pub(in crate::view) fn render_diff_horizontal_scrollbar(
        theme: AppTheme,
        id: &'static str,
        handle: UniformListScrollHandle,
        right_inset: Pixels,
        _debug_selector: &'static str,
    ) -> AnyElement {
        let scrollbar = components::Scrollbar::horizontal(id, handle).always_visible();
        #[cfg(test)]
        let scrollbar = scrollbar.debug_selector(_debug_selector);

        div()
            .absolute()
            .left_0()
            .right(right_inset.max(px(0.0)))
            .bottom_0()
            .h(components::Scrollbar::gutter(
                components::ScrollbarAxis::Horizontal,
            ))
            .child(scrollbar.render(theme))
            .into_any_element()
    }

    pub(in crate::view) fn conflict_resolver_strategy(
        conflict: Option<gitcomet_core::domain::FileConflictKind>,
        is_binary: bool,
    ) -> Option<gitcomet_core::conflict_session::ConflictResolverStrategy> {
        conflict.map(|kind| {
            gitcomet_core::conflict_session::ConflictResolverStrategy::for_conflict(kind, is_binary)
        })
    }

    pub(super) fn render_selected_file_diff(
        &mut self,
        theme: AppTheme,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let rendered_preview_kind =
            crate::view::diff_target_rendered_preview_kind(self.rendered_diff_target());
        let has_image = self
            .rendered_file_image_diff_loadable()
            .is_some_and(|file| !matches!(file, Loadable::NotLoaded));
        // An image has no collapsed form — the rendered picture is the whole
        // file — so the image view stays available in either diff mode. Only
        // the SVG Image/Code toggle can send an image target down the text path.
        let wants_image = has_image
            && (!matches!(rendered_preview_kind, Some(RenderedPreviewKind::Svg))
                || self.rendered_preview_modes.get(RenderedPreviewKind::Svg)
                    == RenderedPreviewMode::Rendered);
        let wants_markdown_preview = self.diff_content_mode == DiffContentMode::Full
            && rendered_preview_kind == Some(RenderedPreviewKind::Markdown)
            && self
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Rendered;

        if wants_image {
            enum DiffFileImageState {
                NotLoaded,
                Loading,
                Error(String),
                Ready { has_file: bool },
            }

            let diff_file_state = match self.rendered_file_image_diff_loadable() {
                None => {
                    return components::empty_state(theme, "Diff", "No repository.")
                        .into_any_element();
                }
                Some(Loadable::NotLoaded) => DiffFileImageState::NotLoaded,
                Some(Loadable::Loading) => DiffFileImageState::Loading,
                Some(Loadable::Error(e)) => DiffFileImageState::Error(e.clone()),
                Some(Loadable::Ready(file)) => DiffFileImageState::Ready {
                    has_file: file.is_some(),
                },
            };

            self.ensure_file_image_diff_cache(cx);
            match diff_file_state {
                DiffFileImageState::NotLoaded => {
                    components::empty_state(theme, "Diff", "Select a file.").into_any_element()
                }
                DiffFileImageState::Loading => {
                    components::empty_state(theme, "Diff", "Loading").into_any_element()
                }
                DiffFileImageState::Error(e) => {
                    self.diff_raw_input.update(cx, |input, cx| {
                        input.set_theme(theme, cx);
                        input.set_text(e, cx);
                        input.set_read_only(true, cx);
                    });
                    div()
                        .id("diff_file_image_error_scroll")
                        .bg(theme.colors.surface.canvas)
                        .font_family(editor_font_family.clone())
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .child(self.diff_raw_input.clone())
                        .into_any_element()
                }
                DiffFileImageState::Ready { has_file } => {
                    if !has_file {
                        components::empty_state(theme, "Diff", "No image contents available.")
                            .into_any_element()
                    } else if image_diff_ready_shows_processing(
                        has_file,
                        self.is_file_image_diff_view_active(),
                    ) {
                        components::empty_state(theme, "Diff", "Processing image...")
                            .into_any_element()
                    } else if self.file_image_diff_cache_failed {
                        components::empty_state(theme, "Diff", "Preview unavailable.")
                            .into_any_element()
                    } else {
                        enum CachedDiffImageSource {
                            Path(std::path::PathBuf),
                            Render(Arc<gpui::RenderImage>, usize),
                        }

                        let old_render = self.file_image_diff_cache_old.clone();
                        let new_render = self.file_image_diff_cache_new.clone();
                        let [old_frame, new_frame] = self.update_file_image_preview_animation(
                            old_render.as_ref(),
                            new_render.as_ref(),
                            window,
                            cx,
                        );
                        let old = self
                            .file_image_diff_cache_old_svg_path
                            .clone()
                            .map(CachedDiffImageSource::Path)
                            .or_else(|| {
                                old_render
                                    .map(|image| CachedDiffImageSource::Render(image, old_frame))
                            });
                        let new = self
                            .file_image_diff_cache_new_svg_path
                            .clone()
                            .map(CachedDiffImageSource::Path)
                            .or_else(|| {
                                new_render
                                    .map(|image| CachedDiffImageSource::Render(image, new_frame))
                            });

                        // Breathing room around the artwork. Without it an SVG
                        // renders edge to edge and reads as cramped against
                        // the column header, the split divider, and the pane
                        // edges.
                        let cell_padding = crate::ui_scale::design_px_from_percent(
                            IMAGE_PREVIEW_CELL_PADDING_PX,
                            ui_scale_percent,
                        );
                        let clamp_preview_size = self
                            .file_image_diff_cache_path
                            .as_deref()
                            .is_some_and(preview_path_uses_scale_down);
                        let cell = |id: &'static str, image: Option<CachedDiffImageSource>| {
                            let muted = theme.colors.foreground.secondary;
                            div()
                                .id(id)
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .justify_center()
                                .p(cell_padding)
                                .child(match image {
                                    Some(CachedDiffImageSource::Path(path)) => gpui::img(path)
                                        .w_full()
                                        .h_full()
                                        .object_fit(gpui::ObjectFit::Contain)
                                        .with_loading(move || {
                                            div()
                                                .text_sm()
                                                .text_color(muted)
                                                .child("Processing image...")
                                                .into_any_element()
                                        })
                                        .with_fallback(move || {
                                            div()
                                                .text_sm()
                                                .text_color(muted)
                                                .child("Preview unavailable.")
                                                .into_any_element()
                                        })
                                        .into_any_element(),
                                    Some(CachedDiffImageSource::Render(img_data, frame_index)) => {
                                        preview_render_image_element(
                                            img_data,
                                            frame_index,
                                            clamp_preview_size,
                                        )
                                        .w_full()
                                        .h_full()
                                        .into_any_element()
                                    }
                                    None => div()
                                        .text_sm()
                                        .text_color(theme.colors.foreground.secondary)
                                        .child("No image")
                                        .into_any_element(),
                                })
                        };

                        // A content view is one file, not a comparison: opening
                        // a picture from the explorer shows the picture, with no
                        // A/B header and no empty "before" half to explain away.
                        let is_content_view = self
                            .active_repo()
                            .is_some_and(|repo| repo.diff_state.content_preview);
                        if is_content_view {
                            return div()
                                .id("diff_image_container")
                                .debug_selector(|| "diff_image_single".to_string())
                                .relative()
                                .h_full()
                                .min_h(px(0.0))
                                .flex()
                                .flex_col()
                                .bg(theme.colors.surface.canvas)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h(px(0.0))
                                        .flex()
                                        .child(cell("diff_image_single_cell", new.or(old))),
                                )
                                .into_any_element();
                        }

                        let columns_header = components::split_columns_header(
                            theme,
                            ui_scale_percent,
                            "A (before)",
                            "B (after)",
                        );

                        div()
                            .id("diff_image_container")
                            .relative()
                            .h_full()
                            .min_h(px(0.0))
                            .flex()
                            .flex_col()
                            .bg(theme.colors.surface.canvas)
                            .child(columns_header)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .flex()
                                    .child(cell("diff_image_left", old))
                                    .child(
                                        div().w(px(1.0)).h_full().bg(theme.colors.stroke.default),
                                    )
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

            let diff_file_state = match self.rendered_file_diff_loadable() {
                None => {
                    return components::empty_state(theme, "Diff", "No repository.")
                        .into_any_element();
                }
                Some(Loadable::NotLoaded) => DiffFileState::NotLoaded,
                Some(Loadable::Loading) => DiffFileState::Loading,
                Some(Loadable::Error(e)) => DiffFileState::Error(e.clone()),
                Some(Loadable::Ready(file)) => DiffFileState::Ready {
                    has_file: file.is_some(),
                },
            };

            if !wants_markdown_preview {
                self.ensure_file_diff_cache(cx);
            }

            match diff_file_state {
                DiffFileState::NotLoaded => {
                    components::empty_state(theme, "Diff", "Select a file.").into_any_element()
                }
                DiffFileState::Loading => {
                    let label = if wants_markdown_preview {
                        "Preview"
                    } else {
                        "Diff"
                    };
                    components::empty_state(theme, label, "Loading").into_any_element()
                }
                DiffFileState::Error(e) => {
                    if wants_markdown_preview {
                        components::empty_state(theme, "Preview", e).into_any_element()
                    } else {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(e, cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("diff_file_error_scroll")
                            .bg(theme.colors.surface.canvas)
                            .font_family(editor_font_family.clone())
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    }
                }
                DiffFileState::Ready { has_file } if wants_markdown_preview => {
                    if !has_file {
                        components::empty_state(theme, "Preview", "No file contents available.")
                            .into_any_element()
                    } else {
                        self.ensure_file_markdown_preview_cache(cx);
                        match &self.file_markdown_preview {
                            Loadable::NotLoaded | Loadable::Loading => {
                                components::empty_state(theme, "Preview", "Processing preview...")
                                    .into_any_element()
                            }
                            Loadable::Error(e) => {
                                components::empty_state(theme, "Preview", e.clone())
                                    .into_any_element()
                            }
                            Loadable::Ready(preview) => {
                                let preview = std::sync::Arc::clone(preview);
                                let document_rev = self.file_markdown_preview_seq;
                                let (old_len, new_len, inline_len) = self
                                    .ensure_markdown_preview_wrap_plans(
                                        &preview,
                                        document_rev,
                                        window,
                                        cx,
                                    );
                                self.render_markdown_diff_preview(
                                    theme, old_len, new_len, inline_len, cx,
                                )
                            }
                        }
                    }
                }
                DiffFileState::Ready { has_file } => {
                    let text_cache_active = match self.effective_diff_content_mode() {
                        DiffContentMode::Full => self.is_file_diff_view_active(),
                        DiffContentMode::Collapsed => self.is_collapsed_diff_projection_active(),
                    };
                    if !has_file {
                        components::empty_state(theme, "Diff", "No file contents available.")
                            .into_any_element()
                    } else if let Some(error) = self.file_diff_cache_error.clone() {
                        self.diff_raw_input.update(cx, |input, cx| {
                            input.set_theme(theme, cx);
                            input.set_text(error, cx);
                            input.set_read_only(true, cx);
                        });
                        div()
                            .id("diff_file_error_scroll")
                            .bg(theme.colors.surface.canvas)
                            .font_family(editor_font_family.clone())
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(self.diff_raw_input.clone())
                            .into_any_element()
                    } else if file_diff_ready_shows_processing(
                        has_file,
                        text_cache_active,
                        self.file_diff_cache_inflight.is_some(),
                        self.file_diff_cache_content_signature.is_some(),
                    ) {
                        components::empty_state(theme, "Diff", "Processing file...")
                            .into_any_element()
                    } else {
                        self.ensure_diff_visible_indices();
                        self.ensure_diff_wrap_visible_rows(window, cx);
                        self.maybe_autoscroll_diff_to_first_change();

                        let total_len = if self.is_collapsed_diff_projection_active() {
                            self.collapsed_diff_visible_rows.len()
                        } else {
                            match self.diff_view {
                                DiffViewMode::Inline => self.file_diff_inline_row_len(),
                                DiffViewMode::Split => self.file_diff_split_row_len(),
                            }
                        };
                        if total_len == 0 {
                            empty_diff_text_document(
                                cx.entity(),
                                DiffTextRegion::Inline,
                                components::empty_state(theme, "Diff", "Empty file.")
                                    .into_any_element(),
                            )
                        } else if self.diff_visible_len() == 0 {
                            components::empty_state(theme, "Diff", "Nothing to render.")
                                .into_any_element()
                        } else {
                            let markers = self.diff_scrollbar_markers_cache.clone();
                            match self.diff_view {
                                DiffViewMode::Inline => {
                                    let horizontal_scrollbar_gutter = components::Scrollbar::gutter(
                                        components::ScrollbarAxis::Horizontal,
                                    );
                                    let scrollbar_gutter = self
                                        .diff_vertical_scrollbar_gutter_for_column(
                                            DiffHorizontalScrollColumn::Primary,
                                            self.diff_scroll.clone(),
                                        );
                                    let list = uniform_list(
                                        "diff",
                                        self.diff_visible_len(),
                                        cx.processor(Self::render_diff_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .pb(if self.diff_word_wrap {
                                        px(0.0)
                                    } else {
                                        horizontal_scrollbar_gutter
                                    })
                                    .track_scroll(&self.diff_scroll)
                                    .with_decoration(DiffTextEmptySpaceDecoration {
                                        view: cx.entity(),
                                        region: DiffTextRegion::Inline,
                                    })
                                    .when(
                                        !self.diff_word_wrap,
                                        |list| {
                                            list.with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            )
                                        },
                                    );
                                    div()
                                        .id("diff_scroll_container")
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .bg(theme.colors.surface.canvas)
                                        .font_family(editor_font_family.clone())
                                        .child(
                                            div()
                                                .h_full()
                                                .min_h(px(0.0))
                                                .pr(scrollbar_gutter)
                                                .child(list),
                                        )
                                        // Anchored to the rows container so the
                                        // handle's hover highlight matches the
                                        // annotation column height exactly.
                                        .when(self.annotation_active(), |d| {
                                            d.child(self.annotate_resize_handle(
                                                ui_scale_percent,
                                                theme,
                                                cx,
                                            ))
                                        })
                                        .child(
                                            components::Scrollbar::new(
                                                "diff_scrollbar",
                                                self.diff_scroll.clone(),
                                            )
                                            .markers(markers)
                                            .always_visible()
                                            .render(theme),
                                        )
                                        .when(!self.diff_word_wrap, |d| {
                                            d.child(Self::render_diff_horizontal_scrollbar(
                                                theme,
                                                "diff_hscrollbar",
                                                self.diff_scroll.clone(),
                                                scrollbar_gutter,
                                                "diff_hscrollbar",
                                            ))
                                        })
                                        .into_any_element()
                                }
                                DiffViewMode::Split => {
                                    self.sync_diff_split_scroll();
                                    let vertical_sync_enabled =
                                        self.diff_scroll_sync.includes_vertical();
                                    let count = self.diff_visible_len();
                                    let horizontal_scrollbar_gutter = components::Scrollbar::gutter(
                                        components::ScrollbarAxis::Horizontal,
                                    );
                                    let left_scrollbar_gutter = self
                                        .diff_vertical_scrollbar_gutter_for_column(
                                            DiffHorizontalScrollColumn::Primary,
                                            self.diff_scroll.clone(),
                                        );
                                    let right_scrollbar_gutter = self
                                        .diff_vertical_scrollbar_gutter_for_column(
                                            DiffHorizontalScrollColumn::SplitRight,
                                            self.diff_split_right_scroll.clone(),
                                        );
                                    let shared_scrollbar_gutter = if vertical_sync_enabled {
                                        left_scrollbar_gutter
                                    } else {
                                        px(0.0)
                                    };
                                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                                    let main_w = (self.main_pane_content_width(cx)
                                        - shared_scrollbar_gutter)
                                        .max(px(0.0));
                                    let (_, min_col_w) = diff_split_drag_params(main_w);
                                    let (left_w, right_w) =
                                        diff_split_column_widths(main_w, self.diff_split_ratio);
                                    let left = uniform_list(
                                        "diff_split_left",
                                        count,
                                        cx.processor(Self::render_diff_split_left_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .pb(if self.diff_word_wrap {
                                        px(0.0)
                                    } else {
                                        horizontal_scrollbar_gutter
                                    })
                                    .track_scroll(&self.diff_scroll)
                                    .with_decoration(DiffTextEmptySpaceDecoration {
                                        view: cx.entity(),
                                        region: DiffTextRegion::SplitLeft,
                                    })
                                    .when(
                                        !self.diff_word_wrap,
                                        |list| {
                                            list.with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            )
                                        },
                                    );
                                    let right = uniform_list(
                                        "diff_split_right",
                                        count,
                                        cx.processor(Self::render_diff_split_right_rows),
                                    )
                                    .h_full()
                                    .min_h(px(0.0))
                                    .pb(if self.diff_word_wrap {
                                        px(0.0)
                                    } else {
                                        horizontal_scrollbar_gutter
                                    })
                                    .track_scroll(&self.diff_split_right_scroll)
                                    .with_decoration(DiffTextEmptySpaceDecoration {
                                        view: cx.entity(),
                                        region: DiffTextRegion::SplitRight,
                                    })
                                    .when(
                                        !self.diff_word_wrap,
                                        |list| {
                                            list.with_horizontal_sizing_behavior(
                                                gpui::ListHorizontalSizingBehavior::Unconstrained,
                                            )
                                        },
                                    );
                                    let collapsed_file_stat = self
                                        .is_collapsed_diff_projection_active()
                                        .then(|| self.collapsed_diff_total_file_stat())
                                        .flatten();
                                    let (left_label, right_label) = self.split_diff_pane_labels();
                                    let left_header = Self::split_column_header_label(
                                        left_label,
                                        collapsed_file_stat.map(|(_, removed)| removed),
                                        '-',
                                        theme.colors.diff.removed.foreground,
                                    );
                                    let right_header = Self::split_column_header_label(
                                        right_label,
                                        collapsed_file_stat.map(|(added, _)| added),
                                        '+',
                                        theme.colors.diff.added.foreground,
                                    );

                                    // Built before `resize_handle` captures `cx`.
                                    let split_annotate_handle =
                                        self.annotation_active().then(|| {
                                            self.annotate_resize_handle(ui_scale_percent, theme, cx)
                                        });

                                    let split_dragging = self.diff_split_resize.is_some();
                                    let resize_handle = |id: &'static str| {
                                        div()
                                            .id(id)
                                            .group(id)
                                            .w(handle_w)
                                            .h_full()
                                            .cursor(CursorStyle::ResizeLeftRight)
                                            .child(components::resize_grip(
                                                theme,
                                                ui_scale_percent,
                                                id,
                                                components::ResizeGripAxis::Vertical,
                                                split_dragging,
                                                Some(theme.colors.stroke.default),
                                            ))
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
                                                        crate::press_gesture::claim_press(cx);
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

                                                    let scrollbar_gutter = if this
                                                        .diff_scroll_sync
                                                        .includes_vertical()
                                                    {
                                                        components::Scrollbar::visible_gutter(
                                                            this.diff_scroll.clone(),
                                                            components::ScrollbarAxis::Vertical,
                                                        )
                                                    } else {
                                                        px(0.0)
                                                    };
                                                    let main_w = (this.main_pane_content_width(cx)
                                                        - scrollbar_gutter)
                                                        .max(px(0.0));
                                                    let available =
                                                        (main_w - handle_w).max(px(0.0));
                                                    let dx =
                                                        e.event.position.x - state.start_x;
                                                    match next_diff_split_drag_ratio(
                                                        available,
                                                        min_col_w,
                                                        state.start_ratio,
                                                        dx,
                                                    ) {
                                                        None => {
                                                            if (this.diff_split_ratio - 0.5)
                                                                .abs()
                                                                > f32::EPSILON
                                                            {
                                                                this.diff_split_ratio = 0.5;
                                                                cx.notify();
                                                            }
                                                        }
                                                        Some(next_ratio) => {
                                                            if (this.diff_split_ratio
                                                                - next_ratio)
                                                                .abs()
                                                                > f32::EPSILON
                                                            {
                                                                this.diff_split_ratio =
                                                                    next_ratio;
                                                                cx.notify();
                                                            }
                                                        }
                                                    }
                                                },
                                            ))
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|this, _e, _w, cx| {
                                                    if this.diff_split_resize.take().is_some() {
                                                        cx.notify();
                                                    }
                                                }),
                                            )
                                            .on_mouse_up_out(
                                                MouseButton::Left,
                                                cx.listener(|this, _e, _w, cx| {
                                                    if this.diff_split_resize.take().is_some() {
                                                        cx.notify();
                                                    }
                                                }),
                                            )
                                    };

                                    let columns_header = div()
                                        .id("diff_split_columns_header")
                                        .debug_selector(|| "diff_split_columns_header".to_string())
                                        .w_full()
                                        // Same right inset as the body below, so both rows divide
                                        // the identical content box and the column divider lines
                                        // up. Padding keeps the band and its bottom border
                                        // full-bleed.
                                        .pr(shared_scrollbar_gutter)
                                        .h(components::control_height(ui_scale_percent))
                                        .flex()
                                        .items_center()
                                        .text_xs()
                                        .text_color(theme.colors.foreground.secondary)
                                        .bg(crate::theme::content_header_bg(theme))
                                        .border_b_1()
                                        .border_color(theme.colors.stroke.default)
                                        .child(
                                            div()
                                                .w(left_w)
                                                .min_w(px(0.0))
                                                .px_2()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .child(left_header),
                                        )
                                        .child(resize_handle("diff_split_resize_handle_header"))
                                        .child(
                                            div()
                                                .w(right_w)
                                                .min_w(px(0.0))
                                                .px_2()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .child(right_header),
                                        );

                                    div()
                                        .id("diff_split_scroll_container")
                                        .relative()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .flex()
                                        .flex_col()
                                        .bg(theme.colors.surface.canvas)
                                        .font_family(editor_font_family.clone())
                                        .child(columns_header)
                                        .child(
                                            div()
                                                .relative()
                                                .pr(shared_scrollbar_gutter)
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_h(px(0.0))
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
                                                                .child(
                                                                    div()
                                                                        .h_full()
                                                                        .min_h(px(0.0))
                                                                        .pr(if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            left_scrollbar_gutter
                                                                        })
                                                                        .child(left),
                                                                )
                                                                .when(!vertical_sync_enabled, |d| {
                                                                    d.child(
                                                                        components::Scrollbar::new(
                                                                            "diff_split_left_scrollbar",
                                                                            self.diff_scroll.clone(),
                                                                        )
                                                                        .markers(markers.clone())
                                                                        .always_visible()
                                                                        .render(theme),
                                                                    )
                                                                })
                                                                .when(!self.diff_word_wrap, |d| {
                                                                    d.child(
                                                                        Self::render_diff_horizontal_scrollbar(
                                                                            theme,
                                                                            "diff_split_left_hscrollbar",
                                                                            self.diff_scroll.clone(),
                                                                            if vertical_sync_enabled {
                                                                                px(0.0)
                                                                            } else {
                                                                                left_scrollbar_gutter
                                                                            },
                                                                            "diff_split_left_hscrollbar",
                                                                        ),
                                                                    )
                                                                })
                                                                .when_some(
                                                                    split_annotate_handle,
                                                                    |d, handle| d.child(handle),
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
                                                                .child(
                                                                    div()
                                                                        .h_full()
                                                                        .min_h(px(0.0))
                                                                        .pr(if vertical_sync_enabled {
                                                                            px(0.0)
                                                                        } else {
                                                                            right_scrollbar_gutter
                                                                        })
                                                                        .child(right),
                                                                )
                                                                .when(!vertical_sync_enabled, |d| {
                                                                    d.child(
                                                                        components::Scrollbar::new(
                                                                            "diff_split_right_scrollbar",
                                                                            self.diff_split_right_scroll.clone(),
                                                                        )
                                                                        .markers(markers.clone())
                                                                        .always_visible()
                                                                        .render(theme),
                                                                    )
                                                                })
                                                                .when(!self.diff_word_wrap, |d| {
                                                                    d.child(
                                                                        Self::render_diff_horizontal_scrollbar(
                                                                            theme,
                                                                            "diff_split_right_hscrollbar",
                                                                            self.diff_split_right_scroll.clone(),
                                                                            if vertical_sync_enabled {
                                                                                px(0.0)
                                                                            } else {
                                                                                right_scrollbar_gutter
                                                                            },
                                                                            "diff_split_right_hscrollbar",
                                                                        ),
                                                                    )
                                                                }),
                                                        ),
                                                )
                                                .when(vertical_sync_enabled, |d| {
                                                    d.child(
                                                        components::Scrollbar::new(
                                                            "diff_scrollbar",
                                                            self.diff_scroll.clone(),
                                                        )
                                                        .markers(markers)
                                                        .always_visible()
                                                        .render(theme),
                                                    )
                                                }),
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

    fn render_markdown_diff_preview(
        &mut self,
        theme: AppTheme,
        old_len: usize,
        new_len: usize,
        inline_len: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        if old_len == 0 && new_len == 0 {
            return empty_diff_text_document(
                cx.entity(),
                DiffTextRegion::Inline,
                components::empty_state(theme, "Preview", "Empty file.").into_any_element(),
            );
        }

        self.maybe_autoscroll_diff_to_first_change();

        let scrollbar_markers = match &self.file_markdown_preview {
            Loadable::Ready(preview) => match self.diff_view {
                DiffViewMode::Inline => {
                    crate::view::markdown_preview::scrollbar_markers_for_document(&preview.inline)
                }
                DiffViewMode::Split => {
                    crate::view::markdown_preview::scrollbar_markers_for_diff_preview(
                        preview.as_ref(),
                    )
                }
            },
            _ => Vec::new(),
        };

        let empty_column = |region| {
            empty_diff_text_document(
                cx.entity(),
                region,
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child("Empty file.")
                    .into_any_element(),
            )
        };

        let vertical_sync_enabled = self.diff_scroll_sync.includes_vertical();
        let mk_column = |id: &'static str,
                         vscrollbar_id: &'static str,
                         hscrollbar_id: &'static str,
                         list: AnyElement,
                         scroll: UniformListScrollHandle,
                         scroll_handle: gpui::ScrollHandle|
         -> AnyElement {
            let vertical_scrollbar_gutter = if vertical_sync_enabled {
                px(0.0)
            } else {
                components::Scrollbar::visible_gutter(
                    scroll.clone(),
                    components::ScrollbarAxis::Vertical,
                )
            };
            div()
                .id(id)
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(
                    div()
                        .h_full()
                        .min_h(px(0.0))
                        .pr(vertical_scrollbar_gutter)
                        .child(list),
                )
                .when(!vertical_sync_enabled, |d| {
                    d.child(
                        components::Scrollbar::new(vscrollbar_id, scroll.clone())
                            .always_visible()
                            .render(theme),
                    )
                })
                .child(
                    components::Scrollbar::horizontal(hscrollbar_id, scroll_handle)
                        .always_visible()
                        .render(theme),
                )
                .into_any_element()
        };

        let document_edge_gap = crate::ui_scale::design_px_from_percent(
            MARKDOWN_PREVIEW_DOCUMENT_EDGE_GAP_PX,
            ui_scale_percent,
        );
        macro_rules! mk_list {
            ($name:expr, $len:expr, $region:expr, $scroll:expr, $proc:expr) => {
                uniform_list($name, $len, $proc)
                    .h_full()
                    .min_h(px(0.0))
                    .pt(document_edge_gap)
                    .pb(document_edge_gap)
                    .track_scroll(&$scroll)
                    .with_decoration(DiffTextEmptySpaceDecoration {
                        view: cx.entity(),
                        region: $region,
                    })
                    .with_horizontal_sizing_behavior(
                        gpui::ListHorizontalSizingBehavior::Unconstrained,
                    )
                    .into_any_element()
            };
        }

        if self.diff_view == DiffViewMode::Inline {
            if inline_len == 0 {
                return components::empty_state(theme, "Preview", "Nothing to render.")
                    .into_any_element();
            }

            let scroll_handle = self.diff_scroll.0.borrow().base_handle.clone();
            let list = mk_list!(
                "diff_markdown_preview_inline",
                inline_len,
                DiffTextRegion::Inline,
                self.diff_scroll.clone(),
                cx.processor(Self::render_markdown_diff_inline_rows)
            );

            return div()
                .id("diff_markdown_preview_container")
                .relative()
                .h_full()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .bg(theme.colors.surface.canvas)
                .child(
                    div()
                        .id("diff_markdown_preview_inline_container")
                        .relative()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .h_full()
                                .min_h(px(0.0))
                                .pr(components::Scrollbar::visible_gutter(
                                    self.diff_scroll.clone(),
                                    components::ScrollbarAxis::Vertical,
                                ))
                                .child(list),
                        )
                        .child(
                            components::Scrollbar::horizontal(
                                "diff_markdown_preview_inline_hscrollbar",
                                scroll_handle.clone(),
                            )
                            .always_visible()
                            .render(theme),
                        ),
                )
                .child(
                    components::Scrollbar::new(
                        "diff_markdown_preview_scrollbar",
                        self.diff_scroll.clone(),
                    )
                    .markers(scrollbar_markers)
                    .always_visible()
                    .render(theme),
                )
                .into_any_element();
        }

        let (left_column, right_column, vertical_scroll_handle) = if old_len == 0 {
            let handle = self.diff_scroll.0.borrow().base_handle.clone();
            let list = mk_list!(
                "diff_markdown_preview_right_single",
                new_len,
                DiffTextRegion::SplitRight,
                self.diff_scroll.clone(),
                cx.processor(Self::render_markdown_diff_right_rows)
            );
            (
                empty_column(DiffTextRegion::SplitLeft),
                mk_column(
                    "diff_markdown_preview_right",
                    "diff_markdown_preview_right_scrollbar",
                    "diff_markdown_preview_right_hscrollbar",
                    list,
                    self.diff_scroll.clone(),
                    handle.clone(),
                ),
                handle,
            )
        } else if new_len == 0 {
            let handle = self.diff_scroll.0.borrow().base_handle.clone();
            let list = mk_list!(
                "diff_markdown_preview_left_single",
                old_len,
                DiffTextRegion::SplitLeft,
                self.diff_scroll.clone(),
                cx.processor(Self::render_markdown_diff_left_rows)
            );
            (
                mk_column(
                    "diff_markdown_preview_left",
                    "diff_markdown_preview_left_scrollbar",
                    "diff_markdown_preview_left_hscrollbar",
                    list,
                    self.diff_scroll.clone(),
                    handle.clone(),
                ),
                empty_column(DiffTextRegion::SplitRight),
                handle,
            )
        } else {
            self.sync_diff_split_scroll();
            let left_handle = self.diff_scroll.0.borrow().base_handle.clone();
            let right_handle = self.diff_split_right_scroll.0.borrow().base_handle.clone();
            let vertical_scroll_handle = if new_len > old_len {
                right_handle.clone()
            } else {
                left_handle.clone()
            };
            let left_list = mk_list!(
                "diff_markdown_preview_left",
                old_len,
                DiffTextRegion::SplitLeft,
                self.diff_scroll.clone(),
                cx.processor(Self::render_markdown_diff_left_rows)
            );
            let right_list = mk_list!(
                "diff_markdown_preview_right",
                new_len,
                DiffTextRegion::SplitRight,
                self.diff_split_right_scroll.clone(),
                cx.processor(Self::render_markdown_diff_right_rows)
            );
            (
                mk_column(
                    "diff_markdown_preview_left",
                    "diff_markdown_preview_left_scrollbar",
                    "diff_markdown_preview_left_hscrollbar",
                    left_list,
                    self.diff_scroll.clone(),
                    left_handle.clone(),
                ),
                mk_column(
                    "diff_markdown_preview_right",
                    "diff_markdown_preview_right_scrollbar",
                    "diff_markdown_preview_right_hscrollbar",
                    right_list,
                    self.diff_split_right_scroll.clone(),
                    right_handle.clone(),
                ),
                vertical_scroll_handle,
            )
        };

        div()
            .id("diff_markdown_preview_container")
            .relative()
            .h_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .child(
                div()
                    .pr(if vertical_sync_enabled {
                        components::Scrollbar::visible_gutter(
                            vertical_scroll_handle.clone(),
                            components::ScrollbarAxis::Vertical,
                        )
                    } else {
                        px(0.0)
                    })
                    .flex()
                    .flex_col()
                    .h_full()
                    .min_h(px(0.0))
                    .child(components::split_columns_header(
                        theme,
                        ui_scale_percent,
                        "A (before)",
                        "B (after)",
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex()
                            .child(left_column)
                            .child(div().w(px(1.0)).h_full().bg(theme.colors.stroke.default))
                            .child(right_column),
                    ),
            )
            .when(vertical_sync_enabled, |d| {
                d.child(
                    components::Scrollbar::new(
                        "diff_markdown_preview_scrollbar",
                        vertical_scroll_handle,
                    )
                    .markers(scrollbar_markers)
                    .always_visible()
                    .render(theme),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_diff_ready_state_prefers_processing_when_cache_is_stale() {
        assert!(file_diff_ready_shows_processing(true, false, false, false));
        assert!(file_diff_ready_shows_processing(true, true, true, false));
        assert!(!file_diff_ready_shows_processing(true, true, false, false));
        assert!(!file_diff_ready_shows_processing(false, false, true, false));
        // Rows from the previous build are shown instead of a placeholder while
        // the same file is rebuilt.
        assert!(!file_diff_ready_shows_processing(true, true, true, true));
        assert!(!file_diff_ready_shows_processing(true, false, false, true));
    }

    #[test]
    fn image_diff_ready_state_prefers_processing_when_cache_is_stale() {
        assert!(image_diff_ready_shows_processing(true, false));
        assert!(!image_diff_ready_shows_processing(true, true));
        assert!(!image_diff_ready_shows_processing(false, false));
    }
}
