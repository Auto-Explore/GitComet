use super::*;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Parser, Tag, TagEnd};

mod html_tables;
use html_tables::PendingHtmlTable;

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

    flattener.finish_pending_html_table()?;
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
    /// Markdown column alignments, resolved onto cells as they are read.
    alignments: Vec<MarkdownTextAlign>,
    /// The row being read: where it starts and whether it is the header.
    row: Option<(usize, bool)>,
    cells: Vec<MarkdownTableCell>,
    starts_table: bool,
}

/// An HTML block's lines, joined so a tag or comment broken over lines is
/// read whole.
#[derive(Default)]
struct HtmlBlockBuffer {
    text: String,
    /// Each line's range in `text` and in the source.
    lines: Vec<(Range<usize>, Range<usize>)>,
    /// Markdown pulldown recognized between the HTML blocks of a table.
    markdown: Vec<HtmlMarkdownFragment>,
    /// Source ranges of code blocks and inline code, opaque to HTML parsing.
    literals: Vec<Range<usize>>,
}

#[derive(Clone)]
struct HtmlMarkdownFragment {
    source: Range<usize>,
    kind: HtmlMarkdownKind,
}

#[derive(Clone)]
enum HtmlMarkdownKind {
    Paragraph,
    CodeBlock(String),
}

impl HtmlBlockBuffer {
    fn push(&mut self, line: &str, source: Range<usize>) {
        let start = self.text.len();
        self.text.push_str(line);
        self.lines.push((start..self.text.len(), source));
    }

    /// The source range of `range` in `text`.
    fn source_range(&self, range: Range<usize>) -> Range<usize> {
        // An end on a line boundary belongs to the line it ends.
        let start_line = self
            .lines
            .partition_point(|(text, _)| text.start <= range.start);
        let end_line = self
            .lines
            .partition_point(|(text, _)| text.start < range.end);
        let at = |line: usize, offset: usize| {
            self.lines
                .get(line.saturating_sub(1))
                .map_or(0, |(text, source)| {
                    (source.start + offset.saturating_sub(text.start)).min(source.end)
                })
        };
        at(start_line, range.start)..at(end_line, range.end)
    }
}

/// One thing read from an HTML block, as a range of its buffered text.
enum HtmlPiece {
    Tag(HtmlHandling, Range<usize>),
    Text(Range<usize>),
    /// `<summary>…</summary>`: the label, and the whole element.
    Summary {
        label: Range<usize>,
        whole: Range<usize>,
    },
}

/// A `<p>`, `<div>`, `<center>` or `<hN>` still open.
struct OpenHtmlContainer {
    kind: HtmlContainerKind,
    align: MarkdownTextAlign,
    /// How many markdown containers were open around it; it cannot outlive
    /// them.
    depth: usize,
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
    /// Open inline styles, each marked when an HTML tag opened it, so a stray
    /// `</b>` cannot end markdown's `**`.
    styles: Vec<(MarkdownInlineStyle, bool)>,
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
    html_block: Option<HtmlBlockBuffer>,
    pending_html_table: Option<PendingHtmlTable>,
    captured_html_ends: Vec<TagEnd>,
    html_containers: Vec<OpenHtmlContainer>,
    /// The `<hN>` being read: its level and where it starts.
    html_heading: Option<(u8, usize)>,
    /// The row's length when its last byte is a space that block HTML put
    /// after a word, which goes if the row ends there.
    html_trailing_space: Option<usize>,
    /// How many of `styles` and `links` an HTML block left open. They stay
    /// open across blocks until their closing tag, as `<a href>` written as a
    /// block of its own around markdown is; anything above them ends with
    /// its block.
    html_floor: (usize, usize),
    /// An HTML block element just split the row: the space before the next
    /// word is not part of the new row.
    trim_next_start: bool,
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
            html_block: None,
            pending_html_table: None,
            captured_html_ends: Vec::new(),
            html_containers: Vec::new(),
            html_heading: None,
            html_trailing_space: None,
            html_floor: (0, 0),
            trim_next_start: false,
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) -> Option<()> {
        if self.capture_table_event(&event, &range)? {
            return Some(());
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag, range),
            // Inside an HTML block the only text event is the indent pulldown
            // synthesizes for its first line.
            Event::Text(_) if self.html_block.is_some() => Some(()),
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
            // pulldown only sends block HTML between `HtmlBlock` tags.
            Event::Html(html) => {
                if let Some(block) = self.html_block.as_mut() {
                    block.push(&html, range);
                }
                Some(())
            }
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
            Tag::HtmlBlock => {
                // Raw HTML continues what HTML before it left open.
                self.flush_before_block(true)?;
                self.html_block = Some(HtmlBlockBuffer::default());
            }
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
                    alignments: alignments
                        .iter()
                        .map(|alignment| match alignment {
                            pulldown_cmark::Alignment::None => MarkdownTextAlign::None,
                            pulldown_cmark::Alignment::Left => MarkdownTextAlign::Left,
                            pulldown_cmark::Alignment::Center => MarkdownTextAlign::Center,
                            pulldown_cmark::Alignment::Right => MarkdownTextAlign::Right,
                        })
                        .collect(),
                    row: None,
                    cells: Vec::new(),
                    starts_table: true,
                });
            }
            Tag::TableCell => self.start_table_cell(false, MarkdownTextAlign::None),
            Tag::TableHead | Tag::TableRow => {
                self.clear_row();
                if let Some(table) = self.table.as_mut() {
                    table.row = Some((range.start, matches!(tag, Tag::TableHead)));
                    table.cells.clear();
                }
            }
            Tag::Emphasis => self.styles.push((MarkdownInlineStyle::Italic, false)),
            Tag::Strong => self.styles.push((MarkdownInlineStyle::Bold, false)),
            Tag::Strikethrough => self
                .styles
                .push((MarkdownInlineStyle::Strikethrough, false)),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.styles.push((MarkdownInlineStyle::Link, false));
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
            TagEnd::Paragraph => self.end_html_flow(range.end)?,
            TagEnd::Heading(level) => {
                let start = self.heading.take().unwrap_or(range.start);
                self.push_heading_row(level as u8, start..range.end)?;
                self.end_open_html();
            }
            TagEnd::HtmlBlock => {
                if let Some(block) = self.html_block.take() {
                    self.read_html_block(&block)?;
                }
                // The row ends with the block, but a `<p>`, `<a>` or `<b>` it
                // left open does not: the next raw HTML block continues it,
                // and the next markdown block closes what a browser would.
                self.end_html_row(range.end)?;
                self.html_floor = (self.styles.len(), self.links.len());
            }
            TagEnd::BlockQuote(_) => {
                self.end_html_row(range.end)?;
                self.end_open_html();
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
                self.end_html_row(range.end)?;
                self.end_open_html();
                self.pop_container();
            }
            TagEnd::FootnoteDefinition => {
                self.end_html_row(range.end)?;
                self.end_open_html();
                self.footnote = None;
                self.pop_container();
            }
            TagEnd::Table => self.table = None,
            TagEnd::TableHead | TagEnd::TableRow => self.end_table_row(range.end)?,
            TagEnd::TableCell => {
                self.end_open_html();
                self.end_table_cell();
            }
            // An HTML formatting tag may be open above the markdown one, so
            // each end removes its own style.
            TagEnd::Emphasis => self.pop_style(MarkdownInlineStyle::Italic, false),
            TagEnd::Strong => self.pop_style(MarkdownInlineStyle::Bold, false),
            TagEnd::Strikethrough => self.pop_style(MarkdownInlineStyle::Strikethrough, false),
            TagEnd::Link => {
                self.pop_style(MarkdownInlineStyle::Link, false);
                self.links.pop();
                self.clamp_html_floor();
            }
            TagEnd::Image => self.close_image(range),
            _ => {}
        }
        Some(())
    }

    fn start_table_cell(&mut self, is_header: bool, align: MarkdownTextAlign) {
        if let Some(table) = self.table.as_mut() {
            let align = if align != MarkdownTextAlign::None {
                align
            } else if is_header || table.row.is_some_and(|(_, header)| header) {
                MarkdownTextAlign::Center
            } else {
                table
                    .alignments
                    .get(table.cells.len())
                    .copied()
                    .unwrap_or_default()
            };
            table.cells.push(MarkdownTableCell {
                is_header,
                align,
                ..MarkdownTableCell::new(self.text.len()..self.text.len())
            });
        }
    }

    fn end_table_cell(&mut self) {
        if let Some(cell) = self.table.as_mut().and_then(|t| t.cells.last_mut()) {
            cell.range.end = self.text.len();
        }
        self.text.push('\t');
    }

    fn end_table_row(&mut self, end: usize) -> Option<()> {
        let table = self.table.as_mut()?;
        let (start, is_header) = table.row.take()?;
        let cells = std::mem::take(&mut table.cells);
        let metadata = MarkdownTableRow {
            cells: Arc::from(cells),
            starts_table: std::mem::replace(&mut table.starts_table, false),
        };
        if self.text.ends_with('\t') {
            self.text.pop();
        }
        let text = std::mem::take(&mut self.text);
        let spans = std::mem::take(&mut self.spans);
        self.clear_row();
        self.push_row(
            MarkdownPreviewRowInput::plain(
                MarkdownPreviewRowKind::TableRow { is_header },
                &text,
                &spans,
                self.line_range(start..end),
                self.indent_level(),
                self.blockquote_level(),
            ),
            None,
            false,
        )?;
        self.rows.last_mut()?.table = Some(metadata);
        Some(())
    }

    // ── Inline content ───────────────────────────────────────────────────

    fn push_text(&mut self, text: &str, range: Range<usize>, is_code: bool) {
        if self.image.is_none() && self.code.is_none() {
            self.note_content(range);
        }
        self.append_text(text, is_code);
    }

    /// Add text to the row, styled, without claiming any source for it.
    fn append_text(&mut self, text: &str, is_code: bool) {
        if let Some(image) = self.image.as_mut() {
            image.alt.push_str(text);
            return;
        }
        let text = if std::mem::take(&mut self.trim_next_start) && self.text.is_empty() {
            text.trim_start_matches(|ch: char| ch.is_ascii_whitespace())
        } else {
            text
        };
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
        let style = if is_code {
            MarkdownInlineStyle::Code
        } else {
            resolve_style_stack(self.styles.iter().map(|(style, _)| *style))
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
        if self.code.is_some() || (self.text.is_empty() && self.images.is_empty()) {
            return Some(());
        }
        if self.heading.is_some() || self.html_heading.is_some() || self.in_table_row() {
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
        self.note_content(range.clone());
        let alt = normalize_whitespace(image.alt.trim());
        let byte_offset = self.text.len();
        let alt = self.insert_image_alt(alt, range);
        self.images.push(MarkdownInlineImage {
            byte_offset,
            source_byte: image.source_byte,
            // Markdown image syntax cannot declare a size.
            image: Arc::new(MarkdownImage {
                source: image.source,
                width_px: None,
                height_px: None,
            }),
            alt,
            link_url: current_link_url(&self.links),
        });
    }

    fn insert_image_alt(&mut self, alt: String, range: Range<usize>) -> SharedString {
        if !self.in_table_row() {
            return alt.into();
        }
        let start = self.text.len();
        self.push_text(&alt, range, false);
        // Cell image ranges use the description's byte length. Keep it equal
        // to the text actually inserted after leading whitespace is trimmed.
        self.text[start..].to_owned().into()
    }

    fn html(&mut self, html: &str, range: Range<usize>, block: bool) -> Option<()> {
        if self.table.is_none()
            && !self.html_containers_inert()
            && is_html_open_tag(&html.to_ascii_lowercase(), "table")
        {
            let mut buffer = HtmlBlockBuffer::default();
            buffer.push(html, range);
            return self.read_html_block(&buffer);
        }
        self.apply_html(classify_supported_html(html), html, range, block)
    }

    /// Act on HTML. A `block` fragment stands alone and closes its own row; a
    /// tag read out of a block or a paragraph leaves that to what is around it.
    fn apply_html(
        &mut self,
        handling: HtmlHandling,
        html: &str,
        range: Range<usize>,
        block: bool,
    ) -> Option<()> {
        match handling {
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
            HtmlHandling::StartInlineStyle(style) => self.styles.push((style, true)),
            HtmlHandling::EndInlineStyle(style) => self.pop_style(style, true),
            HtmlHandling::StartLink(destination) => {
                self.styles.push((MarkdownInlineStyle::Link, true));
                self.links.push(destination);
                self.html_links += 1;
            }
            HtmlHandling::EndLink => {
                if self.html_links > 0 {
                    self.html_links -= 1;
                    self.pop_style(MarkdownInlineStyle::Link, true);
                    self.links.pop();
                    self.clamp_html_floor();
                }
            }
            HtmlHandling::Images(images) => {
                // An `<img>` records itself the way a markdown image does; the
                // row it closes decides whether it is inline or a block.
                self.note_content(range.clone());
                for image in images {
                    let byte_offset = self.text.len();
                    let alt = if self.in_table_row() {
                        normalize_whitespace(&image.alt)
                    } else {
                        image.alt
                    };
                    let alt = self.insert_image_alt(alt, range.clone());
                    self.images.push(MarkdownInlineImage {
                        byte_offset,
                        // Several tags can share one event, so the id is the
                        // tag's own position, not the event's.
                        source_byte: range.start.saturating_add(image.tag_offset),
                        image: Arc::new(image.image),
                        alt,
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
            // A table row, a heading or a picture's description cannot be
            // split, so a block element there only separates words.
            HtmlHandling::OpenContainer(..) | HtmlHandling::Rule
                if self.html_containers_inert() =>
            {
                match self.image.as_mut() {
                    Some(image) => push_separator(&mut image.alt),
                    None => push_separator(&mut self.text),
                }
            }
            HtmlHandling::CloseContainer(_) if self.html_containers_inert() => {}
            HtmlHandling::OpenContainer(kind, align) => {
                // A block element starts a row of its own.
                self.split_row_at(range.start)?;
                // As in HTML, it closes a `<p>` left open, and a heading closes
                // a heading.
                self.close_html_container(HtmlContainerKind::Paragraph);
                if let HtmlContainerKind::Heading(level) = kind {
                    self.close_html_container(kind);
                    self.html_heading = Some((level, range.start));
                }
                if kind.ends_with_its_block() {
                    // The row owns its tags, so a diff of only them marks it.
                    self.note_content(range);
                }
                self.open_html_container(kind, align);
            }
            HtmlHandling::CloseContainer(kind) => {
                if !self
                    .html_containers
                    .iter()
                    .any(|open| kind.closes(open.kind))
                {
                    return Some(());
                }
                if kind.ends_with_its_block() {
                    self.note_content(range.clone());
                }
                self.split_row_at(range.end)?;
                self.close_html_container(kind);
            }
            HtmlHandling::Rule => {
                self.split_row_at(range.start)?;
                self.close_html_container(HtmlContainerKind::Paragraph);
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
                )?;
            }
        }
        Some(())
    }

    /// End the row being read where an HTML block element starts or ends,
    /// without the space before it; the next row starts at its first word.
    fn split_row_at(&mut self, at: usize) -> Option<()> {
        self.trim_row_end();
        self.end_html_row(at)?;
        self.trim_next_start = true;
        Some(())
    }

    fn open_html_container(&mut self, kind: HtmlContainerKind, align: MarkdownTextAlign) {
        self.html_containers.push(OpenHtmlContainer {
            kind,
            align,
            depth: self.containers.len(),
        });
    }

    /// Close the innermost open container `kind` closes, and any inside it.
    fn close_html_container(&mut self, kind: HtmlContainerKind) {
        if let Some(ix) = self
            .html_containers
            .iter()
            .rposition(|open| kind.closes(open.kind))
        {
            self.html_containers.truncate(ix);
        }
    }

    /// Read a buffered HTML block tag by tag, as a browser would, when the
    /// preview knows every tag in it; otherwise line by line, verbatim where
    /// it cannot interpret a line.
    fn read_html_flow_block(&mut self, block: &HtmlBlockBuffer) -> Option<()> {
        let tokens: Vec<HtmlToken> = html_tokens(&block.text).collect();
        let mut pieces = Vec::with_capacity(tokens.len());
        let mut ix = 0;
        while let Some(token) = tokens.get(ix) {
            ix += 1;
            let range = match token {
                HtmlToken::Text(range) => {
                    pieces.push(HtmlPiece::Text(range.clone()));
                    continue;
                }
                HtmlToken::Tag(range) => range.clone(),
            };
            let handling = classify_html_tag(&block.text[range.clone()]);
            if handling != HtmlHandling::AppendLiteral {
                pieces.push(HtmlPiece::Tag(handling, range));
                continue;
            }
            // A `<summary>` label is read whole, as markdown; any other tag
            // the preview does not know leaves the block to be shown as written.
            let lower = block.text[range.clone()].to_ascii_lowercase();
            let close = is_html_open_tag(&lower, "summary")
                .then(|| {
                    tokens[ix..].iter().position(|token| {
                        matches!(token, HtmlToken::Tag(tag)
                            if is_html_close_tag(&block.text[tag.clone()].to_ascii_lowercase(), "summary"))
                    })
                })
                .flatten();
            let Some(close) = close else {
                return self.read_html_block_lines(block);
            };
            let HtmlToken::Tag(close_range) = &tokens[ix + close] else {
                return self.read_html_block_lines(block);
            };
            pieces.push(HtmlPiece::Summary {
                label: range.end..close_range.start,
                whole: range.start..close_range.end,
            });
            ix += close + 1;
        }
        for piece in pieces {
            match piece {
                HtmlPiece::Tag(handling, range) => {
                    let source = block.source_range(range.clone());
                    // A tag owns its line, so a diff that only adds or drops
                    // one still marks the row it shapes.
                    if matches!(
                        handling,
                        HtmlHandling::StartInlineStyle(_)
                            | HtmlHandling::EndInlineStyle(_)
                            | HtmlHandling::StartLink(_)
                            | HtmlHandling::EndLink
                            | HtmlHandling::HardBreak
                    ) {
                        self.note_content(source.clone());
                    }
                    self.apply_html(handling, &block.text[range], source, false)?;
                }
                HtmlPiece::Summary { label, whole } => {
                    let summary = HtmlHandling::DetailsSummary(block.text[label].to_owned());
                    self.apply_html(summary, "", block.source_range(whole), false)?;
                }
                HtmlPiece::Text(range) => {
                    let text = &block.text[range.clone()];
                    // The words' own source, not the whitespace around them.
                    let lead = text.len() - text.trim_ascii_start().len();
                    let end = range.start + text.trim_ascii_end().len();
                    let words = block.source_range((range.start + lead).min(end)..end);
                    self.html_text(text, words);
                }
            }
        }
        Some(())
    }

    /// A block holding a tag the preview does not know, line by line: a line
    /// that is one tag acts as it, anything else is shown verbatim.
    ///
    /// The containers verbatim lines open and close still count, or one
    /// closed there would leak its `align` into the rest of the document.
    /// They are read from whole tags across the block, so one inside a
    /// comment or an attribute value does not count; a line's opening tags
    /// take effect before its content and its closing tags after.
    fn read_html_block_lines(&mut self, block: &HtmlBlockBuffer) -> Option<()> {
        // `<script>`, `<pre>` and the like hold text, not markup.
        let raw_text = is_raw_text_html(&block.text);
        let containers: Vec<_> = if raw_text {
            Vec::new()
        } else {
            html_container_tags(&block.text).collect()
        };
        let comments: Vec<Range<usize>> = html_tokens(&block.text)
            .filter_map(|token| match token {
                HtmlToken::Tag(tag) if block.text[tag.clone()].starts_with("<!--") => Some(tag),
                _ => None,
            })
            .collect();
        for (text, source) in &block.lines {
            let line = &block.text[text.clone()];
            let commented = line
                .char_indices()
                .filter(|(_, ch)| !ch.is_ascii_whitespace())
                .all(|(at, _)| {
                    comments
                        .iter()
                        .any(|comment| comment.contains(&(text.start + at)))
                });
            let handling = if commented {
                HtmlHandling::Ignore
            } else if raw_text {
                HtmlHandling::AppendLiteral
            } else {
                classify_supported_html(line)
            };
            let single = matches!(
                handling,
                HtmlHandling::OpenContainer(..) | HtmlHandling::CloseContainer(_)
            );
            let on_line = || {
                containers
                    .iter()
                    .filter(|(tag, ..)| !single && text.contains(&tag.start))
            };
            for (_, kind, align, _) in on_line().filter(|(.., closing)| !closing) {
                self.open_html_container(*kind, *align);
            }
            self.apply_html(handling, line, source.clone(), true)?;
            for (_, kind, ..) in on_line().filter(|(.., closing)| *closing) {
                self.close_html_container(*kind);
            }
        }
        Some(())
    }

    /// Text between tags in block HTML: entities decoded and runs of
    /// whitespace collapsed to one space, as a browser lays it out. `words`
    /// is the source of its non-blank part.
    fn html_text(&mut self, raw: &str, words: Range<usize>) {
        let text = decode_html_entities(raw);
        let mut collapsed = String::with_capacity(text.len());
        let mut space = false;
        for ch in text.chars() {
            if ch.is_ascii_whitespace() {
                space = true;
                continue;
            }
            if space && (!collapsed.is_empty() || ends_word(&self.text)) {
                collapsed.push(' ');
            }
            space = false;
            collapsed.push(ch);
        }
        let has_words = !collapsed.is_empty();
        // A trailing space is only known to be needed once more text follows;
        // it is dropped again if the row ends instead.
        let trailing = space && (has_words || ends_word(&self.text));
        if trailing {
            collapsed.push(' ');
        }
        if has_words {
            self.push_text(&collapsed, words, false);
        } else if trailing {
            self.append_text(&collapsed, false);
        }
        if trailing {
            self.html_trailing_space = Some(self.text.len());
        }
    }

    /// Whether HTML containers are ignored here, where a row cannot break.
    fn html_containers_inert(&self) -> bool {
        self.in_table_row() || self.heading.is_some() || self.image.is_some()
    }

    /// Where a paragraph or an HTML block ends: the row, then the `<p>` and
    /// `<hN>` it was in and any formatting tag left open.
    fn end_html_flow(&mut self, at: usize) -> Option<()> {
        self.end_html_row(at)?;
        self.end_open_html();
        Some(())
    }

    /// Emit the row being read: the `<hN>` it is in, or a row of its own.
    fn end_html_row(&mut self, at: usize) -> Option<()> {
        match self.html_heading.take() {
            Some((level, start)) => {
                // A heading never ends in a space; a `<br>` before its close
                // would otherwise leave one.
                self.trim_row_end();
                if self.text.is_empty() && self.images.is_empty() {
                    self.clear_row();
                    return Some(());
                }
                self.push_heading_row(level, start..at)
            }
            None => self.flush_row(at),
        }
    }

    /// What HTML left open inside a markdown block ends with it: a `<p>` or
    /// `<hN>`, formatting, links. Markdown's own are closed by now; what an
    /// HTML block of its own left open (`html_floor`) and `<div>`/`<center>`
    /// carry on.
    fn end_open_html(&mut self) {
        self.html_heading = None;
        self.close_html_paragraphs();
        let (styles, links) = self.html_floor;
        self.styles.truncate(styles);
        self.links.truncate(links);
        self.html_links = self.html_links.min(self.links.len());
    }

    /// A markdown block opening closes a `<p>` or `<hN>` left open, as its own
    /// element would in HTML.
    fn close_html_paragraphs(&mut self) {
        self.html_containers
            .retain(|open| !open.kind.ends_with_its_block());
    }

    /// Remove the innermost open `style`, of markdown or of HTML as
    /// `from_html` says; a closing tag of one cannot end the other.
    fn pop_style(&mut self, style: MarkdownInlineStyle, from_html: bool) {
        if let Some(ix) = self
            .styles
            .iter()
            .rposition(|open| *open == (style, from_html))
        {
            self.styles.remove(ix);
        }
        self.clamp_html_floor();
    }

    fn clamp_html_floor(&mut self) {
        let (styles, links) = self.html_floor;
        self.html_floor = (styles.min(self.styles.len()), links.min(self.links.len()));
    }

    /// Drop the whitespace the row ends in, keeping spans and pictures on
    /// the characters they cover.
    fn trim_row_end(&mut self) {
        let len = self
            .text
            .trim_end_matches(|ch: char| ch.is_ascii_whitespace())
            .len();
        if len == self.text.len() {
            return;
        }
        self.text.truncate(len);
        self.html_trailing_space = None;
        self.spans.retain_mut(|span| {
            span.byte_range.end = span.byte_range.end.min(len);
            span.byte_range.start < span.byte_range.end
        });
        for image in &mut self.images {
            image.byte_offset = image.byte_offset.min(len);
        }
    }

    fn trim_html_trailing_space(&mut self) {
        if self.html_trailing_space.take() != Some(self.text.len()) {
            return;
        }
        self.text.pop();
        let len = self.text.len();
        self.spans.retain_mut(|span| {
            span.byte_range.end = span.byte_range.end.min(len);
            span.byte_range.start < span.byte_range.end
        });
        for image in &mut self.images {
            image.byte_offset = image.byte_offset.min(len);
        }
    }

    /// The `align` of the innermost HTML container that sets one.
    fn html_align(&self) -> MarkdownTextAlign {
        self.html_containers
            .iter()
            .rev()
            .map(|open| open.align)
            .find(|align| *align != MarkdownTextAlign::None)
            .unwrap_or_default()
    }

    // ── Rows ─────────────────────────────────────────────────────────────

    /// A block opens: text still pending belongs to the list item around it,
    /// so it becomes that item's row first. An item holding only a checkbox
    /// emits it here too — unless the block is the paragraph that will carry
    /// it.
    ///
    /// A markdown block also closes a `<p>` or `<hN>` HTML left open.
    fn begin_block(&mut self, claim_task: bool) -> Option<()> {
        self.flush_before_block(claim_task)?;
        self.close_html_paragraphs();
        self.trim_next_start = false;
        Some(())
    }

    fn flush_before_block(&mut self, claim_task: bool) -> Option<()> {
        let task_waiting = claim_task
            && self.innermost_text_container().is_some_and(
                |container| matches!(container, Container::Item(item) if item.task.is_some()),
            );
        if !self.text.is_empty() || !self.images.is_empty() || task_waiting {
            let at = self.content.as_ref().map_or(0, |content| content.end);
            // A `<hN>` opened inside the item ends here, as its heading.
            self.end_html_row(at)?;
        }
        Some(())
    }

    /// Emit the gathered text as a row of the kind its container gives it.
    /// `at` stands in for the source position when nothing was gathered from
    /// the source (a checkbox alone).
    fn flush_row(&mut self, at: usize) -> Option<()> {
        self.trim_html_trailing_space();
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

    fn push_heading_row(&mut self, level: u8, source: Range<usize>) -> Option<()> {
        let lines = self.line_range(source);
        let (indent, quotes) = (self.indent_level(), self.blockquote_level());
        let text = std::mem::take(&mut self.text);
        let spans = std::mem::take(&mut self.spans);
        self.content = None;
        self.push_row(
            MarkdownPreviewRowInput::plain(
                MarkdownPreviewRowKind::Heading { level },
                &text,
                &spans,
                lines,
                indent,
                quotes,
            ),
            None,
            false,
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
        // A footnote's labelled row is drawn like a list item, which has no
        // alignment.
        let aligned = footnote_label.is_none()
            && matches!(
                row.kind,
                MarkdownPreviewRowKind::Paragraph
                    | MarkdownPreviewRowKind::Heading { .. }
                    | MarkdownPreviewRowKind::DetailsSummary
            );
        let mut decoration = MarkdownPreviewRowDecoration {
            footnote_label,
            task,
            continues_item,
            align: if aligned {
                self.html_align()
            } else {
                MarkdownTextAlign::None
            },
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
        self.html_trailing_space = None;
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
        let depth = self.containers.len();
        self.html_containers.retain(|open| open.depth <= depth);
    }
}

/// A space between words, unless one is already there.
fn push_separator(text: &mut String) {
    if ends_word(text) {
        text.push(' ');
    }
}

/// Whether `text` ends in a word that a space would separate from the next.
fn ends_word(text: &str) -> bool {
    !text.is_empty() && !text.ends_with([' ', '\t'])
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
