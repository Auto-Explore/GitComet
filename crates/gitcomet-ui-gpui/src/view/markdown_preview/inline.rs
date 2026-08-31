use super::*;

pub(crate) fn parse_inline_markdown_fragment(source: &str) -> (String, Vec<MarkdownInlineSpan>) {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut text_buf = String::new();
    let mut span_stack = Vec::new();
    let mut link_stack: Vec<Option<SharedString>> = Vec::new();
    let mut inline_spans = Vec::new();

    for event in Parser::new_ext(source, markdown_parser_options()) {
        match event {
            Event::Start(Tag::Strong) => span_stack.push(MarkdownInlineStyle::Bold),
            Event::Start(Tag::Emphasis) => span_stack.push(MarkdownInlineStyle::Italic),
            Event::Start(Tag::Strikethrough) => {
                span_stack.push(MarkdownInlineStyle::Strikethrough);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                span_stack.push(MarkdownInlineStyle::Link);
                link_stack.push(web_link_url(dest_url.as_ref()));
            }
            Event::End(TagEnd::Link) => {
                span_stack.pop();
                link_stack.pop();
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
                span_stack.pop();
            }
            Event::Text(cow) => {
                let style = resolve_style_stack(&span_stack);
                let link_url = current_link_url(&link_stack);
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                if style != MarkdownInlineStyle::Normal || link_url.is_some() {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style,
                        link_url,
                    });
                }
            }
            Event::Code(cow) => {
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                inline_spans.push(MarkdownInlineSpan {
                    byte_range: start..end,
                    style: MarkdownInlineStyle::Code,
                    link_url: current_link_url(&link_stack),
                });
            }
            Event::FootnoteReference(label) => {
                let start = text_buf.len();
                text_buf.push('[');
                text_buf.push_str(&label);
                text_buf.push(']');
                let end = text_buf.len();
                inline_spans.push(MarkdownInlineSpan {
                    byte_range: start..end,
                    style: MarkdownInlineStyle::Link,
                    link_url: None,
                });
            }
            Event::SoftBreak | Event::HardBreak if !text_buf.is_empty() => {
                text_buf.push(' ');
            }
            Event::Html(cow) | Event::InlineHtml(cow) => {
                match classify_supported_html(cow.as_ref()) {
                    HtmlHandling::Ignore => {}
                    HtmlHandling::HardBreak => {
                        if !text_buf.is_empty() {
                            text_buf.push(' ');
                        }
                    }
                    HtmlHandling::DetailsSummary(summary_source) => {
                        let summary_text = strip_generic_html_tags(&summary_source);
                        if !summary_text.is_empty() {
                            if !text_buf.is_empty() {
                                text_buf.push(' ');
                            }
                            text_buf.push_str(&summary_text);
                        }
                    }
                    HtmlHandling::StartInlineStyle(style) => span_stack.push(style),
                    HtmlHandling::EndInlineStyle(style) => {
                        pop_matching_inline_style(&mut span_stack, style);
                    }
                    // An inline fragment (a `<summary>` label) has nowhere to
                    // put a block, so an image there keeps describing itself.
                    HtmlHandling::Images(images) => {
                        for (_, _, alt) in images {
                            text_buf.push_str(&alt);
                        }
                    }
                    HtmlHandling::AppendText(text) => {
                        text_buf.push_str(&text);
                    }
                    HtmlHandling::AppendLiteral => {
                        text_buf.push_str(&strip_generic_html_tags(cow.as_ref()));
                    }
                }
            }
            _ => {}
        }
    }

    normalize_whitespace_with_spans(&text_buf, &inline_spans)
}

pub(crate) fn push_row_with_context(
    rows: &mut Vec<MarkdownPreviewRow>,
    mut row: MarkdownPreviewRowInput<'_>,
    footnote_context: Option<&mut MarkdownFootnoteContext>,
    row_ctx: &mut MarkdownRowContext,
) -> Option<()> {
    let pending_images = std::mem::take(&mut row_ctx.pending_images);
    let footnote_label = footnote_context.and_then(|ctx| {
        if ctx.emitted_label {
            None
        } else {
            ctx.emitted_label = true;
            Some(ctx.label.clone())
        }
    });

    let mut decoration = MarkdownPreviewRowDecoration {
        footnote_label,
        ..MarkdownPreviewRowDecoration::default()
    };
    if let Some(alert_ix) = row_ctx
        .blockquote_stack
        .iter()
        .rposition(|ctx| ctx.alert_kind.is_some())
    {
        let ctx = &mut row_ctx.blockquote_stack[alert_ix];
        decoration.alert_kind = ctx.alert_kind;
        if !ctx.emitted_row {
            ctx.emitted_row = true;
            decoration.starts_alert = true;
        }
    }

    // A picture alone in a plain paragraph reads as a block — it gets the width
    // of the document and a band of rows to itself. Everywhere else it stays
    // inline: sharing its line with text or other pictures keeps a row of
    // badges on one line and a logo beside its heading, and a row that carries
    // a bullet, a quote bar, or an indent has to keep drawing them, which a
    // block row does not.
    if let [only] = pending_images.as_slice()
        && row.text.trim().is_empty()
        && row.image.is_none()
        && row.kind == MarkdownPreviewRowKind::Paragraph
        && row.indent_level == 0
        && row.blockquote_level == 0
    {
        return push_image_block_rows(rows, only, &row, decoration);
    }

    row.inline_images = Arc::from(pending_images);
    push_row(rows, row, decoration)
}

/// Emit the band rows one block image occupies.
///
/// The preview paints into a uniform (fixed row height) list, so a picture that
/// stands on its own covers several rows and each one draws its own band.
pub(crate) fn push_image_block_rows(
    rows: &mut Vec<MarkdownPreviewRow>,
    inline: &MarkdownInlineImage,
    row: &MarkdownPreviewRowInput<'_>,
    decoration: MarkdownPreviewRowDecoration,
) -> Option<()> {
    let slice_count = inline.image.block_rows();
    // The alert badge and the footnote label belong to the first band only; the
    // rest continue the same picture, and only inherit its alert.
    let continuation = MarkdownPreviewRowDecoration {
        alert_kind: decoration.alert_kind,
        ..MarkdownPreviewRowDecoration::default()
    };
    let mut decoration = Some(decoration);
    for slice_ix in 0..slice_count {
        push_row(
            rows,
            MarkdownPreviewRowInput::image(
                slice_ix,
                slice_count,
                inline.alt.as_ref(),
                Arc::clone(&inline.image),
                row.source_line_range.clone(),
                row.indent_level,
                row.blockquote_level,
            ),
            decoration.take().unwrap_or_else(|| continuation.clone()),
        )?;
    }
    Some(())
}

pub(crate) fn push_row(
    rows: &mut Vec<MarkdownPreviewRow>,
    row: MarkdownPreviewRowInput<'_>,
    decoration: MarkdownPreviewRowDecoration,
) -> Option<()> {
    let (row_text, row_spans) = match row.kind {
        // Paragraph-like rows collapse whitespace, so remap inline spans to
        // the normalized text instead of leaving them pointed at stale bytes.
        MarkdownPreviewRowKind::Paragraph
        | MarkdownPreviewRowKind::DetailsSummary
        | MarkdownPreviewRowKind::BlockquoteLine => {
            normalize_whitespace_with_spans(row.text, row.inline_spans)
        }
        _ => (row.text.to_owned(), row.inline_spans.to_vec()),
    };
    let (row_text, row_spans, inline_images) = if row.inline_images.is_empty() {
        (row_text, row_spans, row.inline_images)
    } else {
        trim_around_inline_images(row_text, row_spans, &row.inline_images)
    };
    let spans = if row_spans.len() > MAX_INLINE_SPANS_PER_ROW {
        Arc::new(Vec::new())
    } else {
        Arc::new(row_spans)
    };

    rows.push(MarkdownPreviewRow {
        kind: row.kind,
        text: SharedString::from(row_text),
        inline_spans: spans,
        code_language: row.code_language,
        code_block_horizontal_scroll_hint: row.code_block_horizontal_scroll_hint,
        source_line_range: row.source_line_range,
        change_hint: MarkdownChangeHint::None,
        indent_level: row.indent_level,
        blockquote_level: row.blockquote_level,
        footnote_label: decoration.footnote_label,
        alert_kind: decoration.alert_kind,
        starts_alert: decoration.starts_alert,
        image: row.image,
        inline_images,
        styled_text_cache: MarkdownPreviewRowStyledTextCache::default(),
        measured_width_px: MarkdownPreviewRowWidthCache::default(),
    });

    (rows.len() <= MAX_PREVIEW_ROWS).then_some(())
}

/// Trim the whitespace a picture leaves behind when it is lifted out of the
/// line, keeping spans and picture offsets on the characters they described.
///
/// `## <img/> GitComet` puts a space between the tag and the word; without this
/// the heading would start with that gap.
pub(crate) fn trim_around_inline_images(
    text: String,
    spans: Vec<MarkdownInlineSpan>,
    images: &[MarkdownInlineImage],
) -> (String, Vec<MarkdownInlineSpan>, Arc<[MarkdownInlineImage]>) {
    let start = text.len() - text.trim_start().len();
    let trimmed = text.trim().to_owned();
    let end = start + trimmed.len();
    let shift = |offset: usize| offset.clamp(start, end) - start;

    let spans = spans
        .into_iter()
        .filter_map(|span| {
            let range = shift(span.byte_range.start)..shift(span.byte_range.end);
            (range.start < range.end).then(|| span.restyled(range))
        })
        .collect();
    let images = images
        .iter()
        .map(|inline| MarkdownInlineImage {
            byte_offset: shift(inline.byte_offset),
            ..inline.clone()
        })
        .collect::<Vec<_>>();

    (trimmed, spans, Arc::from(images))
}

/// Drop or shorten spans that reach past `len`, and keep the rest.
pub(crate) fn clamp_inline_spans_to_len(spans: &mut Vec<MarkdownInlineSpan>, len: usize) {
    spans.retain_mut(|span| {
        span.byte_range.end = span.byte_range.end.min(len);
        span.byte_range.start < span.byte_range.end
    });
}

/// Emit unparseable content verbatim, one row per line.
///
/// This is the one row producer that does not go through
/// `push_row_with_context`, because a fallback row inherits no footnote label
/// and no alert. It still has to take the pending pictures, or they would be
/// carried past it and land on an unrelated row.
pub(crate) fn push_plain_fallback_rows(
    rows: &mut Vec<MarkdownPreviewRow>,
    text: &str,
    start_byte: usize,
    end_byte: usize,
    line_starts: &[usize],
    indent_level: u8,
    blockquote_level: u8,
    row_ctx: &mut MarkdownRowContext,
) -> Option<()> {
    let range = source_line_range(start_byte, end_byte, line_starts);
    let segments = if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect::<Vec<_>>()
    };
    let end_line = range.end.saturating_sub(1);
    let mut pending_images = std::mem::take(&mut row_ctx.pending_images);
    let segment_count = segments.len();

    for (ix, segment) in segments.into_iter().enumerate() {
        let line_ix = (range.start + ix).min(end_line);
        let mut row = MarkdownPreviewRowInput::plain(
            MarkdownPreviewRowKind::PlainFallback,
            segment,
            &[],
            line_ix..line_ix.saturating_add(1),
            indent_level,
            blockquote_level,
        );
        // Each picture goes on the line it was written on, which is what its
        // source offset says. The last row sweeps up anything that did not
        // resolve, so nothing is dropped.
        let is_last = ix + 1 == segment_count;
        let (mine, rest) = pending_images.into_iter().partition(|inline| {
            is_last || source_line_for_byte(inline.source_byte, line_starts) == line_ix
        });
        pending_images = rest;
        row.inline_images = Arc::from(mine);
        push_row(rows, row, MarkdownPreviewRowDecoration::default())?;
    }

    Some(())
}

/// Zero-based source line containing `byte`.
pub(crate) fn source_line_for_byte(byte: usize, line_starts: &[usize]) -> usize {
    line_starts.partition_point(|start| *start <= byte).max(1) - 1
}
