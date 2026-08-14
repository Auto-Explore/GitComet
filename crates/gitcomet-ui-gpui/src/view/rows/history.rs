use super::diff_canvas;
use super::diff_text::*;
use super::history_canvas;
use super::*;
use crate::view::caches::HistoryListRow;

use crate::view::markdown_preview::{
    MarkdownAlertKind, MarkdownChangeHint, MarkdownInlineImage, MarkdownInlineStyle,
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind, MarkdownPreviewVisualRow,
    MarkdownPreviewWrapPlan,
};
use crate::view::panes::main::diff_search::DiffSearchMatcher;
use crate::view::perf::{self, ViewPerfRenderLane, ViewPerfSpan};
use gitcomet_state::msg::CommitSelectMode;
use rustc_hash::FxHasher;

#[derive(Clone)]
struct WorktreePreviewPreparedSyntaxSource {
    document_text: Arc<str>,
    line_starts: Arc<[usize]>,
    document: rows::PreparedDiffSyntaxDocument,
}

fn worktree_preview_apply_query_overlay(
    theme: AppTheme,
    styled: CachedDiffStyledText,
    query_matcher: Option<&DiffSearchMatcher>,
) -> CachedDiffStyledText {
    query_matcher
        .map(|matcher| build_cached_diff_query_overlay_styled_text(theme, &styled, matcher))
        .unwrap_or(styled)
}

fn worktree_preview_streamed_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    line_ix: usize,
    query: &SharedString,
    query_options: super::super::panes::main::diff_search::DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
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
            query_options,
            query_matcher,
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
        let min_width = this.diff_horizontal_content_width();
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = (!query.as_ref().is_empty())
            .then(|| Arc::new(DiffSearchMatcher::new(query.as_ref(), query_options)));
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
        // Blame annotations for the file content view: a fixed left column when
        // annotate is on and blame for this target is loaded.
        let annotation_width = if this.annotate_enabled {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let blame_ctx = this.blame_render_ctx();

        // With word wrap on, a list position is a visual row and one file line
        // owns several of them. Everything that describes the *line* — its
        // text, syntax, blame, number — is looked up by `line_ix`; everything
        // that addresses the *row* on screen keeps `visible_ix`.
        let visible_len = this.worktree_preview_visible_len().unwrap_or(line_count);
        range
            .take_while(|ix| *ix < visible_len)
            .map(|visible_ix| {
                let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
                let is_continuation = wrap.is_some_and(|wrap| wrap.wrap_ix > 0);
                let ix = this
                    .diff_source_visible_ix_for_visible_ix(visible_ix)
                    .unwrap_or(visible_ix)
                    .min(line_count.saturating_sub(1));
                // A wrapped line is one line however many rows it takes, so its
                // number and its blame belong to the first of them.
                let line_no = if is_continuation {
                    SharedString::default()
                } else {
                    line_number_string(u32::try_from(ix + 1).ok())
                };
                let blame = blame_ctx.as_ref().filter(|_| !is_continuation).and_then(|ctx| {
                    // The file-content view renders every line contiguously, so the
                    // previous rendered line is `ix` (1-based), absent for line 1.
                    let prev_new_line = u32::try_from(ix).ok().filter(|&p| p >= 1);
                    // The full file-content view has no diff sidedness, so it
                    // cannot tell staged from unstaged per line; pass
                    // `is_context = false` so uncommitted lines fall back to the
                    // blamed area's default (staged area → "Staged", unstaged area
                    // → "Unstaged") rather than being mislabeled.
                    super::diff::build_row_blame_paint(
                        ctx,
                        false,
                        None,
                        u32::try_from(ix + 1).ok(),
                        prev_new_line,
                        theme,
                    )
                });
                let Some(raw_text) = this.worktree_preview_line_raw_text(ix) else {
                    return diff_canvas::worktree_preview_row_canvas(
                        theme,
                        cx.entity(),
                        ui_scale_percent,
                        visible_ix,
                        min_width,
                        annotation_width,
                        blame,
                        bar_color,
                        line_no,
                        None,
                        None,
                        None,
                        this.reveal_whitespace_chars,
                        wrap,
                    );
                };
                let streamed_spec = worktree_preview_streamed_spec(
                    raw_text.clone(),
                    ix,
                    &query,
                    query_options,
                    query_matcher.clone(),
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
                                    query: "",
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
                    let styled = worktree_preview_apply_query_overlay(
                        theme,
                        styled,
                        query_matcher.as_deref(),
                    );
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

                diff_canvas::worktree_preview_row_canvas(
                    theme,
                    cx.entity(),
                    ui_scale_percent,
                    visible_ix,
                    min_width,
                    annotation_width,
                    blame,
                    bar_color,
                    line_no,
                    styled,
                    streamed_spec,
                    Some(raw_text.as_ref()),
                    this.reveal_whitespace_chars,
                    wrap,
                )
            })
            .collect()
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
            editor_font_family.as_ref(),
            window,
            cx,
        );
        let region = match this.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => DiffTextRegion::SplitLeft,
        };
        let view = cx.entity().clone();
        let image_base_dir: Option<Arc<std::path::Path>> = this
            .markdown_preview_image_base_dir()
            .map(|dir| Arc::from(dir.as_path()));
        let min_width = this.diff_horizontal_content_width().max(viewport_width);
        render_markdown_preview_document_rows(
            &preview.old,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                min_width,
                editor_font_family,
                ui_scale_percent,
                view: Some(view),
                text_region: region,
                wrap_plan: this.markdown_preview_wrap_plan(MarkdownPreviewList::Old),
                image_base_dir: image_base_dir.clone(),
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
            editor_font_family.as_ref(),
            window,
            cx,
        );
        let view = cx.entity().clone();
        let image_base_dir: Option<Arc<std::path::Path>> = this
            .markdown_preview_image_base_dir()
            .map(|dir| Arc::from(dir.as_path()));
        let min_width = this.diff_horizontal_content_width().max(viewport_width);
        render_markdown_preview_document_rows(
            &preview.inline,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                min_width,
                editor_font_family,
                ui_scale_percent,
                view: Some(view),
                text_region: DiffTextRegion::Inline,
                wrap_plan: this.markdown_preview_wrap_plan(MarkdownPreviewList::Inline),
                image_base_dir: image_base_dir.clone(),
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
            editor_font_family.as_ref(),
            window,
            cx,
        );
        let view = cx.entity().clone();
        let image_base_dir: Option<Arc<std::path::Path>> = this
            .markdown_preview_image_base_dir()
            .map(|dir| Arc::from(dir.as_path()));
        let min_width = this.diff_horizontal_content_width().max(viewport_width);
        render_markdown_preview_document_rows(
            &preview.new,
            range,
            &MarkdownPreviewRenderContext {
                theme,
                min_width,
                editor_font_family,
                ui_scale_percent,
                view: Some(view),
                text_region: DiffTextRegion::SplitRight,
                wrap_plan: this.markdown_preview_wrap_plan(MarkdownPreviewList::New),
                image_base_dir: image_base_dir.clone(),
            },
        )
    }

    /// Rebuild the wrapped visual-row mapping for one preview list if the
    /// width, font, scale, change bar, or document it was measured against
    /// changed.
    ///
    /// Returns the number of rows the list should render: the wrapped visual
    /// row count while word wrap is on, and the plain source row count
    /// otherwise.
    pub(in crate::view) fn ensure_markdown_preview_wrap_plan(
        &mut self,
        list: MarkdownPreviewList,
        document: &MarkdownPreviewDocument,
        document_rev: u64,
        available_width: Pixels,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        let Some(measure) = self.markdown_preview_wrap_measure(document_rev, available_width, cx)
        else {
            self.markdown_preview_wrap.clear_list(list);
            return document.rows.len();
        };

        if !self.markdown_preview_wrap.is_current(list, measure.key) {
            let plan = crate::view::markdown_preview::build_markdown_preview_wrap_plan(
                document,
                measure.wrap_row_fn(window, self.theme),
            );
            self.markdown_preview_wrap.store(list, measure.key, plan);
        }

        self.markdown_preview_wrap
            .plan_len(list)
            .unwrap_or(document.rows.len())
    }

    /// Rebuild both split-preview wrap plans together so the two columns stay
    /// row-aligned, and return each list's row count.
    pub(in crate::view) fn ensure_markdown_preview_split_wrap_plans(
        &mut self,
        old_doc: &MarkdownPreviewDocument,
        new_doc: &MarkdownPreviewDocument,
        document_rev: u64,
        available_width: Pixels,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> (usize, usize) {
        let measure = self.markdown_preview_wrap_measure(document_rev, available_width, cx);

        match measure {
            None => {
                self.markdown_preview_wrap
                    .clear_list(MarkdownPreviewList::Old);
                self.markdown_preview_wrap
                    .clear_list(MarkdownPreviewList::New);
            }
            Some(measure)
                if !self
                    .markdown_preview_wrap
                    .is_current(MarkdownPreviewList::Old, measure.key)
                    || !self
                        .markdown_preview_wrap
                        .is_current(MarkdownPreviewList::New, measure.key) =>
            {
                let (old_plan, new_plan) =
                    crate::view::markdown_preview::build_markdown_preview_split_wrap_plans(
                        old_doc,
                        new_doc,
                        measure.wrap_row_fn(window, self.theme),
                    )
                    .unzip();
                self.markdown_preview_wrap
                    .store(MarkdownPreviewList::Old, measure.key, old_plan);
                self.markdown_preview_wrap
                    .store(MarkdownPreviewList::New, measure.key, new_plan);
            }
            Some(_) => {}
        }

        (
            self.markdown_preview_wrap
                .plan_len(MarkdownPreviewList::Old)
                .unwrap_or(old_doc.rows.len()),
            self.markdown_preview_wrap
                .plan_len(MarkdownPreviewList::New)
                .unwrap_or(new_doc.rows.len()),
        )
    }

    /// Wrap plans for whichever preview lists the current view mode paints,
    /// returning `(old, new, inline)` row counts.
    ///
    /// Owning the mode switch here keeps the "only painted lists hold a plan"
    /// invariant in one place instead of spread through the render tree, and
    /// stops an unpainted column from being wrapped at a width it never uses.
    pub(in crate::view) fn ensure_markdown_preview_wrap_plans(
        &mut self,
        preview: &crate::view::markdown_preview::MarkdownPreviewDiff,
        document_rev: u64,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> (usize, usize, usize) {
        let (inline_width, split_width) = self.markdown_preview_wrap_widths(cx);
        match self.diff_view {
            DiffViewMode::Inline => {
                self.markdown_preview_wrap
                    .clear_list(MarkdownPreviewList::Old);
                self.markdown_preview_wrap
                    .clear_list(MarkdownPreviewList::New);
                let inline_len = self.ensure_markdown_preview_wrap_plan(
                    MarkdownPreviewList::Inline,
                    &preview.inline,
                    document_rev,
                    inline_width,
                    window,
                    cx,
                );
                (preview.old.rows.len(), preview.new.rows.len(), inline_len)
            }
            DiffViewMode::Split => {
                self.markdown_preview_wrap
                    .clear_list(MarkdownPreviewList::Inline);
                let (old_len, new_len) = self.ensure_markdown_preview_split_wrap_plans(
                    &preview.old,
                    &preview.new,
                    document_rev,
                    split_width,
                    window,
                    cx,
                );
                (old_len, new_len, preview.inline.rows.len())
            }
        }
    }

    /// Everything needed to wrap a preview list at the current width, or
    /// `None` when word wrap is off and the list should render unwrapped.
    ///
    /// The width is quantised so dragging a window edge does not invalidate
    /// the plan on every pixel — re-wrapping a whole document is far more
    /// expensive than the sub-bucket accuracy it would buy, and wrapping to
    /// the rounded-down width keeps rows inside the viewport.
    fn markdown_preview_wrap_measure(
        &self,
        document_rev: u64,
        available_width: Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> Option<MarkdownPreviewWrapMeasure> {
        const WRAP_WIDTH_BUCKET_PX: u32 = 8;

        if !self.diff_word_wrap || available_width <= px(0.0) {
            return None;
        }

        let width_px = (u32::from(available_width.floor()) / WRAP_WIDTH_BUCKET_PX)
            .saturating_mul(WRAP_WIDTH_BUCKET_PX);
        if width_px == 0 {
            return None;
        }

        let editor_font_family: SharedString =
            crate::font_preferences::current_editor_font_family(cx).into();
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        Some(MarkdownPreviewWrapMeasure {
            key: MarkdownPreviewWrapKey {
                width_px,
                ui_scale_percent,
                theme_is_dark: self.theme.is_dark,
                editor_font_family_hash: markdown_preview_font_family_hash(&editor_font_family),
                document_rev,
            },
            wrap_width: px(width_px as f32),
            editor_font_family,
            ui_scale_percent,
        })
    }

    pub(in crate::view) fn update_markdown_preview_horizontal_min_width(
        &mut self,
        document: &MarkdownPreviewDocument,
        range: Range<usize>,
        editor_font_family: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap {
            // Wrapped rows never exceed the viewport, so there is no content
            // width to grow; `set_diff_word_wrap` already reset the
            // horizontal scroll state.
            return;
        }
        let mut min_width = self.diff_horizontal_content_width();
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let editor_font_family: SharedString = editor_font_family.to_owned().into();
        for row in range.filter_map(|ix| document.rows.get(ix)) {
            let required = markdown_preview_row_required_width(
                window,
                self.theme,
                row,
                &editor_font_family,
                ui_scale_percent,
            );
            if required > min_width {
                min_width = required;
            }
        }

        self.record_diff_horizontal_content_width(min_width, cx);
    }
}

const MARKDOWN_PREVIEW_ROW_HEIGHT_PX: f32 = 28.0;
pub(super) const MARKDOWN_PREVIEW_BASE_FONT_PX: f32 = 13.0;
const MARKDOWN_PREVIEW_BASE_LINE_HEIGHT_PX: f32 = 20.0;
pub(in crate::view) const MARKDOWN_PREVIEW_CONTENT_PAD_X_PX: f32 = 18.0;
const MARKDOWN_PREVIEW_BOXED_EDGE_GAP_PX: f32 = 8.0;
pub(super) const MARKDOWN_PREVIEW_INDENT_STEP_PX: f32 = 24.0;
pub(super) const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX: f32 = 4.0;
const MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_GAP_PX: f32 = 8.0;
const MARKDOWN_PREVIEW_BLOCKQUOTE_GUTTER_MARGIN_RIGHT_PX: f32 = 12.0;
pub(super) const MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX: f32 = 22.0;
pub(super) const MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX: f32 = 10.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_FONT_PX: f32 = 11.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_PAD_X_PX: f32 = 6.0;
const MARKDOWN_PREVIEW_ALERT_BADGE_GAP_PX: f32 = 10.0;
pub(super) const MARKDOWN_PREVIEW_SHELL_PAD_X_PX: f32 = 12.0;
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MarkdownPreviewRowLayout {
    top_inset_px: f32,
    bottom_inset_px: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MarkdownPreviewRowHorizontalPadding {
    left_px: f32,
    right_px: f32,
}

/// Inputs shared by every row of one wrap pass.
struct MarkdownPreviewWrapMeasure {
    key: MarkdownPreviewWrapKey,
    wrap_width: Pixels,
    editor_font_family: SharedString,
    ui_scale_percent: u32,
}

impl MarkdownPreviewWrapMeasure {
    /// Per-row wrap callback for the plan builders.
    fn wrap_row_fn<'a>(
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

pub(super) struct MarkdownPreviewRenderContext<'a> {
    pub(super) theme: AppTheme,
    pub(super) min_width: Pixels,
    pub(super) editor_font_family: SharedString,
    pub(super) ui_scale_percent: u32,
    pub(super) view: Option<Entity<MainPaneView>>,
    pub(super) text_region: DiffTextRegion,
    /// Visual-row mapping when word wrap is on; `None` renders one row per
    /// source row with horizontal overflow clipped.
    pub(super) wrap_plan: Option<&'a MarkdownPreviewWrapPlan>,
    /// Directory relative image paths resolve against.
    pub(super) image_base_dir: Option<Arc<std::path::Path>>,
}

pub(super) fn render_markdown_preview_document_rows(
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
                    .flex_grow()
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
    let full_styled = markdown_preview_row_styled_text(theme, row);
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
                .flex_grow()
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

fn markdown_preview_row_required_width(
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
fn markdown_preview_row_chrome_width(
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
pub(super) fn markdown_preview_row_wrap_ranges(
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
fn markdown_preview_wrap_fragments(text: &str, tab_width: Pixels) -> Vec<gpui::LineFragment<'_>> {
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
fn markdown_preview_expanded_slice_range(
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
pub(in crate::view) type MarkdownPreviewPictureSizes = Arc<HashMap<SharedString, (u32, u32)>>;

/// Shared stand-in for a preview that measured nothing. The diff preview draws
/// its pictures into fixed-height bands, so it has no use for their real sizes.
fn markdown_preview_no_picture_sizes() -> &'static MarkdownPreviewPictureSizes {
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
fn markdown_preview_remote_image_url(source: &str) -> Option<SharedString> {
    let scheme_end = source.find("://")?;
    let scheme = &source[..scheme_end];
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then(|| SharedString::from(source.to_owned()))
}

/// Styled text for one row, shared with the flowing single-document renderer.
pub(in crate::view) fn markdown_preview_styled_row(
    theme: AppTheme,
    row: &MarkdownPreviewRow,
) -> &CachedDiffStyledText {
    markdown_preview_row_styled_text(theme, row)
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
struct MarkdownPreviewPictureSkeleton {
    /// Widest the picture will draw, or `None` to fill the document.
    width: Option<Pixels>,
    /// Width over height, or `None` when only a height is known.
    aspect_ratio: Option<f32>,
    /// Used when the aspect ratio is unknown: the rows the parser set aside.
    reserved_height: Pixels,
}

impl MarkdownPreviewPictureSkeleton {
    fn render(self, theme: AppTheme) -> AnyElement {
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

fn markdown_preview_picture_skeleton(
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
fn markdown_preview_image_block_rows(row: &MarkdownPreviewRow) -> u8 {
    match row.kind {
        MarkdownPreviewRowKind::Image { slice_count, .. } => slice_count,
        _ => 1,
    }
}

/// Tallest an inline picture may be when the document declares no size, so a
/// stray screenshot written mid-sentence cannot push the line open.
const MARKDOWN_PREVIEW_INLINE_IMAGE_MAX_HEIGHT_PX: f32 = 26.0;

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
const MARKDOWN_PREVIEW_INLINE_IMAGE_LOADING_WIDTH_PX: f32 = 90.0;

/// A picture element that keeps per-frame state.
///
/// The id matters: `gpui` only remembers which frame an animated image is
/// showing for elements that have one, so an `img` without an id freezes on the
/// first frame of a GIF.
fn markdown_preview_image_element(
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
fn markdown_preview_inline_image_placeholder(
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
fn markdown_preview_image_label(row: &MarkdownPreviewRow, reason: &str) -> SharedString {
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
fn markdown_preview_image_reason(reason: &str, described: &SharedString) -> SharedString {
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
fn markdown_preview_resolved_picture(
    source: &str,
    id: gpui::ElementId,
    image_base_dir: Option<&std::path::Path>,
) -> Option<gpui::Stateful<gpui::Img>> {
    markdown_preview_image_source(image_base_dir, source)
        .map(|source| markdown_preview_image_element(source, id))
}

/// Stand-in shown in place of a picture, so the row is never silently blank.
fn markdown_preview_image_placeholder_element(
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
fn markdown_preview_image_placeholder(
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
fn markdown_preview_image_row(
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

fn markdown_preview_font_family_hash(font_family: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    font_family.hash(&mut hasher);
    hasher.finish()
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

fn worktree_preview_bar_color(this: &MainPaneView, theme: AppTheme) -> Option<gpui::Rgba> {
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
        MarkdownAlertKind::Note => theme.colors.accent.foreground,
        MarkdownAlertKind::Tip => theme.colors.status.success.foreground,
        MarkdownAlertKind::Important => with_alpha(theme.colors.accent.foreground, 0.85),
        MarkdownAlertKind::Warning => theme.colors.status.warning.foreground,
        MarkdownAlertKind::Caution => theme.colors.status.danger.foreground,
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
                    theme.colors.interaction.selected_background,
                    if theme.is_dark { 0.75 } else { 0.55 },
                )
                .into(),
            ),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Strikethrough => gpui::HighlightStyle {
            color: Some(theme.colors.foreground.secondary.into()),
            strikethrough: Some(gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.secondary.into()),
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Link => gpui::HighlightStyle {
            color: Some(theme.colors.accent.foreground.into()),
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.accent.foreground.into()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
        MarkdownInlineStyle::Underline => gpui::HighlightStyle {
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.colors.foreground.primary.into()),
                wavy: false,
            }),
            ..gpui::HighlightStyle::default()
        },
    }
}

fn markdown_preview_row_text_color(theme: AppTheme, row: &MarkdownPreviewRow) -> gpui::Rgba {
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

fn markdown_preview_row_layout(
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
        with_alpha(theme.colors.surface.raised, 0.88)
    } else {
        with_alpha(theme.colors.surface.panel, 0.86)
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

impl HistoryView {
    pub(in super::super) fn render_history_table_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let (_, worktree_counts) = this.ensure_history_worktree_summary_cache();
        let plan = this.ensure_history_list_plan();
        let show_working_tree_summary_row = plan.show_working_tree_summary_row();
        let stash_ids = this.ensure_history_stash_ids_cache();
        // Cloned out of `this` so the row loop below can borrow it immutably
        // alongside the repo snapshot; it is an `Arc` bitset, so this is cheap.
        // The cache behind it is scheduled from the panel's own render, never
        // built here -- this runs during layout.
        // One lane keeps full colour; the rest wash out. Resolved once here rather
        // than per row -- it is a scan of the page behind a memo.
        let selected_lane_color_ix =
            this.history_selected_lane_color_ix(plan.show_working_tree_summary_row());

        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let show_graph_color_marker =
            history_scope_shows_graph_color_marker(repo.history_state.history_scope);

        let theme = this.theme;
        let col_branch = this.history_col_branch;
        let col_graph = this.history_col_graph;
        let col_author = this.history_col_author;
        let col_date = this.history_col_date;
        let col_sha = this.history_col_sha;
        let ui_scale = this.ui_scale();
        let (show_graph, show_author, show_date, show_sha) = this.history_visible_columns();
        let display_key = HistoryDisplayKey::new(
            this.date_time_format,
            this.timezone,
            this.show_timezone,
            this.history_relative_dates,
        );

        let page = Self::display_log_page_for_repo(repo);
        let cache = this
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo.id);
        let worktree_node_color_ix =
            history_worktree_node_color_ix(cache.map(|cache| cache.base.graph_rows.as_ref()));

        let worktree_dirty = match &repo.worktree_dirty {
            Loadable::Ready(dirty) => Some(Arc::clone(dirty)),
            _ => None,
        };
        range
            .filter_map(|list_ix| {
                let row = plan.row_at(list_ix)?;
                if let HistoryListRow::WorktreeUncommitted {
                    visible_ix,
                    worktree_ix,
                } = row
                {
                    let cache = cache?;
                    let summary = worktree_dirty.as_ref()?.get(worktree_ix)?;
                    // The row shows the lanes of the commit it sits on top of,
                    // so it needs that row's paint data.
                    let graph_row = cache.base.graph_rows.get(visible_ix)?;
                    // Whatever sits directly above draws a connector down into
                    // this row; carry it through so the lane is not broken.
                    let connect_from_top_col =
                        list_ix.checked_sub(1).and_then(|above| match plan.row_at(above) {
                            Some(HistoryListRow::WorkingTreeSummary) => Some(0),
                            // Two worktrees on one commit share an anchor, and so
                            // the same node column.
                            Some(HistoryListRow::WorktreeUncommitted { .. }) => {
                                Some(usize::from(
                                    super::history_graph_paint::band_node_for(
                                        graph_row,
                                        summary.branch.is_some() && !summary.detached,
                                    )
                                    .col,
                                ))
                            }
                            _ => None,
                        });
                    return Some(worktree_uncommitted_history_row(
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
                        graph_row,
                        connect_from_top_col,
                        selected_lane_color_ix,
                        show_graph_color_marker,
                        repo.id,
                        list_ix,
                        repo.history_state.worktree_selection.as_deref()
                            == Some(summary.path.as_path()),
                        (summary.added, summary.modified, summary.deleted),
                        summary,
                        cx,
                    ));
                }

                if matches!(row, HistoryListRow::WorkingTreeSummary) {
                    // A selected worktree row also leaves `selected_commit`
                    // empty, and only one row may read as selected.
                    let selected = repo.history_state.selected_commit.is_none()
                        && repo.history_state.worktree_selection.is_none();
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
                        worktree_node_color_ix,
                        selected_lane_color_ix,
                        show_graph_color_marker,
                        repo.id,
                        selected,
                        worktree_counts,
                        cx,
                    ));
                }

                let HistoryListRow::Commit { visible_ix } = row else {
                    return None;
                };

                let page = page.as_deref()?;
                let cache = cache?;

                let commit_ix = cache.base.visible_indices.get(visible_ix)?;
                let commit = page.commits.get(commit_ix)?;
                cache.base.graph_rows.get(visible_ix)?;
                let base_row_vm = cache.base.row_vms.get(visible_ix)?;
                let decoration_row_vm = cache.decorations.row_vms.get(visible_ix)?;
                let connect_from_top_col = match list_ix
                    .checked_sub(1)
                    .and_then(|above| plan.row_at(above))
                {
                    // A worktree band above connects its node down into this
                    // commit, so this row draws the matching stub upwards even
                    // when the lane is born here.
                    Some(HistoryListRow::WorktreeUncommitted { worktree_ix, .. }) => cache
                        .base
                        .graph_rows
                        .get(visible_ix)
                        .map(|row| {
                            let on_branch = worktree_dirty
                                .as_ref()
                                .and_then(|dirty| dirty.get(worktree_ix))
                                .is_some_and(|s| s.branch.is_some() && !s.detached);
                            usize::from(
                                super::history_graph_paint::band_node_for(row, on_branch).col,
                            )
                        }),
                    _ => (show_working_tree_summary_row && visible_ix == 0).then_some(0),
                };
                let selected = repo.history_state.selected_commit.as_ref() == Some(&commit.id)
                    || repo.history_state.multi_selection.is_multi()
                        && repo.history_state.multi_selection.contains(&commit.id);
                let selected_branch = this.selected_branch_for_history_row(repo.id, selected);
                let is_stash_node = base_row_vm.is_stash
                    || stash_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&commit.id));
                let when = base_row_vm.when.resolve(display_key);
                let short_sha = base_row_vm.short_sha.resolve();

                let lane_branch_name = decoration_row_vm
                    .lane_branch
                    .and_then(|ix| cache.decorations.branch_names.get(usize::from(ix)))
                    .cloned();

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
                    Arc::clone(&cache.base.graph_rows),
                    visible_ix,
                    connect_from_top_col,
                    Arc::clone(&decoration_row_vm.tag_names),
                    Arc::clone(&decoration_row_vm.ref_items),
                    selected_branch,
                    selected_lane_color_ix,
                    lane_branch_name,
                    base_row_vm.author.clone(),
                    base_row_vm.summary.clone(),
                    when,
                    short_sha,
                    selected,
                    base_row_vm.is_head,
                    is_stash_node,
                    this.active_context_menu_invoker.as_ref(),
                    cx,
                ))
            })
            .collect()
    }
}

const HISTORY_ROW_HEIGHT_PX: f32 = 28.0;
/// Widest a worktree row's badge may grow before its branch label truncates.
/// Matches the sidebar's branch-row worktree pill.
const HISTORY_WORKTREE_BADGE_MAX_W_PX: f32 = 200.0;
/// Matches the history table's ref chips so the badge sits on the same rhythm.
const HISTORY_WORKTREE_BADGE_HEIGHT_PX: f32 = 18.0;

fn history_worktree_node_color_ix(
    graph_rows: Option<&[history_graph::GraphRow]>,
) -> history_graph::LaneColorIx {
    graph_rows
        .and_then(|rows| rows.first())
        .and_then(|row| {
            // Column 0 can be a hole, whose `color_ix` is a real palette index
            // rather than a lane's colour.
            row.lanes_now
                .first()
                .filter(|lane| lane.is_active())
                .map(|lane| lane.color_ix)
        })
        .unwrap_or(0)
}

/// The lane-coloured border down the left edge of a message cell, matching the
/// one the commit rows paint on their canvas.
///
/// Absolutely positioned so the label keeps the same left offset it has on a
/// commit row — a flow child would push the text over by the border's width.
fn history_message_border(
    ui_scale: ui_scale::UiScale,
    color: gpui::Rgba,
) -> impl IntoElement {
    let border_w = ui_scale.px(HISTORY_MESSAGE_BORDER_W_PX);
    let inset_y = ui_scale.px(HISTORY_MESSAGE_BORDER_INSET_Y_PX);
    div()
        .absolute()
        .left_0()
        .top(inset_y)
        .bottom(inset_y)
        .w(border_w)
        .rounded(border_w * 0.5)
        .bg(color)
}

fn history_row_height(ui_scale: ui_scale::UiScale) -> Pixels {
    ui_scale.px(HISTORY_ROW_HEIGHT_PX)
}

fn history_scope_shows_graph_color_marker(scope: gitcomet_core::domain::LogScope) -> bool {
    !matches!(scope, gitcomet_core::domain::LogScope::FirstParent)
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
    tag_names: Arc<[HistoryTextVm]>,
    ref_items: Arc<[HistoryRefListItem]>,
    selected_branch: Option<SelectedHistoryBranch>,
    // Colour index of the lane the selection sits on; every other lane washes
    // out. A property of the lane, not of this row.
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    // Branch this commit belongs to, shown as a faded badge while the row is
    // hovered. Inherited down the lane, so unlabelled commits have one too.
    lane_branch_name: Option<SharedString>,
    author: HistoryTextVm,
    summary: HistoryTextVm,
    when: HistoryTextVm,
    short_sha: HistoryTextVm,
    selected: bool,
    is_head: bool,
    is_stash_node: bool,
    active_context_menu_invoker: Option<&SharedString>,
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let context_menu_invoker: SharedString =
        format!("history_commit_menu_{}_{}", repo_id.0, commit.id.as_ref()).into();
    let context_menu_active = active_context_menu_invoker == Some(&context_menu_invoker);
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
        ref_items,
        selected_branch,
        selected_lane_color_ix,
        lane_branch_name,
        author,
        summary,
        when,
        short_sha,
    );

    let commit_id = commit.id.clone();
    let row_height = history_row_height(ui_scale);
    let mut row = div()
        .id(ix)
        .debug_selector(move || format!("history_row_{ix}"))
        .relative()
        .h(row_height)
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| {
            if context_menu_active {
                s.bg(theme.colors.interaction.pressed_background)
            } else {
                s.bg(theme.colors.interaction.hover_background)
            }
        })
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .child(commit_row)
        // Selecting on press, like the sidebar rows: the row the gesture
        // *starts* on owns it, so a release that merely drifted here — the end
        // of a text-selection drag in the details pane, say — selects nothing.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                let modifiers = e.modifiers;
                let mode = if modifiers.shift {
                    CommitSelectMode::Range
                } else if modifiers.secondary() || modifiers.control || modifiers.platform {
                    CommitSelectMode::Toggle
                } else {
                    CommitSelectMode::Single
                };
                let visible_order = (mode == CommitSelectMode::Range)
                    .then(|| this.visible_commit_ids_for_repo(repo_id))
                    .flatten();
                this.store.dispatch(Msg::SelectCommitMulti {
                    repo_id,
                    commit_id: commit_id.clone(),
                    mode,
                    clicked_index: Some(graph_row_ix),
                    visible_order,
                });
                cx.notify();
            }),
        );

    if is_head && !selected && !context_menu_active {
        // A quiet tint keeps HEAD findable without competing with selection.
        row = row.bg(with_alpha(theme.colors.accent.foreground, 0.06));
    }
    if selected {
        row = row.bg(with_alpha(theme.colors.accent.foreground, 0.15));
    }
    if context_menu_active {
        row = row.bg(theme.colors.interaction.pressed_background);
    }

    if is_head {
        row = row.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(ui_scale.px(3.0))
                .bg(with_alpha(theme.colors.accent.foreground, 0.90)),
        );
    }

    row.into_any_element()
}

/// One linked worktree's uncommitted changes, rendered directly above the commit
/// that worktree has checked out.
///
/// Unlike the working-tree summary row this one is not pinned to the top of the
/// list, so it paints the full lane band of the commit below it rather than a
/// single connector stub: the lanes have to run through it uninterrupted.
#[allow(clippy::too_many_arguments)]
fn worktree_uncommitted_history_row(
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
    graph_row: &history_graph::GraphRow,
    connect_from_top_col: Option<usize>,
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    show_graph_color_marker: bool,
    repo_id: RepoId,
    list_ix: usize,
    selected: bool,
    counts: (usize, usize, usize),
    summary: &gitcomet_core::domain::WorktreeDirtySummary,
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let scaled_px = |value| ui_scale.px(value);
    let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
    let band_node = super::history_graph_paint::band_node_for(
        graph_row,
        summary.branch.is_some() && !summary.detached,
    );
    // The node washes with its lane, like every other node in the graph -- the
    // text beside it still follows the row's relation to the selection.
    let node_color = super::history_graph_paint::lane_wash_color(
        theme,
        band_node.color_ix,
        selected_lane_color_ix,
    );
    // Everything on the row washes with the lane it sits on, text included.
    let on_selected_lane = selected_lane_color_ix.map(|selected| selected == band_node.color_ix);
    let label_color = history_canvas::selection_related_summary_color(theme, on_selected_lane);

    // A pass-through band: whatever entered the commit below from above runs
    // straight through this row, so inserting it leaves the graph unbroken.
    // `None` when the node sits on a lane of its own already (a branch head's
    // fork), which needs only a straight connector down.
    let node_exit_col = (band_node.exit_col != band_node.col).then_some(band_node.exit_col);
    let band = history_graph::GraphRow {
        lanes_now: graph_row.lanes_now.clone(),
        lanes_next: graph_row.lanes_now.clone(),
        joins_in: Default::default(),
        edges_out: Default::default(),
        node_col: band_node.col,
        node_color_ix: band_node.color_ix,
        is_merge: false,
    };
    let graph = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            super::history_graph_paint::paint_history_graph_band(
                theme,
                &band,
                connect_from_top_col,
                selected_lane_color_ix,
                super::history_graph_paint::BandNodePaint {
                    color: node_color,
                    exit_col: node_exit_col,
                },
                show_graph_color_marker,
                bounds,
                window,
                cx,
            );
        },
    )
    .w_full()
    .h_full();

    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, scaled_px(12.0)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(count.to_string()),
            )
            .into_any_element()
    };
    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.status.warning.foreground,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count(
            "icons/plus.svg",
            theme.colors.status.success.foreground,
            added,
        ));
    }
    if deleted > 0 {
        parts.push(icon_count(
            "icons/minus.svg",
            theme.colors.status.danger.foreground,
            deleted,
        ));
    }

    let palette = super::sidebar::worktree_badge_palette(theme);
    let badge_label = super::sidebar::worktree_origin_label(
        summary.branch.as_deref(),
        summary.detached,
        &summary.path,
    );
    let open_path = summary.path.clone();
    let badge_tooltip: SharedString =
        format!("Open this worktree\n{}", summary.path.display()).into();

    let badge = super::sidebar::worktree_origin_chip(
        theme,
        badge_label,
        scaled_px(9.0),
        scaled_px(HISTORY_WORKTREE_BADGE_HEIGHT_PX),
        scaled_px(HISTORY_WORKTREE_BADGE_MAX_W_PX),
        scaled_px(6.0),
    )
    .id(("history_worktree_badge", list_ix))
    .cursor(CursorStyle::PointingHand)
    .hover(move |s| {
        s.border_color(palette.hover_border)
            .text_color(palette.hover_text)
    })
    .gitcomet_tooltip(theme, badge_tooltip)
    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
        this.store.dispatch(Msg::OpenRepo(open_path.clone()));
        cx.notify();
    }));

    let select_path = summary.path.clone();
    let mut row = div()
        .id(("history_worktree_uncommitted", list_ix))
        .h(history_row_height(ui_scale))
        .flex()
        .w_full()
        .items_center()
        .px_2()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(theme.colors.interaction.hover_background))
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.store.dispatch(Msg::SelectWorktreeUncommitted {
                repo_id,
                path: select_path.clone(),
            });
            cx.notify();
        }))
        .child(
            div()
                .w(col_branch)
                .text_xs()
                .line_clamp(1)
                .whitespace_nowrap()
                .child(div()),
        )
        .when(show_graph, |row| {
            row.child(div().w(col_graph).h_full().overflow_hidden().child(graph))
        })
        .child({
            let mut summary = div()
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                // Same offset the commit rows put their text at, so the message
                // column reads as one column down the whole list.
                .pl(ui_scale.px(history_message_text_left_px(show_graph_color_marker)))
                .pr(cell_pad_x)
                .when(show_graph_color_marker, |cell| {
                    cell.child(history_message_border(ui_scale, node_color))
                });
            summary = summary.child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(label_color)
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child("Worktree changes"),
            );
            if !parts.is_empty() {
                summary = summary.child(div().flex().items_center().gap_2().children(parts));
            }
            summary.child(div().flex_1().min_w(px(0.0))).child(badge)
        })
        .when(show_author, |row| row.child(div().w(col_author)))
        .when(show_date, |row| row.child(div().w(col_date)))
        .when(show_sha, |row| row.child(div().w(col_sha)));

    if selected {
        row = row.bg(with_alpha(theme.colors.accent.foreground, 0.15));
    }

    let _ = repo_id;
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
    node_color_ix: history_graph::LaneColorIx,
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    show_graph_color_marker: bool,
    repo_id: RepoId,
    selected: bool,
    counts: (usize, usize, usize),
    cx: &mut gpui::Context<HistoryView>,
) -> AnyElement {
    let scaled_px = |value| ui_scale.px(value);
    let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
    // The connector washes with its lane, like every other node in the graph;
    // the label still follows the row's relation to the selection.
    let node_color =
        super::history_graph_paint::lane_wash_color(theme, node_color_ix, selected_lane_color_ix);
    let on_selected_lane = selected_lane_color_ix.map(|selected| selected == node_color_ix);
    let label_color = history_canvas::selection_related_summary_color(theme, on_selected_lane);
    let icon_count = |icon_path: &'static str, color: gpui::Rgba, count: usize| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(svg_icon(icon_path, color, scaled_px(12.0)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.foreground.secondary)
                    .child(count.to_string()),
            )
            .into_any_element()
    };

    let (added, modified, deleted) = counts;
    let mut parts: Vec<AnyElement> = Vec::with_capacity(3);
    if modified > 0 {
        parts.push(icon_count(
            "icons/pencil.svg",
            theme.colors.status.warning.foreground,
            modified,
        ));
    }
    if added > 0 {
        parts.push(icon_count(
            "icons/plus.svg",
            theme.colors.status.success.foreground,
            added,
        ));
    }
    if deleted > 0 {
        parts.push(icon_count(
            "icons/minus.svg",
            theme.colors.status.danger.foreground,
            deleted,
        ));
    }

    // The history list's own surface, so the node's middle hides the lane
    // running through its column.
    let node_background = theme.colors.surface.canvas;
    let circle = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            use gpui::{PathBuilder, point};
            let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
            let scaled_px = |value| px(value * design_scale_factor);
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

            if show_graph_color_marker {
                super::history_graph_paint::paint_graph_fade(
                    node_color,
                    bounds,
                    scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX),
                    window,
                );
            }

            super::history_graph_paint::paint_ring_icon_node(
                center.x,
                center.y,
                icons::UNCOMMITTED_NODE_ICON_PATH,
                node_color,
                node_background,
                window,
                cx,
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
        .hover(move |s| s.bg(theme.colors.interaction.hover_background))
        .active(move |s| s.bg(theme.colors.interaction.pressed_background))
        .child(
            div()
                .w(col_branch)
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
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
                .relative()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                // Same offset the commit rows put their text at, so the message
                // column reads as one column down the whole list.
                .pl(ui_scale.px(history_message_text_left_px(show_graph_color_marker)))
                .pr(cell_pad_x)
                .when(show_graph_color_marker, |cell| {
                    cell.child(history_message_border(ui_scale, node_color))
                });
            summary = summary.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .text_color(label_color)
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
                    .text_color(theme.colors.foreground.secondary)
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
        row = row.bg(with_alpha(theme.colors.accent.foreground, 0.15));
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        MarkdownChangeHint, MarkdownInlineStyle, MarkdownPreviewImageSource,
        MarkdownPreviewPictureSizes, MarkdownPreviewRow, MarkdownPreviewRowKind,
        build_cached_diff_styled_text, history_message_text_left_px,
        history_scope_shows_graph_color_marker, history_worktree_node_color_ix,
        markdown_preview_alert_title_label,
        markdown_preview_expanded_slice_range, markdown_preview_image_source,
        markdown_preview_inline_highlight, markdown_preview_no_picture_sizes,
        markdown_preview_picture_skeleton, markdown_preview_row_background,
        markdown_preview_row_height, markdown_preview_row_horizontal_padding,
        markdown_preview_row_layout, markdown_preview_row_marker, markdown_preview_row_styled_text,
        markdown_preview_row_typography, worktree_preview_apply_query_overlay,
    };
    use crate::view::{
        HISTORY_COL_HANDLE_PX, HISTORY_MESSAGE_BORDER_GAP_PX, HISTORY_MESSAGE_BORDER_W_PX,
    };
    use crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY;
    use crate::view::history_graph;
    use crate::view::markdown_preview::MarkdownInlineSpan;
    use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};
    use crate::view::rows::diff_text::DIFF_WRAP_TAB_EXPANDED_COLUMNS;
    use crate::view::{AppTheme, DateTimeFormat, Timezone, format_datetime, format_datetime_utc};
    use gitcomet_core::domain::LogScope;
    use gpui::{FontWeight, SharedString, px};
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
            image: None,
            inline_images: Arc::from(Vec::new()),
            styled_text_cache: Default::default(),
            measured_width_px: Default::default(),
        }
    }

    #[test]
    fn worktree_preview_query_overlay_honors_search_options_for_cached_rows() {
        let theme = AppTheme::gitcomet_dark();
        let base = build_cached_diff_styled_text(
            theme,
            "Render render cat concat cat",
            &[],
            "",
            None,
            super::DiffSyntaxMode::Auto,
            None,
        );

        let case_sensitive_options = DiffSearchOptions {
            match_case: true,
            ..Default::default()
        };
        let case_sensitive_matcher = DiffSearchMatcher::new("render", case_sensitive_options);
        let case_sensitive = worktree_preview_apply_query_overlay(
            theme,
            base.clone(),
            Some(&case_sensitive_matcher),
        );
        let case_sensitive_ranges: Vec<_> = case_sensitive
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(case_sensitive_ranges, vec![7..13]);

        let whole_word_options = DiffSearchOptions {
            whole_word: true,
            ..Default::default()
        };
        let whole_word_matcher = DiffSearchMatcher::new("cat", whole_word_options);
        let whole_word =
            worktree_preview_apply_query_overlay(theme, base.clone(), Some(&whole_word_matcher));
        let whole_word_ranges: Vec<_> = whole_word
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(whole_word_ranges, vec![14..17, 25..28]);

        let regex_options = DiffSearchOptions {
            regex: true,
            ..Default::default()
        };
        let regex_matcher = DiffSearchMatcher::new(r"r.n.e.", regex_options);
        let regex = worktree_preview_apply_query_overlay(theme, base, Some(&regex_matcher));
        let regex_ranges: Vec<_> = regex
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect();
        assert_eq!(regex_ranges, vec![0..6, 7..13]);
    }

    /// The working-tree row borrows the lane colour *index* of the first commit
    /// so its connector can be washed like any other lane, rather than taking a
    /// resolved colour it could no longer compare against the selection.
    /// The commit rows paint their text on a canvas and the two
    /// uncommitted-changes rows lay theirs out as elements, so the offset they
    /// agree on has to come from one place — otherwise the message column steps
    /// sideways at every synthetic row.
    #[test]
    fn the_message_text_clears_the_lane_border_by_a_fixed_gap() {
        assert_eq!(
            history_message_text_left_px(true),
            HISTORY_MESSAGE_BORDER_W_PX + HISTORY_MESSAGE_BORDER_GAP_PX
        );
        assert!(
            history_message_text_left_px(true) > HISTORY_MESSAGE_BORDER_W_PX,
            "text that starts inside the border reads as touching it"
        );
    }

    /// Without the border there is nothing to clear, so the cell's own padding
    /// applies and the text does not jump left when the marker is off.
    #[test]
    fn the_message_text_falls_back_to_the_cell_padding_without_a_border() {
        assert_eq!(
            history_message_text_left_px(false),
            HISTORY_COL_HANDLE_PX / 2.0
        );
    }

    #[test]
    fn history_worktree_node_color_falls_back_to_the_primary_lane() {
        assert_eq!(history_worktree_node_color_ix(None), 0);
    }

    #[test]
    fn history_graph_color_marker_is_shown_for_all_non_first_parent_modes() {
        assert!(history_scope_shows_graph_color_marker(
            LogScope::FullReachable
        ));
        assert!(!history_scope_shows_graph_color_marker(
            LogScope::FirstParent
        ));
        assert!(history_scope_shows_graph_color_marker(LogScope::NoMerges));
        assert!(history_scope_shows_graph_color_marker(LogScope::MergesOnly));
        assert!(history_scope_shows_graph_color_marker(
            LogScope::AllBranches
        ));
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
                link_url: None,
            },
            MarkdownInlineSpan {
                byte_range: 5..10,
                style: MarkdownInlineStyle::Underline,
                link_url: None,
            },
            MarkdownInlineSpan {
                byte_range: 11..17,
                style: MarkdownInlineStyle::Strikethrough,
                link_url: None,
            },
            MarkdownInlineSpan {
                byte_range: 18..23,
                style: MarkdownInlineStyle::Normal,
                link_url: None,
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
    fn wrapped_slices_map_onto_the_tab_expanded_painted_text() {
        // Wrap ranges are measured on `row.text`, where a tab is one byte, but
        // the painted text expands each tab to four spaces. Slicing the
        // painted text with raw offsets shifted every wrapped row and dropped
        // the tail of the line.
        let raw = "\tab\tcd";
        let expanded_len =
            raw.len() + raw.matches('\t').count() * (DIFF_WRAP_TAB_EXPANDED_COLUMNS - 1);

        // "\tab" -> "    ab", "\tcd" -> "    cd"
        assert_eq!(
            markdown_preview_expanded_slice_range(raw, expanded_len, &(0..3)),
            0..6
        );
        assert_eq!(
            markdown_preview_expanded_slice_range(raw, expanded_len, &(3..raw.len())),
            6..expanded_len
        );
        // A row without tabs keeps its ranges untouched.
        assert_eq!(
            markdown_preview_expanded_slice_range("abcd", 4, &(1..3)),
            1..3
        );
    }

    #[test]
    fn image_paths_resolve_only_inside_the_documents_own_directory() {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet_md_image_path_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let nested = dir.join("assets");
        std::fs::create_dir_all(&nested).expect("create fixture dirs");
        let image = nested.join("shot.png");
        std::fs::write(&image, b"not really a png").expect("write fixture image");
        let outside = dir.parent().expect("temp dir parent").join("outside.png");
        std::fs::write(&outside, b"not really a png").expect("write outside fixture");

        let resolve = |source: &str| markdown_preview_image_source(Some(dir.as_path()), source);
        let file = |path: &std::path::Path| Some(MarkdownPreviewImageSource::File(path.to_owned()));
        let remote = |url: &str| {
            Some(MarkdownPreviewImageSource::Remote(SharedString::from(
                url.to_owned(),
            )))
        };

        assert_eq!(resolve("assets/shot.png"), file(&image));
        assert_eq!(resolve("./assets/shot.png"), file(&image));
        // Query and fragment suffixes are common in markdown image sources and
        // are not part of the file name.
        assert_eq!(resolve("assets/shot.png?v=2"), file(&image));
        assert_eq!(resolve("assets/shot.png#frag"), file(&image));

        // Badges and hosted screenshots resolve to the URL, query string and
        // all — that is what identifies the image.
        assert_eq!(
            resolve("https://img.shields.io/badge/a-b.svg?logo=x"),
            remote("https://img.shields.io/badge/a-b.svg?logo=x")
        );
        assert_eq!(
            resolve("http://example.com/a.png"),
            remote("http://example.com/a.png")
        );
        // Remote sources resolve without a base directory, since nothing is
        // resolved against the document's location.
        assert_eq!(
            markdown_preview_image_source(None, "https://example.com/a.png"),
            remote("https://example.com/a.png")
        );

        // A file that exists but sits outside the document's tree is refused,
        // so document content cannot aim the preview at arbitrary files.
        assert_eq!(resolve("../outside.png"), None);
        // Schemes a preview has no business dereferencing.
        assert_eq!(resolve("data:image/png;base64,AAAA"), None);
        assert_eq!(resolve("file:///etc/passwd"), None);
        assert_eq!(resolve("javascript:alert(1)"), None);
        // Missing files, empty sources, and a missing base directory resolve
        // to nothing.
        assert_eq!(resolve("assets/absent.png"), None);
        assert_eq!(resolve("   "), None);
        assert_eq!(markdown_preview_image_source(None, "assets/shot.png"), None);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    /// A picture row carrying `source`, and whatever size the document declared.
    fn picture_row(
        source: &str,
        width_px: Option<u32>,
        height_px: Option<u32>,
    ) -> MarkdownPreviewRow {
        let mut row = markdown_row(MarkdownPreviewRowKind::Image {
            slice_ix: 0,
            slice_count: 8,
        });
        row.image = Some(Arc::new(crate::view::markdown_preview::MarkdownImage {
            source: SharedString::from(source.to_owned()),
            width_px,
            height_px,
        }));
        row
    }

    fn measured(source: &str, width: u32, height: u32) -> MarkdownPreviewPictureSizes {
        Arc::new(
            [(SharedString::from(source.to_owned()), (width, height))]
                .into_iter()
                .collect(),
        )
    }

    #[test]
    fn a_skeleton_holds_the_box_the_picture_will_fill() {
        // The whole point of measuring a picture's header is that the space it
        // is going to take is reserved before it has been decoded, so the
        // document does not jump when it arrives.
        let empty = markdown_preview_no_picture_sizes();

        // Read from the file: the picture's own pixels, which is what an
        // undeclared picture lays out at.
        let skeleton = markdown_preview_picture_skeleton(
            &picture_row("demo.gif", None, None),
            100,
            &measured("demo.gif", 1280, 720),
        );
        assert_eq!(skeleton.width, Some(px(1280.0)));
        assert_eq!(skeleton.aspect_ratio, Some(1280.0 / 720.0));

        // A declared size wins, and scales with the UI the way the picture will.
        let skeleton = markdown_preview_picture_skeleton(
            &picture_row("demo.gif", Some(200), Some(100)),
            200,
            &measured("demo.gif", 1280, 720),
        );
        assert_eq!(skeleton.width, Some(px(400.0)));
        assert_eq!(skeleton.aspect_ratio, Some(2.0));

        // Nothing to go on: fall back to the rows the parser set aside, which
        // is all the row grid ever had.
        let skeleton =
            markdown_preview_picture_skeleton(&picture_row("demo.gif", None, None), 100, empty);
        assert_eq!(skeleton.width, None);
        assert_eq!(skeleton.aspect_ratio, None);
        assert_eq!(
            skeleton.reserved_height,
            markdown_preview_row_height(100) * 8.0
        );
    }

    #[test]
    fn a_picture_is_named_the_same_way_wherever_it_is_asked_about() {
        // The element that draws a picture and the pane waiting to hear that it
        // decoded look it up in the same cache, so both have to arrive at the
        // key `gpui` filed it under. Building the element one way and the key
        // another would leave the pane waiting on an entry nobody writes.
        let path = std::path::PathBuf::from("assets").join("shot.png");
        assert_eq!(
            MarkdownPreviewImageSource::File(path.clone()).to_resource(),
            gpui::Resource::Path(path.as_path().into())
        );
        assert_eq!(
            MarkdownPreviewImageSource::Remote(SharedString::from("https://example.com/a.png"))
                .to_resource(),
            gpui::Resource::Uri(gpui::SharedUri::from(
                "https://example.com/a.png".to_owned()
            ))
        );
    }

    #[test]
    fn heading_rows_are_inset_evenly_above_and_below() {
        // Headings used to carry more space below than above, so the text rode
        // high in its row instead of sitting centred in the break.
        for level in 1..=6u8 {
            let row = markdown_row(MarkdownPreviewRowKind::Heading { level });
            let layout =
                markdown_preview_row_layout(&row, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
            assert_eq!(
                layout.top_inset_px, layout.bottom_inset_px,
                "h{level} should be inset evenly: {layout:?}"
            );
        }
    }

    #[test]
    fn markdown_preview_row_styled_text_repairs_spans_that_split_a_multibyte_char() {
        // A span pointing inside a multi-byte character used to reach `gpui`
        // as a text run whose length splits that character, aborting the
        // process inside `str::split_at` while shaping the line.
        let theme = AppTheme::gitcomet_light();

        let mut row = markdown_row(MarkdownPreviewRowKind::Paragraph);
        row.text = SharedString::from("— dash —");
        row.inline_spans = Arc::new(vec![
            MarkdownInlineSpan {
                byte_range: 0..1,
                style: MarkdownInlineStyle::Bold,
                link_url: None,
            },
            MarkdownInlineSpan {
                byte_range: 6..9,
                style: MarkdownInlineStyle::Italic,
                link_url: None,
            },
        ]);

        let styled = markdown_preview_row_styled_text(theme, &row);
        let text = styled.text.as_ref();

        for (range, _) in styled.highlights.iter() {
            assert!(
                text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                "highlight {range:?} splits a char in {text:?}"
            );
        }
        assert_eq!(styled.highlights[0].0, 0..3);
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
            image: None,
            inline_images: Arc::from(Vec::new()),
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
