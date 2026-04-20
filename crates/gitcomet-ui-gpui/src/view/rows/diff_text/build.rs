use super::*;

fn maybe_expand_tabs(s: &str) -> SharedString {
    if !s.contains('\t') {
        return SharedString::new(s);
    }

    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
    }
    out.into()
}

#[inline]
pub(super) fn segment_overlaps_sorted_ranges(
    segment_start: usize,
    segment_end: usize,
    ranges: &[Range<usize>],
    cursor: &mut usize,
) -> bool {
    while *cursor < ranges.len() && ranges[*cursor].end <= segment_start {
        *cursor += 1;
    }

    ranges
        .get(*cursor)
        .is_some_and(|range| segment_start < range.end && segment_end > range.start)
}

#[cfg(test)]
pub(super) fn build_diff_text_segments(
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    syntax_tokens_override: Option<&[syntax::SyntaxToken]>,
) -> Vec<CachedDiffTextSegment> {
    if text.is_empty() {
        return Vec::new();
    }

    let query = query.trim();
    if word_ranges.is_empty()
        && query.is_empty()
        && language.is_none()
        && syntax_tokens_override.is_none()
    {
        return vec![CachedDiffTextSegment {
            text: maybe_expand_tabs(text),
            in_word: false,
            in_query: false,
            syntax: SyntaxTokenKind::None,
        }];
    }

    let owned_syntax_tokens = if syntax_tokens_override.is_none() {
        language.map(|language| {
            let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
            syntax::syntax_tokens_for_line_shared(text, language, syntax_mode)
        })
    } else {
        None
    };
    let syntax_tokens = if let Some(tokens) = syntax_tokens_override {
        tokens
    } else if let Some(language) = language {
        let _ = language;
        owned_syntax_tokens.as_deref().unwrap_or(&[])
    } else {
        &[]
    };

    let _word_query_scope = perf::span(ViewPerfSpan::WordQueryHighlighting);
    let query_ranges = if !query.is_empty() {
        find_all_ascii_case_insensitive(text, query)
    } else {
        Default::default()
    };

    thread_local! {
        static BOUNDARY_BUF: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    BOUNDARY_BUF.with_borrow_mut(|boundaries| {
        boundaries.clear();
        boundaries.push(0);
        boundaries.push(text.len());
        for r in word_ranges {
            boundaries.push(r.start.min(text.len()));
            boundaries.push(r.end.min(text.len()));
        }
        for r in &query_ranges {
            boundaries.push(r.start);
            boundaries.push(r.end);
        }
        for t in syntax_tokens {
            boundaries.push(t.range.start.min(text.len()));
            boundaries.push(t.range.end.min(text.len()));
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut token_ix = 0usize;
        let mut word_ix = 0usize;
        let mut query_ix = 0usize;
        let mut segments = Vec::with_capacity(boundaries.len().saturating_sub(1));
        for w in boundaries.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b || a >= text.len() {
                continue;
            }
            let b = b.min(text.len());
            let Some(seg) = text.get(a..b) else {
                return vec![CachedDiffTextSegment {
                    text: maybe_expand_tabs(text),
                    in_word: false,
                    in_query: false,
                    syntax: SyntaxTokenKind::None,
                }];
            };

            while token_ix < syntax_tokens.len() && syntax_tokens[token_ix].range.end <= a {
                token_ix += 1;
            }
            let syntax = syntax_tokens
                .get(token_ix)
                .filter(|t| t.range.start <= a && t.range.end >= b)
                .map(|t| t.kind)
                .unwrap_or(SyntaxTokenKind::None);

            let in_word = segment_overlaps_sorted_ranges(a, b, word_ranges, &mut word_ix);
            let in_query = segment_overlaps_sorted_ranges(a, b, &query_ranges, &mut query_ix);

            segments.push(CachedDiffTextSegment {
                text: maybe_expand_tabs(seg),
                in_word,
                in_query,
                syntax,
            });
        }

        segments
    })
}

pub(in super::super) fn selectable_cached_diff_text(
    visible_ix: usize,
    region: DiffTextRegion,
    double_click_kind: DiffClickKind,
    base_fg: gpui::Rgba,
    styled: Option<&CachedDiffStyledText>,
    fallback_text: SharedString,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let view = cx.entity();
    let (text, highlights) = if let Some(styled) = styled {
        (styled.text.clone(), Arc::clone(&styled.highlights))
    } else {
        (fallback_text, empty_highlights())
    };

    let overlay_text = text.clone();
    let overlay = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(DiffTextSelectionOverlay {
            view: view.clone(),
            visible_ix,
            region,
            text: overlay_text,
        });

    let content = if text.is_empty() {
        div().into_any_element()
    } else if highlights.is_empty() {
        div()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(text.clone())
            .into_any_element()
    } else {
        div()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(gpui::StyledText::new(text.clone()).with_highlights(highlights.iter().cloned()))
            .into_any_element()
    };

    div()
        .relative()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(base_fg)
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                window.focus(&this.diff_panel_focus_handle, cx);
                if e.click_count >= 2 {
                    cx.stop_propagation();
                }
                this.handle_diff_text_mouse_down(visible_ix, region, e.position, e.click_count, cx);
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _w, cx| {
            if !this.diff_text_selecting {
                return;
            }
            let before = this.diff_text_head;
            this.update_diff_text_selection_from_mouse(e.position);
            if this.diff_text_head != before {
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                this.end_diff_text_selection();
                cx.notify();
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                this.end_diff_text_selection();
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                if double_click_kind == DiffClickKind::HunkHeader {
                    return;
                }
                cx.stop_propagation();
                this.open_diff_editor_context_menu(visible_ix, region, e.position, window, cx);
            }),
        )
        .child(overlay)
        .child(content)
        .into_any_element()
}

fn empty_highlights() -> SharedDiffTextHighlights {
    static EMPTY: OnceLock<SharedDiffTextHighlights> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

pub(super) fn hash_text_content(text: &str) -> u64 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

fn hash_rgba_bits(hasher: &mut FxHasher, rgba: gpui::Rgba) {
    rgba.r.to_bits().hash(hasher);
    rgba.g.to_bits().hash(hasher);
    rgba.b.to_bits().hash(hasher);
    rgba.a.to_bits().hash(hasher);
}

pub(super) fn syntax_theme_signature(theme: AppTheme) -> u64 {
    let mut hasher = FxHasher::default();
    let syntax = theme.syntax;
    hash_rgba_bits(&mut hasher, syntax.comment);
    hash_rgba_bits(&mut hasher, syntax.comment_doc);
    hash_rgba_bits(&mut hasher, syntax.string);
    hash_rgba_bits(&mut hasher, syntax.string_escape);
    hash_rgba_bits(&mut hasher, syntax.keyword);
    hash_rgba_bits(&mut hasher, syntax.keyword_control);
    hash_rgba_bits(&mut hasher, syntax.number);
    hash_rgba_bits(&mut hasher, syntax.boolean);
    hash_rgba_bits(&mut hasher, syntax.function);
    hash_rgba_bits(&mut hasher, syntax.function_method);
    hash_rgba_bits(&mut hasher, syntax.function_special);
    hash_rgba_bits(&mut hasher, syntax.type_name);
    hash_rgba_bits(&mut hasher, syntax.type_builtin);
    hash_rgba_bits(&mut hasher, syntax.type_interface);
    syntax.variable.is_some().hash(&mut hasher);
    if let Some(variable) = syntax.variable {
        hash_rgba_bits(&mut hasher, variable);
    }
    hash_rgba_bits(&mut hasher, syntax.variable_parameter);
    hash_rgba_bits(&mut hasher, syntax.variable_special);
    hash_rgba_bits(&mut hasher, syntax.property);
    hash_rgba_bits(&mut hasher, syntax.constant);
    hash_rgba_bits(&mut hasher, syntax.operator);
    hash_rgba_bits(&mut hasher, syntax.punctuation);
    hash_rgba_bits(&mut hasher, syntax.punctuation_bracket);
    hash_rgba_bits(&mut hasher, syntax.punctuation_delimiter);
    hash_rgba_bits(&mut hasher, syntax.tag);
    hash_rgba_bits(&mut hasher, syntax.attribute);
    hash_rgba_bits(&mut hasher, syntax.lifetime);
    hasher.finish()
}

pub(super) fn should_cache_single_line_styled_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= SINGLE_LINE_STYLED_TEXT_CACHE_MAX_SOURCE_BYTES
}

fn styled_text_to_cached(
    text: SharedString,
    highlights: Vec<DiffTextHighlight>,
) -> CachedDiffStyledText {
    let text_hash = hash_text_content(text.as_ref());

    if highlights.is_empty() {
        return CachedDiffStyledText {
            text,
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash,
        };
    }

    let highlights_hash = hash_highlights(&highlights);
    CachedDiffStyledText {
        text,
        highlights: Arc::from(highlights),
        highlights_hash,
        text_hash,
    }
}

/// Like [`styled_text_to_cached`] but borrows the highlights buffer instead of
/// consuming it.  Creates the `Arc<[…]>` from a slice copy so the caller can
/// reuse the Vec across calls (thread_local pattern).
pub(super) fn styled_text_to_cached_from_buf(
    text: &str,
    highlights: &DiffTextHighlights,
) -> CachedDiffStyledText {
    if text.is_empty() {
        return empty_styled_text();
    }

    let text = if !text.contains('\t') {
        SharedString::new(text)
    } else {
        let (expanded, remapped) = expanded_text_and_remapped_relative_highlights(text, highlights);
        let text_hash = hash_text_content(expanded.as_ref());

        if remapped.is_empty() {
            return CachedDiffStyledText {
                text: expanded,
                highlights: empty_highlights(),
                highlights_hash: 0,
                text_hash,
            };
        }

        let highlights_hash = hash_highlights(&remapped);
        return CachedDiffStyledText {
            text: expanded,
            highlights: Arc::from(remapped),
            highlights_hash,
            text_hash,
        };
    };

    let text_hash = hash_text_content(text.as_ref());

    if highlights.is_empty() {
        return CachedDiffStyledText {
            text,
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash,
        };
    }

    let highlights_hash = hash_highlights(highlights);
    CachedDiffStyledText {
        text,
        highlights: Arc::from(highlights),
        highlights_hash,
        text_hash,
    }
}

#[cfg(test)]
pub(super) fn segments_to_cached_styled_text(
    theme: AppTheme,
    segments: &[CachedDiffTextSegment],
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    let (expanded_text, highlights) = styled_text_for_diff_segments(theme, segments, word_color);
    styled_text_to_cached(expanded_text, highlights)
}

/// Fused version of `build_diff_text_segments` + `segments_to_cached_styled_text`.
///
/// Builds the combined styled text and highlights in a single pass over the
/// boundary windows, skipping intermediate `Vec<CachedDiffTextSegment>` and
/// per-segment `SharedString` allocations.
pub(super) fn build_styled_text_fused(
    theme: AppTheme,
    request: FusedDiffTextBuildRequest<'_>,
) -> CachedDiffStyledText {
    let text = request.build.text;
    let word_ranges = request.build.word_ranges;
    let query = request.build.query;
    let word_color = request.build.word_color;
    let DiffSyntaxConfig {
        language,
        mode: syntax_mode,
    } = request.build.syntax;
    let syntax_tokens_override = request.syntax_tokens_override;

    if text.is_empty() {
        return empty_styled_text();
    }

    let query = query.trim();
    if word_ranges.is_empty()
        && query.is_empty()
        && language.is_none()
        && syntax_tokens_override.is_none()
    {
        let expanded = maybe_expand_tabs(text);
        return styled_text_to_cached(expanded, Vec::new());
    }

    let owned_syntax_tokens = if syntax_tokens_override.is_none() {
        language.map(|language| {
            let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
            syntax::syntax_tokens_for_line_shared(text, language, syntax_mode)
        })
    } else {
        None
    };
    let syntax_tokens = if let Some(tokens) = syntax_tokens_override {
        tokens
    } else if let Some(language) = language {
        let _ = language;
        owned_syntax_tokens.as_deref().unwrap_or(&[])
    } else {
        &[]
    };

    let _word_query_scope = perf::span(ViewPerfSpan::WordQueryHighlighting);
    let query_ranges = if !query.is_empty() {
        find_all_ascii_case_insensitive(text, query)
    } else {
        Default::default()
    };

    thread_local! {
        static FUSED_BOUNDARY_BUF: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    FUSED_BOUNDARY_BUF.with_borrow_mut(|boundaries| {
        boundaries.clear();
        boundaries.push(0);
        boundaries.push(text.len());
        for r in word_ranges {
            boundaries.push(r.start.min(text.len()));
            boundaries.push(r.end.min(text.len()));
        }
        for r in &query_ranges {
            boundaries.push(r.start);
            boundaries.push(r.end);
        }
        for t in syntax_tokens {
            boundaries.push(t.range.start.min(text.len()));
            boundaries.push(t.range.end.min(text.len()));
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        let has_tabs = text.contains('\t');
        let mut combined = String::with_capacity(if has_tabs {
            text.len() + text.len() / 8
        } else {
            text.len()
        });
        let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> =
            Vec::with_capacity(boundaries.len().saturating_sub(1));

        let mut token_ix = 0usize;
        let mut word_ix = 0usize;
        let mut query_ix = 0usize;

        for w in boundaries.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b || a >= text.len() {
                continue;
            }
            let b = b.min(text.len());
            let Some(seg) = text.get(a..b) else {
                // Fallback: return whole text expanded, no highlights.
                let expanded = maybe_expand_tabs(text);
                return styled_text_to_cached(expanded, Vec::new());
            };

            while token_ix < syntax_tokens.len() && syntax_tokens[token_ix].range.end <= a {
                token_ix += 1;
            }
            let syntax = syntax_tokens
                .get(token_ix)
                .filter(|t| t.range.start <= a && t.range.end >= b)
                .map(|t| t.kind)
                .unwrap_or(SyntaxTokenKind::None);

            let in_word = segment_overlaps_sorted_ranges(a, b, word_ranges, &mut word_ix);
            let in_query = segment_overlaps_sorted_ranges(a, b, &query_ranges, &mut query_ix);

            let offset = combined.len();
            if has_tabs && seg.contains('\t') {
                for ch in seg.chars() {
                    match ch {
                        '\t' => combined.push_str("    "),
                        _ => combined.push(ch),
                    }
                }
            } else {
                combined.push_str(seg);
            }
            let next_offset = combined.len();

            let mut style = gpui::HighlightStyle::default();

            if in_word && let Some(mut c) = word_color {
                c.a = if theme.is_dark { 0.22 } else { 0.16 };
                style.background_color = Some(c.into());
            }

            if in_query {
                style.background_color = Some(
                    with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 }).into(),
                );
            }

            let syntax_fg = syntax_highlight_color(theme, syntax);
            if let Some(fg) = syntax_fg {
                style.color = Some(fg.into());
            }

            if style != gpui::HighlightStyle::default() && offset < next_offset {
                highlights.push((offset..next_offset, style));
            }
        }

        styled_text_to_cached(combined.into(), highlights)
    })
}

pub(in super::super) fn build_cached_diff_styled_text_from_relative_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> CachedDiffStyledText {
    build_cached_diff_styled_text_from_owned_relative_highlights(text, highlights.to_vec())
}

fn build_cached_diff_styled_text_from_owned_relative_highlights(
    text: &str,
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
) -> CachedDiffStyledText {
    if text.is_empty() {
        return empty_styled_text();
    }

    if !text.contains('\t') {
        return styled_text_to_cached(SharedString::new(text), highlights);
    }

    let (expanded_text, remapped_highlights) =
        expanded_text_and_remapped_relative_highlights(text, &highlights);
    styled_text_to_cached(expanded_text, remapped_highlights)
}

fn empty_styled_text() -> CachedDiffStyledText {
    styled_text_to_cached("".into(), Vec::new())
}

pub(in super::super) fn build_cached_diff_styled_text(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    build_cached_diff_styled_text_with_optional_palette(
        theme,
        None,
        DiffTextBuildRequest {
            text,
            word_ranges,
            query,
            syntax: DiffSyntaxConfig {
                language,
                mode: syntax_mode,
            },
            word_color,
        },
        None,
    )
}

pub(in super::super) fn build_cached_diff_styled_text_with_source_identity(
    theme: AppTheme,
    text: &str,
    source_identity: Option<DiffTextSourceIdentity>,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    build_cached_diff_styled_text_with_optional_palette(
        theme,
        None,
        DiffTextBuildRequest {
            text,
            word_ranges,
            query,
            syntax: DiffSyntaxConfig {
                language,
                mode: syntax_mode,
            },
            word_color,
        },
        source_identity,
    )
}

#[cfg(feature = "benchmarks")]
pub(in super::super) fn build_cached_diff_styled_text_with_palette(
    theme: AppTheme,
    highlight_palette: &SyntaxHighlightPalette,
    request: DiffTextBuildRequest<'_>,
) -> CachedDiffStyledText {
    build_cached_diff_styled_text_with_optional_palette(
        theme,
        Some(highlight_palette),
        request,
        None,
    )
}

thread_local! {
    pub(super) static SYNTAX_HIGHLIGHTS_BUF: RefCell<Vec<DiffTextHighlight>> = const { RefCell::new(Vec::new()) };
    pub(super) static SINGLE_LINE_STYLED_TEXT_CACHE: RefCell<SingleLineStyledTextCache> = RefCell::new(SingleLineStyledTextCache::new());
}

fn build_cached_diff_styled_text_with_optional_palette(
    theme: AppTheme,
    highlight_palette: Option<&SyntaxHighlightPalette>,
    request: DiffTextBuildRequest<'_>,
    source_identity: Option<DiffTextSourceIdentity>,
) -> CachedDiffStyledText {
    let text = request.text;
    let word_ranges = request.word_ranges;
    let query = request.query;
    let DiffSyntaxConfig {
        language,
        mode: syntax_mode,
    } = request.syntax;

    if text.is_empty() {
        return empty_styled_text();
    }

    let query = query.trim();
    if word_ranges.is_empty() && query.is_empty() {
        let build_syntax_only = || {
            SYNTAX_HIGHLIGHTS_BUF.with_borrow_mut(|buf| {
                if let Some(language) = language {
                    let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
                    let tokens = syntax::syntax_tokens_for_line_shared(text, language, syntax_mode);
                    match highlight_palette {
                        Some(palette) => {
                            prepared_document_line_highlights_from_tokens_into_with_palette(
                                palette,
                                text.len(),
                                &tokens,
                                buf,
                            );
                        }
                        None => {
                            prepared_document_line_highlights_from_tokens_into(
                                theme,
                                text.len(),
                                &tokens,
                                buf,
                            );
                        }
                    }
                } else {
                    buf.clear();
                }
                styled_text_to_cached_from_buf(text, buf)
            })
        };

        if highlight_palette.is_none()
            && let Some(language) = language
            && should_cache_single_line_styled_text(text)
        {
            let (key, cached) = SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                let key = cache.key_for(theme, language, syntax_mode, text, source_identity);
                let styled = cache.get(key, text);
                (key, styled)
            });
            if let Some(styled) = cached {
                return styled;
            }

            let styled = build_syntax_only();
            SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
                cache.borrow_mut().insert(key, text, styled.clone());
            });
            return styled;
        }

        return build_syntax_only();
    }

    if highlight_palette.is_none()
        && query.is_empty()
        && request.word_color.is_none()
        && !word_ranges.is_empty()
        && should_cache_single_line_styled_text(text)
    {
        let (key, cached) = SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let key = cache.word_highlighted_key_for(
                theme,
                language,
                syntax_mode,
                text,
                source_identity,
                word_ranges,
            );
            let styled = cache.get_word_highlighted(key, text, word_ranges);
            (key, styled)
        });
        if let Some(styled) = cached {
            return styled;
        }

        let styled = build_styled_text_fused(
            theme,
            FusedDiffTextBuildRequest {
                build: request,
                syntax_tokens_override: None,
            },
        );
        SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .insert_word_highlighted(key, text, word_ranges, styled.clone());
        });
        return styled;
    }

    build_styled_text_fused(
        theme,
        FusedDiffTextBuildRequest {
            build: request,
            syntax_tokens_override: None,
        },
    )
}

pub(in super::super) fn build_cached_diff_query_overlay_styled_text(
    theme: AppTheme,
    base: &CachedDiffStyledText,
    query: &str,
) -> CachedDiffStyledText {
    let query = query.trim();
    if query.is_empty() || base.text.is_empty() {
        return base.clone();
    }

    thread_local! {
        static QUERY_OVERLAY_RANGES_BUF: RefCell<Vec<Range<usize>>> = const { RefCell::new(Vec::new()) };
    }

    QUERY_OVERLAY_RANGES_BUF.with_borrow_mut(|query_ranges| {
        find_all_ascii_case_insensitive_into(base.text.as_ref(), query, query_ranges);
        if query_ranges.is_empty() {
            return base.clone();
        }

        let base_highlights = base.highlights.as_ref();
        let query_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 }).into();
        if base_highlights.is_empty() {
            let mut merged = Vec::with_capacity(query_ranges.len());
            for range in query_ranges.iter().cloned() {
                push_or_extend_highlight(
                    &mut merged,
                    range,
                    gpui::HighlightStyle {
                        background_color: Some(query_bg),
                        ..gpui::HighlightStyle::default()
                    },
                );
            }
            let highlights_hash =
                hash_query_overlay_highlights(base.highlights_hash, query_ranges, query_bg);
            return CachedDiffStyledText {
                text: base.text.clone(),
                highlights: Arc::from(merged),
                highlights_hash,
                text_hash: base.text_hash,
            };
        }

        let mut merged: Vec<(Range<usize>, gpui::HighlightStyle)> =
            Vec::with_capacity(base_highlights.len() + query_ranges.len() * 2);
        let mut base_ix = 0usize;
        let mut query_ix = 0usize;
        let mut cursor = 0usize;
        let text_len = base.text.len();
        let default_style = gpui::HighlightStyle::default();

        while cursor < text_len {
            while base_ix < base_highlights.len() && base_highlights[base_ix].0.end <= cursor {
                base_ix += 1;
            }
            while query_ix < query_ranges.len() && query_ranges[query_ix].end <= cursor {
                query_ix += 1;
            }

            let active_base = base_highlights
                .get(base_ix)
                .filter(|(range, _)| range.start <= cursor && range.end > cursor);
            let active_query = query_ranges
                .get(query_ix)
                .filter(|range| range.start <= cursor && range.end > cursor);

            let mut next_boundary = text_len;
            if let Some((range, _)) = active_base {
                next_boundary = next_boundary.min(range.end.min(text_len));
            } else if let Some((range, _)) = base_highlights.get(base_ix) {
                next_boundary = next_boundary.min(range.start.min(text_len));
            }
            if let Some(range) = active_query {
                next_boundary = next_boundary.min(range.end.min(text_len));
            } else if let Some(range) = query_ranges.get(query_ix) {
                next_boundary = next_boundary.min(range.start.min(text_len));
            }

            if next_boundary <= cursor {
                break;
            }

            let mut style = active_base.map(|(_, style)| *style).unwrap_or_default();
            if active_query.is_some() {
                style.background_color = Some(query_bg);
            }

            if style != default_style {
                push_or_extend_highlight(&mut merged, cursor..next_boundary, style);
            }

            cursor = next_boundary;
        }

        if merged.is_empty() {
            return CachedDiffStyledText {
                text: base.text.clone(),
                highlights: empty_highlights(),
                highlights_hash: 0,
                text_hash: base.text_hash,
            };
        }

        let highlights_hash =
            hash_query_overlay_highlights(base.highlights_hash, query_ranges, query_bg);
        CachedDiffStyledText {
            text: base.text.clone(),
            highlights: Arc::from(merged),
            highlights_hash,
            text_hash: base.text_hash,
        }
    })
}

fn push_or_extend_highlight(
    merged: &mut Vec<DiffTextHighlight>,
    range: Range<usize>,
    style: gpui::HighlightStyle,
) {
    if range.is_empty() {
        return;
    }

    if let Some(last) = merged.last_mut()
        && last.0.end == range.start
        && last.1 == style
    {
        last.0.end = range.end;
        return;
    }

    merged.push((range, style));
}

fn hash_highlights(highlights: &[(Range<usize>, gpui::HighlightStyle)]) -> u64 {
    let mut hasher = FxHasher::default();
    for (range, style) in highlights {
        range.hash(&mut hasher);
        style.hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_query_overlay_highlights(
    base_highlights_hash: u64,
    query_ranges: &[Range<usize>],
    query_bg: gpui::Hsla,
) -> u64 {
    let mut hasher = FxHasher::default();
    base_highlights_hash.hash(&mut hasher);
    query_bg.hash(&mut hasher);
    for range in query_ranges {
        range.hash(&mut hasher);
    }
    hasher.finish()
}

pub(super) fn hash_word_ranges(ranges: &[Range<usize>]) -> u64 {
    let mut hasher = FxHasher::default();
    for range in ranges {
        range.hash(&mut hasher);
    }
    hasher.finish()
}

fn syntax_highlight_color(theme: AppTheme, kind: SyntaxTokenKind) -> Option<gpui::Rgba> {
    match kind {
        SyntaxTokenKind::None => None,
        SyntaxTokenKind::Comment => Some(theme.syntax.comment),
        SyntaxTokenKind::CommentDoc => Some(theme.syntax.comment_doc),
        SyntaxTokenKind::String => Some(theme.syntax.string),
        SyntaxTokenKind::StringEscape => Some(theme.syntax.string_escape),
        SyntaxTokenKind::Keyword => Some(theme.syntax.keyword),
        SyntaxTokenKind::KeywordControl => Some(theme.syntax.keyword_control),
        SyntaxTokenKind::Number => Some(theme.syntax.number),
        SyntaxTokenKind::Boolean => Some(theme.syntax.boolean),
        SyntaxTokenKind::Function => Some(theme.syntax.function),
        SyntaxTokenKind::FunctionMethod => Some(theme.syntax.function_method),
        SyntaxTokenKind::FunctionSpecial => Some(theme.syntax.function_special),
        SyntaxTokenKind::Type => Some(theme.syntax.type_name),
        SyntaxTokenKind::TypeBuiltin => Some(theme.syntax.type_builtin),
        SyntaxTokenKind::TypeInterface => Some(theme.syntax.type_interface),
        SyntaxTokenKind::Variable => theme.syntax.variable,
        SyntaxTokenKind::VariableParameter => Some(theme.syntax.variable_parameter),
        SyntaxTokenKind::VariableSpecial => Some(theme.syntax.variable_special),
        SyntaxTokenKind::Property => Some(theme.syntax.property),
        SyntaxTokenKind::Constant => Some(theme.syntax.constant),
        SyntaxTokenKind::Operator => Some(theme.syntax.operator),
        SyntaxTokenKind::Punctuation => Some(theme.syntax.punctuation),
        SyntaxTokenKind::PunctuationBracket => Some(theme.syntax.punctuation_bracket),
        SyntaxTokenKind::PunctuationDelimiter => Some(theme.syntax.punctuation_delimiter),
        SyntaxTokenKind::Tag => Some(theme.syntax.tag),
        SyntaxTokenKind::Attribute => Some(theme.syntax.attribute),
        SyntaxTokenKind::Lifetime => Some(theme.syntax.lifetime),
    }
}

pub(super) fn syntax_highlight_style(
    theme: AppTheme,
    kind: SyntaxTokenKind,
) -> Option<gpui::HighlightStyle> {
    let fg = syntax_highlight_color(theme, kind)?;
    let mut style = gpui::HighlightStyle {
        color: Some(fg.into()),
        ..gpui::HighlightStyle::default()
    };
    match kind {
        // Doc comments render italic to distinguish from regular comments.
        SyntaxTokenKind::CommentDoc => {
            style.font_style = Some(gpui::FontStyle::Italic);
        }
        // Control-flow keywords (if/else/for/while/return/match) render semibold.
        SyntaxTokenKind::KeywordControl => {
            style.font_weight = Some(gpui::FontWeight::SEMIBOLD);
        }
        _ => {}
    }
    Some(style)
}

fn expanded_text_and_remapped_relative_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    if !text.contains('\t') {
        return (SharedString::new(text), highlights.to_vec());
    }

    let mut out = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];

    for (start, ch) in text.char_indices() {
        byte_map[start] = out.len();
        match ch {
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
        let end = start + ch.len_utf8();
        let mapped_end = out.len();
        for mapped in byte_map.iter_mut().take(end + 1).skip(start + 1) {
            *mapped = mapped_end;
        }
    }

    let mut remapped = Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        let start = *byte_map
            .get(range.start.min(text.len()))
            .unwrap_or(&out.len());
        let end = *byte_map
            .get(range.end.min(text.len()))
            .unwrap_or(&out.len());
        if start < end {
            remapped.push((start..end, *style));
        }
    }

    (out.into(), remapped)
}

pub(super) fn line_range_for_absolute_byte_window(
    line_starts: &[usize],
    text_len: usize,
    byte_range: &Range<usize>,
) -> Range<usize> {
    if line_starts.is_empty() || text_len == 0 {
        return 0..0;
    }

    let start = byte_range.start.min(text_len);
    let end = byte_range.end.min(text_len);
    if start >= end {
        return 0..0;
    }

    let start_line = line_starts
        .partition_point(|&line_start| line_start <= start)
        .saturating_sub(1);
    let end_line = line_starts
        .partition_point(|&line_start| line_start <= end.saturating_sub(1))
        .saturating_sub(1);
    start_line..end_line.saturating_add(1)
}

/// Returns `(line_start, line_end)` byte offsets for a zero-based line index,
/// stripping any trailing newline. Both values are clamped to `text.len()`.
pub(super) fn line_byte_bounds(
    text: &str,
    line_starts: &[usize],
    line_ix: usize,
) -> (usize, usize) {
    let text_len = text.len();
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    (start, end)
}

/// Clip a line-relative range to an absolute clamped window and push if non-empty.
fn clip_and_push_line_highlight(
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
    line_start: usize,
    line_end: usize,
    clamped_range: &Range<usize>,
    relative_range: Range<usize>,
    style: gpui::HighlightStyle,
) {
    if relative_range.start >= relative_range.end {
        return;
    }
    let absolute_start = line_start
        .saturating_add(relative_range.start)
        .min(line_end);
    let absolute_end = line_start.saturating_add(relative_range.end).min(line_end);
    let clipped_start = absolute_start.max(clamped_range.start);
    let clipped_end = absolute_end.min(clamped_range.end);
    if clipped_start < clipped_end {
        highlights.push((clipped_start..clipped_end, style));
    }
}

pub(super) fn prepared_document_line_highlights_from_tokens(
    theme: AppTheme,
    line_len: usize,
    tokens: &[syntax::SyntaxToken],
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let mut highlights = Vec::with_capacity(tokens.len());
    prepared_document_line_highlights_from_tokens_into(theme, line_len, tokens, &mut highlights);
    highlights
}

pub(super) fn prepared_document_line_highlights_from_tokens_into(
    theme: AppTheme,
    line_len: usize,
    tokens: &[syntax::SyntaxToken],
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
) {
    highlights.clear();
    if tokens.is_empty() {
        return;
    }
    let additional = tokens.len().saturating_sub(highlights.capacity());
    if additional > 0 {
        highlights.reserve(additional);
    }
    for token in tokens {
        if let Some((range, style)) = prepared_document_highlight_from_token(theme, line_len, token)
        {
            highlights.push((range, style));
        }
    }
}

pub(super) fn prepared_document_line_highlights_from_tokens_into_with_palette(
    highlight_palette: &SyntaxHighlightPalette,
    line_len: usize,
    tokens: &[syntax::SyntaxToken],
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
) {
    highlights.clear();
    if tokens.is_empty() {
        return;
    }
    let additional = tokens.len().saturating_sub(highlights.capacity());
    if additional > 0 {
        highlights.reserve(additional);
    }
    for token in tokens {
        if let Some((range, style)) =
            prepared_document_highlight_from_token_with_palette(highlight_palette, line_len, token)
        {
            highlights.push((range, style));
        }
    }
}

fn prepared_document_highlight_from_token(
    theme: AppTheme,
    line_len: usize,
    token: &syntax::SyntaxToken,
) -> Option<(Range<usize>, gpui::HighlightStyle)> {
    prepared_document_highlight_from_token_with_style(
        line_len,
        token,
        syntax_highlight_style(theme, token.kind),
    )
}

fn prepared_document_highlight_from_token_with_palette(
    highlight_palette: &SyntaxHighlightPalette,
    line_len: usize,
    token: &syntax::SyntaxToken,
) -> Option<(Range<usize>, gpui::HighlightStyle)> {
    prepared_document_highlight_from_token_with_style(
        line_len,
        token,
        highlight_palette.style(token.kind),
    )
}

fn prepared_document_highlight_from_token_with_style(
    line_len: usize,
    token: &syntax::SyntaxToken,
    style: Option<gpui::HighlightStyle>,
) -> Option<(Range<usize>, gpui::HighlightStyle)> {
    let style = style?;
    if token.range.start >= token.range.end || token.range.start >= line_len {
        return None;
    }
    let end = token.range.end.min(line_len);
    (token.range.start < end).then_some((token.range.start..end, style))
}

pub(super) fn push_clipped_absolute_prepared_document_token_highlights(
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
    highlight_palette: &SyntaxHighlightPalette,
    line_start: usize,
    line_end: usize,
    clamped_range: &Range<usize>,
    tokens: &[syntax::SyntaxToken],
) {
    let line_len = line_end.saturating_sub(line_start);
    let line_fully_visible = clamped_range.start <= line_start && line_end <= clamped_range.end;
    for token in tokens {
        if let Some((range, style)) =
            prepared_document_highlight_from_token_with_palette(highlight_palette, line_len, token)
        {
            if line_fully_visible {
                highlights.push((
                    line_start.saturating_add(range.start)..line_start.saturating_add(range.end),
                    style,
                ));
            } else {
                clip_and_push_line_highlight(
                    highlights,
                    line_start,
                    line_end,
                    clamped_range,
                    range,
                    style,
                );
            }
        }
    }
}

#[derive(Clone, Default)]
pub(in crate::view) struct PreparedDocumentByteRangeHighlights {
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

#[derive(Clone, Default)]
pub(in crate::view) struct PreparedDocumentLineHighlights {
    pub line_ix: usize,
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

pub(in crate::view) fn syntax_highlights_for_line(
    theme: AppTheme,
    text: &str,
    language: DiffSyntaxLanguage,
    syntax_mode: DiffSyntaxMode,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    if text.is_empty() {
        return Vec::new();
    }

    let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
    let tokens = syntax::syntax_tokens_for_line_shared(text, language, syntax_mode);
    prepared_document_line_highlights_from_tokens(theme, text.len(), &tokens)
}

#[cfg(test)]
pub(super) fn styled_text_for_diff_segments(
    theme: AppTheme,
    segments: &[CachedDiffTextSegment],
    word_color: Option<gpui::Rgba>,
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    let combined_len: usize = segments.iter().map(|s| s.text.len()).sum();
    let mut combined = String::with_capacity(combined_len);
    let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> =
        Vec::with_capacity(segments.len());

    let mut offset = 0usize;
    for seg in segments {
        combined.push_str(seg.text.as_ref());
        let next_offset = offset + seg.text.len();

        let mut style = gpui::HighlightStyle::default();

        if seg.in_word
            && let Some(mut c) = word_color
        {
            c.a = if theme.is_dark { 0.22 } else { 0.16 };
            style.background_color = Some(c.into());
        }

        if seg.in_query {
            style.background_color = Some(
                with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 }).into(),
            );
        }

        let syntax_fg = syntax_highlight_color(theme, seg.syntax);
        if let Some(fg) = syntax_fg {
            style.color = Some(fg.into());
        }

        if style != gpui::HighlightStyle::default() && offset < next_offset {
            highlights.push((offset..next_offset, style));
        }

        offset = next_offset;
    }

    (combined.into(), highlights)
}

fn find_all_ascii_case_insensitive(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    find_all_ascii_case_insensitive_into(haystack, needle, &mut out);
    out
}

fn find_all_ascii_case_insensitive_into(haystack: &str, needle: &str, out: &mut Vec<Range<usize>>) {
    const MAX_MATCHES: usize = 64;
    out.clear();

    let needle_bytes = needle.as_bytes();
    let Some((&first, &last)) = needle_bytes.first().zip(needle_bytes.last()) else {
        return;
    };

    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return;
    }

    let needle_len = needle_bytes.len();
    let first_lower = first.to_ascii_lowercase();
    let first_upper = first.to_ascii_uppercase();
    if needle_len == 1 {
        for start in memchr2_iter(first_lower, first_upper, haystack_bytes).take(MAX_MATCHES) {
            out.push(start..(start + 1));
        }
        return;
    }

    let last_start = haystack_bytes.len() - needle_len;
    let middle = &needle_bytes[1..needle_len - 1];
    let last_lower = last.to_ascii_lowercase();
    let last_upper = last.to_ascii_uppercase();
    let mut next_allowed_start = 0usize;
    'candidate: for start in memchr2_iter(first_lower, first_upper, &haystack_bytes[..=last_start])
    {
        if start < next_allowed_start {
            continue;
        }

        let haystack_last = haystack_bytes[start + needle_len - 1];
        if haystack_last != last_lower && haystack_last != last_upper {
            continue;
        }

        for (offset, needle_byte) in middle.iter().copied().enumerate() {
            if !haystack_bytes[start + offset + 1].eq_ignore_ascii_case(&needle_byte) {
                continue 'candidate;
            }
        }

        out.push(start..(start + needle_len));
        if out.len() == MAX_MATCHES {
            break;
        }
        next_allowed_start = start + needle_len;
    }
}

pub(in super::super) fn diff_line_colors(
    theme: AppTheme,
    kind: gitcomet_core::domain::DiffLineKind,
) -> (gpui::Rgba, gpui::Rgba, gpui::Rgba) {
    use gitcomet_core::domain::DiffLineKind::*;

    match (theme.is_dark, kind) {
        (_, Header) => (
            theme.colors.window_bg,
            theme.colors.text_muted,
            theme.colors.text_muted,
        ),
        (_, Hunk) => (
            theme.colors.window_bg,
            theme.colors.accent,
            theme.colors.text_muted,
        ),
        (_, Add) => (
            theme.colors.diff_add_bg,
            theme.colors.diff_add_text,
            theme.colors.diff_add_text,
        ),
        (_, Remove) => (
            theme.colors.diff_remove_bg,
            theme.colors.diff_remove_text,
            theme.colors.diff_remove_text,
        ),
        (_, Context) => (
            theme.colors.window_bg,
            theme.colors.text,
            theme.colors.text_muted,
        ),
    }
}
