use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;

pub(super) struct TextElement {
    pub(super) input: Entity<TextInput>,
}

pub(super) struct PrepaintState {
    layout: Option<TextInputLayout>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    line_starts: Option<Arc<[usize]>>,
    wrap_cache: Option<WrapCache>,
    scroll_x: Pixels,
    visible_line_range: Range<usize>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_height = input.effective_line_height(window);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if input.multiline {
            let line_count = input.content.line_starts().len().max(1) as f32;
            if input.soft_wrap
                && let Some(cache) = input.wrap.cache
                && cache.rows > 0
                && cache.width > px(0.0)
            {
                style.size.height = (line_height * cache.rows as f32).into();
            } else if input.soft_wrap
                && let Some(rows) = input.wrap.last_rows
                && rows > 0
            {
                // Preserve the previous wrapped row count until the next wrap pass finishes.
                style.size.height = (line_height * rows as f32).into();
            } else {
                style.size.height = (line_height * line_count).into();
            }
        } else {
            style.size.height = line_height.into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.input.update(cx, |input, cx| {
            let content = input.content.snapshot();
            let selected_range = input.selection.range.clone();
            let cursor = input.cursor_offset();
            let style_colors = input.style;
            let soft_wrap = input.soft_wrap && input.multiline;
            let style = window.text_style();
            let has_content = !content.is_empty();

            let (display_text, text_color) = if content.is_empty() {
                (input.placeholder.clone(), style_colors.placeholder)
            } else if input.masked {
                (
                    mask_text_for_display(content.as_ref()).into(),
                    style_colors.text,
                )
            } else {
                (content.as_shared_string(), style_colors.text)
            };

            let font_size = style.font_size.to_pixels(window.rem_size());
            let line_height = input.effective_line_height(window);
            let base_font = style.font();

            let display_text_str = display_text.as_ref();
            let line_starts: Arc<[usize]> = if has_content && !input.masked {
                content.shared_line_starts()
            } else {
                compute_line_starts(display_text_str).into()
            };
            let line_count = line_starts.len().max(1);
            let (visible_top, visible_bottom) =
                visible_vertical_window(bounds, input.interaction.vertical_scroll_handle.as_ref());

            // Resolve highlights: use the provider path for large documents,
            // otherwise use the pre-materialized highlight vector.
            let highlights = if !has_content {
                None
            } else if input.highlight.provider.is_some() {
                let byte_range = provider_prefetch_byte_range_for_visible_window(
                    line_starts.as_ref(),
                    display_text_str.len(),
                    line_count,
                    line_height,
                    visible_top,
                    visible_bottom,
                );
                let resolved = input.resolve_provider_highlights(byte_range.start, byte_range.end);
                if resolved.pending {
                    input.ensure_highlight_provider_poll(cx);
                }
                Some(resolved.highlights)
            } else {
                Some(Arc::clone(&input.highlight.highlights))
            };
            let highlight_slice = highlights.as_ref().map(|h| h.as_slice());
            let shape_style = TextShapeStyle {
                base_font: &base_font,
                text_color,
                highlights: highlight_slice,
                font_size,
            };

            if !soft_wrap {
                if !input.multiline
                    && input.read_only
                    && input.display_truncation.is_some()
                    && has_content
                    && !input.masked
                {
                    let mut base_text_style = style.clone();
                    base_text_style.color = text_color;
                    let truncated_line = shape_truncated_line_cached(
                        window,
                        cx,
                        &base_text_style,
                        &display_text,
                        Some(bounds.size.width.max(px(0.0))),
                        input
                            .display_truncation
                            .unwrap_or(TextTruncationProfile::End),
                        highlight_slice.unwrap_or(&[]),
                        None,
                    );
                    let mut selections = Vec::with_capacity(4);
                    let cursor_quad = if selected_range.is_empty() {
                        let control_height =
                            crate::ui_scale::design_px_from_window(CONTROL_HEIGHT_PX, window);
                        let x = truncated_line_x_for_source_offset(&truncated_line, cursor);
                        let caret_inset_y = px(3.0);
                        let caret_h = if !input.chromeless {
                            (control_height - px(2.0) - caret_inset_y * 2.0).max(px(2.0))
                        } else {
                            (line_height - caret_inset_y * 2.0).max(px(2.0))
                        };
                        let caret_top_inset = (line_height - caret_h) / 2.0;
                        let top = bounds.top() + caret_top_inset;
                        Some(fill(
                            Bounds::new(point(bounds.left() + x, top), size(px(1.0), caret_h)),
                            style_colors.cursor,
                        ))
                    } else {
                        for range in truncated_line
                            .projection
                            .selection_display_ranges(selected_range.clone())
                        {
                            let x0 = truncated_line.shaped_line.x_for_index(range.start);
                            let x1 = truncated_line.shaped_line.x_for_index(range.end);
                            if x1 <= x0 {
                                continue;
                            }
                            selections.push(fill(
                                Bounds::from_corners(
                                    point(bounds.left() + x0, bounds.top()),
                                    point(bounds.left() + x1, bounds.top() + line_height),
                                ),
                                style_colors.selection,
                            ));
                        }
                        None
                    };

                    return PrepaintState {
                        layout: Some(TextInputLayout::TruncatedSingleLine(truncated_line)),
                        cursor: cursor_quad,
                        selections,
                        line_starts: Some(line_starts),
                        wrap_cache: None,
                        scroll_x: px(0.0),
                        visible_line_range: 0..1,
                    };
                }

                let mut scroll_x = if input.multiline {
                    px(0.0)
                } else {
                    input.layout.scroll_x
                };
                let mut lines = vec![ShapedLine::default(); line_count];
                let mut visible_line_range = if input.multiline {
                    visible_plain_line_range(
                        line_count,
                        line_height,
                        visible_top,
                        visible_bottom,
                        TEXT_INPUT_GUARD_ROWS,
                    )
                } else {
                    0..line_count
                };
                if visible_line_range.is_empty() {
                    visible_line_range = 0..line_count.min(1);
                }
                let streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                    display_text_str,
                    line_starts.as_ref(),
                    visible_line_range.clone(),
                    &shape_style,
                );

                for line_ix in visible_line_range.clone() {
                    let precomputed_runs = visible_window_runs_for_line_ix(
                        streamed_line_runs.as_deref(),
                        visible_line_range.start,
                        line_ix,
                    );
                    let shaped = input.shape_plain_line_cached(
                        LineShapeInput {
                            line_ix,
                            line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                line_ix,
                            ),
                        },
                        precomputed_runs,
                        &shape_style,
                        window,
                    );
                    if let Some(slot) = lines.get_mut(line_ix) {
                        *slot = shaped;
                    }
                }

                let cursor_line_ix =
                    line_index_for_offset(line_starts.as_ref(), cursor, line_count);
                if cursor_line_ix < line_count
                    && (cursor_line_ix < visible_line_range.start
                        || cursor_line_ix >= visible_line_range.end)
                {
                    let shaped = input.shape_plain_line_cached(
                        LineShapeInput {
                            line_ix: cursor_line_ix,
                            line_start: line_starts.get(cursor_line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                cursor_line_ix,
                            ),
                        },
                        None,
                        &shape_style,
                        window,
                    );
                    if let Some(slot) = lines.get_mut(cursor_line_ix) {
                        *slot = shaped;
                    }
                }

                if !input.multiline && !lines.is_empty() {
                    let viewport_w = bounds.size.width.max(px(0.0));
                    let pad = px(8.0).min(viewport_w / 4.0);
                    let (line_ix, local_ix) = line_for_offset(line_starts.as_ref(), &lines, cursor);
                    let cursor_x = lines[line_ix].x_for_index(local_ix);
                    let max_scroll_x = (lines[line_ix].width - viewport_w).max(px(0.0));

                    let left = scroll_x;
                    let right = scroll_x + viewport_w;
                    if cursor_x < left + pad {
                        scroll_x = (cursor_x - pad).max(px(0.0));
                    } else if cursor_x > right - pad {
                        scroll_x = (cursor_x + pad - viewport_w).max(px(0.0));
                    }
                    scroll_x = scroll_x.min(max_scroll_x);
                } else {
                    scroll_x = px(0.0);
                }

                let mut selections = Vec::with_capacity(visible_line_range.len().max(1));
                let cursor_quad = if selected_range.is_empty() {
                    let control_height =
                        crate::ui_scale::design_px_from_window(CONTROL_HEIGHT_PX, window);
                    let (line_ix, local_ix) = line_for_offset(line_starts.as_ref(), &lines, cursor);
                    let x = lines[line_ix].x_for_index(local_ix) - scroll_x;
                    let caret_inset_y = px(3.0);
                    let caret_h = if !input.multiline && !input.chromeless {
                        // Cap caret to fit within the fixed-height container
                        // (CONTROL_HEIGHT_PX minus 2px border minus insets).
                        (control_height - px(2.0) - caret_inset_y * 2.0).max(px(2.0))
                    } else {
                        (line_height - caret_inset_y * 2.0).max(px(2.0))
                    };
                    let caret_top_inset = (line_height - caret_h) / 2.0;
                    let top = bounds.top() + line_height * line_ix as f32 + caret_top_inset;
                    Some(fill(
                        Bounds::new(point(bounds.left() + x, top), size(px(1.0), caret_h)),
                        style_colors.cursor,
                    ))
                } else {
                    for ix in visible_line_range.clone() {
                        let start = line_starts.get(ix).copied().unwrap_or(0);
                        let next_start = line_starts
                            .get(ix + 1)
                            .copied()
                            .unwrap_or(display_text.len());
                        let line_len = lines[ix].len();
                        let line_end = start + line_len;

                        let seg_start = selected_range.start.max(start);
                        let seg_end = selected_range.end.min(next_start);
                        if seg_start >= seg_end {
                            continue;
                        }

                        let local_start = seg_start.min(line_end) - start;
                        let local_end = seg_end.min(line_end) - start;

                        let x0 = lines[ix].x_for_index(local_start) - scroll_x;
                        let x1 = lines[ix].x_for_index(local_end) - scroll_x;
                        let top = bounds.top() + line_height * ix as f32;
                        selections.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x0, top),
                                point(bounds.left() + x1, top + line_height),
                            ),
                            style_colors.selection,
                        ));
                    }
                    None
                };

                return PrepaintState {
                    layout: Some(TextInputLayout::Plain(lines)),
                    cursor: cursor_quad,
                    selections,
                    line_starts: Some(line_starts),
                    wrap_cache: None,
                    scroll_x,
                    visible_line_range,
                };
            }

            let wrap_width = bounds.size.width.max(px(0.0));
            let rounded_wrap_width = wrap_width.round();
            let wrap_width_key = wrap_width_cache_key(rounded_wrap_width);
            if input.wrap.row_counts.len() != line_count {
                input.wrap.row_counts.resize(line_count, 1);
                input.request_wrap_recompute();
            }
            if input.wrap.row_counts_width != Some(rounded_wrap_width) {
                input.wrap.row_counts_width = Some(rounded_wrap_width);
                input.request_wrap_recompute();
            }
            for rows in &mut input.wrap.row_counts {
                *rows = (*rows).max(1);
            }
            let started_wrap_job = input.maybe_recompute_wrap_rows(
                display_text_str,
                line_starts.as_ref(),
                rounded_wrap_width,
                font_size,
                line_count,
                cx,
            );

            let mut row_counts_changed = input.apply_pending_dirty_wrap_updates(
                display_text_str,
                line_starts.as_ref(),
                rounded_wrap_width,
                font_size,
                !started_wrap_job,
            );

            let mut y_offsets = vec![Pixels::ZERO; line_count];
            let mut y = Pixels::ZERO;
            for (ix, rows) in input.wrap.row_counts.iter().enumerate() {
                y_offsets[ix] = y;
                y += line_height * (*rows as f32).max(1.0);
            }

            let mut visible_line_range = visible_wrapped_line_range(
                &y_offsets,
                input.wrap.row_counts.as_slice(),
                line_height,
                visible_top,
                visible_bottom,
                TEXT_INPUT_GUARD_ROWS,
            );
            let mut lines = (0..line_count)
                .map(|_| WrappedLine::default())
                .collect::<Vec<_>>();
            let mut shaped_mask = vec![false; line_count];
            let job_accepts_interpolation = pending_wrap_job_accepts_interpolated_patch(
                input.wrap.pending_job.as_ref(),
                wrap_width_key,
                line_count,
                !started_wrap_job,
            );
            let mut streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                display_text_str,
                line_starts.as_ref(),
                visible_line_range.clone(),
                &shape_style,
            );

            for line_ix in visible_line_range.clone() {
                let precomputed_runs = visible_window_runs_for_line_ix(
                    streamed_line_runs.as_deref(),
                    visible_line_range.start,
                    line_ix,
                );
                let wrapped = input.shape_wrapped_line_cached(
                    LineShapeInput {
                        line_ix,
                        line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                        line_text: line_text_for_index(
                            display_text_str,
                            line_starts.as_ref(),
                            line_ix,
                        ),
                    },
                    wrap_width,
                    precomputed_runs,
                    &shape_style,
                    window,
                );
                let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                let old_rows = input
                    .wrap
                    .row_counts
                    .get(line_ix)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                if old_rows != rows {
                    if let Some(slot) = input.wrap.row_counts.get_mut(line_ix) {
                        *slot = rows;
                    }
                    row_counts_changed = true;
                    if job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(wrap_width_key, line_ix, old_rows, rows);
                    }
                }
                if let Some(slot) = lines.get_mut(line_ix) {
                    *slot = wrapped;
                }
                if let Some(mask) = shaped_mask.get_mut(line_ix) {
                    *mask = true;
                }
            }

            let cursor_line_ix = line_index_for_offset(line_starts.as_ref(), cursor, line_count);
            if cursor_line_ix < line_count
                && (cursor_line_ix < visible_line_range.start
                    || cursor_line_ix >= visible_line_range.end)
            {
                let wrapped = input.shape_wrapped_line_cached(
                    LineShapeInput {
                        line_ix: cursor_line_ix,
                        line_start: line_starts.get(cursor_line_ix).copied().unwrap_or(0),
                        line_text: line_text_for_index(
                            display_text_str,
                            line_starts.as_ref(),
                            cursor_line_ix,
                        ),
                    },
                    wrap_width,
                    None,
                    &shape_style,
                    window,
                );
                let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                let old_rows = input
                    .wrap
                    .row_counts
                    .get(cursor_line_ix)
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                if old_rows != rows {
                    if let Some(slot) = input.wrap.row_counts.get_mut(cursor_line_ix) {
                        *slot = rows;
                    }
                    row_counts_changed = true;
                    if job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(
                            wrap_width_key,
                            cursor_line_ix,
                            old_rows,
                            rows,
                        );
                    }
                }
                if let Some(slot) = lines.get_mut(cursor_line_ix) {
                    *slot = wrapped;
                }
                if let Some(mask) = shaped_mask.get_mut(cursor_line_ix) {
                    *mask = true;
                }
            }

            if row_counts_changed {
                y = Pixels::ZERO;
                for (ix, rows) in input.wrap.row_counts.iter().enumerate() {
                    y_offsets[ix] = y;
                    y += line_height * (*rows as f32).max(1.0);
                }
                visible_line_range = visible_wrapped_line_range(
                    &y_offsets,
                    input.wrap.row_counts.as_slice(),
                    line_height,
                    visible_top,
                    visible_bottom,
                    TEXT_INPUT_GUARD_ROWS,
                );
                streamed_line_runs = input.streamed_highlight_runs_for_visible_window(
                    display_text_str,
                    line_starts.as_ref(),
                    visible_line_range.clone(),
                    &shape_style,
                );
                for line_ix in visible_line_range.clone() {
                    if shaped_mask.get(line_ix).copied().unwrap_or(false) {
                        continue;
                    }
                    let precomputed_runs = visible_window_runs_for_line_ix(
                        streamed_line_runs.as_deref(),
                        visible_line_range.start,
                        line_ix,
                    );
                    let wrapped = input.shape_wrapped_line_cached(
                        LineShapeInput {
                            line_ix,
                            line_start: line_starts.get(line_ix).copied().unwrap_or(0),
                            line_text: line_text_for_index(
                                display_text_str,
                                line_starts.as_ref(),
                                line_ix,
                            ),
                        },
                        wrap_width,
                        precomputed_runs,
                        &shape_style,
                        window,
                    );
                    let rows = wrapped.wrap_boundaries().len().saturating_add(1).max(1);
                    let old_rows = input
                        .wrap
                        .row_counts
                        .get(line_ix)
                        .copied()
                        .unwrap_or(1)
                        .max(1);
                    if let Some(slot) = input.wrap.row_counts.get_mut(line_ix) {
                        *slot = rows;
                    }
                    if old_rows != rows && job_accepts_interpolation {
                        input.push_interpolated_wrap_patch(wrap_width_key, line_ix, old_rows, rows);
                    }
                    if let Some(slot) = lines.get_mut(line_ix) {
                        *slot = wrapped;
                    }
                    if let Some(mask) = shaped_mask.get_mut(line_ix) {
                        *mask = true;
                    }
                }
            }

            let total_rows = total_wrap_rows(input.wrap.row_counts.as_slice());
            let wrap_cache = Some(WrapCache {
                width: rounded_wrap_width,
                rows: total_rows,
            });

            let mut selections = Vec::with_capacity(visible_line_range.len().max(1));
            let cursor_quad = if selected_range.is_empty() {
                let line_ix = line_index_for_offset(line_starts.as_ref(), cursor, line_count);
                let start = line_starts.get(line_ix).copied().unwrap_or(0);
                let local = cursor.saturating_sub(start).min(lines[line_ix].len());
                let caret_inset_y = px(3.0);
                let caret_h = (line_height - caret_inset_y * 2.0).max(px(2.0));
                let pos = lines[line_ix]
                    .position_for_index(local, line_height)
                    .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                let top = bounds.top() + y_offsets[line_ix] + pos.y + caret_inset_y;
                Some(fill(
                    Bounds::new(point(bounds.left() + pos.x, top), size(px(1.0), caret_h)),
                    style_colors.cursor,
                ))
            } else {
                for ix in visible_line_range.clone() {
                    let start = line_starts.get(ix).copied().unwrap_or(0);
                    let next_start = line_starts
                        .get(ix + 1)
                        .copied()
                        .unwrap_or(display_text.len());
                    let line_len = lines[ix].len();
                    let line_end = start + line_len;

                    let seg_start = selected_range.start.max(start);
                    let seg_end = selected_range.end.min(next_start);
                    if seg_start >= seg_end {
                        continue;
                    }

                    let local_start = seg_start.min(line_end) - start;
                    let local_end = seg_end.min(line_end) - start;

                    let start_pos = lines[ix]
                        .position_for_index(local_start, line_height)
                        .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                    let end_pos = lines[ix]
                        .position_for_index(local_end, line_height)
                        .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));

                    let start_row = (start_pos.y / line_height).floor().max(0.0) as usize;
                    let end_row = (end_pos.y / line_height).floor().max(0.0) as usize;

                    for row in start_row..=end_row {
                        let top = bounds.top() + y_offsets[ix] + line_height * row as f32;
                        let (x0, x1) = if start_row == end_row {
                            (start_pos.x, end_pos.x)
                        } else if row == start_row {
                            (start_pos.x, bounds.size.width)
                        } else if row == end_row {
                            (Pixels::ZERO, end_pos.x)
                        } else {
                            (Pixels::ZERO, bounds.size.width)
                        };
                        selections.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x0, top),
                                point(bounds.left() + x1, top + line_height),
                            ),
                            style_colors.selection,
                        ));
                    }
                }
                None
            };

            PrepaintState {
                layout: Some(TextInputLayout::Wrapped {
                    lines,
                    y_offsets,
                    row_counts: input.wrap.row_counts.clone(),
                }),
                cursor: cursor_quad,
                selections,
                line_starts: Some(line_starts),
                wrap_cache,
                scroll_x: px(0.0),
                visible_line_range,
            }
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        if self.input.read(cx).interaction.is_selecting {
            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
                input.update(cx, |input, cx| {
                    if input.interaction.is_selecting {
                        input.select_to(input.index_for_mouse_position(event.position), cx);
                    }
                });
            });

            let input = self.input.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, cx| {
                if event.button != MouseButton::Left {
                    return;
                }
                input.update(cx, |input, _cx| {
                    input.interaction.is_selecting = false;
                });
            });
        }

        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        let line_height = self.input.read(cx).effective_line_height(window);
        if let Some(layout) = prepaint.layout.as_ref() {
            match layout {
                TextInputLayout::Plain(lines) => {
                    for ix in prepaint.visible_line_range.clone() {
                        let Some(line) = lines.get(ix) else {
                            continue;
                        };
                        let painted = line.paint(
                            point(
                                bounds.origin.x - prepaint.scroll_x,
                                bounds.origin.y + line_height * ix as f32,
                            ),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        debug_assert!(
                            painted.is_ok(),
                            "TextInput plain line paint failed at line index {ix}"
                        );
                    }
                }
                TextInputLayout::TruncatedSingleLine(line) => {
                    if line.has_background_runs {
                        let _ = line.shaped_line.paint_background(
                            point(bounds.origin.x, bounds.origin.y),
                            line.line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                    let _ = line.shaped_line.paint(
                        point(bounds.origin.x, bounds.origin.y),
                        line.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                TextInputLayout::Wrapped {
                    lines, y_offsets, ..
                } => {
                    for ix in prepaint.visible_line_range.clone() {
                        let Some(line) = lines.get(ix) else {
                            continue;
                        };
                        let y = y_offsets.get(ix).copied().unwrap_or(Pixels::ZERO);
                        let _ = line.paint(
                            point(bounds.origin.x, bounds.origin.y + y),
                            line_height,
                            TextAlign::Left,
                            Some(bounds),
                            window,
                            cx,
                        );
                    }
                }
            }
        }

        let cursor_blink_visible = self.input.read(cx).interaction.cursor_blink_visible;
        if focus_handle.is_focused(window)
            && cursor_blink_visible
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, cx| {
            let prev_height_rows = if input.multiline && input.soft_wrap {
                input
                    .wrap
                    .cache
                    .map(|cache| cache.rows)
                    .or(input.wrap.last_rows)
            } else {
                None
            };
            let had_pending_cursor_autoscroll = input.interaction.pending_cursor_autoscroll;
            input.layout.last = prepaint.layout.take();
            input.layout.line_starts = prepaint.line_starts.clone();
            input.layout.bounds = Some(bounds);
            input.layout.line_height = line_height;
            input.wrap.cache = prepaint.wrap_cache;
            if input.multiline && input.soft_wrap {
                if let Some(cache) = input.wrap.cache {
                    input.wrap.last_rows = Some(cache.rows);
                }
            } else {
                input.wrap.last_rows = None;
            }
            input.layout.scroll_x = prepaint.scroll_x;
            if had_pending_cursor_autoscroll {
                input.ensure_cursor_visible_in_vertical_scroll(cx);
            }
            let next_height_rows = if input.multiline && input.soft_wrap {
                input
                    .wrap
                    .cache
                    .map(|cache| cache.rows)
                    .or(input.wrap.last_rows)
            } else {
                None
            };
            if prev_height_rows != next_height_rows {
                // Wrapped height changes land one frame later in the parent scroll container.
                // Keep one follow-up pass so Enter-at-EOF remains pinned to the true bottom.
                if had_pending_cursor_autoscroll && input.cursor_offset() == input.content.len() {
                    input.interaction.pending_cursor_autoscroll = true;
                }
                cx.notify();
            }
        });
    }
}
