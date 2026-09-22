use super::*;

#[derive(Clone)]
pub(in crate::view) struct WorktreePreviewPreparedSyntaxSource {
    pub(in crate::view) document_text: Arc<str>,
    pub(in crate::view) line_starts: Arc<[usize]>,
    pub(in crate::view) document: rows::PreparedDiffSyntaxDocument,
}

pub(in crate::view) fn worktree_preview_apply_query_overlay(
    theme: AppTheme,
    styled: CachedDiffStyledText,
    query_matcher: Option<&DiffSearchMatcher>,
    emphasis: DiffSearchMatchEmphasis,
) -> CachedDiffStyledText {
    query_matcher
        .map(|matcher| {
            build_cached_diff_query_overlay_styled_text(theme, &styled, matcher, emphasis)
        })
        .unwrap_or(styled)
}

pub(in crate::view) fn worktree_preview_streamed_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    line_ix: usize,
    query: &SharedString,
    query_options: super::super::panes::main::diff_search::DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    query_emphasis: DiffSearchMatchEmphasis,
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
            query_emphasis,
            word_ranges: Arc::from([]),
            word_kind: None,
            syntax,
        }
    })
}

impl MainPaneView {
    pub(in crate::view) fn render_worktree_preview_rows(
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

        let current_match_line = this.diff_search_current_match_row();
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
                // This view has no selection of its own, so the row the cursor is
                // on wears the selection wash to stand out from the rest.
                let emphasis = if current_match_line == Some(ix) {
                    DiffSearchMatchEmphasis::Current
                } else {
                    DiffSearchMatchEmphasis::Other
                };
                let is_current_match = emphasis == DiffSearchMatchEmphasis::Current;
                let streamed_spec = worktree_preview_streamed_spec(
                    raw_text.clone(),
                    ix,
                    &query,
                    query_options,
                    query_matcher.clone(),
                    emphasis,
                    language,
                    syntax_mode,
                    prepared_syntax_source.as_ref(),
                );
                let mut pending_styled = None;
                // The current row is rebuilt rather than read from the cache,
                // which holds the plain wash: re-washing an already-washed row
                // would keep the foreground the first pass pinned on light themes.
                // One row per frame, the cost of a cache miss.
                if streamed_spec.is_none()
                    && (is_current_match || this.worktree_preview_segments_cache_get(ix).is_none())
                {
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
                                    word_kind: None,
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
                        emphasis,
                    );
                    if is_pending {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                        pending_styled = Some(styled);
                    } else {
                        // Never cached while current: the cursor moves off it, and
                        // a cached entry would leave that row painted as current.
                        if defer_cache_write || is_current_match {
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
            let row = markdown_preview_list_row(row);
            let required = markdown_preview_row_required_width(
                window,
                self.theme,
                &row,
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
