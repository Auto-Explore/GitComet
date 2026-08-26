use super::*;

pub(in crate::view) const MARKDOWN_PREVIEW_ROW_HEIGHT_PX: f32 = 28.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BASE_FONT_PX: f32 = 13.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BASE_LINE_HEIGHT_PX: f32 = 20.0;
pub(in crate::view) const MARKDOWN_PREVIEW_CONTENT_PAD_X_PX: f32 = 18.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX: f32 = 8.0;
pub(in crate::view) const MARKDOWN_PREVIEW_INDENT_STEP_PX: f32 = 24.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX: f32 = 4.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_GAP_PX: f32 = 8.0;
pub(in crate::view) const MARKDOWN_PREVIEW_BLOCKQUOTE_GUTTER_MARGIN_RIGHT_PX: f32 = 12.0;
pub(in crate::view) const MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX: f32 = 22.0;
pub(in crate::view) const MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX: f32 = 10.0;
pub(in crate::view) const MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX: f32 = 11.0;
pub(in crate::view) const MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX: f32 = 6.0;
pub(in crate::view) const MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX: f32 = 10.0;
pub(in crate::view) const MARKDOWN_PREVIEW_SHELL_PAD_X_PX: f32 = 12.0;
pub(in crate::view) const MARKDOWN_PREVIEW_CODE_BORDER_PX: f32 = 1.0;

pub(in crate::view) fn markdown_preview_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

pub(in crate::view) fn markdown_preview_scaled_value(value: f32, ui_scale_percent: u32) -> f32 {
    let scaled: f32 = markdown_preview_scaled_px(value, ui_scale_percent).into();
    scaled
}

pub(in crate::view) fn markdown_preview_row_height(ui_scale_percent: u32) -> Pixels {
    markdown_preview_scaled_px(MARKDOWN_PREVIEW_ROW_HEIGHT_PX, ui_scale_percent)
}

pub(in crate::view) struct MarkdownPreviewRowTypography {
    pub(in crate::view) font_size: f32,
    pub(in crate::view) line_height: f32,
    pub(in crate::view) font_weight: Option<FontWeight>,
    pub(in crate::view) font_family: Option<SharedString>,
    pub(in crate::view) text_color: gpui::Rgba,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::view) struct MarkdownPreviewRowLayout {
    pub(in crate::view) top_inset_px: f32,
    pub(in crate::view) bottom_inset_px: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::view) struct MarkdownPreviewRowHorizontalPadding {
    pub(in crate::view) left_px: f32,
    pub(in crate::view) right_px: f32,
}

/// Inputs shared by every row of one wrap pass.
pub(in crate::view) struct MarkdownPreviewWrapMeasure {
    pub(in crate::view) key: MarkdownPreviewWrapKey,
    pub(in crate::view) wrap_width: Pixels,
    pub(in crate::view) editor_font_family: SharedString,
    pub(in crate::view) ui_scale_percent: u32,
}

impl MarkdownPreviewWrapMeasure {
    /// Per-row wrap callback for the plan builders.
    pub(in crate::view) fn wrap_row_fn<'a>(
        &'a self,
        window: &'a mut Window,
        theme: AppTheme,
    ) -> impl FnMut(&MarkdownPreviewRow) -> Vec<Range<usize>> + 'a {
        move |row| {
            markdown_preview_row_wrap_ranges(
                window,
                theme,
                row,
                self.wrap_width,
                &self.editor_font_family,
                self.ui_scale_percent,
            )
        }
    }
}

pub(in crate::view) struct MarkdownPreviewRenderContext<'a> {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) min_width: Pixels,
    pub(in crate::view) editor_font_family: SharedString,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) view: Option<Entity<MainPaneView>>,
    pub(in crate::view) text_region: DiffTextRegion,
    /// Visual-row mapping when word wrap is on; `None` renders one row per
    /// source row with horizontal overflow clipped.
    pub(in crate::view) wrap_plan: Option<&'a MarkdownPreviewWrapPlan>,
    /// Directory relative image paths resolve against.
    pub(in crate::view) image_base_dir: Option<Arc<std::path::Path>>,
    /// Quick-search state, when the search box is open over this preview.
    pub(in crate::view) query: Option<MarkdownPreviewQuery>,
}

pub(in crate::view) fn render_markdown_preview_document_rows(
    document: &MarkdownPreviewDocument,
    range: Range<usize>,
    context: &MarkdownPreviewRenderContext<'_>,
) -> Vec<AnyElement> {
    let requested_rows = range.len();
    let mut rows = Vec::with_capacity(requested_rows);
    if let Some(plan) = context.wrap_plan {
        let start = range.start.min(plan.len());
        let end = range.end.min(plan.len());
        for visual_ix in start..end {
            let Some(visual_row) = plan.get(visual_ix) else {
                continue;
            };
            let Some(row) = document.rows.get(visual_row.row_ix) else {
                continue;
            };
            rows.push(markdown_preview_row_element(
                row,
                visual_ix,
                Some(visual_row),
                context,
            ));
        }
    } else {
        let start = range.start.min(document.rows.len());
        let end = range.end.min(document.rows.len());
        for (offset, row) in document.rows[start..end].iter().enumerate() {
            rows.push(markdown_preview_row_element(
                row,
                start + offset,
                None,
                context,
            ));
        }
    }
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        requested_rows,
        rows.len(),
    );
    rows
}

pub(in crate::view) struct MarkdownPreviewSharedHighlightsText {
    pub(in crate::view) text: SharedString,
    pub(in crate::view) highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    pub(in crate::view) inner: Option<gpui::StyledText>,
}

impl MarkdownPreviewSharedHighlightsText {
    pub(in crate::view) fn new(
        text: SharedString,
        highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    ) -> Self {
        Self {
            text,
            highlights,
            inner: None,
        }
    }
}

impl gpui::Element for MarkdownPreviewSharedHighlightsText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut inner = gpui::StyledText::new(self.text.clone())
            .with_default_highlights(&window.text_style(), self.highlights.iter().cloned());
        let layout = inner.request_layout(id, inspector_id, window, cx);
        self.inner = Some(inner);
        layout
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .as_mut()
            .expect("markdown preview shared-highlights text should be laid out before prepaint")
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .as_mut()
            .expect("markdown preview shared-highlights text should be laid out before paint")
            .paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );
    }
}

impl gpui::IntoElement for MarkdownPreviewSharedHighlightsText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub(in crate::view) fn markdown_preview_row_element(
    row: &MarkdownPreviewRow,
    row_ix: usize,
    visual_row: Option<&MarkdownPreviewVisualRow>,
    context: &MarkdownPreviewRenderContext<'_>,
) -> AnyElement {
    let theme = context.theme;
    let min_width = context.min_width;
    let text_region = context.text_region;
    let ui_scale_percent = context.ui_scale_percent;
    let is_interactive = context.view.is_some();
    let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewStyledRowBuild);
    if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
        return div()
            .relative()
            .h(markdown_preview_row_height(ui_scale_percent))
            .min_h(markdown_preview_row_height(ui_scale_percent))
            .w(min_width)
            .min_w(min_width)
            .into_any_element();
    }

    if let MarkdownPreviewRowKind::Image {
        slice_ix,
        slice_count,
    } = row.kind
    {
        // Image bands carry none of the text machinery — no marker, no
        // selection overlay, no styled runs — so they short-circuit here.
        let padding = markdown_preview_row_horizontal_padding(row, ui_scale_percent);
        return div()
            .relative()
            .h(markdown_preview_row_height(ui_scale_percent))
            .min_h(markdown_preview_row_height(ui_scale_percent))
            .w(min_width)
            .min_w(min_width)
            .flex()
            .items_center()
            .when_some(markdown_preview_row_background(theme, row), |div, bg| {
                div.bg(bg)
            })
            .child(
                div()
                    .flex_grow(1.)
                    .min_w(px(0.0))
                    .w_full()
                    .h_full()
                    .pl(px(padding.left_px))
                    .pr(px(padding.right_px))
                    .child(markdown_preview_image_row(
                        row,
                        row_ix,
                        slice_ix,
                        slice_count,
                        context,
                    )),
            )
            .into_any_element();
    }

    let is_continuation = visual_row.is_some_and(MarkdownPreviewVisualRow::is_continuation);
    let row_layout = markdown_preview_row_layout(row, ui_scale_percent);
    let typography =
        markdown_preview_row_typography(theme, row, &context.editor_font_family, ui_scale_percent);
    let full_styled =
        markdown_preview_styled_row_with_query(theme, row, row_ix, context.query.as_ref());
    let full_styled = full_styled.as_ref();
    // Wrapped rows paint one slice of the row's text each; the marker and
    // alert badge belong to the first slice so continuations stay aligned
    // under the text they continue.
    let sliced_styled = visual_row
        .filter(|visual| visual.byte_range != (0..row.text.len()))
        .map(|visual| {
            slice_cached_diff_styled_text(
                full_styled,
                markdown_preview_expanded_slice_range(
                    row.text.as_ref(),
                    full_styled.text.len(),
                    &visual.byte_range,
                ),
            )
        });
    let styled = sliced_styled.as_ref().unwrap_or(full_styled);
    let horizontal_padding = markdown_preview_row_horizontal_padding(row, ui_scale_percent);
    // Continuations keep the marker slot but leave it blank, so wrapped list
    // and footnote text stays indented under the first line instead of
    // sliding back under the bullet.
    let marker = markdown_preview_row_marker(row).map(|marker| {
        if is_continuation {
            SharedString::default()
        } else {
            marker
        }
    });
    let alert_title = markdown_preview_alert_title_label(row).filter(|_| !is_continuation);
    // Pictures written on this line. A wrapped continuation already showed
    // them on its first visual row.
    let inline_images: &[MarkdownInlineImage] = if is_continuation {
        &[]
    } else {
        row.inline_images.as_ref()
    };

    // Rows that need a content_shell wrapper for border/background styling.
    let needs_content_shell = matches!(
        row.kind,
        MarkdownPreviewRowKind::Heading { level: 1 | 2 }
            | MarkdownPreviewRowKind::CodeLine { .. }
            | MarkdownPreviewRowKind::TableRow { .. }
            | MarkdownPreviewRowKind::PlainFallback
    );
    let flatten_shell_text_directly = !is_interactive
        && needs_content_shell
        && marker.is_none()
        && alert_title.is_none()
        && inline_images.is_empty();

    let build_content_shell = || {
        let mut content_shell = div()
            .flex_grow(1.)
            .min_w(px(0.0))
            .w_full()
            .h_full()
            .relative()
            .flex()
            .items_center();
        content_shell = match row.kind {
            MarkdownPreviewRowKind::Heading { level: 1 | 2 } => {
                content_shell.border_b_1().border_color(with_alpha(
                    theme.colors.stroke.default,
                    if theme.is_dark { 0.85 } else { 0.92 },
                ))
            }
            MarkdownPreviewRowKind::CodeLine { is_first, is_last } => {
                let code_border = with_alpha(
                    theme.colors.stroke.default,
                    if theme.is_dark { 0.90 } else { 0.80 },
                );
                let mut shell = content_shell
                    .px(markdown_preview_scaled_px(
                        MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
                        ui_scale_percent,
                    ))
                    .bg(markdown_preview_code_background(theme))
                    .border_l_1()
                    .border_r_1()
                    .border_color(code_border);
                if is_first {
                    shell = shell.border_t_1();
                }
                if is_last {
                    shell = shell.border_b_1();
                }
                shell
            }
            MarkdownPreviewRowKind::TableRow { is_header } => {
                let bg = if is_header {
                    with_alpha(
                        theme.colors.surface.raised,
                        if theme.is_dark { 0.64 } else { 0.86 },
                    )
                } else {
                    with_alpha(
                        theme.colors.surface.raised,
                        if theme.is_dark { 0.42 } else { 0.72 },
                    )
                };
                content_shell
                    .px(markdown_preview_scaled_px(
                        MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
                        ui_scale_percent,
                    ))
                    .bg(bg)
                    .border_b_1()
                    .border_color(with_alpha(
                        theme.colors.stroke.default,
                        if theme.is_dark { 0.88 } else { 0.86 },
                    ))
            }
            MarkdownPreviewRowKind::PlainFallback => content_shell
                .px(markdown_preview_scaled_px(
                    MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
                    ui_scale_percent,
                ))
                .bg(with_alpha(
                    theme.colors.status.warning.foreground,
                    if theme.is_dark { 0.12 } else { 0.08 },
                )),
            _ => unreachable!(),
        };
        if matches!(row.kind, MarkdownPreviewRowKind::CodeLine { .. }) && is_interactive {
            content_shell =
                content_shell.debug_selector(|| format!("markdown_preview_code_shell_{row_ix}"));
        }
        content_shell
    };

    let row_body = if flatten_shell_text_directly {
        // Benchmarked non-interactive rows do not need the extra inner content
        // wrapper when a shell already provides sizing/background/border styles.
        let mut content_shell = build_content_shell()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(typography.font_size))
            .line_height(px(typography.line_height))
            .text_color(typography.text_color);
        if let Some(font_weight) = typography.font_weight {
            content_shell = content_shell.font_weight(font_weight);
        }
        if let Some(font_family) = typography.font_family.clone() {
            content_shell = content_shell.font_family(font_family);
        }
        if styled.highlights.is_empty() {
            content_shell.child(styled.text.clone())
        } else {
            content_shell.child(MarkdownPreviewSharedHighlightsText::new(
                styled.text.clone(),
                Arc::clone(&styled.highlights),
            ))
        }
    } else {
        let mut content = div()
            .relative()
            .flex_grow(1.)
            .min_w(px(0.0))
            .w_full()
            .h(px(typography.line_height))
            .min_h(px(typography.line_height))
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(typography.font_size))
            .line_height(px(typography.line_height))
            .text_color(typography.text_color);
        if is_interactive {
            // Preview text is selectable, so the pointer should say so.
            content = content
                .cursor(gpui::CursorStyle::IBeam)
                .debug_selector(|| format!("markdown_preview_text_box_{row_ix}"));
        }

        if let Some(font_weight) = typography.font_weight {
            content = content.font_weight(font_weight);
        }
        if let Some(font_family) = typography.font_family.clone() {
            content = content.font_family(font_family);
        }
        if let Some(view) = context.view.clone() {
            // Hit testing and copy resolve rows through
            // `markdown_preview_row_text`, which works in `row.text`
            // coordinates, so the overlay shapes the raw slice rather than the
            // tab-expanded one this row paints.
            let selection_text = match visual_row {
                Some(visual) if sliced_styled.is_some() => visual.text_slice(row),
                _ => row.text.clone(),
            };
            content = content.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(DiffTextSelectionOverlay {
                        view,
                        visible_ix: row_ix,
                        region: text_region,
                        text: selection_text,
                    }),
            );
        }

        let body = match row.kind {
            MarkdownPreviewRowKind::ThematicBreak => div()
                .flex_grow(1.)
                .min_w(px(0.0))
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.0)).bg(with_alpha(
                    theme.colors.stroke.default,
                    if theme.is_dark { 0.92 } else { 0.88 },
                ))),
            _ if marker.is_none() && alert_title.is_none() && inline_images.is_empty() => {
                // Fast path: no marker or alert badge — use content div directly
                // as body, skipping the intermediate line wrapper div.
                if styled.highlights.is_empty() {
                    content.child(styled.text.clone())
                } else {
                    content.child(MarkdownPreviewSharedHighlightsText::new(
                        styled.text.clone(),
                        Arc::clone(&styled.highlights),
                    ))
                }
            }
            _ => {
                let text = if styled.highlights.is_empty() {
                    content.child(styled.text.clone()).into_any_element()
                } else {
                    content
                        .child(MarkdownPreviewSharedHighlightsText::new(
                            styled.text.clone(),
                            Arc::clone(&styled.highlights),
                        ))
                        .into_any_element()
                };

                let mut line = div()
                    .flex_grow(1.)
                    .min_w(px(0.0))
                    .w_full()
                    .h_full()
                    .flex()
                    .items_center();
                if let Some(marker) = marker {
                    line = line.child(
                        div()
                            .flex_none()
                            .h_full()
                            .min_w(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX,
                                ui_scale_percent,
                            ))
                            .mr(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX,
                                ui_scale_percent,
                            ))
                            .flex()
                            .items_center()
                            .justify_end()
                            .text_size(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_BASE_FONT_PX,
                                ui_scale_percent,
                            ))
                            .line_height(px(typography.line_height))
                            .text_color(theme.colors.foreground.secondary)
                            .child(marker),
                    );
                }
                if let Some(alert_title) = alert_title {
                    let alert_color = markdown_preview_alert_color(theme, row.alert_kind.unwrap());
                    line = line.child(
                        div()
                            .flex_none()
                            .mr(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX,
                                ui_scale_percent,
                            ))
                            .px(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX,
                                ui_scale_percent,
                            ))
                            .py(markdown_preview_scaled_px(2.0, ui_scale_percent))
                            .rounded(markdown_preview_scaled_px(2.0, ui_scale_percent))
                            .bg(with_alpha(
                                alert_color,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .text_size(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX,
                                ui_scale_percent,
                            ))
                            .font_weight(FontWeight::BOLD)
                            .text_color(alert_color)
                            .child(alert_title),
                    );
                }
                // The diff preview's rows are a fixed height, so an inline
                // picture is capped to the line and sits ahead of the text
                // rather than flowing at the offset it was written at.
                for inline in inline_images.iter() {
                    line = line.child(
                        div()
                            .flex_none()
                            .h_full()
                            .mr(markdown_preview_scaled_px(
                                MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX,
                                ui_scale_percent,
                            ))
                            .overflow_hidden()
                            .child(markdown_preview_inline_image(
                                inline,
                                theme,
                                ui_scale_percent,
                                context.image_base_dir.as_deref(),
                                markdown_preview_no_picture_sizes(),
                            )),
                    );
                }
                line.child(text)
            }
        };

        if needs_content_shell {
            build_content_shell().child(body)
        } else {
            body
        }
    };
    // The row's horizontal padding always lives on a wrapper, never on the
    // text box itself: the selection overlay is absolutely positioned inside
    // that box, so padding applied there would shift the highlight left of the
    // glyphs it is meant to cover and cut it short at the end of the line.
    let build_row_content = move || {
        let mut row_content = div()
            .flex_grow(1.)
            .min_w(px(0.0))
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .pl(px(horizontal_padding.left_px))
            .pr(px(horizontal_padding.right_px));
        if let Some(blockquote_gutter) = markdown_preview_blockquote_gutter(
            theme,
            row.blockquote_level,
            row.alert_kind,
            ui_scale_percent,
        ) {
            row_content = row_content.child(blockquote_gutter);
        }
        row_content
    };

    if let Some(view) = context.view.clone() {
        // Interactive markdown preview row with text selection + context menu.
        let row_container = div()
            .id(("md_preview_row", row_ix))
            .debug_selector(|| format!("markdown_preview_row_box_{row_ix}"))
            .relative()
            .h(markdown_preview_row_height(ui_scale_percent))
            .min_h(markdown_preview_row_height(ui_scale_percent))
            .w(min_width)
            .flex()
            .items_center()
            .pt(px(row_layout.top_inset_px))
            .pb(px(row_layout.bottom_inset_px))
            .when_some(markdown_preview_row_background(theme, row), |div, bg| {
                div.bg(bg)
            })
            .min_w(min_width)
            .on_mouse_down(gpui::MouseButton::Left, {
                let view = view.clone();
                move |event, window, cx| {
                    let focus = view.read(cx).diff_panel_focus_handle.clone();
                    window.focus(&focus, cx);
                    let click_count = event.click_count;
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        if !this.handle_markdown_preview_link_click(
                            row_ix,
                            text_region,
                            position,
                            click_count,
                            window,
                            cx,
                        ) {
                            this.handle_diff_text_mouse_down(
                                row_ix,
                                text_region,
                                position,
                                click_count,
                                cx,
                            );
                        }
                        cx.notify();
                    });
                }
            })
            .on_mouse_down(gpui::MouseButton::Right, {
                let view = view.clone();
                move |event, window, cx| {
                    view.update(cx, |this, cx| {
                        this.open_diff_editor_context_menu(
                            row_ix,
                            text_region,
                            event.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    });
                }
            });
        row_container
            .child(build_row_content().child(row_body))
            .into_any_element()
    } else {
        // Non-interactive markdown preview row (benchmarks, conflict resolver).
        let row_container = div()
            .relative()
            .h(markdown_preview_row_height(ui_scale_percent))
            .min_h(markdown_preview_row_height(ui_scale_percent))
            .w(min_width)
            .flex()
            .items_center()
            .pt(px(row_layout.top_inset_px))
            .pb(px(row_layout.bottom_inset_px))
            .when_some(markdown_preview_row_background(theme, row), |div, bg| {
                div.bg(bg)
            })
            .min_w(min_width);
        row_container
            .child(build_row_content().child(row_body))
            .into_any_element()
    }
}

pub(in crate::view) fn markdown_preview_row_required_width(
    window: &mut Window,
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    editor_font_family: &SharedString,
    ui_scale_percent: u32,
) -> Pixels {
    if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
        return px(0.0);
    }

    let typography =
        markdown_preview_row_typography(theme, row, editor_font_family, ui_scale_percent);
    // Word wrap measures every row of the document, so the ambient text style
    // — which `Window::text_style` rebuilds from the style stack on each call
    // — is only consulted for rows that do not carry their own family.
    let resolved_font_family = typography
        .font_family
        .clone()
        .unwrap_or_else(|| window.text_style().font_family.clone());
    let cache_key = markdown_preview_row_width_cache_key(
        typography.font_size,
        typography.font_weight.unwrap_or(FontWeight::NORMAL),
        resolved_font_family.as_ref(),
    );
    let base_width = row.measured_width_px.get_or_init(cache_key, || {
        let base_font_weight = typography.font_weight.unwrap_or(FontWeight::NORMAL);
        let text_width = if matches!(row.kind, MarkdownPreviewRowKind::ThematicBreak) {
            px(0.0)
        } else {
            let highlights = markdown_preview_width_affecting_highlights(theme, row);
            markdown_preview_shape_text_width(
                window,
                row.text.clone(),
                typography.font_size,
                base_font_weight,
                typography.font_family.as_ref().map(SharedString::as_ref),
                &highlights,
            )
        };

        let width = text_width + markdown_preview_row_chrome_width(window, row, ui_scale_percent);
        u32::from(width.round())
    });

    px(base_width as f32)
}

/// Width a row spends on everything that is not its text: padding, blockquote
/// gutter, list marker, alert badge, and the code/table shell.
///
/// `markdown_preview_row_required_width` adds this to the shaped text width;
/// word wrap subtracts it from the viewport to get the width the text may
/// occupy.
pub(in crate::view) fn markdown_preview_row_chrome_width(
    window: &mut Window,
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
) -> Pixels {
    let horizontal_padding = markdown_preview_row_horizontal_padding(row, ui_scale_percent);
    let mut width = px(horizontal_padding.left_px + horizontal_padding.right_px);

    if row.blockquote_level > 0 {
        width += px(f32::from(row.blockquote_level)
            * markdown_preview_scaled_value(
                MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX,
                ui_scale_percent,
            )
            + f32::from(row.blockquote_level.saturating_sub(1))
                * markdown_preview_scaled_value(
                    MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_GAP_PX,
                    ui_scale_percent,
                )
            + markdown_preview_scaled_value(
                MARKDOWN_PREVIEW_BLOCKQUOTE_GUTTER_MARGIN_RIGHT_PX,
                ui_scale_percent,
            ));
    }

    if let Some(marker) = markdown_preview_row_marker(row) {
        let marker_width = markdown_preview_shape_text_width(
            window,
            marker,
            markdown_preview_scaled_value(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent),
            FontWeight::NORMAL,
            None,
            &[],
        );
        width += marker_width.max(markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX,
            ui_scale_percent,
        ));
        width += markdown_preview_scaled_px(MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, ui_scale_percent);
    }

    if let Some(alert_title) = markdown_preview_alert_title_label(row) {
        let alert_width = markdown_preview_shape_text_width(
            window,
            alert_title,
            markdown_preview_scaled_value(MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX, ui_scale_percent),
            FontWeight::BOLD,
            None,
            &[],
        );
        width += alert_width
            + markdown_preview_scaled_px(
                MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX * 2.0,
                ui_scale_percent,
            );
        width += markdown_preview_scaled_px(MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX, ui_scale_percent);
    }

    // Pictures painted on this line push the text right and widen the row.
    // Their natural size is only known once loaded, so a declared width is used
    // where there is one and the inline height cap stands in otherwise — the
    // point is that the row is not measured as if the pictures were absent.
    for inline in row.inline_images.iter() {
        let reserved = inline
            .image
            .width_px
            .map(|width| width as f32)
            .unwrap_or(MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX);
        width += markdown_preview_scaled_px(reserved, ui_scale_percent);
        width += markdown_preview_scaled_px(MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX, ui_scale_percent);
    }

    width += match row.kind {
        MarkdownPreviewRowKind::CodeLine { .. } => markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_SHELL_PAD_X_PX * 2.0 + MARKDOWN_PREVIEW_CODE_BORDER_PX * 2.0,
            ui_scale_percent,
        ),
        MarkdownPreviewRowKind::TableRow { .. } | MarkdownPreviewRowKind::PlainFallback => {
            markdown_preview_scaled_px(MARKDOWN_PREVIEW_SHELL_PAD_X_PX * 2.0, ui_scale_percent)
        }
        _ => px(0.0),
    };

    width
}

/// Byte ranges of `row.text` that fit `available_width`, one per visual row.
///
/// Returns fewer than two ranges when the row needs no wrapping, which
/// `build_markdown_preview_wrap_plan` collapses back to a single visual row.
/// Wrapping is measured with the row's own typography — headings, code, and
/// body text all use different fonts — via `gpui`'s line wrapper rather than a
/// character-count approximation, because preview text is proportional.
///
/// Ranges are in `row.text` coordinates; the renderer maps them onto the
/// tab-expanded text it paints (see `markdown_preview_expanded_slice_range`).
pub(in crate::view) fn markdown_preview_row_wrap_ranges(
    window: &mut Window,
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    available_width: Pixels,
    editor_font_family: &SharedString,
    ui_scale_percent: u32,
) -> Vec<Range<usize>> {
    if row.text.is_empty()
        || matches!(
            row.kind,
            MarkdownPreviewRowKind::Spacer | MarkdownPreviewRowKind::ThematicBreak
        )
    {
        return Vec::new();
    }

    // Rows that already fit need no wrapper pass at all. The required width is
    // cached per row and keyed only by font, so on a resize this is a hash and
    // a comparison rather than a re-measure — which is what keeps a wide
    // document from re-shaping every row on every frame of a resize drag.
    if markdown_preview_row_required_width(window, theme, row, editor_font_family, ui_scale_percent)
        <= available_width
    {
        return Vec::new();
    }

    let chrome = markdown_preview_row_chrome_width(window, row, ui_scale_percent);
    let wrap_width = available_width - chrome;
    if wrap_width <= px(0.0) {
        return Vec::new();
    }

    let typography =
        markdown_preview_row_typography(theme, row, editor_font_family, ui_scale_percent);
    let mut font = window.text_style().font();
    if let Some(font_family) = typography.font_family.clone() {
        font.family = font_family;
    }
    if let Some(font_weight) = typography.font_weight {
        font.weight = font_weight;
    }

    let text = row.text.clone();
    // A tab is painted as four spaces, so it is fed to the wrapper as an
    // element of that width rather than as a single character.
    let tab_width = text.contains('\t').then(|| {
        markdown_preview_shape_text_width(
            window,
            "    ",
            typography.font_size,
            typography.font_weight.unwrap_or(FontWeight::NORMAL),
            typography.font_family.as_ref().map(SharedString::as_ref),
            &[],
        )
    });
    let mut handle = window
        .text_system()
        .line_wrapper(font, px(typography.font_size));
    // Prose has no tabs, so the common case stays on the stack.
    let tabbed_fragments =
        tab_width.map(|width| markdown_preview_wrap_fragments(text.as_ref(), width));
    let plain_fragment = [gpui::LineFragment::text(text.as_ref())];
    let fragments: &[gpui::LineFragment<'_>] = match tabbed_fragments.as_deref() {
        Some(fragments) => fragments,
        None => &plain_fragment,
    };
    let mut ranges = Vec::new();
    let mut start = 0usize;
    for boundary in handle.wrap_line(fragments, wrap_width) {
        if boundary.ix <= start || !text.is_char_boundary(boundary.ix) {
            continue;
        }
        ranges.push(start..boundary.ix);
        start = boundary.ix;
    }
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges.push(start..text.len());
    ranges
}

/// Split `text` into wrap fragments, giving each tab the width it is painted
/// at ([`DIFF_WRAP_TAB_EXPANDED_COLUMNS`] spaces) instead of a single character.
pub(in crate::view) fn markdown_preview_wrap_fragments(
    text: &str,
    tab_width: Pixels,
) -> Vec<gpui::LineFragment<'_>> {
    let mut fragments = Vec::new();
    let mut segment_start = 0usize;
    for (ix, _) in text.match_indices('\t') {
        if ix > segment_start {
            fragments.push(gpui::LineFragment::text(&text[segment_start..ix]));
        }
        fragments.push(gpui::LineFragment::element(tab_width, 1));
        segment_start = ix + 1;
    }
    if segment_start < text.len() {
        fragments.push(gpui::LineFragment::text(&text[segment_start..]));
    }
    fragments
}

/// Map a `row.text` byte range onto the tab-expanded text that is painted.
///
/// Styled preview text replaces every tab with [`DIFF_WRAP_TAB_EXPANDED_COLUMNS`]
/// spaces, so raw offsets would slice the painted text in the wrong place —
/// shifted by three bytes per preceding tab, and cutting the tail short.
pub(in crate::view) fn markdown_preview_expanded_slice_range(
    raw_text: &str,
    expanded_len: usize,
    range: &Range<usize>,
) -> Range<usize> {
    if expanded_len == raw_text.len() {
        return range.clone();
    }

    let expand = |offset: usize| {
        let offset = offset.min(raw_text.len());
        let tabs = raw_text.as_bytes()[..offset]
            .iter()
            .filter(|byte| **byte == b'\t')
            .count();
        offset + tabs * (DIFF_WRAP_TAB_EXPANDED_COLUMNS - 1)
    };

    expand(range.start)..expand(range.end)
}

/// Pixel sizes read from picture headers, keyed by the source the document
/// wrote. Empty for anything that could not be measured without decoding.
pub(in crate::view) type MarkdownPreviewPictureSizes = Arc<FxHashMap<SharedString, (u32, u32)>>;

/// Shared stand-in for a preview that measured nothing. The diff preview draws
/// its pictures into fixed-height bands, so it has no use for their real sizes.
pub(in crate::view) fn markdown_preview_no_picture_sizes() -> &'static MarkdownPreviewPictureSizes {
    static EMPTY: std::sync::OnceLock<MarkdownPreviewPictureSizes> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Default::default)
}

/// Where a markdown image source resolves to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownPreviewImageSource {
    /// A file inside the previewed document's own directory tree.
    File(std::path::PathBuf),
    /// An `http(s)` URL, fetched and cached by `gpui`'s image loader.
    Remote(SharedString),
}

impl MarkdownPreviewImageSource {
    /// The key `gpui` stores this picture's decoded frames under.
    ///
    /// Everything that wants to know whether a picture is ready — the element
    /// that draws it and the pane waiting to be told it finished decoding —
    /// has to name it the same way, or they would be asking about two
    /// different entries in the asset cache.
    pub(in crate::view) fn to_resource(&self) -> gpui::Resource {
        match self {
            Self::File(path) => gpui::Resource::from(path.clone()),
            Self::Remote(url) => gpui::Resource::Uri(gpui::SharedUri::from(url.to_string())),
        }
    }
}

/// Resolve a markdown image source to something the preview can draw.
///
/// A local path must stay inside the previewed document's own directory tree,
/// so document content cannot aim the preview at arbitrary files on disk.
/// Anything else — `data:` payloads, other schemes, paths that climb out of
/// the tree — resolves to nothing and falls back to the alt text.
pub(in crate::view) fn markdown_preview_image_source(
    base_dir: Option<&std::path::Path>,
    source: &str,
) -> Option<MarkdownPreviewImageSource> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    if let Some(remote) = markdown_preview_remote_image_url(source) {
        return Some(MarkdownPreviewImageSource::Remote(remote));
    }
    if source.contains("://") || source.starts_with("data:") {
        return None;
    }

    // Query and fragment suffixes are common on image sources and are not part
    // of the file name.
    let path = source.split(['#', '?']).next().unwrap_or(source);
    let relative = std::path::Path::new(path);
    if relative.is_absolute() {
        return None;
    }
    let mut resolved = base_dir?.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => resolved.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }

    resolved
        .is_file()
        .then_some(MarkdownPreviewImageSource::File(resolved))
}

/// The `http(s)` URL an image source names, if it names one.
///
/// Only these two schemes are followed; anything else a document might carry
/// (`file:`, `javascript:`, and so on) is not something a preview should
/// dereference.
pub(in crate::view) fn markdown_preview_remote_image_url(source: &str) -> Option<SharedString> {
    let scheme_end = source.find("://")?;
    let scheme = &source[..scheme_end];
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then(|| SharedString::from(source.to_owned()))
}

/// The quick-search state a markdown preview renders under.
///
/// Carried by both preview renderers — the virtualized lists and the flowing
/// single document — so a Ctrl+F match is washed in place instead of the view
/// having to fall back to the markdown source.
#[derive(Clone)]
pub(in crate::view) struct MarkdownPreviewQuery {
    pub(in crate::view) matcher: Arc<DiffSearchMatcher>,
    /// Visible index of the row the search cursor is on, if it is in this list.
    pub(in crate::view) current_row: Option<usize>,
}

impl MarkdownPreviewQuery {
    pub(in crate::view) fn emphasis(&self, visible_ix: usize) -> DiffSearchMatchEmphasis {
        if self.current_row == Some(visible_ix) {
            DiffSearchMatchEmphasis::Current
        } else {
            DiffSearchMatchEmphasis::Other
        }
    }
}

/// A pending "bring this row into view" request for the flowing markdown
/// preview.
///
/// The flowing document has no fixed row height and is not a `uniform_list`, so
/// there is no `scroll_to_item` to hand the work to: the offset can only be
/// computed once the target row has been laid out. The request is therefore
/// shared into the renderer, which reports the row's bounds back through
/// [`Self::take`] during prepaint and applies the scroll then.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownPreviewRevealRequest(
    std::rc::Rc<std::cell::Cell<Option<usize>>>,
);

impl MarkdownPreviewRevealRequest {
    pub(in crate::view) fn request(&self, row_ix: usize) {
        self.0.set(Some(row_ix));
    }

    pub(in crate::view) fn clear(&self) {
        self.0.set(None);
    }

    pub(in crate::view) fn pending(&self) -> Option<usize> {
        self.0.get()
    }

    /// Claim the request, so the reveal runs once instead of fighting the user
    /// on every later frame.
    pub(in crate::view) fn take(&self) -> Option<usize> {
        self.0.take()
    }
}

/// The vertical extent of a laid-out row, from the bounds of its parts.
///
/// A row shell holds a marker, an alert badge and the text line; the row is the
/// band they span together.
pub(in crate::view) fn markdown_preview_row_extent(
    children: &[gpui::Bounds<Pixels>],
) -> Option<(Pixels, Pixels)> {
    let top = children
        .iter()
        .map(|bounds| bounds.origin.y)
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    let bottom = children
        .iter()
        .map(|bounds| bounds.bottom())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?;
    Some((top, (bottom - top).max(px(0.0))))
}

/// Where a row sits inside a scroll container, and how tall it is.
///
/// Split out from the prepaint listener so the arithmetic that decides the new
/// offset is testable without a window.
pub(in crate::view) fn markdown_preview_reveal_offset_y(
    row_top_in_content: Pixels,
    row_height: Pixels,
    viewport_height: Pixels,
    max_offset_y: Pixels,
    current_y: Pixels,
) -> Option<Pixels> {
    if viewport_height <= px(0.0) {
        return None;
    }
    // Centre the row the way a uniform list would, then clamp into the
    // scrollable range. Offsets are negative as you scroll down.
    let centered = row_top_in_content + row_height / 2.0 - viewport_height / 2.0;
    let target = (-centered).clamp(-max_offset_y.max(px(0.0)), px(0.0));
    (target != current_y).then_some(target)
}

/// Styled text for one row with the search wash layered on, shared with the
/// flowing renderer.
///
/// The base styling lives in a `OnceLock` on the row itself — it belongs to the
/// document, which outlives any one query — so the wash is merged on top per
/// frame rather than stored. Rows with no match return the base untouched, so
/// the extra work is a substring scan per visible row.
pub(in crate::view) fn markdown_preview_styled_row_with_query<'a>(
    theme: AppTheme,
    row: &'a MarkdownPreviewRow,
    visible_ix: usize,
    query: Option<&MarkdownPreviewQuery>,
) -> std::borrow::Cow<'a, CachedDiffStyledText> {
    let base = markdown_preview_row_styled_text(theme, row);
    let Some(query) = query else {
        return std::borrow::Cow::Borrowed(base);
    };
    if !query.matcher.is_match(base.text.as_ref()) {
        return std::borrow::Cow::Borrowed(base);
    }
    std::borrow::Cow::Owned(build_cached_diff_query_overlay_styled_text(
        theme,
        base,
        &query.matcher,
        query.emphasis(visible_ix),
    ))
}

/// Text element carrying inline highlights, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_highlighted_text(
    text: SharedString,
    highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
) -> impl IntoElement {
    MarkdownPreviewSharedHighlightsText::new(text, highlights)
}

/// List bullet or number for a row, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_marker_label(
    row: &MarkdownPreviewRow,
) -> Option<SharedString> {
    markdown_preview_row_marker(row)
}

/// Accent colour for an alert blockquote, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_alert_bar_color(
    theme: AppTheme,
    kind: MarkdownAlertKind,
) -> gpui::Rgba {
    markdown_preview_alert_color(theme, kind)
}

/// Badge label for an alert blockquote, shared with the flowing renderer.
pub(in crate::view) fn markdown_preview_alert_label(
    kind: MarkdownAlertKind,
) -> Option<SharedString> {
    Some(SharedString::new_static(match kind {
        MarkdownAlertKind::Note => "NOTE",
        MarkdownAlertKind::Tip => "TIP",
        MarkdownAlertKind::Important => "IMPORTANT",
        MarkdownAlertKind::Warning => "WARNING",
        MarkdownAlertKind::Caution => "CAUTION",
    }))
}

/// An image sized the way the document asked, for the flowing renderer.
///
/// Unlike the diff preview's banded block, this is one element that keeps its
/// aspect ratio and never reserves rows it does not need.
pub(in crate::view) fn markdown_preview_flow_image(
    row: &MarkdownPreviewRow,
    row_ix: usize,
    theme: AppTheme,
    ui_scale_percent: u32,
    image_base_dir: Option<&std::path::Path>,
    picture_sizes: &MarkdownPreviewPictureSizes,
) -> AnyElement {
    let label_color = theme.colors.foreground.secondary;
    let font_size = markdown_preview_scaled_px(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent);

    let picture = row.image.as_ref().and_then(|image| {
        markdown_preview_resolved_picture(
            image.source.as_ref(),
            ("markdown_preview_block_image", row_ix).into(),
            image_base_dir,
        )
    });
    let Some(image) = picture else {
        return markdown_preview_image_placeholder_element(
            markdown_preview_image_label(row, "Image unavailable"),
            font_size,
            label_color,
        )
        .into_any_element();
    };

    let declared = row.image.as_ref().and_then(|image| image.width_px);
    let failed_label = markdown_preview_image_label(row, "Failed to load");
    let skeleton = markdown_preview_picture_skeleton(row, ui_scale_percent, picture_sizes);
    let image = match declared {
        Some(width) => image.w(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        // Without a declared size the picture keeps its own, up to the width
        // of the document.
        None => image.max_w_full(),
    };

    div()
        .w_full()
        .min_w(px(0.0))
        .child(
            image
                .debug_selector(move || format!("markdown_preview_block_image_{row_ix}"))
                .with_fallback(move || {
                    markdown_preview_image_placeholder_element(
                        failed_label.clone(),
                        font_size,
                        label_color,
                    )
                    .into_any_element()
                })
                .with_loading(move || skeleton.render(theme)),
        )
        .into_any_element()
}

/// The box a picture will occupy, worked out before it has been decoded.
///
/// `gpui` reads every frame of an animated picture before it reports a size, so
/// a block that waited for that would leave a hole in the document and then
/// shove everything down when the picture arrived. What the document declared
/// comes first; the picture's own header fills in the rest.
#[derive(Clone, Copy)]
pub(in crate::view) struct MarkdownPreviewPictureSkeleton {
    /// Widest the picture will draw, or `None` to fill the document.
    pub(in crate::view) width: Option<Pixels>,
    /// Width over height, or `None` when only a height is known.
    pub(in crate::view) aspect_ratio: Option<f32>,
    /// Used when the aspect ratio is unknown: the rows the parser set aside.
    pub(in crate::view) reserved_height: Pixels,
}

impl MarkdownPreviewPictureSkeleton {
    pub(in crate::view) fn render(self, theme: AppTheme) -> AnyElement {
        let mut block = components::skeleton(theme)
            .debug_selector(|| "markdown_preview_picture_skeleton".to_string());
        block = match self.width {
            Some(width) => block.w(width).max_w_full(),
            None => block.w_full(),
        };
        block = match self.aspect_ratio {
            Some(ratio) => block.aspect_ratio(ratio),
            None => block.h(self.reserved_height),
        };
        block.into_any_element()
    }
}

pub(in crate::view) fn markdown_preview_picture_skeleton(
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
    picture_sizes: &MarkdownPreviewPictureSizes,
) -> MarkdownPreviewPictureSkeleton {
    let image = row.image.as_ref();
    let declared_width = image.and_then(|image| image.width_px).filter(|w| *w > 0);
    let declared_height = image.and_then(|image| image.height_px).filter(|h| *h > 0);
    // A declared size is in design pixels and scales with the UI; a size read
    // from the file is in the picture's own pixels, which is what `gpui` lays
    // an undeclared picture out at.
    let measured = image
        .and_then(|image| picture_sizes.get(&image.source))
        .copied();

    let width = match (declared_width, measured) {
        (Some(width), _) => Some(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        (None, Some((width, _))) => Some(px(width as f32)),
        (None, None) => None,
    };
    let aspect_ratio = match (declared_width, declared_height, measured) {
        (Some(width), Some(height), _) => Some(width as f32 / height as f32),
        (_, _, Some((width, height))) => Some(width as f32 / height as f32),
        _ => None,
    };

    MarkdownPreviewPictureSkeleton {
        width,
        aspect_ratio,
        reserved_height: markdown_preview_row_height(ui_scale_percent)
            * f32::from(markdown_preview_image_block_rows(row).max(1)),
    }
}

/// Rows an image block was given, which is the height it reserved.
pub(in crate::view) fn markdown_preview_image_block_rows(row: &MarkdownPreviewRow) -> u8 {
    match row.kind {
        MarkdownPreviewRowKind::Image { slice_count, .. } => slice_count,
        _ => 1,
    }
}

/// Tallest an inline picture may be when the document declares no size, so a
/// stray screenshot written mid-sentence cannot push the line open.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX: f32 = 26.0;

/// Space between an inline picture and whatever shares its line.
///
/// Both previews use it, but only the row grid has to reserve it: that preview
/// measures a row's width to drive horizontal scrolling, so the gap is part of
/// the row chrome there and purely visual in the flowing renderer.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX: f32 = 4.0;

/// One picture drawn on the same line as the text around it.
///
/// Badges, shields, and a logo beside a heading are all written inline, so they
/// are sized to the line rather than to the document: a declared width wins,
/// and anything else keeps its own size up to the inline height cap.
pub(in crate::view) fn markdown_preview_inline_image(
    inline: &MarkdownInlineImage,
    theme: AppTheme,
    ui_scale_percent: u32,
    image_base_dir: Option<&std::path::Path>,
    picture_sizes: &MarkdownPreviewPictureSizes,
) -> AnyElement {
    let source_byte = inline.source_byte;
    let label_color = theme.colors.foreground.secondary;
    let font_size = markdown_preview_scaled_px(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent);
    let described = if inline.alt.is_empty() {
        inline.image.source.clone()
    } else {
        inline.alt.clone()
    };
    let measured_aspect_ratio = picture_sizes
        .get(&inline.image.source)
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| *width as f32 / *height as f32);

    let picture = markdown_preview_resolved_picture(
        inline.image.source.as_ref(),
        ("markdown_preview_inline_image", source_byte).into(),
        image_base_dir,
    );
    let Some(image) = picture else {
        return markdown_preview_inline_image_placeholder(
            markdown_preview_image_reason("Image unavailable", &described),
            source_byte,
            font_size,
            label_color,
        );
    };

    let failed_label = markdown_preview_image_reason("Failed to load", &described);
    let image =
        image.debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"));
    let image = match inline.image.width_px {
        Some(width) => image.w(markdown_preview_scaled_px(width as f32, ui_scale_percent)),
        None => image.max_h(markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX,
            ui_scale_percent,
        )),
    }
    // The height cap leaves a wide, short banner unbounded, and a declared
    // width can be larger than the pane; either would push the document into
    // horizontal overflow.
    .max_w_full();

    // A badge that has not arrived yet still holds its slot, so the line it
    // shares does not reflow the moment it does. Inline pictures are sized to
    // the line rather than to their own pixels, so the cap is the height and a
    // measured picture only decides how wide the slot is.
    let loading_height = markdown_preview_scaled_px(
        MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX,
        ui_scale_percent,
    );
    let loading_width = match (inline.image.width_px, measured_aspect_ratio) {
        (Some(width), _) => markdown_preview_scaled_px(width as f32, ui_scale_percent),
        (None, Some(ratio)) => loading_height * ratio,
        (None, None) => markdown_preview_scaled_px(
            MARKDOWN_PREVIEW_INLINE_IMAGE_LOADING_WIDTH_PX,
            ui_scale_percent,
        ),
    };

    image
        .with_fallback(move || {
            markdown_preview_inline_image_placeholder(
                failed_label.clone(),
                source_byte,
                font_size,
                label_color,
            )
        })
        .with_loading(move || {
            components::skeleton(theme)
                .debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"))
                .flex_none()
                .w(loading_width)
                .h(loading_height)
                .max_w_full()
                .into_any_element()
        })
        .into_any_element()
}

/// Slot an inline picture of unknown size holds while it loads. Wide enough for
/// the badges a README opens with, which is what this mostly stands in for.
pub(in crate::view) const MARKDOWN_PREVIEW_INLINE_IMAGE_LOADING_WIDTH_PX: f32 = 90.0;

/// A picture element that keeps per-frame state.
///
/// The id matters: `gpui` only remembers which frame an animated image is
/// showing for elements that have one, so an `img` without an id freezes on the
/// first frame of a GIF.
pub(in crate::view) fn markdown_preview_image_element(
    source: MarkdownPreviewImageSource,
    id: gpui::ElementId,
) -> gpui::Stateful<gpui::Img> {
    gpui::img(gpui::ImageSource::Resource(source.to_resource())).id(id)
}

/// Stand-in for a picture that cannot be drawn.
///
/// It carries the picture's selector too: the slot has to hold its place
/// whether or not the source loaded, and a test asking whether the picture was
/// drawn is really asking whether that slot exists.
pub(in crate::view) fn markdown_preview_inline_image_placeholder(
    label: SharedString,
    source_byte: usize,
    font_size: Pixels,
    color: gpui::Rgba,
) -> AnyElement {
    div()
        .debug_selector(move || format!("markdown_preview_inline_image_{source_byte}"))
        .flex_none()
        .text_size(font_size)
        .text_color(color)
        .child(label)
        .into_any_element()
}

/// Label for a picture that is not on screen: the reason, plus the alt text or
/// the source so the reader can tell which image is missing.
pub(in crate::view) fn markdown_preview_image_label(
    row: &MarkdownPreviewRow,
    reason: &str,
) -> SharedString {
    let described = if row.text.is_empty() {
        row.image
            .as_ref()
            .map(|image| image.source.clone())
            .unwrap_or_default()
    } else {
        row.text.clone()
    };
    markdown_preview_image_reason(reason, &described)
}

/// "reason: what the picture was", or just the reason when nothing describes it.
pub(in crate::view) fn markdown_preview_image_reason(
    reason: &str,
    described: &SharedString,
) -> SharedString {
    if described.is_empty() {
        SharedString::from(reason.to_owned())
    } else {
        SharedString::from(format!("{reason}: {described}"))
    }
}

/// The picture element for `source`, or `None` when the source does not resolve
/// to something drawable at all.
///
/// Both previews take the same two steps — resolve the source against the
/// document's directory, then build an element that keeps per-frame state — and
/// differ only in how they size the result and what they show in its place.
pub(in crate::view) fn markdown_preview_resolved_picture(
    source: &str,
    id: gpui::ElementId,
    image_base_dir: Option<&std::path::Path>,
) -> Option<gpui::Stateful<gpui::Img>> {
    markdown_preview_image_source(image_base_dir, source)
        .map(|source| markdown_preview_image_element(source, id))
}

/// Stand-in shown in place of a picture, so the row is never silently blank.
pub(in crate::view) fn markdown_preview_image_placeholder_element(
    label: SharedString,
    font_size: Pixels,
    color: gpui::Rgba,
) -> gpui::Div {
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(font_size)
        .text_color(color)
        .child(label)
}

/// Stand-in for a source that could not be resolved at all.
pub(in crate::view) fn markdown_preview_image_placeholder(
    row: &MarkdownPreviewRow,
    context: &MarkdownPreviewRenderContext<'_>,
    reason: &str,
) -> gpui::Div {
    markdown_preview_image_placeholder_element(
        markdown_preview_image_label(row, reason),
        markdown_preview_scaled_px(MARKDOWN_PREVIEW_BASE_FONT_PX, context.ui_scale_percent),
        context.theme.colors.foreground.secondary,
    )
}

/// One horizontal band of an image block.
pub(in crate::view) fn markdown_preview_image_row(
    row: &MarkdownPreviewRow,
    row_ix: usize,
    slice_ix: u8,
    slice_count: u8,
    context: &MarkdownPreviewRenderContext<'_>,
) -> AnyElement {
    let ui_scale_percent = context.ui_scale_percent;
    let row_height = markdown_preview_row_height(ui_scale_percent);
    let block_height = row_height * f32::from(slice_count.max(1));
    let picture = row.image.as_ref().and_then(|image| {
        markdown_preview_resolved_picture(
            image.source.as_ref(),
            ("markdown_preview_image_band", row_ix).into(),
            context.image_base_dir.as_deref(),
        )
    });
    // A declared width is the size the document asked for; without one the
    // picture fills the block.
    let declared_width = row
        .image
        .as_ref()
        .and_then(|image| image.width_px)
        .map(|width| markdown_preview_scaled_px(width as f32, ui_scale_percent));

    let band = div().relative().w_full().h(row_height).overflow_hidden();
    let Some(image) = picture else {
        // Nothing to draw: the first band describes the picture instead, and
        // the rest stay blank so the block keeps its shape.
        if slice_ix != 0 {
            return band.into_any_element();
        }
        return band
            .child(markdown_preview_image_placeholder(
                row,
                context,
                "Image unavailable",
            ))
            .into_any_element();
    };

    // `with_fallback` is called on demand, so the placeholder is rebuilt from
    // owned pieces rather than cloning a built element.
    let failed_label = markdown_preview_image_label(row, "Failed to load");
    let failed_font_size =
        markdown_preview_scaled_px(MARKDOWN_PREVIEW_BASE_FONT_PX, ui_scale_percent);
    let failed_color = context.theme.colors.foreground.secondary;
    // `Contain` keeps the aspect ratio inside whichever box the document asked
    // for, so a declared width never stretches the picture across the row.
    let image = match declared_width {
        Some(width) => image.w(width).max_w(width),
        None => image.w_full(),
    };
    band.child(
        div()
            .absolute()
            .left_0()
            .right_0()
            // Every band draws the whole picture and clips to its own slice, so
            // a block that is half scrolled off screen still renders correctly.
            .top(-(row_height * f32::from(slice_ix)))
            .h(block_height)
            .child(
                image
                    .h(block_height)
                    .object_fit(gpui::ObjectFit::Contain)
                    // A source that resolved but would not load — a 404 badge,
                    // an unreachable host, an undecodable file — says so rather
                    // than leaving a blank band.
                    .with_fallback(move || {
                        markdown_preview_image_placeholder_element(
                            failed_label.clone(),
                            failed_font_size,
                            failed_color,
                        )
                        .into_any_element()
                    }),
            ),
    )
    .into_any_element()
}

pub(in crate::view) fn markdown_preview_font_family_hash(font_family: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    font_family.hash(&mut hasher);
    hasher.finish()
}

pub(in crate::view) fn markdown_preview_row_width_cache_key(
    font_size: f32,
    font_weight: FontWeight,
    font_family: &str,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    font_size.to_bits().hash(&mut hasher);
    font_weight.hash(&mut hasher);
    font_family.hash(&mut hasher);
    hasher.finish()
}

pub(in crate::view) fn markdown_preview_width_affecting_highlights(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    row.inline_spans
        .iter()
        .filter_map(|span| {
            let style = markdown_preview_inline_highlight(theme, span.style);
            (style.font_weight.is_some() || style.font_style.is_some())
                .then_some((span.byte_range.start..span.byte_range.end, style))
        })
        .collect()
}

pub(in crate::view) fn markdown_preview_shape_text_width(
    window: &mut Window,
    text: impl Into<SharedString>,
    font_size_px: f32,
    font_weight: FontWeight,
    font_family: Option<&str>,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> Pixels {
    let text: SharedString = text.into();
    if text.is_empty() {
        return px(0.0);
    }

    let mut style = window.text_style();
    style.font_weight = font_weight;
    if let Some(font_family) = font_family {
        style.font_family = font_family.to_string().into();
    }

    let runs = crate::text_runs::text_runs_for_highlights(text.as_ref(), &style, highlights);

    window
        .text_system()
        .shape_line(text, px(font_size_px), &runs, None)
        .width
}

/// Gutter colour the flowing markdown preview marks a wholly added or removed
/// file with, shared with the source preview so the two agree.
pub(in crate::view) fn worktree_markdown_preview_bar_color(
    this: &MainPaneView,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    worktree_preview_bar_color(this, theme)
}

pub(in crate::view) fn worktree_preview_bar_color(
    this: &MainPaneView,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    let highlight_deleted_file = this.deleted_file_preview_abs_path().is_some();
    let highlight_new_file = this.untracked_worktree_preview_path().is_some()
        || this.added_file_preview_abs_path().is_some()
        || this.diff_preview_is_new_file;
    if highlight_deleted_file {
        Some(theme.colors.status.danger.foreground)
    } else if highlight_new_file {
        Some(theme.colors.status.success.foreground)
    } else {
        None
    }
}

pub(in crate::view) fn markdown_preview_row_styled_text(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> &CachedDiffStyledText {
    row.styled_text_cache.get_or_init(theme.is_dark, || {
        if matches!(row.kind, MarkdownPreviewRowKind::CodeLine { .. }) {
            return build_cached_diff_styled_text(
                theme,
                row.text.as_ref(),
                &[],
                "",
                row.code_language,
                DiffSyntaxMode::Auto,
                None,
            );
        }

        let highlights = row
            .inline_spans
            .iter()
            .filter_map(|span| {
                let style = markdown_preview_inline_highlight(theme, span.style);
                (style != gpui::HighlightStyle::default())
                    .then_some((span.byte_range.start..span.byte_range.end, style))
            })
            .collect::<Vec<_>>();
        build_cached_diff_styled_text_from_relative_highlights(row.text.as_ref(), &highlights)
    })
}

pub(in crate::view) fn markdown_preview_row_marker(
    row: &MarkdownPreviewRow,
) -> Option<SharedString> {
    if let Some(label) = row.footnote_label.as_ref() {
        return Some(format!("[^{}]:", label.as_ref()).into());
    }

    match row.kind {
        MarkdownPreviewRowKind::DetailsSummary => Some("v".into()),
        MarkdownPreviewRowKind::ListItem { number: Some(n) } => Some(format!("{n}.").into()),
        MarkdownPreviewRowKind::ListItem { number: None } => Some("•".into()),
        _ => None,
    }
}

pub(in crate::view) fn markdown_preview_alert_title_label(
    row: &MarkdownPreviewRow,
) -> Option<&'static str> {
    if !row.starts_alert {
        return None;
    }

    match row.alert_kind? {
        MarkdownAlertKind::Note => Some("NOTE"),
        MarkdownAlertKind::Tip => Some("TIP"),
        MarkdownAlertKind::Important => Some("IMPORTANT"),
        MarkdownAlertKind::Warning => Some("WARNING"),
        MarkdownAlertKind::Caution => Some("CAUTION"),
    }
}

pub(in crate::view) fn markdown_preview_alert_color(
    theme: AppTheme,
    kind: MarkdownAlertKind,
) -> gpui::Rgba {
    match kind {
        MarkdownAlertKind::Note => theme.colors.accent.foreground,
        MarkdownAlertKind::Tip => theme.colors.status.success.foreground,
        MarkdownAlertKind::Important => with_alpha(theme.colors.accent.foreground, 0.85),
        MarkdownAlertKind::Warning => theme.colors.status.warning.foreground,
        MarkdownAlertKind::Caution => theme.colors.status.danger.foreground,
    }
}

pub(in crate::view) fn markdown_preview_blockquote_gutter(
    theme: AppTheme,
    blockquote_level: u8,
    alert_kind: Option<MarkdownAlertKind>,
    ui_scale_percent: u32,
) -> Option<AnyElement> {
    if blockquote_level == 0 {
        return None;
    }

    let quote_bar_color = with_alpha(
        theme.colors.stroke.default,
        if theme.is_dark { 0.96 } else { 0.86 },
    );
    let alert_bar_color = alert_kind.map(|kind| markdown_preview_alert_color(theme, kind));
    let bars = (0..blockquote_level)
        .map(|ix| {
            let bar_color = if ix + 1 == blockquote_level {
                alert_bar_color.unwrap_or(quote_bar_color)
            } else {
                quote_bar_color
            };
            div()
                .w(markdown_preview_scaled_px(
                    MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX,
                    ui_scale_percent,
                ))
                .h_full()
                .bg(bar_color)
                .rounded(markdown_preview_scaled_px(2.0, ui_scale_percent))
                .into_any_element()
        })
        .collect::<Vec<_>>();

    Some(
        div()
            .flex_none()
            .h_full()
            .flex()
            .gap(markdown_preview_scaled_px(
                MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_GAP_PX,
                ui_scale_percent,
            ))
            .mr(markdown_preview_scaled_px(
                MARKDOWN_PREVIEW_BLOCKQUOTE_GUTTER_MARGIN_RIGHT_PX,
                ui_scale_percent,
            ))
            .children(bars)
            .into_any_element(),
    )
}

pub(in crate::view) fn markdown_preview_inline_highlight(
    theme: AppTheme,
    style: MarkdownInlineStyle,
) -> gpui::HighlightStyle {
    match style {
        MarkdownInlineStyle::Normal => gpui::HighlightStyle::default(),
        MarkdownInlineStyle::Bold => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Italic => gpui::HighlightStyle {
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::BoldItalic => gpui::HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            font_style: Some(gpui::FontStyle::Italic),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Code => gpui::HighlightStyle {
            background_color: Some(
                with_alpha(
                    theme.colors.interaction.selected_background,
                    if theme.is_dark { 0.75 } else { 0.55 },
                )
                .into_color(),
            ),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Strikethrough => gpui::HighlightStyle {
            color: Some(theme.colors.foreground.secondary.into_color()),
            strikethrough: Some(gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.secondary.into_color()),
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Link => gpui::HighlightStyle {
            color: Some(theme.colors.accent.foreground.into_color()),
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.accent.foreground.into_color()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Underline => gpui::HighlightStyle {
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.primary.into_color()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
    }
}

pub(in crate::view) fn markdown_preview_row_text_color(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> gpui::Rgba {
    if row.alert_kind.is_some() {
        return theme.colors.foreground.primary;
    }

    match row.kind {
        MarkdownPreviewRowKind::Heading { level: 6 } | MarkdownPreviewRowKind::BlockquoteLine => {
            theme.colors.foreground.secondary
        }
        MarkdownPreviewRowKind::Heading { .. } => theme.colors.foreground.primary,
        MarkdownPreviewRowKind::ThematicBreak => theme.colors.foreground.secondary,
        MarkdownPreviewRowKind::PlainFallback => theme.colors.status.warning.foreground,
        _ => theme.colors.foreground.primary,
    }
}

pub(in crate::view) fn markdown_preview_row_layout(
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
) -> MarkdownPreviewRowLayout {
    let scaled = |value: f32| markdown_preview_scaled_value(value, ui_scale_percent);
    match row.kind {
        // Headings are inset evenly so the text sits centred in its row rather
        // than riding high with a gap underneath. The section break above a
        // top-level heading is a spacer row; these insets are the smaller gap
        // that surrounds the heading text itself.
        MarkdownPreviewRowKind::Heading { level: 1 | 2 } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(2.0),
            bottom_inset_px: scaled(2.0),
        },
        MarkdownPreviewRowKind::Heading { level: 3 } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(3.0),
            bottom_inset_px: scaled(3.0),
        },
        MarkdownPreviewRowKind::Heading { .. } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(4.0),
            bottom_inset_px: scaled(4.0),
        },
        MarkdownPreviewRowKind::DetailsSummary => MarkdownPreviewRowLayout {
            top_inset_px: scaled(0.0),
            bottom_inset_px: scaled(0.0),
        },
        MarkdownPreviewRowKind::Paragraph => MarkdownPreviewRowLayout {
            top_inset_px: scaled(2.0),
            bottom_inset_px: scaled(6.0),
        },
        MarkdownPreviewRowKind::BlockquoteLine => MarkdownPreviewRowLayout {
            top_inset_px: scaled(2.0),
            bottom_inset_px: scaled(6.0),
        },
        MarkdownPreviewRowKind::ListItem { .. } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(0.0),
            bottom_inset_px: scaled(0.0),
        },
        MarkdownPreviewRowKind::CodeLine { is_first, is_last } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(if is_first { 5.0 } else { 0.0 }),
            bottom_inset_px: scaled(if is_last { 5.0 } else { 0.0 }),
        },
        MarkdownPreviewRowKind::ThematicBreak => MarkdownPreviewRowLayout {
            top_inset_px: scaled(6.0),
            bottom_inset_px: scaled(6.0),
        },
        // The bands of an image block must tile without gaps.
        MarkdownPreviewRowKind::Image { .. } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(0.0),
            bottom_inset_px: scaled(0.0),
        },
        MarkdownPreviewRowKind::Spacer => MarkdownPreviewRowLayout {
            top_inset_px: scaled(0.0),
            bottom_inset_px: scaled(0.0),
        },
        MarkdownPreviewRowKind::TableRow { .. } | MarkdownPreviewRowKind::PlainFallback => {
            MarkdownPreviewRowLayout {
                top_inset_px: scaled(2.0),
                bottom_inset_px: scaled(2.0),
            }
        }
    }
}

pub(in crate::view) fn markdown_preview_row_typography(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    editor_font_family: &SharedString,
    ui_scale_percent: u32,
) -> MarkdownPreviewRowTypography {
    let text_color = markdown_preview_row_text_color(theme, row);
    let scaled = |value: f32| markdown_preview_scaled_value(value, ui_scale_percent);
    match row.kind {
        MarkdownPreviewRowKind::Heading { level: 1 } => MarkdownPreviewRowTypography {
            font_size: scaled(28.0),
            line_height: scaled(28.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::Heading { level: 2 } => MarkdownPreviewRowTypography {
            font_size: scaled(24.0),
            line_height: scaled(24.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::Heading { level: 3 } => MarkdownPreviewRowTypography {
            font_size: scaled(20.0),
            line_height: scaled(22.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::Heading { level: 4 } => MarkdownPreviewRowTypography {
            font_size: scaled(18.0),
            line_height: scaled(20.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::Heading { level: 5 } => MarkdownPreviewRowTypography {
            font_size: scaled(16.0),
            line_height: scaled(18.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::Heading { level: 6 } => MarkdownPreviewRowTypography {
            font_size: scaled(14.0),
            line_height: scaled(16.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::DetailsSummary => MarkdownPreviewRowTypography {
            font_size: scaled(MARKDOWN_PREVIEW_BASE_FONT_PX),
            line_height: scaled(28.0),
            font_weight: Some(FontWeight::BOLD),
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::ListItem { .. } => MarkdownPreviewRowTypography {
            font_size: scaled(MARKDOWN_PREVIEW_BASE_FONT_PX),
            line_height: scaled(MARKDOWN_PREVIEW_BASE_LINE_HEIGHT_PX),
            font_weight: None,
            font_family: None,
            text_color,
        },
        MarkdownPreviewRowKind::CodeLine { .. } => MarkdownPreviewRowTypography {
            font_size: scaled(12.0),
            line_height: scaled(18.0),
            font_weight: None,
            font_family: Some(editor_font_family.clone()),
            text_color,
        },
        MarkdownPreviewRowKind::TableRow { is_header } => MarkdownPreviewRowTypography {
            font_size: scaled(12.0),
            line_height: scaled(18.0),
            font_weight: is_header.then_some(FontWeight::BOLD),
            font_family: Some(editor_font_family.clone()),
            text_color,
        },
        MarkdownPreviewRowKind::PlainFallback => MarkdownPreviewRowTypography {
            font_size: scaled(12.0),
            line_height: scaled(18.0),
            font_weight: None,
            font_family: Some(editor_font_family.clone()),
            text_color,
        },
        _ => MarkdownPreviewRowTypography {
            font_size: scaled(MARKDOWN_PREVIEW_BASE_FONT_PX),
            line_height: scaled(MARKDOWN_PREVIEW_BASE_LINE_HEIGHT_PX),
            font_weight: None,
            font_family: None,
            text_color,
        },
    }
}

pub(in crate::view) fn markdown_preview_code_background(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        with_alpha(theme.colors.surface.raised, 0.88)
    } else {
        with_alpha(theme.colors.surface.panel, 0.86)
    }
}

pub(in crate::view) fn markdown_preview_row_horizontal_padding(
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
) -> MarkdownPreviewRowHorizontalPadding {
    let indent_steps = f32::from(row.indent_level.saturating_sub(1));
    let default_left_px = markdown_preview_scaled_value(
        MARKDOWN_PREVIEW_CONTENT_PAD_X_PX + indent_steps * MARKDOWN_PREVIEW_INDENT_STEP_PX,
        ui_scale_percent,
    );

    match row.kind {
        MarkdownPreviewRowKind::CodeLine { .. } => MarkdownPreviewRowHorizontalPadding {
            // Fenced code blocks ignore surrounding list indentation but keep
            // a small edge gap so the boxed shell does not touch the preview edge.
            left_px: markdown_preview_scaled_value(
                MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX,
                ui_scale_percent,
            ),
            right_px: markdown_preview_scaled_value(
                MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX,
                ui_scale_percent,
            ),
        },
        _ => MarkdownPreviewRowHorizontalPadding {
            left_px: default_left_px,
            right_px: markdown_preview_scaled_value(
                MARKDOWN_PREVIEW_CONTENT_PAD_X_PX,
                ui_scale_percent,
            ),
        },
    }
}

/// The wash a row carries in its own right: a diff change hint, an alert's
/// tint, or the warning band on a line the parser could not interpret.
pub(in crate::view) fn markdown_preview_row_background(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> Option<gpui::Rgba> {
    use MarkdownChangeHint as Hint;
    use MarkdownPreviewRowKind as Kind;

    match row.change_hint {
        Hint::Added => Some(with_alpha(
            theme.colors.status.success.foreground,
            if theme.is_dark { 0.18 } else { 0.12 },
        )),
        Hint::Removed => Some(with_alpha(
            theme.colors.status.danger.foreground,
            if theme.is_dark { 0.16 } else { 0.10 },
        )),
        Hint::Modified => Some(with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.18 } else { 0.10 },
        )),
        Hint::None => {
            if let Some(alert_kind) = row.alert_kind {
                return Some(with_alpha(
                    markdown_preview_alert_color(theme, alert_kind),
                    if theme.is_dark { 0.10 } else { 0.06 },
                ));
            }

            match row.kind {
                Kind::PlainFallback => Some(with_alpha(
                    theme.colors.status.warning.foreground,
                    if theme.is_dark { 0.08 } else { 0.06 },
                )),
                _ => None,
            }
        }
    }
}
