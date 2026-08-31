use super::*;

/// Flatten markdown events into preview rows.
pub(crate) fn flatten_to_rows(
    source: &str,
    line_starts: &[usize],
) -> Option<Vec<MarkdownPreviewRow>> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ListContext {
        Unordered,
        Ordered { next_number: u64 },
    }

    impl ListContext {
        fn next_item_kind(&mut self) -> MarkdownPreviewRowKind {
            match self {
                Self::Unordered => MarkdownPreviewRowKind::ListItem { number: None },
                Self::Ordered { next_number } => {
                    let number = *next_number;
                    *next_number = next_number.saturating_add(1);
                    MarkdownPreviewRowKind::ListItem {
                        number: Some(number),
                    }
                }
            }
        }
    }

    let options = markdown_parser_options();

    // Rows are per markdown *block*, not per line, and `push_row_with_context`
    // bails at MAX_PREVIEW_ROWS regardless, so the line count is only an upper
    // bound worth honouring up to that cap.
    let mut rows = Vec::with_capacity(line_starts.len().min(MAX_PREVIEW_ROWS));
    let mut text_buf = String::new();
    struct PendingImage {
        source: SharedString,
        alt_start: usize,
    }

    let mut span_stack: Vec<MarkdownInlineStyle> = Vec::new();
    let mut link_stack: Vec<Option<SharedString>> = Vec::new();
    let mut pending_image: Option<PendingImage> = None;
    let mut inline_spans: Vec<MarkdownInlineSpan> = Vec::new();
    let mut source_start_byte: usize = 0;
    let mut indent_level: u8 = 0;
    let mut list_stack: Vec<ListContext> = Vec::new();
    let mut list_item_stack: Vec<MarkdownPreviewRowKind> = Vec::new();
    let mut in_heading = false;
    let mut in_paragraph = false;
    let mut in_blockquote: u8 = 0;
    let mut row_ctx = MarkdownRowContext::default();
    let mut in_code_block = false;
    let mut in_table_row = false;
    let mut table_row_is_header = false;
    let mut code_block_start_byte: usize = 0;
    let mut code_block_starts_after_fence = false;
    let mut code_block_language: Option<crate::view::rows::DiffSyntaxLanguage> = None;
    let mut footnote_context: Option<MarkdownFootnoteContext> = None;

    for (event, event_range) in Parser::new_ext(source, options).into_offset_iter() {
        // Block-level HTML stands on its own line with no paragraph around it,
        // so anything it produces has to close its own row.
        let is_block_html = matches!(event, Event::Html(_));
        match event {
            Event::Start(Tag::Heading { .. }) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_heading = true;
            }
            Event::End(TagEnd::Heading(level)) => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::Heading { level: level as u8 },
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut row_ctx,
                )?;
                in_heading = false;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::Paragraph) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_paragraph = true;
            }
            Event::End(TagEnd::Paragraph) => {
                // A paragraph whose only content was block-level HTML has
                // already closed its own row; closing it again would add a
                // blank row under it.
                if text_buf.is_empty()
                    && row_ctx.pending_images.is_empty()
                    && rows.last().is_some_and(|row| row.kind.is_image())
                {
                    in_paragraph = false;
                    inline_spans.clear();
                    continue;
                }
                let kind = current_row_kind(&list_item_stack, in_blockquote);

                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        kind,
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut row_ctx,
                )?;
                in_paragraph = false;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::List(first_number)) => {
                // Flush any accumulated item text — or picture — before entering
                // the sub-list, so the parent item gets its own row at the
                // current indent level.
                if (!text_buf.is_empty() || row_ctx.has_pending_images())
                    && !list_item_stack.is_empty()
                {
                    let kind = list_item_stack
                        .last()
                        .copied()
                        .unwrap_or(MarkdownPreviewRowKind::ListItem { number: None });
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            kind,
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.start, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut row_ctx,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                }
                list_stack.push(match first_number {
                    Some(next_number) => ListContext::Ordered { next_number },
                    None => ListContext::Unordered,
                });
                indent_level = indent_level.saturating_add(1);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                indent_level = indent_level.saturating_sub(1);
            }

            Event::Start(Tag::Item) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                if let Some(context) = list_stack.last_mut() {
                    list_item_stack.push(context.next_item_kind());
                }
            }
            Event::End(TagEnd::Item) => {
                // Only emit a row if there is text that hasn't already been
                // emitted by a nested paragraph or sub-list — or a picture,
                // which a tight list item like `- ![badge](b.svg)` leaves as
                // the item's only content.
                if !text_buf.is_empty() || row_ctx.has_pending_images() {
                    let kind = list_item_stack
                        .last()
                        .copied()
                        .unwrap_or(MarkdownPreviewRowKind::ListItem { number: None });
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            kind,
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.end, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut row_ctx,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                }
                list_item_stack.pop();
            }

            Event::Start(Tag::BlockQuote(kind)) => {
                row_ctx.blockquote_stack.push(MarkdownBlockQuoteContext {
                    alert_kind: kind.and_then(markdown_alert_kind_from_blockquote_kind),
                    emitted_row: false,
                });
                in_blockquote = in_blockquote.saturating_add(1);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                row_ctx.blockquote_stack.pop();
                in_blockquote = in_blockquote.saturating_sub(1);
            }

            Event::Start(Tag::FootnoteDefinition(label)) => {
                footnote_context = Some(MarkdownFootnoteContext {
                    label: label.to_string().into(),
                    emitted_label: false,
                });
                indent_level = indent_level.saturating_add(1);
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                footnote_context = None;
                indent_level = indent_level.saturating_sub(1);
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_block_start_byte = event_range.start;
                code_block_language = match &kind {
                    CodeBlockKind::Fenced(info) => {
                        crate::view::rows::diff_syntax_language_for_code_fence_info(info.as_ref())
                    }
                    CodeBlockKind::Indented => None,
                };
                code_block_starts_after_fence = matches!(kind, CodeBlockKind::Fenced(_));
                text_buf.clear();
                inline_spans.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                // Emit one row per code line.
                let block_range =
                    source_line_range(code_block_start_byte, event_range.end, line_starts);
                let block_start_line = block_range.start;
                let block_end_line = block_range.end.saturating_sub(1);
                let content_start_line =
                    block_start_line + usize::from(code_block_starts_after_fence);
                let code_text = text_buf.strip_suffix('\n').unwrap_or(&text_buf);
                let code_lines: Vec<&str> = if code_text.is_empty() {
                    vec![""]
                } else {
                    code_text.split('\n').collect()
                };
                let code_block_horizontal_scroll_hint = code_lines
                    .iter()
                    .any(|line| line.contains('\t') || line.chars().count() > 80);
                let last_ix = code_lines.len().saturating_sub(1);
                for (i, line) in code_lines.iter().enumerate() {
                    let line_ix = (content_start_line + i).min(block_end_line);
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::code(
                            MarkdownPreviewRowKind::CodeLine {
                                is_first: i == 0,
                                is_last: i == last_ix,
                            },
                            line,
                            line_ix..line_ix + 1,
                            code_block_language,
                            code_block_horizontal_scroll_hint,
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut row_ctx,
                    )?;
                }
                in_code_block = false;
                code_block_starts_after_fence = false;
                code_block_language = None;
                text_buf.clear();
                inline_spans.clear();
            }

            Event::Start(Tag::TableHead) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_table_row = true;
                table_row_is_header = true;
            }
            Event::Start(Tag::TableRow) => {
                text_buf.clear();
                inline_spans.clear();
                source_start_byte = event_range.start;
                in_table_row = true;
                table_row_is_header = false;
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::TableRow {
                            is_header: table_row_is_header,
                        },
                        &text_buf,
                        &inline_spans,
                        source_line_range(source_start_byte, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut row_ctx,
                )?;
                in_table_row = false;
                table_row_is_header = false;
                text_buf.clear();
                inline_spans.clear();
            }
            Event::End(TagEnd::TableCell) => {
                // Separate cells with a tab character for display.
                text_buf.push('\t');
            }

            // Inline styling tags
            Event::Start(Tag::Strong) => {
                span_stack.push(MarkdownInlineStyle::Bold);
            }
            Event::Start(Tag::Emphasis) => {
                span_stack.push(MarkdownInlineStyle::Italic);
            }
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

            // Pulldown reports an image's alt text as ordinary text between
            // Start and End, so the alt is taken back out of the buffer and the
            // picture is recorded at the offset it occupies. Whether it ends up
            // inline or as a block of its own is decided when the row closes.
            Event::Start(Tag::Image { dest_url, .. }) => {
                pending_image = Some(PendingImage {
                    source: SharedString::from(dest_url.as_ref().to_owned()),
                    alt_start: text_buf.len(),
                });
            }
            Event::End(TagEnd::Image) => {
                let Some(pending) = pending_image.take() else {
                    continue;
                };
                let alt = text_buf.split_off(pending.alt_start.min(text_buf.len()));
                if in_table_row {
                    // A table row is painted as one string whose columns are
                    // aligned by padding, so a picture cannot sit in a cell
                    // without breaking that alignment. Its description stays in
                    // the cell instead, which keeps the column readable and in
                    // the right place.
                    text_buf.push_str(&alt);
                    continue;
                }
                // The alt text is not painted, so anything styled inside it —
                // an image inside a link records the link on its alt — would
                // leave a span pointing past the end of the row.
                clamp_inline_spans_to_len(&mut inline_spans, text_buf.len());
                row_ctx.pending_images.push(MarkdownInlineImage {
                    byte_offset: text_buf.len(),
                    source_byte: event_range.start,
                    // Markdown image syntax cannot declare a size.
                    image: Arc::new(MarkdownImage {
                        source: pending.source,
                        width_px: None,
                        height_px: None,
                    }),
                    alt: SharedString::from(alt),
                    link_url: current_link_url(&link_stack),
                });
            }

            Event::Text(cow) => {
                let style = resolve_style_stack(&span_stack);
                let link_url = current_link_url(&link_stack);
                let start = text_buf.len();
                text_buf.push_str(&cow);
                let end = text_buf.len();
                if (style != MarkdownInlineStyle::Normal || link_url.is_some()) && !in_code_block {
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
                if !in_code_block {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style: MarkdownInlineStyle::Code,
                        link_url: current_link_url(&link_stack),
                    });
                }
            }

            Event::FootnoteReference(label) => {
                let start = text_buf.len();
                text_buf.push('[');
                text_buf.push_str(&label);
                text_buf.push(']');
                let end = text_buf.len();
                if !in_code_block {
                    inline_spans.push(MarkdownInlineSpan {
                        byte_range: start..end,
                        style: MarkdownInlineStyle::Link,
                        // A footnote reference points inside the document, not
                        // at the web.
                        link_url: None,
                    });
                }
            }

            Event::SoftBreak => {
                if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                    if !text_buf.is_empty() {
                        push_row_with_context(
                            &mut rows,
                            MarkdownPreviewRowInput::plain(
                                MarkdownPreviewRowKind::BlockquoteLine,
                                &text_buf,
                                &inline_spans,
                                source_line_range(
                                    source_start_byte,
                                    event_range.start,
                                    line_starts,
                                ),
                                indent_level,
                                in_blockquote,
                            ),
                            footnote_context.as_mut(),
                            &mut row_ctx,
                        )?;
                        text_buf.clear();
                        inline_spans.clear();
                    }
                    source_start_byte = event_range.end;
                } else if !text_buf.is_empty() {
                    text_buf.push(' ');
                }
            }
            Event::HardBreak => {
                if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                    if !text_buf.is_empty() {
                        push_row_with_context(
                            &mut rows,
                            MarkdownPreviewRowInput::plain(
                                MarkdownPreviewRowKind::BlockquoteLine,
                                &text_buf,
                                &inline_spans,
                                source_line_range(
                                    source_start_byte,
                                    event_range.start,
                                    line_starts,
                                ),
                                indent_level,
                                in_blockquote,
                            ),
                            footnote_context.as_mut(),
                            &mut row_ctx,
                        )?;
                        text_buf.clear();
                        inline_spans.clear();
                    }
                    source_start_byte = event_range.end;
                } else if !in_code_block && !in_heading && !text_buf.is_empty() {
                    push_row_with_context(
                        &mut rows,
                        MarkdownPreviewRowInput::plain(
                            current_row_kind(&list_item_stack, in_blockquote),
                            &text_buf,
                            &inline_spans,
                            source_line_range(source_start_byte, event_range.start, line_starts),
                            indent_level,
                            in_blockquote,
                        ),
                        footnote_context.as_mut(),
                        &mut row_ctx,
                    )?;
                    text_buf.clear();
                    inline_spans.clear();
                    source_start_byte = event_range.end;
                } else if !text_buf.is_empty() {
                    text_buf.push(' ');
                }
            }

            Event::Rule => {
                push_row_with_context(
                    &mut rows,
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::ThematicBreak,
                        "───",
                        &[],
                        source_line_range(event_range.start, event_range.end, line_starts),
                        indent_level,
                        in_blockquote,
                    ),
                    footnote_context.as_mut(),
                    &mut row_ctx,
                )?;
            }

            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                text_buf.insert_str(0, marker);
                // Shift existing span byte ranges.
                let shift = marker.len();
                for span in &mut inline_spans {
                    span.byte_range.start += shift;
                    span.byte_range.end += shift;
                }
            }

            Event::Html(cow) | Event::InlineHtml(cow) => {
                match classify_supported_html(cow.as_ref()) {
                    HtmlHandling::Ignore => continue,
                    HtmlHandling::HardBreak => {
                        if in_blockquote > 0 && list_item_stack.is_empty() && !in_code_block {
                            if !text_buf.is_empty() {
                                push_row_with_context(
                                    &mut rows,
                                    MarkdownPreviewRowInput::plain(
                                        MarkdownPreviewRowKind::BlockquoteLine,
                                        &text_buf,
                                        &inline_spans,
                                        source_line_range(
                                            source_start_byte,
                                            event_range.start,
                                            line_starts,
                                        ),
                                        indent_level,
                                        in_blockquote,
                                    ),
                                    footnote_context.as_mut(),
                                    &mut row_ctx,
                                )?;
                                text_buf.clear();
                                inline_spans.clear();
                            }
                            source_start_byte = event_range.end;
                            continue;
                        }
                        if !in_code_block && !in_heading && !text_buf.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text_buf,
                                    &inline_spans,
                                    source_line_range(
                                        source_start_byte,
                                        event_range.start,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut row_ctx,
                            )?;
                            text_buf.clear();
                            inline_spans.clear();
                            source_start_byte = event_range.end;
                        }
                        continue;
                    }
                    HtmlHandling::DetailsSummary(summary_source) => {
                        if !text_buf.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text_buf,
                                    &inline_spans,
                                    source_line_range(
                                        source_start_byte,
                                        event_range.start,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut row_ctx,
                            )?;
                            text_buf.clear();
                            inline_spans.clear();
                        }

                        let (summary_text, summary_spans) =
                            parse_inline_markdown_fragment(&summary_source);
                        if !summary_text.is_empty() {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    MarkdownPreviewRowKind::DetailsSummary,
                                    &summary_text,
                                    &summary_spans,
                                    source_line_range(
                                        event_range.start,
                                        event_range.end,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut row_ctx,
                            )?;
                        }
                        source_start_byte = event_range.end;
                        continue;
                    }
                    HtmlHandling::StartInlineStyle(style) => {
                        span_stack.push(style);
                        continue;
                    }
                    HtmlHandling::EndInlineStyle(style) => {
                        pop_matching_inline_style(&mut span_stack, style);
                        continue;
                    }
                    HtmlHandling::Images(images) => {
                        if in_table_row {
                            // As with a markdown image: a table cell keeps the
                            // description rather than a picture that cannot be
                            // placed in its column.
                            for (_, _, alt) in &images {
                                text_buf.push_str(alt);
                            }
                            continue;
                        }
                        // An `<img>` records itself the way a markdown image
                        // does; the row it closes decides whether it is inline
                        // or a block.
                        for (tag_offset, image, alt) in images {
                            row_ctx.pending_images.push(MarkdownInlineImage {
                                byte_offset: text_buf.len(),
                                // Several tags can share one event, so the id
                                // is the tag's own position, not the event's.
                                source_byte: event_range.start.saturating_add(tag_offset),
                                image: Arc::new(image),
                                alt: SharedString::from(alt),
                                link_url: current_link_url(&link_stack),
                            });
                        }
                        // A block-level tag has no paragraph to close it, so it
                        // flushes its own row.
                        if is_block_html {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text_buf,
                                    &inline_spans,
                                    source_line_range(
                                        source_start_byte,
                                        event_range.end,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut row_ctx,
                            )?;
                            text_buf.clear();
                            inline_spans.clear();
                            source_start_byte = event_range.end;
                        }
                        continue;
                    }
                    HtmlHandling::AppendText(text) => {
                        let should_append = html_event_should_append(
                            in_paragraph,
                            in_heading,
                            !list_stack.is_empty(),
                            in_blockquote,
                            in_code_block,
                            in_table_row,
                        );
                        if should_append {
                            text_buf.push_str(&text);
                        } else {
                            push_row_with_context(
                                &mut rows,
                                MarkdownPreviewRowInput::plain(
                                    current_row_kind(&list_item_stack, in_blockquote),
                                    &text,
                                    &[],
                                    source_line_range(
                                        event_range.start,
                                        event_range.end,
                                        line_starts,
                                    ),
                                    indent_level,
                                    in_blockquote,
                                ),
                                footnote_context.as_mut(),
                                &mut row_ctx,
                            )?;
                        }
                        continue;
                    }
                    HtmlHandling::AppendLiteral => {}
                }

                let should_append = html_event_should_append(
                    in_paragraph,
                    in_heading,
                    !list_stack.is_empty(),
                    in_blockquote,
                    in_code_block,
                    in_table_row,
                );
                if should_append {
                    text_buf.push_str(&cow);
                } else {
                    push_plain_fallback_rows(
                        &mut rows,
                        cow.as_ref(),
                        event_range.start,
                        event_range.end,
                        line_starts,
                        indent_level,
                        in_blockquote,
                        &mut row_ctx,
                    )?;
                }
            }

            // Ignore footnotes, metadata, and math in v1.
            _ => {}
        }
    }

    align_table_columns(&mut rows);
    insert_top_level_heading_spacer_rows(&mut rows);
    Some(rows)
}

pub(crate) fn insert_top_level_heading_spacer_rows(rows: &mut Vec<MarkdownPreviewRow>) {
    if rows.len() < 2 {
        return;
    }

    let mut spaced_rows = Vec::with_capacity(rows.len() + rows.len() / 4);
    let mut pending_gap_after_heading: Option<Range<usize>> = None;

    for row in rows.drain(..) {
        let is_top_level_heading = markdown_row_is_top_level_heading(&row);
        if let Some(source_line_range) = pending_gap_after_heading.take()
            && !is_top_level_heading
            && !matches!(row.kind, MarkdownPreviewRowKind::Spacer)
        {
            spaced_rows.push(markdown_preview_spacer_row_with_range(source_line_range));
        }

        if is_top_level_heading {
            let has_content_before_heading = matches!(
                spaced_rows.last(),
                Some(previous_row)
                    if !matches!(
                        previous_row.kind,
                        MarkdownPreviewRowKind::Spacer | MarkdownPreviewRowKind::Heading { .. }
                    )
            );

            // One spacer row is the section break. Adding a second one under
            // the heading doubles it to two blank rows, which reads as a hole
            // in the document; the heading's own vertical insets carry the
            // smaller gap beneath it instead.
            if has_content_before_heading {
                spaced_rows.push(markdown_preview_spacer_row_with_range(
                    row.source_line_range.clone(),
                ));
            } else {
                pending_gap_after_heading = Some(row.source_line_range.clone());
            }
        }

        spaced_rows.push(row);
    }

    *rows = spaced_rows;
}

pub(crate) fn markdown_row_is_top_level_heading(row: &MarkdownPreviewRow) -> bool {
    matches!(row.kind, MarkdownPreviewRowKind::Heading { .. })
        && row.indent_level == 0
        && row.blockquote_level == 0
}
