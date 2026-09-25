use super::*;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Parser, Tag, TagEnd};

/// Flatten markdown events into preview rows.
pub(crate) fn flatten_to_rows(
    source: &str,
    line_starts: &[usize],
) -> Option<Vec<MarkdownPreviewRow>> {
    let mut flattener = Flattener::new(source, line_starts);
    // A byte-order mark is not text: pulldown reads `\u{feff}# Title` as a
    // paragraph that starts with it.
    let mut body_start = if source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    if let Some(front_matter) = front_matter(source, body_start) {
        flattener.push_front_matter(&front_matter)?;
        body_start = front_matter.end;
    }
    let body = &source[body_start..];
    for (event, range) in Parser::new_ext(body, markdown_parser_options()).into_offset_iter() {
        flattener.event(event, (range.start + body_start)..(range.end + body_start))?;
    }

    let mut rows = flattener.rows;
    finish_table_blocks(&mut rows);
    insert_top_level_heading_spacer_rows(&mut rows);
    Some(rows)
}

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

/// A list item being read.
struct OpenItem {
    kind: MarkdownPreviewRowKind,
    /// Its first row has been emitted and drew the bullet or number.
    marker_drawn: bool,
    /// The checkbox its first row will carry.
    task: Option<MarkdownTaskMarker>,
}

/// The block constructs enclosing the text being read, outermost first.
enum Container {
    List(ListContext),
    Item(OpenItem),
    Quote,
    Footnote,
}

/// An image whose description is being read: it is collected apart from the
/// row text, so a line break inside it cannot split the row under it.
struct OpenImage {
    source: SharedString,
    alt: String,
    source_byte: usize,
    /// Images written inside this one's description; their text is its text.
    nested: usize,
}

struct OpenCodeBlock {
    start: usize,
    after_fence: bool,
    language: Option<crate::view::rows::DiffSyntaxLanguage>,
}

struct OpenTable {
    info: Arc<MarkdownTableInfo>,
    /// The row being read: where it starts and whether it is the header.
    row: Option<(usize, bool)>,
}

/// Turns pulldown-cmark's event stream into preview rows.
///
/// Text accumulates into one row at a time. A row is closed when its block ends
/// or a line break splits it, and — so no text is lost — whenever another block
/// opens while text is still pending, as it does inside a tight list item.
struct Flattener<'a> {
    source: &'a str,
    line_starts: &'a [usize],
    rows: Vec<MarkdownPreviewRow>,
    /// The row being gathered.
    text: String,
    spans: Vec<MarkdownInlineSpan>,
    images: Vec<MarkdownInlineImage>,
    /// Source bytes the gathered row was read from, which is what its line
    /// range reports: a row must not claim the lines of the block around it.
    content: Option<Range<usize>>,
    styles: Vec<MarkdownInlineStyle>,
    links: Vec<Option<SharedString>>,
    /// `<a href>` tags still open, so a stray `</a>` cannot close a markdown
    /// link.
    html_links: usize,
    image: Option<OpenImage>,
    containers: Vec<Container>,
    quotes: Vec<MarkdownBlockQuoteContext>,
    footnote: Option<MarkdownFootnoteContext>,
    /// Where the heading being read starts.
    heading: Option<usize>,
    code: Option<OpenCodeBlock>,
    table: Option<OpenTable>,
}

impl<'a> Flattener<'a> {
    fn new(source: &'a str, line_starts: &'a [usize]) -> Self {
        Self {
            source,
            line_starts,
            // Rows are per markdown *block*, not per line, and `push_row` bails
            // at MAX_PREVIEW_ROWS regardless, so the line count is only an
            // upper bound worth honouring up to that cap.
            rows: Vec::with_capacity(line_starts.len().min(MAX_PREVIEW_ROWS)),
            text: String::new(),
            spans: Vec::new(),
            images: Vec::new(),
            content: None,
            styles: Vec::new(),
            links: Vec::new(),
            html_links: 0,
            image: None,
            containers: Vec::new(),
            quotes: Vec::new(),
            footnote: None,
            heading: None,
            code: None,
            table: None,
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) -> Option<()> {
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag, range),
            Event::Text(text) => {
                self.push_text(&text, range, false);
                Some(())
            }
            Event::Code(code) => {
                self.push_text(&code, range, true);
                Some(())
            }
            Event::FootnoteReference(label) => {
                if let Some(image) = self.image.as_mut() {
                    image.alt.push_str(&format!("[{label}]"));
                    return Some(());
                }
                let start = self.text.len();
                self.text.push('[');
                self.text.push_str(&label);
                self.text.push(']');
                self.note_content(range);
                self.spans.push(MarkdownInlineSpan {
                    byte_range: start..self.text.len(),
                    style: MarkdownInlineStyle::Link,
                    // A footnote reference points inside the document, not at
                    // the web.
                    link_url: None,
                });
                Some(())
            }
            Event::SoftBreak => {
                self.soft_break();
                Some(())
            }
            Event::HardBreak => self.line_break(),
            Event::Rule => {
                self.begin_block(true)?;
                let lines = self.line_range(range);
                let (indent, quotes) = (self.indent_level(), self.blockquote_level());
                self.push_row(
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::ThematicBreak,
                        "───",
                        &[],
                        lines,
                        indent,
                        quotes,
                    ),
                    None,
                    false,
                )
            }
            Event::TaskListMarker(checked) => {
                // The marker's range can start at the whitespace before it.
                let bracket = range.start + self.source[range.clone()].find('[').unwrap_or(0);
                let line = byte_offset_to_line(bracket, self.line_starts);
                let line_start = self.line_starts.get(line).copied().unwrap_or(0);
                let line_end = self
                    .line_starts
                    .get(line + 1)
                    .map_or(self.source.len(), |next| next.saturating_sub(1));
                let column = bracket - line_start;
                let line_hash =
                    task_line_hash(&self.source.as_bytes()[line_start..line_end], column);
                if let Some(item) = self.innermost_item_mut() {
                    item.task = Some(MarkdownTaskMarker {
                        checked,
                        line,
                        column,
                        line_hash,
                    });
                }
                Some(())
            }
            Event::Html(html) => self.html(&html, range, true),
            Event::InlineHtml(html) => self.html(&html, range, false),
            // Math and metadata blocks are not enabled.
            _ => Some(()),
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) -> Option<()> {
        match tag {
            Tag::Paragraph => self.begin_block(false)?,
            Tag::Heading { .. } => {
                self.begin_block(true)?;
                self.heading = Some(range.start);
            }
            Tag::BlockQuote(kind) => {
                self.begin_block(true)?;
                self.quotes.push(MarkdownBlockQuoteContext {
                    alert_kind: kind.and_then(markdown_alert_kind_from_blockquote_kind),
                    emitted_row: false,
                });
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.begin_block(true)?;
                self.code = Some(OpenCodeBlock {
                    start: range.start,
                    after_fence: matches!(kind, CodeBlockKind::Fenced(_)),
                    language: match &kind {
                        CodeBlockKind::Fenced(info) => {
                            crate::view::rows::diff_syntax_language_for_code_fence_info(
                                info.as_ref(),
                            )
                        }
                        CodeBlockKind::Indented => None,
                    },
                });
            }
            Tag::HtmlBlock => self.begin_block(true)?,
            Tag::List(first_number) => {
                // The parent item's text — or picture, or checkbox — gets its
                // own row at the current indent before the sub-list opens.
                self.begin_block(true)?;
                self.containers.push(Container::List(match first_number {
                    Some(next_number) => ListContext::Ordered { next_number },
                    None => ListContext::Unordered,
                }));
            }
            Tag::Item => {
                self.begin_block(true)?;
                let kind = self
                    .containers
                    .iter_mut()
                    .rev()
                    .find_map(|container| match container {
                        Container::List(list) => Some(list.next_item_kind()),
                        _ => None,
                    })
                    .unwrap_or(MarkdownPreviewRowKind::ListItem { number: None });
                self.containers.push(Container::Item(OpenItem {
                    kind,
                    marker_drawn: false,
                    task: None,
                }));
            }
            Tag::FootnoteDefinition(label) => {
                self.begin_block(true)?;
                self.footnote = Some(MarkdownFootnoteContext {
                    label: label.to_string().into(),
                    emitted_label: false,
                });
                self.containers.push(Container::Footnote);
            }
            Tag::Table(alignments) => {
                self.begin_block(true)?;
                self.table = Some(OpenTable {
                    info: Arc::new(MarkdownTableInfo {
                        alignments: alignments
                            .iter()
                            .map(|alignment| match alignment {
                                pulldown_cmark::Alignment::None => MarkdownTableAlign::None,
                                pulldown_cmark::Alignment::Left => MarkdownTableAlign::Left,
                                pulldown_cmark::Alignment::Center => MarkdownTableAlign::Center,
                                pulldown_cmark::Alignment::Right => MarkdownTableAlign::Right,
                            })
                            .collect(),
                        column_widths: Vec::new(),
                    }),
                    row: None,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                self.clear_row();
                if let Some(table) = self.table.as_mut() {
                    table.row = Some((range.start, matches!(tag, Tag::TableHead)));
                }
            }
            Tag::Emphasis => self.styles.push(MarkdownInlineStyle::Italic),
            Tag::Strong => self.styles.push(MarkdownInlineStyle::Bold),
            Tag::Strikethrough => self.styles.push(MarkdownInlineStyle::Strikethrough),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.styles.push(MarkdownInlineStyle::Link);
                // An email autolink's destination has no scheme, so it would
                // read as a file in the repository.
                self.links.push(if link_type == LinkType::Email {
                    None
                } else {
                    offered_link_destination(dest_url.as_ref())
                });
            }
            // Pulldown reports an image's alt text as ordinary text between
            // Start and End, so it is collected on the image instead of the
            // row. Whether the picture ends up inline or as a block of its own
            // is decided when the row closes.
            Tag::Image { dest_url, .. } => match self.image.as_mut() {
                Some(image) => image.nested += 1,
                None => {
                    self.image = Some(OpenImage {
                        source: SharedString::from(dest_url.as_ref().to_owned()),
                        alt: String::new(),
                        source_byte: range.start,
                        nested: 0,
                    });
                }
            },
            _ => {}
        }
        Some(())
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) -> Option<()> {
        match tag {
            TagEnd::Paragraph => self.flush_row(range.end)?,
            TagEnd::Heading(level) => {
                let start = self.heading.take().unwrap_or(range.start);
                let lines = self.line_range(start..range.end);
                let (indent, quotes) = (self.indent_level(), self.blockquote_level());
                let text = std::mem::take(&mut self.text);
                let spans = std::mem::take(&mut self.spans);
                self.content = None;
                self.push_row(
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::Heading { level: level as u8 },
                        &text,
                        &spans,
                        lines,
                        indent,
                        quotes,
                    ),
                    None,
                    false,
                )?;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_row(range.end)?;
                self.quotes.pop();
                self.pop_container();
            }
            TagEnd::CodeBlock => {
                let code = self.code.take()?;
                let block = self.line_range(code.start..range.end);
                let first_line = block.start + usize::from(code.after_fence);
                let text = std::mem::take(&mut self.text);
                self.clear_row();
                self.push_code_rows(
                    &text,
                    first_line,
                    block.end.saturating_sub(1),
                    code.language,
                )?;
            }
            TagEnd::List(_) => self.pop_container(),
            TagEnd::Item => {
                // Text a nested block or paragraph has not already emitted —
                // or a picture, or an empty item's checkbox.
                self.flush_row(range.end)?;
                self.pop_container();
            }
            TagEnd::FootnoteDefinition => {
                self.flush_row(range.end)?;
                self.footnote = None;
                self.pop_container();
            }
            TagEnd::Table => self.table = None,
            TagEnd::TableHead | TagEnd::TableRow => {
                let Some((start, is_header)) =
                    self.table.as_mut().and_then(|table| table.row.take())
                else {
                    return Some(());
                };
                let lines = self.line_range(start..range.end);
                let (indent, quotes) = (self.indent_level(), self.blockquote_level());
                let text = std::mem::take(&mut self.text);
                let spans = std::mem::take(&mut self.spans);
                self.clear_row();
                self.push_row(
                    MarkdownPreviewRowInput::plain(
                        MarkdownPreviewRowKind::TableRow { is_header },
                        &text,
                        &spans,
                        lines,
                        indent,
                        quotes,
                    ),
                    None,
                    false,
                )?;
                // Cells are laid out once the whole table is read.
                if let (Some(row), Some(table)) = (self.rows.last_mut(), self.table.as_ref()) {
                    row.table = Some(MarkdownTableRow {
                        cells: Arc::from(Vec::new()),
                        table: Arc::clone(&table.info),
                    });
                }
            }
            // Cells end in a tab; `finish_table_blocks` trims the last one.
            TagEnd::TableCell => self.text.push('\t'),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                self.links.pop();
            }
            TagEnd::Image => self.close_image(range),
            _ => {}
        }
        Some(())
    }

    // ── Inline content ───────────────────────────────────────────────────

    fn push_text(&mut self, text: &str, range: Range<usize>, is_code: bool) {
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(text);
            return;
        }
        let start = self.text.len();
        if self.in_table_row() {
            // Cells are joined by tabs, so a tab inside one would open a column.
            self.text.push_str(&text.replace('\t', " "));
        } else {
            self.text.push_str(text);
        }
        if self.code.is_some() {
            return;
        }
        self.note_content(range);
        let style = if is_code {
            MarkdownInlineStyle::Code
        } else {
            resolve_style_stack(&self.styles)
        };
        let link_url = current_link_url(&self.links);
        if is_code || style != MarkdownInlineStyle::Normal || link_url.is_some() {
            self.spans.push(MarkdownInlineSpan {
                byte_range: start..self.text.len(),
                style,
                link_url,
            });
        }
    }

    fn soft_break(&mut self) {
        match self.image.as_mut() {
            Some(image) => push_separator(&mut image.alt),
            None if self.code.is_none() && !self.text.is_empty() => self.text.push(' '),
            None => {}
        }
    }

    /// A hard break or `<br>`: it ends the row, except where a row cannot
    /// break — a heading, a table cell, a picture's description.
    fn line_break(&mut self) -> Option<()> {
        if let Some(image) = self.image.as_mut() {
            push_separator(&mut image.alt);
            return Some(());
        }
        if self.code.is_some() || self.text.is_empty() {
            return Some(());
        }
        if self.heading.is_some() || self.in_table_row() {
            push_separator(&mut self.text);
            return Some(());
        }
        let at = self.content.as_ref().map_or(0, |content| content.end);
        self.flush_row(at)
    }

    fn close_image(&mut self, range: Range<usize>) {
        let Some(image) = self.image.as_mut() else {
            return;
        };
        if image.nested > 0 {
            image.nested -= 1;
            return;
        }
        let Some(image) = self.image.take() else {
            return;
        };
        if self.in_table_row() {
            // A table row is painted as one string whose columns are aligned by
            // padding, so a picture cannot sit in a cell without breaking that
            // alignment. Its description stays in the cell instead, which keeps
            // the column readable and in the right place.
            self.push_text(&image.alt, range, false);
            return;
        }
        self.note_content(range);
        self.images.push(MarkdownInlineImage {
            byte_offset: self.text.len(),
            source_byte: image.source_byte,
            // Markdown image syntax cannot declare a size.
            image: Arc::new(MarkdownImage {
                source: image.source,
                width_px: None,
                height_px: None,
            }),
            alt: SharedString::from(normalize_whitespace(image.alt.trim())),
            link_url: current_link_url(&self.links),
        });
    }

    fn html(&mut self, html: &str, range: Range<usize>, block: bool) -> Option<()> {
        match classify_supported_html(html) {
            HtmlHandling::Ignore => {}
            HtmlHandling::HardBreak => self.line_break()?,
            HtmlHandling::DetailsSummary(summary) => {
                self.flush_row(range.start)?;
                let (text, spans) = parse_inline_markdown_fragment(&summary);
                if !text.is_empty() {
                    let lines = self.line_range(range);
                    let (indent, quotes) = (self.indent_level(), self.blockquote_level());
                    self.push_row(
                        MarkdownPreviewRowInput::plain(
                            MarkdownPreviewRowKind::DetailsSummary,
                            &text,
                            &spans,
                            lines,
                            indent,
                            quotes,
                        ),
                        None,
                        false,
                    )?;
                }
            }
            HtmlHandling::StartInlineStyle(style) => self.styles.push(style),
            HtmlHandling::EndInlineStyle(style) => {
                pop_matching_inline_style(&mut self.styles, style)
            }
            HtmlHandling::StartLink(destination) => {
                self.styles.push(MarkdownInlineStyle::Link);
                self.links.push(destination);
                self.html_links += 1;
            }
            HtmlHandling::EndLink => {
                if self.html_links > 0 {
                    self.html_links -= 1;
                    pop_matching_inline_style(&mut self.styles, MarkdownInlineStyle::Link);
                    self.links.pop();
                }
            }
            HtmlHandling::Images(images) => {
                if self.in_table_row() {
                    // As with a markdown image: a table cell keeps the
                    // description rather than a picture that cannot be placed
                    // in its column.
                    for image in images {
                        self.push_text(&image.alt, range.clone(), false);
                    }
                    return Some(());
                }
                // An `<img>` records itself the way a markdown image does; the
                // row it closes decides whether it is inline or a block.
                self.note_content(range.clone());
                for image in images {
                    self.images.push(MarkdownInlineImage {
                        byte_offset: self.text.len(),
                        // Several tags can share one event, so the id is the
                        // tag's own position, not the event's.
                        source_byte: range.start.saturating_add(image.tag_offset),
                        image: Arc::new(image.image),
                        alt: SharedString::from(image.alt),
                        link_url: image.link_url.or_else(|| current_link_url(&self.links)),
                    });
                }
                // A block-level tag has no paragraph to close it, so it flushes
                // its own row.
                if block {
                    self.flush_row(range.end)?;
                }
            }
            HtmlHandling::AppendText(text) if block => {
                self.flush_row(range.start)?;
                self.push_text(&text, range.clone(), false);
                self.flush_row(range.end)?;
            }
            HtmlHandling::AppendText(text) => self.push_text(&text, range, false),
            // Block HTML the preview does not interpret is shown verbatim, one
            // row per line, never folded into the text of the item around it.
            HtmlHandling::AppendLiteral if block => {
                self.flush_row(range.start)?;
                self.push_fallback_rows(html, range)?;
            }
            HtmlHandling::AppendLiteral => self.push_text(html, range, false),
        }
        Some(())
    }

    // ── Rows ─────────────────────────────────────────────────────────────

    /// A block opens: text still pending belongs to the list item around it,
    /// so it becomes that item's row first. An item holding only a checkbox
    /// emits it here too — unless the block is the paragraph that will carry
    /// it.
    fn begin_block(&mut self, claim_task: bool) -> Option<()> {
        let task_waiting = claim_task
            && self.innermost_text_container().is_some_and(
                |container| matches!(container, Container::Item(item) if item.task.is_some()),
            );
        if !self.text.is_empty() || !self.images.is_empty() || task_waiting {
            let at = self.content.as_ref().map_or(0, |content| content.end);
            self.flush_row(at)?;
        }
        Some(())
    }

    /// Emit the gathered text as a row of the kind its container gives it.
    /// `at` stands in for the source position when nothing was gathered from
    /// the source (a checkbox alone).
    fn flush_row(&mut self, at: usize) -> Option<()> {
        let (kind, continues_item, task_waiting) = match self.innermost_text_container() {
            Some(Container::Item(item)) => (item.kind, item.marker_drawn, item.task.is_some()),
            Some(Container::Quote) => (MarkdownPreviewRowKind::BlockquoteLine, false, false),
            _ => (MarkdownPreviewRowKind::Paragraph, false, false),
        };
        if self.text.is_empty() && self.images.is_empty() && !task_waiting {
            self.clear_row();
            return Some(());
        }
        let content = self.content.take().unwrap_or(at..at);
        let lines = self.line_range(content);
        let (indent, quotes) = (self.indent_level(), self.blockquote_level());
        let text = std::mem::take(&mut self.text);
        let spans = std::mem::take(&mut self.spans);
        let task = match self.innermost_text_container_mut() {
            Some(Container::Item(item)) => {
                item.marker_drawn = true;
                item.task.take()
            }
            _ => None,
        };
        self.push_row(
            MarkdownPreviewRowInput::plain(kind, &text, &spans, lines, indent, quotes),
            task,
            continues_item,
        )
    }

    /// Emit a row, decorated with what it inherits: the footnote label on a
    /// definition's first row, the alert of the quote around it, and the
    /// pictures read since the last row.
    fn push_row(
        &mut self,
        mut row: MarkdownPreviewRowInput<'_>,
        task: Option<MarkdownTaskMarker>,
        continues_item: bool,
    ) -> Option<()> {
        let pending_images = std::mem::take(&mut self.images);
        let footnote_label = self.footnote.as_mut().and_then(|footnote| {
            (!footnote.emitted_label).then(|| {
                footnote.emitted_label = true;
                footnote.label.clone()
            })
        });
        let mut decoration = MarkdownPreviewRowDecoration {
            footnote_label,
            task,
            continues_item,
            ..MarkdownPreviewRowDecoration::default()
        };
        if let Some(alert) = self
            .quotes
            .iter_mut()
            .rev()
            .find(|quote| quote.alert_kind.is_some())
        {
            decoration.alert_kind = alert.alert_kind;
            if !alert.emitted_row {
                alert.emitted_row = true;
                decoration.starts_alert = true;
            }
        }

        // A picture alone in a plain paragraph reads as a block — it gets the
        // width of the document and a band of rows to itself. Everywhere else
        // it stays inline: sharing its line with text or other pictures keeps a
        // row of badges on one line and a logo beside its heading, and a row
        // that carries a bullet, a quote bar, or an indent has to keep drawing
        // them, which a block row does not.
        if let [only] = pending_images.as_slice()
            && row.text.trim().is_empty()
            && row.image.is_none()
            && row.kind == MarkdownPreviewRowKind::Paragraph
            && row.indent_level == 0
            && row.blockquote_level == 0
        {
            return push_image_block_row(&mut self.rows, only, &row, decoration);
        }

        row.inline_images = Arc::from(pending_images);
        push_row(&mut self.rows, row, decoration)
    }

    fn push_code_rows(
        &mut self,
        code: &str,
        first_line: usize,
        last_line: usize,
        language: Option<crate::view::rows::DiffSyntaxLanguage>,
    ) -> Option<()> {
        let code = code.strip_suffix('\n').unwrap_or(code);
        let lines: Vec<&str> = if code.is_empty() {
            vec![""]
        } else {
            code.split('\n').collect()
        };
        let last_ix = lines.len() - 1;
        let (indent, quotes) = (self.indent_level(), self.blockquote_level());
        for (ix, line) in lines.into_iter().enumerate() {
            let line_ix = (first_line + ix).min(last_line.max(first_line));
            self.push_row(
                MarkdownPreviewRowInput::code(
                    MarkdownPreviewRowKind::CodeLine {
                        is_first: ix == 0,
                        is_last: ix == last_ix,
                    },
                    line.strip_suffix('\r').unwrap_or(line),
                    line_ix..line_ix + 1,
                    language,
                    indent,
                    quotes,
                ),
                None,
                false,
            )?;
        }
        Some(())
    }

    fn push_front_matter(&mut self, front_matter: &FrontMatter) -> Option<()> {
        let first_line = byte_offset_to_line(front_matter.content.start, self.line_starts);
        let last_line =
            byte_offset_to_line(front_matter.content.end.saturating_sub(1), self.line_starts);
        let language =
            crate::view::rows::diff_syntax_language_for_code_fence_info(front_matter.language);
        let source = self.source;
        self.push_code_rows(
            &source[front_matter.content.clone()],
            first_line,
            last_line,
            language,
        )
    }

    /// Show unparseable content verbatim, one row per line.
    ///
    /// A fallback row inherits no footnote label and no alert, but it still
    /// has to take the pending pictures, or they would be carried past it and
    /// land on an unrelated row.
    fn push_fallback_rows(&mut self, text: &str, range: Range<usize>) -> Option<()> {
        let lines = self.line_range(range);
        let segments = if text.is_empty() {
            vec![""]
        } else {
            text.lines().collect::<Vec<_>>()
        };
        let end_line = lines.end.saturating_sub(1);
        let (indent, quotes) = (self.indent_level(), self.blockquote_level());
        let mut pending_images = std::mem::take(&mut self.images);
        let segment_count = segments.len();

        for (ix, segment) in segments.into_iter().enumerate() {
            let line_ix = (lines.start + ix).min(end_line);
            let mut row = MarkdownPreviewRowInput::plain(
                MarkdownPreviewRowKind::PlainFallback,
                segment,
                &[],
                line_ix..line_ix.saturating_add(1),
                indent,
                quotes,
            );
            // Each picture goes on the line it was written on, which is what
            // its source offset says. The last row sweeps up anything that did
            // not resolve, so nothing is dropped.
            let is_last = ix + 1 == segment_count;
            let (mine, rest) = pending_images.into_iter().partition(|inline| {
                is_last || byte_offset_to_line(inline.source_byte, self.line_starts) == line_ix
            });
            pending_images = rest;
            row.inline_images = Arc::from(mine);
            push_row(&mut self.rows, row, MarkdownPreviewRowDecoration::default())?;
        }

        Some(())
    }

    // ── State ────────────────────────────────────────────────────────────

    fn note_content(&mut self, range: Range<usize>) {
        self.content = Some(match self.content.take() {
            Some(content) => content.start.min(range.start)..content.end.max(range.end),
            None => range,
        });
    }

    fn clear_row(&mut self) {
        self.text.clear();
        self.spans.clear();
        self.content = None;
    }

    fn line_range(&self, range: Range<usize>) -> Range<usize> {
        source_line_range(range.start, range.end, self.line_starts)
    }

    fn in_table_row(&self) -> bool {
        self.table.as_ref().is_some_and(|table| table.row.is_some())
    }

    fn indent_level(&self) -> u8 {
        let depth = self
            .containers
            .iter()
            .filter(|container| matches!(container, Container::List(_) | Container::Footnote))
            .count();
        u8::try_from(depth).unwrap_or(u8::MAX)
    }

    fn blockquote_level(&self) -> u8 {
        u8::try_from(self.quotes.len()).unwrap_or(u8::MAX)
    }

    /// The container that decides what a row of text is: the innermost list
    /// item or quote.
    fn innermost_text_container(&self) -> Option<&Container> {
        self.containers
            .iter()
            .rev()
            .find(|container| matches!(container, Container::Item(_) | Container::Quote))
    }

    fn innermost_text_container_mut(&mut self) -> Option<&mut Container> {
        self.containers
            .iter_mut()
            .rev()
            .find(|container| matches!(container, Container::Item(_) | Container::Quote))
    }

    fn innermost_item_mut(&mut self) -> Option<&mut OpenItem> {
        self.containers
            .iter_mut()
            .rev()
            .find_map(|container| match container {
                Container::Item(item) => Some(item),
                _ => None,
            })
    }

    fn pop_container(&mut self) {
        self.containers.pop();
    }
}

/// A space between words, unless one is already there.
fn push_separator(text: &mut String) {
    if !text.is_empty() && !text.ends_with(' ') {
        text.push(' ');
    }
}

/// YAML (`---`) or TOML (`+++`) front matter opening a document.
struct FrontMatter {
    /// The lines between the fences.
    content: Range<usize>,
    /// Where the document after the closing fence starts.
    end: usize,
    language: &'static str,
}

/// Front matter at `start`, which the preview shows as a code block.
///
/// pulldown-cmark's metadata option takes any `---` … `---` pair, which would
/// swallow a document that opens with a rule; here every line between the
/// fences has to read as data, and at least one as a key.
fn front_matter(source: &str, start: usize) -> Option<FrontMatter> {
    let rest = &source[start..];
    let (fence, language) = if rest.starts_with("---") {
        ("---", "yaml")
    } else if rest.starts_with("+++") {
        ("+++", "toml")
    } else {
        return None;
    };
    let mut line_start = start;
    let mut content_start = None;
    let mut saw_key = false;
    loop {
        let newline = source[line_start..].find('\n').map(|ix| line_start + ix);
        let line = source[line_start..newline.unwrap_or(source.len())].trim_end_matches('\r');
        let next = newline.map(|ix| ix + 1);
        let Some(content) = content_start else {
            if line.trim_end() != fence {
                return None;
            }
            content_start = Some(next?);
            line_start = next?;
            continue;
        };
        let trimmed = line.trim_end();
        if trimmed == fence || (language == "yaml" && trimmed == "...") {
            return saw_key.then_some(FrontMatter {
                content: content..line_start,
                end: next.unwrap_or(source.len()),
                language,
            });
        }
        if trimmed.is_empty() {
            // A blank line straight after the opening fence makes it a rule.
            if line_start == content {
                return None;
            }
        } else if front_matter_key(line, language) {
            saw_key = true;
        } else if !front_matter_continuation(line, language) {
            return None;
        }
        line_start = next?;
    }
}

/// `key: value` (YAML) or `key = value` (TOML).
fn front_matter_key(line: &str, language: &str) -> bool {
    let separator = if language == "yaml" { ':' } else { '=' };
    let Some(ix) = line.find(separator) else {
        return false;
    };
    let key = line[..ix].trim_end();
    let after = &line[ix + separator.len_utf8()..];
    !key.is_empty()
        && !line.starts_with(char::is_whitespace)
        && key
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | ' ' | '"' | '\''))
        && (language != "yaml" || after.is_empty() || after.starts_with([' ', '\t']))
}

/// A line inside a value or structure rather than a new key: indentation, a
/// comment, a YAML list item, a TOML table header.
fn front_matter_continuation(line: &str, language: &str) -> bool {
    line.starts_with([' ', '\t', '#'])
        || (language == "yaml" && (line.starts_with("- ") || line == "-"))
        || (language == "toml" && line.starts_with(['[', ']']))
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
