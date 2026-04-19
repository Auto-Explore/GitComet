use super::diff_canvas;
use super::diff_text::*;
use super::history_canvas;
use super::*;

use crate::view::markdown_preview::{
    MarkdownAlertKind, MarkdownChangeHint, MarkdownInlineStyle, MarkdownPreviewDocument,
    MarkdownPreviewRow, MarkdownPreviewRowKind,
};
use crate::view::perf::{self, ViewPerfRenderLane, ViewPerfSpan};
use rustc_hash::FxHasher;

#[derive(Clone)]
struct WorktreePreviewPreparedSyntaxSource {
    document_text: Arc<str>,
    line_starts: Arc<[usize]>,
    document: rows::PreparedDiffSyntaxDocument,
}

fn worktree_preview_streamed_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    line_ix: usize,
    query: &SharedString,
    language: Option<rows::DiffSyntaxLanguage>,
    syntax_mode: rows::DiffSyntaxMode,
    prepared_syntax_source: Option<&WorktreePreviewPreparedSyntaxSource>,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    diff_canvas::is_streamable_diff_text(&raw_text).then(|| {
        let syntax = match (language, prepared_syntax_source) {
            (Some(language), Some(prepared_syntax_source)) => {
                diff_canvas::StreamedDiffTextSyntaxSource::Prepared {
                    document_text: Arc::clone(&prepared_syntax_source.document_text),
                    line_starts: Arc::clone(&prepared_syntax_source.line_starts),
                    document: prepared_syntax_source.document,
                    language,
                    line_ix,
                }
            }
            (Some(language), None) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic {
                language,
                mode: syntax_mode,
            },
            (None, _) => diff_canvas::StreamedDiffTextSyntaxSource::None,
        };
        diff_canvas::StreamedDiffTextPaintSpec {
            raw_text,
            query: query.clone(),
            word_ranges: Arc::from([]),
            word_color: None,
            syntax,
        }
    })
}

impl MainPaneView {
    pub(in super::super) fn render_worktree_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
        let query = this.diff_search_query_or_empty();
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();

        let theme = this.theme;
        let Some(path) = this.worktree_preview_path.as_ref() else {
            return Vec::new();
        };
        let Some(line_count) = this.worktree_preview_line_count() else {
            return Vec::new();
        };

        let should_clear_cache = match this.worktree_preview_segments_cache_path.as_ref() {
            Some(p) => p != path,
            None => true,
        };
        if should_clear_cache {
            this.worktree_preview_segments_cache_path = Some(path.clone());
            this.worktree_preview_syntax_language = diff_syntax_language_for_path(path);
            this.worktree_preview_segments_cache.clear();
        }

        let language = this.worktree_preview_syntax_language;
        let syntax_document = this.worktree_preview_prepared_syntax_document();
        let syntax_mode = syntax_mode_for_prepared_document(syntax_document);
        let prepared_syntax_source = match syntax_document {
            Some(document) if !this.worktree_preview_text.is_empty() => {
                Some(WorktreePreviewPreparedSyntaxSource {
                    document_text: Arc::from(this.worktree_preview_text.as_ref()),
                    line_starts: Arc::clone(&this.worktree_preview_line_starts),
                    document,
                })
            }
            _ => None,
        };
        let highlight_palette = syntax_highlight_palette(theme);

        let bar_color = worktree_preview_bar_color(this, theme);
        let defer_cache_write = this.worktree_preview_cache_write_blocked_until_rev
            == Some(this.worktree_preview_content_rev);

        range
            .take_while(|ix| *ix < line_count)
            .map(|ix| {
                let Some(raw_text) = this.worktree_preview_line_raw_text(ix) else {
                    return diff_canvas::worktree_preview_row_canvas(
                        theme,
                        cx.entity(),
                        ui_scale_percent,
                        ix,
                        min_width,
                        bar_color,
                        line_number_string(u32::try_from(ix + 1).ok()),
                        None,
                        None,
                    );
                };
                let streamed_spec = worktree_preview_streamed_spec(
                    raw_text.clone(),
                    ix,
                    &query,
                    language,
                    syntax_mode,
                    prepared_syntax_source.as_ref(),
                );
                let mut pending_styled = None;
                if streamed_spec.is_none() && this.worktree_preview_segments_cache_get(ix).is_none() {
                    let line = raw_text.as_ref();
                    let (styled, is_pending) =
                        build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                            theme,
                            &highlight_palette,
                            PreparedDiffTextBuildRequest {
                                build: DiffTextBuildRequest {
                                    text: line,
                                    word_ranges: &[],
                                    query: query.as_ref(),
                                    syntax: DiffSyntaxConfig {
                                        language,
                                        mode: syntax_mode,
                                    },
                                    word_color: None,
                                },
                                prepared_line: PreparedDiffSyntaxLine {
                                    document: syntax_document,
                                    line_ix: ix,
                                },
                            },
                        )
                        .into_parts();
                    if is_pending {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                        pending_styled = Some(styled);
                    } else {
                        if defer_cache_write {
                            pending_styled = Some(styled);
                        } else {
                            this.worktree_preview_segments_cache_set(ix, styled);
                        }
                    }
                }

                let cached_styled = this.worktree_preview_segments_cache_get(ix);
                let styled = pending_styled.as_ref().or(cached_styled);

                let line_no = line_number_string(u32::try_from(ix + 1).ok());
                diff_canvas::worktree_preview_row_canvas(
                    theme,
                    cx.entity(),
                    ui_scale_percent,
                    ix,
                    min_width,
                    bar_color,
                    line_no,
                    styled,
                    streamed_spec,
                )
            })
            .collect()
    }

    pub(in super::super) fn render_markdown_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let Loadable::Ready(document) = &this.worktree_markdown_preview else {
            return Vec::new();
        };
        let document = Arc::clone(document);
        let bar_color = worktree_preview_bar_color(this, theme);
        let viewport_width = this
            .worktree_preview_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .width
            .max(px(0.0));
        this.update_markdown_preview_horizontal_min_width(
            document.as_ref(),
            range.clone(),
            bar_color,
            editor_font_family.as_ref(),
            window,
            cx,
        );
        render_markdown_preview_document_rows(
            document.as_ref(),
            range,
            &MarkdownPreviewRenderContext {
                theme,
                bar_color,
                min_width: this.diff_horizontal_min_width.max(viewport_width),
                editor_font_family,
                ui_scale_percent,
                view: Some(cx.entity().clone()),
                text_region: DiffTextRegion::Inline,
            },
        )
    }

    pub(in super::super) fn render_markdown_diff_left_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let Loadable::Ready(preview) = &this.file_markdown_preview else {
            return Vec::new();
        };
        let preview = Arc::clone(preview);
        let viewport_width = this
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .width
            .max(px(0.0));
        this.update_markdown_preview_horizontal_min_width(
            &preview.old,
            range.clone(),
            None,
            editor_font_family.as_ref(),
            window,
            cx,
        );
        let region = match this.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => DiffTextRegion::SplitLeft,
        };
        render_markdown_preview_document_rows(
            &preview.old,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                bar_color: None,
                min_width: this.diff_horizontal_min_width.max(viewport_width),
                editor_font_family,
                ui_scale_percent,
                view: Some(cx.entity().clone()),
                text_region: region,
            },
        )
    }

    pub(in super::super) fn render_markdown_diff_inline_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let Loadable::Ready(preview) = &this.file_markdown_preview else {
            return Vec::new();
        };
        let preview = Arc::clone(preview);
        let viewport_width = this
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .width
            .max(px(0.0));
        this.update_markdown_preview_horizontal_min_width(
            &preview.inline,
            range.clone(),
            None,
            editor_font_family.as_ref(),
            window,
            cx,
        );
        render_markdown_preview_document_rows(
            &preview.inline,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                bar_color: None,
                min_width: this.diff_horizontal_min_width.max(viewport_width),
                editor_font_family,
                ui_scale_percent,
                view: Some(cx.entity().clone()),
                text_region: DiffTextRegion::Inline,
            },
        )
    }

    pub(in super::super) fn render_markdown_diff_right_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let Loadable::Ready(preview) = &this.file_markdown_preview else {
            return Vec::new();
        };
        let preview = Arc::clone(preview);
        let viewport_width = this
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .width
            .max(px(0.0));
        this.update_markdown_preview_horizontal_min_width(
            &preview.new,
            range.clone(),
            None,
            editor_font_family.as_ref(),
            window,
            cx,
        );
        render_markdown_preview_document_rows(
            &preview.new,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                bar_color: None,
                min_width: this.diff_horizontal_min_width.max(viewport_width),
                editor_font_family,
                ui_scale_percent,
                view: Some(cx.entity().clone()),
                text_region: DiffTextRegion::SplitRight,
            },
        )
    }

    pub(in crate::view) fn update_markdown_preview_horizontal_min_width(
        &mut self,
        document: &MarkdownPreviewDocument,
        range: Range<usize>,
        bar_color: Option<gpui::Rgba>,
        editor_font_family: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut min_width = self.diff_horizontal_min_width;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        for row in range.filter_map(|ix| document.rows.get(ix)) {
            let required = markdown_preview_row_required_width(
                window,
                self.theme,
                row,
                bar_color,
                editor_font_family,
                ui_scale_percent,
            );
            if required > min_width {
                min_width = required;
            }
        }

        if min_width > self.diff_horizontal_min_width {
            self.diff_horizontal_min_width = min_width;
            cx.notify();
        }
    }
}

const MARKDOWN_PREVIEW_ROW_HEIGHT_PX: f32 = 28.0;
const MARKDOWN_PREVIEW_BASE_FONT_PX: f32 = 13.0;
const MARKDOWN_PREVIEW_BASE_LINE_HEIGHT_PX: f32 = 20.0;
const MARKDOWN_PREVIEW_CONTENT_PAD_X_PX: f32 = 18.0;
const MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX: f32 = 8.0;
const MARKDOWN_PREVIEW_INDENT_STEP_PX: f32 = 24.0;
const MARKDOWN_PREVIEW_CHANGE_BAR_WIDTH_PX: f32 = 3.0;
const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX: f32 = 4.0;
const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_GAP_PX: f32 = 8.0;
const MARKDOWN_PREVIEW_BLOCKQUOTE_GUTTER_MARGIN_RIGHT_PX: f32 = 12.0;
const MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX: f32 = 22.0;
const MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX: f32 = 10.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX: f32 = 11.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX: f32 = 6.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX: f32 = 10.0;
const MARKDOWN_PREVIEW_SHELL_PAD_X_PX: f32 = 12.0;
const MARKDOWN_PREVIEW_CODE_BORDER_PX: f32 = 1.0;

fn markdown_preview_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

fn markdown_preview_scaled_value(value: f32, ui_scale_percent: u32) -> f32 {
    let scaled: f32 = markdown_preview_scaled_px(value, ui_scale_percent).into();
    scaled
}

fn markdown_preview_row_height(ui_scale_percent: u32) -> Pixels {
    markdown_preview_scaled_px(MARKDOWN_PREVIEW_ROW_HEIGHT_PX, ui_scale_percent)
}

struct MarkdownPreviewRowTypography {
    font_size: f32,
    line_height: f32,
    font_weight: Option<FontWeight>,
    font_family: Option<SharedString>,
    text_color: gpui::Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MarkdownPreviewRowLayout {
    top_inset_px: f32,
    bottom_inset_px: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MarkdownPreviewRowHorizontalPadding {
    left_px: f32,
    right_px: f32,
}

pub(super) struct MarkdownPreviewRenderContext {
    pub(super) theme: AppTheme,
    pub(super) bar_color: Option<gpui::Rgba>,
    pub(super) min_width: Pixels,
    pub(super) editor_font_family: SharedString,
    pub(super) ui_scale_percent: u32,
    pub(super) view: Option<Entity<MainPaneView>>,
    pub(super) text_region: DiffTextRegion,
}

pub(super) fn render_markdown_preview_document_rows(
    document: &MarkdownPreviewDocument,
    range: Range<usize>,
    context: &MarkdownPreviewRenderContext,
) -> Vec<AnyElement> {
    let requested_rows = range.len();
    let start = range.start.min(document.rows.len());
    let end = range.end.min(document.rows.len());
    let mut rows = Vec::with_capacity(end.saturating_sub(start));
    for (offset, row) in document.rows[start..end].iter().enumerate() {
        rows.push(markdown_preview_row_element(row, start + offset, context));
    }
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        requested_rows,
        rows.len(),
    );
    rows
}

struct MarkdownPreviewSharedHighlightsText {
    text: SharedString,
    highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    inner: Option<gpui::StyledText>,
}

impl MarkdownPreviewSharedHighlightsText {
    fn new(text: SharedString, highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>) -> Self {
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

fn markdown_preview_row_element(
    row: &MarkdownPreviewRow,
    row_ix: usize,
    context: &MarkdownPreviewRenderContext,
) -> AnyElement {
    let theme = context.theme;
    let bar_color = context.bar_color;
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

    let row_layout = markdown_preview_row_layout(row, ui_scale_percent);
    let typography =
        markdown_preview_row_typography(theme, row, &context.editor_font_family, ui_scale_percent);
    let styled = markdown_preview_row_styled_text(theme, row);
    let horizontal_padding = markdown_preview_row_horizontal_padding(row, ui_scale_percent);
    let marker = markdown_preview_row_marker(row);
    let alert_title = markdown_preview_alert_title_label(row);

    // Rows that need a content_shell wrapper for border/background styling.
    let needs_content_shell = matches!(
        row.kind,
        MarkdownPreviewRowKind::Heading { level: 1 | 2 }
            | MarkdownPreviewRowKind::CodeLine { .. }
            | MarkdownPreviewRowKind::TableRow { .. }
            | MarkdownPreviewRowKind::PlainFallback
    );
    let flatten_shell_text_directly =
        !is_interactive && needs_content_shell && marker.is_none() && alert_title.is_none();

    let build_content_shell = || {
        let mut content_shell = div()
            .flex_grow()
            .min_w(px(0.0))
            .w_full()
            .h_full()
            .relative()
            .flex()
            .items_center();
        content_shell = match row.kind {
            MarkdownPreviewRowKind::Heading { level: 1 | 2 } => {
                content_shell.border_b_1().border_color(with_alpha(
                    theme.colors.border,
                    if theme.is_dark { 0.85 } else { 0.92 },
                ))
            }
            MarkdownPreviewRowKind::CodeLine { is_first, is_last } => {
                let code_border =
                    with_alpha(theme.colors.border, if theme.is_dark { 0.90 } else { 0.80 });
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
                        theme.colors.surface_bg_elevated,
                        if theme.is_dark { 0.64 } else { 0.86 },
                    )
                } else {
                    with_alpha(
                        theme.colors.surface_bg_elevated,
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
                        theme.colors.border,
                        if theme.is_dark { 0.88 } else { 0.86 },
                    ))
            }
            MarkdownPreviewRowKind::PlainFallback => content_shell
                .px(markdown_preview_scaled_px(
                    MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
                    ui_scale_percent,
                ))
                .bg(with_alpha(
                    theme.colors.warning,
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

    let mut row_body = if flatten_shell_text_directly {
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
            .flex_grow()
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
            content = content.debug_selector(|| format!("markdown_preview_text_box_{row_ix}"));
        }

        if let Some(font_weight) = typography.font_weight {
            content = content.font_weight(font_weight);
        }
        if let Some(font_family) = typography.font_family.clone() {
            content = content.font_family(font_family);
        }
        if let Some(view) = context.view.clone() {
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
                        text: row.text.clone(),
                    }),
            );
        }

        let body = match row.kind {
            MarkdownPreviewRowKind::ThematicBreak => div()
                .flex_grow()
                .min_w(px(0.0))
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .child(div().w_full().h(px(1.0)).bg(with_alpha(
                    theme.colors.border,
                    if theme.is_dark { 0.92 } else { 0.88 },
                ))),
            _ if marker.is_none() && alert_title.is_none() => {
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
                    .flex_grow()
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
                            .text_color(theme.colors.text_muted)
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
                line.child(text)
            }
        };

        if needs_content_shell {
            build_content_shell().child(body)
        } else {
            body
        }
    };
    let needs_row_content_wrapper = bar_color.is_some() || row.blockquote_level > 0;
    if !needs_row_content_wrapper {
        row_body = if needs_content_shell {
            row_body
                .ml(px(horizontal_padding.left_px))
                .mr(px(horizontal_padding.right_px))
        } else {
            row_body
                .pl(px(horizontal_padding.left_px))
                .pr(px(horizontal_padding.right_px))
        };
    }

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
            .when_some(bar_color, |container, color| {
                container.child(
                    div()
                        .h_full()
                        .w(markdown_preview_scaled_px(
                            MARKDOWN_PREVIEW_CHANGE_BAR_WIDTH_PX,
                            ui_scale_percent,
                        ))
                        .bg(color),
                )
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
                        this.handle_diff_text_mouse_down(
                            row_ix,
                            text_region,
                            position,
                            click_count,
                            cx,
                        );
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
        if needs_row_content_wrapper {
            let mut row_content = div()
                .flex_grow()
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
            row_container
                .child(row_content.child(row_body))
                .into_any_element()
        } else {
            row_container.child(row_body).into_any_element()
        }
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
            .when_some(bar_color, |container, color| {
                container.child(
                    div()
                        .h_full()
                        .w(markdown_preview_scaled_px(
                            MARKDOWN_PREVIEW_CHANGE_BAR_WIDTH_PX,
                            ui_scale_percent,
                        ))
                        .bg(color),
                )
            })
            .min_w(min_width);
        if needs_row_content_wrapper {
            let mut row_content = div()
                .flex_grow()
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
            row_container
                .child(row_content.child(row_body))
                .into_any_element()
        } else {
            row_container.child(row_body).into_any_element()
        }
    }
}

fn markdown_preview_row_required_width(
    window: &mut Window,
    theme: AppTheme,
    row: &MarkdownPreviewRow,
    bar_color: Option<gpui::Rgba>,
    editor_font_family: &str,
    ui_scale_percent: u32,
) -> Pixels {
    if matches!(row.kind, MarkdownPreviewRowKind::Spacer) {
        return px(0.0);
    }

    let editor_font_family: SharedString = editor_font_family.to_owned().into();
    let typography =
        markdown_preview_row_typography(theme, row, &editor_font_family, ui_scale_percent);
    let default_font_family = window.text_style().font_family.clone();
    let resolved_font_family = typography
        .font_family
        .clone()
        .unwrap_or_else(|| default_font_family.clone());
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

        let horizontal_padding = markdown_preview_row_horizontal_padding(row, ui_scale_percent);
        let mut width = px(horizontal_padding.left_px + horizontal_padding.right_px);
        width += text_width;

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
            width +=
                markdown_preview_scaled_px(MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, ui_scale_percent);
        }

        if let Some(alert_title) = markdown_preview_alert_title_label(row) {
            let alert_width = markdown_preview_shape_text_width(
                window,
                alert_title,
                markdown_preview_scaled_value(
                    MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX,
                    ui_scale_percent,
                ),
                FontWeight::BOLD,
                None,
                &[],
            );
            width += alert_width
                + markdown_preview_scaled_px(
                    MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX * 2.0,
                    ui_scale_percent,
                );
            width +=
                markdown_preview_scaled_px(MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX, ui_scale_percent);
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

        u32::from(width.round())
    });

    let mut width = px(base_width as f32);
    if bar_color.is_some() {
        width += markdown_preview_scaled_px(MARKDOWN_PREVIEW_CHANGE_BAR_WIDTH_PX, ui_scale_percent);
    }
    width
}

fn markdown_preview_row_width_cache_key(
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

fn markdown_preview_width_affecting_highlights(
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

fn markdown_preview_shape_text_width(
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

    let runs = if highlights.is_empty() {
        vec![style.to_run(text.len())]
    } else {
        markdown_preview_text_runs(text.as_ref(), &style, highlights)
    };

    window
        .text_system()
        .shape_line(text, px(font_size_px), &runs, None)
        .width
}

fn markdown_preview_text_runs(
    text: &str,
    default_style: &gpui::TextStyle,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn worktree_preview_bar_color(this: &MainPaneView, theme: AppTheme) -> Option<gpui::Rgba> {
    let highlight_deleted_file = this.deleted_file_preview_abs_path().is_some();
    let highlight_new_file = this.untracked_worktree_preview_path().is_some()
        || this.added_file_preview_abs_path().is_some()
        || this.diff_preview_is_new_file;
    if highlight_deleted_file {
        Some(theme.colors.danger)
    } else if highlight_new_file {
        Some(theme.colors.success)
    } else {
        None
    }
}

fn markdown_preview_row_styled_text(
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

fn markdown_preview_row_marker(row: &MarkdownPreviewRow) -> Option<SharedString> {
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

fn markdown_preview_alert_title_label(row: &MarkdownPreviewRow) -> Option<&'static str> {
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

fn markdown_preview_alert_color(theme: AppTheme, kind: MarkdownAlertKind) -> gpui::Rgba {
    match kind {
        MarkdownAlertKind::Note => theme.colors.accent,
        MarkdownAlertKind::Tip => theme.colors.success,
        MarkdownAlertKind::Important => with_alpha(theme.colors.accent, 0.85),
        MarkdownAlertKind::Warning => theme.colors.warning,
        MarkdownAlertKind::Caution => theme.colors.danger,
    }
}

fn markdown_preview_blockquote_gutter(
    theme: AppTheme,
    blockquote_level: u8,
    alert_kind: Option<MarkdownAlertKind>,
    ui_scale_percent: u32,
) -> Option<AnyElement> {
    if blockquote_level == 0 {
        return None;
    }

    let quote_bar_color = with_alpha(theme.colors.border, if theme.is_dark { 0.96 } else { 0.86 });
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

fn markdown_preview_inline_highlight(
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
                    theme.colors.active_section,
                    if theme.is_dark { 0.75 } else { 0.55 },
                )
                .into(),
            ),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Strikethrough => gpui::HighlightStyle {
            color: Some(theme.colors.text_muted.into()),
            strikethrough: Some(gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.colors.text_muted.into()),
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Link => gpui::HighlightStyle {
            color: Some(theme.colors.accent.into()),
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.accent.into()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Underline => gpui::HighlightStyle {
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.text.into()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
    }
}

fn markdown_preview_row_text_color(theme: AppTheme, row: &MarkdownPreviewRow) -> gpui::Rgba {
    if row.alert_kind.is_some() {
        return theme.colors.text;
    }

    match row.kind {
        MarkdownPreviewRowKind::Heading { level: 6 } | MarkdownPreviewRowKind::BlockquoteLine => {
            theme.colors.text_muted
        }
        MarkdownPreviewRowKind::Heading { .. } => theme.colors.text,
        MarkdownPreviewRowKind::ThematicBreak => theme.colors.text_muted,
        MarkdownPreviewRowKind::PlainFallback => theme.colors.warning,
        _ => theme.colors.text,
    }
}

fn markdown_preview_row_layout(
    row: &MarkdownPreviewRow,
    ui_scale_percent: u32,
) -> MarkdownPreviewRowLayout {
    let scaled = |value: f32| markdown_preview_scaled_value(value, ui_scale_percent);
    match row.kind {
        MarkdownPreviewRowKind::Heading { level: 1 | 2 } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(0.0),
            bottom_inset_px: scaled(0.0),
        },
        MarkdownPreviewRowKind::Heading { level: 3 } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(2.0),
            bottom_inset_px: scaled(4.0),
        },
        MarkdownPreviewRowKind::Heading { .. } => MarkdownPreviewRowLayout {
            top_inset_px: scaled(2.0),
            bottom_inset_px: scaled(6.0),
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

fn markdown_preview_row_typography(
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

fn markdown_preview_code_background(theme: AppTheme) -> gpui::Rgba {
    if theme.is_dark {
        with_alpha(theme.colors.surface_bg_elevated, 0.88)
    } else {
        with_alpha(theme.colors.surface_bg, 0.86)
    }
}

fn markdown_preview_row_horizontal_padding(
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

fn markdown_preview_row_background(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> Option<gpui::Rgba> {
    use MarkdownChangeHint as Hint;
    use MarkdownPreviewRowKind as Kind;

    match row.change_hint {
        Hint::Added => Some(with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.18 } else { 0.12 },
        )),
        Hint::Removed => Some(with_alpha(
            theme.colors.danger,
            if theme.is_dark { 0.16 } else { 0.10 },
        )),
        Hint::Modified => Some(with_alpha(
            theme.colors.accent,
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
                    theme.colors.warning,
                    if theme.is_dark { 0.08 } else { 0.06 },
                )),
                _ => None,
            }
        }
    }
}

impl HistoryView {
    pub(in super::super) fn render_history_table_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let (show_working_tree_summary_row, worktree_counts) =
            this.ensure_history_worktree_summary_cache();
        let stash_ids = this.ensure_history_stash_ids_cache();

        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };

        let theme = this.theme;
        let col_branch = this.history_col_branch;
        let col_graph = this.history_col_graph;
        let col_author = this.history_col_author;
        let col_date = this.history_col_date;
        let col_sha = this.history_col_sha;
        let ui_scale = this.ui_scale();
        let (show_graph, show_author, show_date, show_sha) = this.history_visible_columns();

        let page = match &repo.log {
            Loadable::Ready(page) => Some(page),
            _ => None,
        };
        let cache = this
            .history_cache
            .as_ref()
            .filter(|c| c.request.repo_id == repo.id);
        let worktree_node_color = cache
            .and_then(|c| c.graph_rows.first())
            .and_then(|row| {
                row.lanes_now
                    .first()
                    .map(|lane| history_graph::lane_color(theme, lane.color_ix))
            })
            .unwrap_or(theme.colors.accent);

        range
            .filter_map(|list_ix| {
                if show_working_tree_summary_row && list_ix == 0 {
                    let selected = repo.history_state.selected_commit.is_none();
                    return Some(working_tree_summary_history_row(
                        theme,
                        ui_scale,
                        col_branch,
                        col_graph,
                        col_author,
                        col_date,
                        col_sha,
                        show_graph,
                        show_author,
                        show_date,
                        show_sha,
                        worktree_node_color,
                        repo.id,
                        selected,
                        worktree_counts,
                        cx,
                    ));
                }

                let offset = usize::from(show_working_tree_summary_row);
                let visible_ix = list_ix.checked_sub(offset)?;

                let page = page?;
                let cache = cache?;

                let commit_ix = cache.visible_indices.get(visible_ix)?;
                let commit = page.commits.get(commit_ix)?;
                cache.graph_rows.get(visible_ix)?;
                let row_vm = cache.commit_row_vms.get(visible_ix)?;
                let connect_from_top_col =
                    (show_working_tree_summary_row && visible_ix == 0).then_some(0);
                let selected = repo.history_state.selected_commit.as_ref() == Some(&commit.id);
                let selected_branch_entry_text = this.selected_branch_entry_text_for_history_row(
                    repo.id,
                    row_vm.is_head,
                    selected,
                );
                let show_graph_color_marker = repo.history_state.history_scope
                    == gitcomet_core::domain::LogScope::AllBranches;
                let is_stash_node = row_vm.is_stash
                    || stash_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&commit.id));

                Some(history_table_row(
                    theme,
                    ui_scale,
                    col_branch,
                    col_graph,
                    col_author,
                    col_date,
                    col_sha,
                    show_graph,
                    show_author,
                    show_date,
                    show_sha,
                    show_graph_color_marker,
                    list_ix,
                    repo.id,
                    commit,
                    Arc::clone(&cache.graph_rows),
                    visible_ix,
                    connect_from_top_col,
                    Arc::clone(&row_vm.tag_names),
                    row_vm.branches_text.clone(),
                    selected_branch_entry_text,
                    row_vm.author.clone(),
                    row_vm.summary.clone(),
                    row_vm.when.resolve(&cache.request),
                    row_vm.short_sha.resolve(),
                    selected,
                    row_vm.is_head,
                    is_stash_node,
                    this.active_context_menu_invoker.as_ref(),
                    cx,
                ))
            })
            .collect()
    }
}

const HISTORY_ROW_HEIGHT_PX: f32 = 24.0;

fn history_row_height(ui_scale: ui_scale::UiScale) -> Pixels {
    ui_scale.px(HISTORY_ROW_HEIGHT_PX)
}

fn history_selected_branch_entry_range(
    branches_text: &str,
    selected_branch_entry_text: &str,
) -> Option<Range<usize>> {
    let mut start = 0usize;
    for part in branches_text.split(", ") {
        let end = start + part.len();
        if part == selected_branch_entry_text {
            return Some(start..end);
        }
        start = end + 2;
    }
    None
}

fn history_branch_text_highlights(
    branches_text: &SharedString,
    selected_branch_entry_text: Option<&SharedString>,
    theme: AppTheme,
) -> Arc<[(Range<usize>, gpui::HighlightStyle)]> {
    let Some(selected_branch_entry_text) = selected_branch_entry_text else {
        return Arc::from([]);
    };
    let Some(range) =
        history_selected_branch_entry_range(branches_text.as_ref(), selected_branch_entry_text)
    else {
        return Arc::from([]);
    };

    vec![(
        range,
        gpui::HighlightStyle {
            color: Some(selected_branch_label_color(theme).into()),
            ..gpui::HighlightStyle::default()
        },
    )]
    .into()
}

#[allow(clippy::too_many_arguments)]
fn history_table_row(
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    show_graph_color_marker: bool,
    ix: usize,
    repo_id: RepoId,
    commit: &Commit,
    graph_rows: Arc<[history_graph::GraphRow]>,
    graph_row_ix: usize,
    connect_from_top_col: Option<usize>,
    tag_names: Arc<[SharedString]>,
    branches_text: SharedString,
    selected_branch_entry_text: Option<SharedString>,
    author: SharedString,
    summary: SharedString,
    when: SharedString,
    short_sha: SharedString,
    selected: bool,
    is_head: bool,
    is_stash_node: bool,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let context_menu_invoker: SharedString =
        format!("history_commit_menu_{}_{}", repo_id.0, commit.id.as_ref()).into();
    let context_menu_active = active_context_menu_invoker == Some(&context_menu_invoker);
    let branch_highlights =
        history_branch_text_highlights(&branches_text, selected_branch_entry_text.as_ref(), theme);
    let commit_row = history_canvas::history_commit_row_canvas(
        theme,
        cx.entity(),
        ix,
        repo_id,
        commit.id.clone(),
        col_branch,
        col_graph,
        col_author,
        col_date,
        col_sha,
        show_graph,
        show_author,
        show_date,
        show_sha,
        show_graph_color_marker,
        is_stash_node,
        connect_from_top_col,
        graph_rows,
        graph_row_ix,
        tag_names,
        branches_text,
        branch_highlights,
        author,
        summary,
        when,
        short_sha,
    );

    let commit_id = commit.id.clone();
    let row_height = history_row_height(ui_scale);
    let mut row = div()
        .id(ix)
        .relative()
        .h(row_height)
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.active)
            } else {
                s.bg(theme.colors.hover)
            }
        })
        .active(move |s| s.bg(theme.colors.active))
        .child(commit_row)
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.store.dispatch(Msg::SelectCommit {
                repo_id,
                commit_id: commit_id.clone(),
            });
            cx.notify();
        }));

    if selected {
        row = row.bg(with_alpha(theme.colors.accent, 0.15));
    }
    if context_menu_active {
        row = row.bg(theme.colors.active);
    }

    if is_head {
        let thickness = ui_scale.px(1.0);
        let color = with_alpha(theme.colors.accent, 0.90);
        row = row
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(thickness)
                    .bg(color),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(thickness)
                    .bg(color),
            );
    }

    row.into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn working_tree_summary_history_row(
    theme: AppTheme,
    ui_scale: ui_scale::UiScale,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    node_color: gpui::Rgba,
    repo_id: RepoId,
    selected: bool,
    counts: (usize, usize, usize),
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let scaled_px = |value| ui_scale.px(value);
    let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, scaled_px(12.0)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child(count.to_string()),
            )
            .into_any_element()
    };

    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.warning,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count("icons/plus.svg", theme.colors.success, added));
    }
    if deleted > 0 {
        parts.push(icon_count("icons/minus.svg", theme.colors.danger, deleted));
    }

    let node_fill = theme.colors.window_bg;
    let circle = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _cx| {
            use gpui::{PathBuilder, fill, point, size};
            let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
            let scaled_px = |value| px(value * design_scale_factor);
            let r = scaled_px(3.0);
            let border = scaled_px(1.0);
            let outer = r + border;
            let margin_x = scaled_px(HISTORY_GRAPH_MARGIN_X_PX);
            let col_gap = scaled_px(HISTORY_GRAPH_COL_GAP_PX);
            let node_x = margin_x + col_gap * 0.0;
            let center = point(
                bounds.left() + node_x,
                bounds.top() + bounds.size.height / 2.0,
            );

            // Connect the working tree node into the history graph below.
            let stroke_width = scaled_px(1.6);
            let mut path = PathBuilder::stroke(stroke_width);
            path.move_to(point(center.x, center.y));
            path.line_to(point(center.x, bounds.bottom()));
            if let Ok(p) = path.build() {
                window.paint_path(p, node_color);
            }

            window.paint_quad(
                fill(
                    gpui::Bounds::new(
                        point(center.x - outer, center.y - outer),
                        size(outer * 2.0, outer * 2.0),
                    ),
                    node_color,
                )
                .corner_radii(outer.min(scaled_px(2.0))),
            );
            window.paint_quad(
                fill(
                    gpui::Bounds::new(point(center.x - r, center.y - r), size(r * 2.0, r * 2.0)),
                    node_fill,
                )
                .corner_radii(r.min(scaled_px(2.0))),
            );
        },
    )
    .w_full()
    .h_full()
    .cursor(CursorStyle::PointingHand);

    let mut row = div()
        .id(("history_worktree_summary", repo_id.0))
        .h(history_row_height(ui_scale))
        .flex()
        .w_full()
        .items_center()
        .px_2()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.hover))
        .active(move |s| s.bg(theme.colors.active))
        .child(
            div()
                .w(col_branch)
                .text_xs()
                .text_color(theme.colors.text_muted)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(div()),
        )
        .when(show_graph, |row| {
            row.child(
                div()
                    .w(col_graph)
                    .h_full()
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(circle),
            )
        })
        .child({
            let mut summary = div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .px(cell_pad_x);
            summary = summary.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child("Uncommitted changes"),
            );
            if !parts.is_empty() {
                summary = summary.child(div().flex().items_center().gap_2().children(parts));
            }
            summary
        })
        .when(show_author, |row| row.child(div().w(col_author)))
        .when(show_date, |row| {
            row.child(
                div()
                    .w(col_date)
                    .flex()
                    .justify_end()
                    .px(cell_pad_x)
                    .text_xs()
                    .font_family(UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.text_muted)
                    .whitespace_nowrap()
                    .child("Click to review"),
            )
        })
        .when(show_sha, |row| row.child(div().w(col_sha)))
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.store.dispatch(Msg::ClearCommitSelection { repo_id });
            this.store.dispatch(Msg::ClearDiffSelection { repo_id });
            cx.notify();
        }));

    if selected {
        row = row.bg(with_alpha(theme.colors.accent, 0.15));
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        MarkdownChangeHint, MarkdownInlineStyle, MarkdownPreviewRow, MarkdownPreviewRowKind,
        history_branch_text_highlights, history_selected_branch_entry_range,
        markdown_preview_alert_title_label, markdown_preview_inline_highlight,
        markdown_preview_row_background, markdown_preview_row_horizontal_padding,
        markdown_preview_row_layout, markdown_preview_row_marker, markdown_preview_row_styled_text,
        markdown_preview_row_typography,
    };
    use crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY;
    use crate::view::markdown_preview::MarkdownInlineSpan;
    use crate::view::{AppTheme, DateTimeFormat, Timezone, format_datetime, format_datetime_utc};
    use gpui::{FontWeight, SharedString};
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn markdown_row(kind: MarkdownPreviewRowKind) -> MarkdownPreviewRow {
        MarkdownPreviewRow {
            kind,
            text: SharedString::from("text"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        }
    }

    #[test]
    fn history_selected_branch_entry_range_matches_head_branch_entry() {
        let text = "HEAD → main, origin/main";
        let range = history_selected_branch_entry_range(text, "HEAD → main")
            .expect("expected head branch entry range");

        assert_eq!(&text[range], "HEAD → main");
    }

    #[test]
    fn history_branch_text_highlights_use_theme_emphasis_text_color() {
        let text: SharedString = "HEAD → main, origin/main".into();
        let selected: SharedString = "origin/main".into();
        let theme = AppTheme::from_json_str(
            r##"{
                "name": "Fixture",
                "themes": [
                    {
                        "key": "fixture",
                        "name": "Fixture",
                        "appearance": "dark",
                        "colors": {
                            "window_bg": "#0d1016ff",
                            "surface_bg": "#1f2127ff",
                            "surface_bg_elevated": "#1f2127ff",
                            "active_section": "#2d2f34ff",
                            "border": "#2d2f34ff",
                            "text": "#bfbdb6ff",
                            "text_muted": "#8a8986ff",
                            "accent": "#5ac1feff",
                            "hover": "#2d2f34ff",
                            "active": { "hex": "#2d2f34ff", "alpha": 0.78 },
                            "focus_ring": { "hex": "#5ac1feff", "alpha": 0.60 },
                            "focus_ring_bg": { "hex": "#5ac1feff", "alpha": 0.16 },
                            "scrollbar_thumb": { "hex": "#8a8986ff", "alpha": 0.30 },
                            "scrollbar_thumb_hover": { "hex": "#8a8986ff", "alpha": 0.42 },
                            "scrollbar_thumb_active": { "hex": "#8a8986ff", "alpha": 0.52 },
                            "danger": "#ef7177ff",
                            "warning": "#feb454ff",
                            "success": "#aad84cff",
                            "emphasis_text": "#123456ff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("theme JSON should parse");
        let highlights = history_branch_text_highlights(&text, Some(&selected), theme);

        assert_eq!(highlights.len(), 1);
        let (range, style) = &highlights[0];
        assert_eq!(&text.as_ref()[range.clone()], "origin/main");
        assert_eq!(style.color, Some(gpui::rgba(0x123456ff).into()));
    }

    #[test]
    fn history_branch_text_highlights_uses_black_text_on_light_theme() {
        let text: SharedString = "HEAD → main, origin/main".into();
        let selected: SharedString = "HEAD → main".into();
        let highlights =
            history_branch_text_highlights(&text, Some(&selected), AppTheme::gitcomet_light());

        assert_eq!(highlights.len(), 1);
        let (range, style) = &highlights[0];
        assert_eq!(&text.as_ref()[range.clone()], "HEAD → main");
        assert_eq!(style.color, Some(gpui::rgba(0x000000ff).into()));
    }

    #[test]
    fn history_branch_text_highlights_is_empty_when_selected_entry_is_missing() {
        let text: SharedString = "HEAD → main, origin/main".into();
        let selected: SharedString = "origin/feature".into();
        let highlights =
            history_branch_text_highlights(&text, Some(&selected), AppTheme::gitcomet_dark());

        assert!(highlights.is_empty());
    }

    #[test]
    fn commit_date_formats_as_yyyy_mm_dd_utc() {
        assert_eq!(
            format_datetime_utc(UNIX_EPOCH, DateTimeFormat::YmdHm),
            "1970-01-01 00:00 UTC"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1970-01-02 00:00 UTC"
        );
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH - Duration::from_secs(86_400),
                DateTimeFormat::YmdHm
            ),
            "1969-12-31 00:00 UTC"
        );

        // 2000-02-29 12:34:56 UTC
        assert_eq!(
            format_datetime_utc(
                UNIX_EPOCH + Duration::from_secs(951_782_400 + 12 * 3600 + 34 * 60 + 56),
                DateTimeFormat::YmdHms
            ),
            "2000-02-29 12:34:56 UTC"
        );
    }

    #[test]
    fn format_datetime_with_timezone_offset() {
        // UTC+5:30 (19800 seconds)
        let tz = Timezone::Fixed(19800);
        assert_eq!(
            format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, true),
            "1970-01-01 05:30 UTC+5:30"
        );

        // UTC-5
        let tz_neg = Timezone::Fixed(-18000);
        assert_eq!(
            format_datetime(
                UNIX_EPOCH + Duration::from_secs(86_400),
                DateTimeFormat::YmdHm,
                tz_neg,
                true,
            ),
            "1970-01-01 19:00 UTC\u{2212}5"
        );
    }

    #[test]
    fn format_datetime_can_hide_timezone_label() {
        let tz = Timezone::Fixed(7200);
        assert_eq!(
            format_datetime(UNIX_EPOCH, DateTimeFormat::YmdHm, tz, false),
            "1970-01-01 02:00"
        );
    }

    #[test]
    fn timezone_key_round_trips() {
        for tz in Timezone::all() {
            let key = tz.key();
            let parsed = Timezone::from_key(&key);
            assert_eq!(parsed, Some(*tz), "round-trip failed for {key}");
        }
    }

    #[test]
    fn worktree_preview_renderer_avoids_full_document_prepare_calls() {
        let source = include_str!("history.rs");
        let render_start = source
            .find("fn render_worktree_preview_rows")
            .expect("render_worktree_preview_rows should exist");
        let render_end = source[render_start..]
            .find("impl HistoryView")
            .map(|offset| render_start + offset)
            .expect("HistoryView impl should follow worktree preview renderer");
        let render_source = &source[render_start..render_end];

        assert!(
            !render_source.contains("prepare_diff_syntax_document("),
            "row renderer should not build prepared syntax documents"
        );
        assert!(
            !render_source.contains("prepare_diff_syntax_document_with_budget_reuse("),
            "row renderer should not run full-document parse prep"
        );
    }

    #[test]
    fn markdown_preview_heading_typography_scales_above_body_text() {
        let theme = AppTheme::gitcomet_light();
        let paragraph = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::Paragraph,
            text: SharedString::from("body"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };
        let h1 = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::Heading { level: 1 },
            ..paragraph.clone()
        };
        let h2 = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::Heading { level: 2 },
            ..paragraph.clone()
        };
        let h6 = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::Heading { level: 6 },
            ..paragraph.clone()
        };

        let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
        let body_typography = markdown_preview_row_typography(
            theme,
            &paragraph,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let h1_typography = markdown_preview_row_typography(
            theme,
            &h1,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let h2_typography = markdown_preview_row_typography(
            theme,
            &h2,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let h6_typography = markdown_preview_row_typography(
            theme,
            &h6,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );

        assert!(h1_typography.font_size > h2_typography.font_size);
        assert!(h2_typography.font_size > body_typography.font_size);
        assert!(h6_typography.font_size > body_typography.font_size);
        assert_eq!(h1_typography.font_weight, Some(FontWeight::BOLD));
        assert_eq!(h2_typography.font_weight, Some(FontWeight::BOLD));
        assert_eq!(h6_typography.font_weight, Some(FontWeight::BOLD));
    }

    #[test]
    fn markdown_preview_list_rows_match_body_line_height_and_keep_tighter_layout() {
        let theme = AppTheme::gitcomet_light();
        let paragraph = markdown_row(MarkdownPreviewRowKind::Paragraph);
        let list_item = markdown_row(MarkdownPreviewRowKind::ListItem { number: None });

        let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
        let paragraph_typography = markdown_preview_row_typography(
            theme,
            &paragraph,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let list_typography = markdown_preview_row_typography(
            theme,
            &list_item,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let paragraph_layout =
            markdown_preview_row_layout(&paragraph, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
        let list_layout =
            markdown_preview_row_layout(&list_item, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

        assert_eq!(
            list_typography.line_height,
            paragraph_typography.line_height
        );
        assert!(paragraph_layout.bottom_inset_px > list_layout.bottom_inset_px);
    }

    #[test]
    fn markdown_preview_details_summary_rows_are_bold_and_marked() {
        let theme = AppTheme::gitcomet_light();
        let row = markdown_row(MarkdownPreviewRowKind::DetailsSummary);

        let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
        let typography = markdown_preview_row_typography(
            theme,
            &row,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );

        assert_eq!(typography.font_weight, Some(FontWeight::BOLD));
        assert_eq!(
            markdown_preview_row_marker(&row)
                .as_ref()
                .map(SharedString::as_ref),
            Some("v")
        );
    }

    #[test]
    fn markdown_preview_code_rows_do_not_reserve_bottom_space_for_local_scrollbar() {
        let first_row = markdown_row(MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: false,
        });
        let last_row = markdown_row(MarkdownPreviewRowKind::CodeLine {
            is_first: false,
            is_last: true,
        });

        let first_layout =
            markdown_preview_row_layout(&first_row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
        let last_layout =
            markdown_preview_row_layout(&last_row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

        assert_eq!(first_layout.top_inset_px, 5.0);
        assert_eq!(last_layout.bottom_inset_px, 5.0);
    }

    #[test]
    fn markdown_preview_nested_code_rows_keep_small_outer_edge_gap() {
        let mut row = markdown_row(MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: false,
        });
        row.indent_level = 3;

        let padding = markdown_preview_row_horizontal_padding(
            &row,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );

        assert_eq!(padding.left_px, super::MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX);
        assert_eq!(padding.right_px, super::MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX);
    }

    #[test]
    fn markdown_preview_row_marker_preserves_ordered_item_number() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::ListItem { number: Some(7) },
            text: SharedString::from("item"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };

        assert_eq!(
            markdown_preview_row_marker(&row)
                .as_ref()
                .map(SharedString::as_ref),
            Some("7.")
        );
    }

    #[test]
    fn markdown_preview_row_marker_is_none_for_blockquotes_without_list_items() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::BlockquoteLine,
            text: SharedString::from("quote"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 2,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };

        assert_eq!(markdown_preview_row_marker(&row), None);
    }

    #[test]
    fn markdown_preview_row_marker_uses_footnote_label_when_present() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::Paragraph,
            text: SharedString::from("reference"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
            footnote_label: Some("1".into()),
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };

        assert_eq!(
            markdown_preview_row_marker(&row)
                .as_ref()
                .map(SharedString::as_ref),
            Some("[^1]:")
        );
    }

    #[test]
    fn markdown_preview_row_marker_returns_unordered_bullet_inside_blockquote() {
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::ListItem { number: None },
            text: SharedString::from("item"),
            inline_spans: Arc::new(Vec::new()),
            code_language: None,
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 1,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };

        assert_eq!(
            markdown_preview_row_marker(&row)
                .as_ref()
                .map(SharedString::as_ref),
            Some("•")
        );
    }

    #[test]
    fn markdown_preview_alert_title_label_requires_alert_start_row() {
        for (kind, label) in [
            (super::MarkdownAlertKind::Note, "NOTE"),
            (super::MarkdownAlertKind::Tip, "TIP"),
            (super::MarkdownAlertKind::Important, "IMPORTANT"),
            (super::MarkdownAlertKind::Warning, "WARNING"),
            (super::MarkdownAlertKind::Caution, "CAUTION"),
        ] {
            let mut row = markdown_row(MarkdownPreviewRowKind::BlockquoteLine);
            row.alert_kind = Some(kind);
            row.starts_alert = true;
            assert_eq!(markdown_preview_alert_title_label(&row), Some(label));

            row.starts_alert = false;
            assert_eq!(markdown_preview_alert_title_label(&row), None);
        }

        let mut row = markdown_row(MarkdownPreviewRowKind::BlockquoteLine);
        row.starts_alert = true;
        assert_eq!(markdown_preview_alert_title_label(&row), None);
    }

    #[test]
    fn markdown_preview_row_background_change_hints_override_alert_and_fallback_states() {
        let theme = AppTheme::gitcomet_light();

        let mut added_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
        added_row.change_hint = MarkdownChangeHint::Added;

        let mut added_alert_row = added_row.clone();
        added_alert_row.alert_kind = Some(super::MarkdownAlertKind::Warning);
        assert_eq!(
            markdown_preview_row_background(theme, &added_alert_row),
            markdown_preview_row_background(theme, &added_row)
        );

        let mut removed_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
        removed_row.change_hint = MarkdownChangeHint::Removed;

        let mut removed_fallback_row = removed_row.clone();
        removed_fallback_row.kind = MarkdownPreviewRowKind::PlainFallback;
        assert_eq!(
            markdown_preview_row_background(theme, &removed_fallback_row),
            markdown_preview_row_background(theme, &removed_row)
        );
    }

    #[test]
    fn markdown_preview_row_background_uses_alert_and_fallback_only_when_unchanged() {
        let theme = AppTheme::gitcomet_dark();

        let plain_row = markdown_row(MarkdownPreviewRowKind::Paragraph);
        assert_eq!(markdown_preview_row_background(theme, &plain_row), None);

        let mut alert_row = plain_row.clone();
        alert_row.alert_kind = Some(super::MarkdownAlertKind::Tip);

        let fallback_row = markdown_row(MarkdownPreviewRowKind::PlainFallback);
        let alert_bg = markdown_preview_row_background(theme, &alert_row);
        let fallback_bg = markdown_preview_row_background(theme, &fallback_row);

        assert!(alert_bg.is_some());
        assert!(fallback_bg.is_some());
        assert_ne!(alert_bg, fallback_bg);
    }

    #[test]
    fn markdown_preview_row_styled_text_maps_inline_styles_and_skips_normal_spans() {
        let theme = AppTheme::gitcomet_light();

        let mut row = markdown_row(MarkdownPreviewRowKind::Paragraph);
        row.text = SharedString::from("link under strike plain");
        row.inline_spans = Arc::new(vec![
            MarkdownInlineSpan {
                byte_range: 0..4,
                style: MarkdownInlineStyle::Link,
            },
            MarkdownInlineSpan {
                byte_range: 5..10,
                style: MarkdownInlineStyle::Underline,
            },
            MarkdownInlineSpan {
                byte_range: 11..17,
                style: MarkdownInlineStyle::Strikethrough,
            },
            MarkdownInlineSpan {
                byte_range: 18..23,
                style: MarkdownInlineStyle::Normal,
            },
        ]);

        let styled = markdown_preview_row_styled_text(theme, &row);
        let highlights = styled.highlights.as_ref();

        assert_eq!(styled.text.as_ref(), "link under strike plain");
        assert_eq!(highlights.len(), 3);
        assert_eq!(highlights[0].0, 0..4);
        assert_eq!(
            highlights[0].1,
            markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Link)
        );
        assert_eq!(highlights[1].0, 5..10);
        assert_eq!(
            highlights[1].1,
            markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Underline)
        );
        assert_eq!(highlights[2].0, 11..17);
        assert_eq!(
            highlights[2].1,
            markdown_preview_inline_highlight(theme, MarkdownInlineStyle::Strikethrough)
        );
    }

    #[test]
    fn markdown_preview_table_rows_use_monospace_typography_and_only_headers_are_bold() {
        let theme = AppTheme::gitcomet_light();
        let header = markdown_row(MarkdownPreviewRowKind::TableRow { is_header: true });
        let body = markdown_row(MarkdownPreviewRowKind::TableRow { is_header: false });

        let editor_font_family: SharedString = EDITOR_MONOSPACE_FONT_FAMILY.into();
        let header_typography = markdown_preview_row_typography(
            theme,
            &header,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );
        let body_typography = markdown_preview_row_typography(
            theme,
            &body,
            &editor_font_family,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        );

        assert_eq!(
            header_typography
                .font_family
                .as_ref()
                .map(SharedString::as_ref),
            Some(EDITOR_MONOSPACE_FONT_FAMILY)
        );
        assert_eq!(
            body_typography
                .font_family
                .as_ref()
                .map(SharedString::as_ref),
            Some(EDITOR_MONOSPACE_FONT_FAMILY)
        );
        assert_eq!(header_typography.font_weight, Some(FontWeight::BOLD));
        assert_eq!(body_typography.font_weight, None);
        assert_eq!(header_typography.font_size, body_typography.font_size);
        assert_eq!(header_typography.line_height, body_typography.line_height);
    }

    #[test]
    fn markdown_preview_code_rows_reuse_diff_syntax_highlighting() {
        let theme = AppTheme::gitcomet_dark();
        let row = MarkdownPreviewRow {
            kind: MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: true,
            },
            text: SharedString::from("fn\tmain() { let x = 1; }"),
            inline_spans: Arc::new(Vec::new()),
            code_language: Some(crate::view::rows::DiffSyntaxLanguage::Rust),
            code_block_horizontal_scroll_hint: false,
            source_line_range: 0..1,
            change_hint: MarkdownChangeHint::None,
            indent_level: 1,
            blockquote_level: 0,
            footnote_label: None,
            alert_kind: None,
            starts_alert: false,
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        };

        let dark_highlights = Arc::clone(&markdown_preview_row_styled_text(theme, &row).highlights);
        let dark = markdown_preview_row_styled_text(theme, &row);
        let light = markdown_preview_row_styled_text(AppTheme::gitcomet_light(), &row);

        assert_eq!(dark.text.as_ref(), "fn    main() { let x = 1; }");
        assert!(
            !dark.highlights.is_empty(),
            "code rows should reuse syntax highlights from the diff text renderer"
        );
        assert!(
            Arc::ptr_eq(&dark_highlights, &dark.highlights),
            "same-theme markdown code rows should reuse cached styled text"
        );
        assert!(
            !Arc::ptr_eq(&dark.highlights, &light.highlights),
            "light and dark markdown preview caches should stay separate"
        );
    }

    #[test]
    fn markdown_preview_spacer_rows_have_no_extra_layout_or_background() {
        let theme = AppTheme::gitcomet_light();
        let row = markdown_row(MarkdownPreviewRowKind::Spacer);

        let layout = markdown_preview_row_layout(&row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);

        assert_eq!(layout.top_inset_px, 0.0);
        assert_eq!(layout.bottom_inset_px, 0.0);
        assert_eq!(markdown_preview_row_background(theme, &row), None);
        assert_eq!(markdown_preview_row_marker(&row), None);
    }
}
