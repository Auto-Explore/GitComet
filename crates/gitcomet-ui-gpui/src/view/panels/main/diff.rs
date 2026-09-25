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
                                                .text_size(theme.ui_text(14.0))
                                                .text_color(muted)
                                                .child("Processing image...")
                                                .into_any_element()
                                        })
                                        .with_fallback(move || {
                                            div()
                                                .text_size(theme.ui_text(14.0))
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
                                        .text_size(theme.ui_text(14.0))
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
                        match &self.diff_markdown.preview {
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
                                self.render_markdown_diff_preview(theme, preview, window, cx)
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
                                                        crate::text_selection_owner::preserve(cx);
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
                                        .h(components::control_height(
                                            ui_scale::UiScale::from_percent(ui_scale_percent)
                                                .with_appearance(theme.metrics),
                                        ))
                                        .flex()
                                        .items_center()
                                        .text_size(theme.ui_text(12.0))
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

    /// Scrollbar markers where last frame drew the changes. The first frame of
    /// a preview has nothing measured yet: it places them by row count and asks
    /// for one more frame, which measures them.
    pub(in crate::view) fn markdown_diff_scrollbar_markers(
        &mut self,
        preview: &crate::view::markdown_preview::MarkdownPreviewDiff,
        scroll: &gpui::ScrollHandle,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<components::ScrollbarMarker> {
        // Measurements are only good for the preview and layout they were taken in.
        let key = (self.diff_markdown.seq, self.diff_view);
        let measured = self.diff_markdown.change_extents.take();
        let measured_key = self.diff_markdown.change_extents_key.replace(key);
        let content_height =
            f32::from(scroll.bounds().size.height + scroll.max_offset().y.max(px(0.0)));
        if measured_key == Some(key) && !measured.is_empty() && content_height > 0.0 {
            return crate::view::markdown_preview::scrollbar_markers_for_extents(
                &measured,
                content_height,
            );
        }
        if self.diff_markdown.change_extents_requested != Some(key) {
            self.diff_markdown.change_extents_requested = Some(key);
            let view = cx.entity();
            window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
        }
        match self.diff_view {
            DiffViewMode::Inline => {
                crate::view::markdown_preview::scrollbar_markers_for_document(&preview.inline)
            }
            DiffViewMode::Split => {
                crate::view::markdown_preview::scrollbar_markers_for_diff_preview(preview)
            }
        }
    }

    fn render_markdown_diff_preview(
        &mut self,
        theme: AppTheme,
        preview: std::sync::Arc<crate::view::markdown_preview::MarkdownPreviewDiff>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        if preview.old_blocks.is_empty() && preview.new_blocks.is_empty() {
            return empty_diff_text_document(
                cx.entity(),
                DiffTextRegion::Inline,
                components::empty_state(theme, "Preview", preview.empty_notice())
                    .into_any_element(),
            );
        }

        self.maybe_autoscroll_diff_to_first_change();

        let scroll_handle = self.diff_scroll.0.borrow().base_handle.clone();
        let scrollbar_markers =
            self.markdown_diff_scrollbar_markers(&preview, &scroll_handle, window, cx);
        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let image_root = self.markdown_preview_image_root();
        // Built once for both sides of a split: each matcher compiles a regex.
        let query = self.markdown_preview_search_query();
        // One document listener serves both sides of a split.
        let row_boxes = rows::MarkdownRowBoxes::default();
        let context =
            |this: &Self, region: DiffTextRegion, side: usize| rows::MarkdownDocumentContext {
                theme,
                ui_scale_percent,
                editor_font_family: editor_font_family.clone(),
                image_root: image_root.clone(),
                remote_image_access: this.markdown_remote_image_access(Some(cx.entity())),
                picture_sizes: Default::default(),
                drawn_pictures: None,
                row_boxes: row_boxes.clone(),
                block_scrolls: this.diff_markdown.block_scrolls[side].clone(),
                blocks: Default::default(),
                view: Some(cx.entity()),
                text_region: region,
                change_bar_color: None,
                query: query.clone(),
                reveal: this.markdown_interaction.reveal.clone(),
                scroll: Some(scroll_handle.clone()),
                hovered_link: this.markdown_interaction.hovered_link.clone(),
                change_extents: Some(this.diff_markdown.change_extents.clone()),
                layout: this.diff_markdown.layouts[side].clone(),
                // Only the split's new side is parsed from the working-tree file.
                tasks_editable: region == DiffTextRegion::SplitRight
                    && this.markdown_preview_tasks_editable(),
            };
        let body = match self.diff_view {
            DiffViewMode::Inline => rows::render_markdown_document_with_blocks(
                &preview.inline,
                &preview.inline_blocks,
                &context(self, DiffTextRegion::Inline, 2),
            ),
            DiffViewMode::Split => rows::render_markdown_diff_split(
                &preview,
                &context(self, DiffTextRegion::SplitLeft, 0),
                &context(self, DiffTextRegion::SplitRight, 1),
            ),
        };

        let edge_gap = crate::ui_scale::design_px_from_percent(
            MARKDOWN_PREVIEW_DOCUMENT_EDGE_GAP_PX,
            ui_scale_percent,
        );
        let scrollbar_gutter = components::Scrollbar::visible_gutter(
            scroll_handle.clone(),
            components::ScrollbarAxis::Vertical,
        );
        div()
            .id("diff_markdown_preview_container")
            .debug_selector(|| "diff_markdown_preview_container".to_string())
            .relative()
            .h_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .when(self.diff_view == DiffViewMode::Split, |container| {
                container.child(
                    div()
                        .pr(scrollbar_gutter)
                        .child(components::split_columns_header(
                            theme,
                            ui_scale_percent,
                            "A (before)",
                            "B (after)",
                        )),
                )
            })
            .child(
                div()
                    .id("diff_markdown_preview_scroll_area")
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .id("diff_markdown_preview_document")
                            .debug_selector(|| "diff_markdown_preview_document".to_string())
                            .size_full()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .track_scroll(&scroll_handle)
                            .pt(edge_gap)
                            .pb(edge_gap)
                            .pr(scrollbar_gutter)
                            .child(body),
                    )
                    .child(
                        components::Scrollbar::new(
                            "diff_markdown_preview_scrollbar",
                            scroll_handle,
                        )
                        .markers(scrollbar_markers)
                        .always_visible()
                        .render(theme),
                    ),
            )
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
