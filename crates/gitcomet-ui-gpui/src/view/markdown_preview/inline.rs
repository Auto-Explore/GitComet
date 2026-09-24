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
                link_stack.push(offered_link_destination(dest_url.as_ref()));
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
                    HtmlHandling::StartLink(destination) => {
                        span_stack.push(MarkdownInlineStyle::Link);
                        link_stack.push(destination);
                    }
                    HtmlHandling::EndLink => {
                        if !link_stack.is_empty() {
                            pop_matching_inline_style(&mut span_stack, MarkdownInlineStyle::Link);
                            link_stack.pop();
                        }
                    }
                    // An inline fragment (a `<summary>` label) has nowhere to
                    // put a block, so an image there keeps describing itself.
                    HtmlHandling::Images(images) => {
                        for image in images {
                            text_buf.push_str(&image.alt);
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

/// Emit the row a picture alone on its line becomes.
pub(crate) fn push_image_block_row(
    rows: &mut Vec<MarkdownPreviewRow>,
    inline: &MarkdownInlineImage,
    row: &MarkdownPreviewRowInput<'_>,
    decoration: MarkdownPreviewRowDecoration,
) -> Option<()> {
    // A picture wrapped in a link stays a link: its description carries it.
    let link = inline
        .link_url
        .clone()
        .filter(|_| !inline.alt.is_empty())
        .map(|url| MarkdownInlineSpan {
            byte_range: 0..inline.alt.len(),
            style: MarkdownInlineStyle::Link,
            link_url: Some(url),
        });
    push_row(
        rows,
        MarkdownPreviewRowInput::image(
            inline.alt.as_ref(),
            link.as_slice(),
            Arc::clone(&inline.image),
            row.source_line_range.clone(),
            row.indent_level,
            row.blockquote_level,
        ),
        decoration,
    )
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
        // Whitespace normalization can shorten the text past an offset.
        let len = row_text.len();
        let images = row
            .inline_images
            .iter()
            .map(|inline| MarkdownInlineImage {
                byte_offset: inline.byte_offset.min(len),
                ..inline.clone()
            })
            .collect::<Vec<_>>();
        trim_around_inline_images(row_text, row_spans, &images)
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
        table: None,
        task: decoration.task,
        continues_item: decoration.continues_item,
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
