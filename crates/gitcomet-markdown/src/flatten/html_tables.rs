//! Buffered HTML tables, committed only after their structure is understood.
use super::*;

pub(super) struct PendingHtmlTable {
    block: HtmlBlockBuffer,
    scanner: TableEndScanner,
}

#[derive(Default)]
struct TableEndScanner {
    scanned: usize,
    depth: usize,
    raw: Option<&'static str>,
}

fn raw_text_open_tag(lower: &str) -> Option<&'static str> {
    ["script", "style", "pre", "textarea"]
        .into_iter()
        .find(|name| is_html_open_tag(lower, name))
}

impl HtmlBlockBuffer {
    /// Map a source fragment back into the buffered text, which may have had
    /// blockquote prefixes removed or may hold only part of the fragment.
    fn text_range(&self, source: Range<usize>) -> Range<usize> {
        let start_line = self
            .lines
            .partition_point(|(_, range)| range.start <= source.start);
        let end_line = self
            .lines
            .partition_point(|(_, range)| range.start < source.end);
        let at = |line: usize, offset: usize| {
            self.lines
                .get(line.saturating_sub(1))
                .map_or(0, |(text, source)| {
                    (text.start + offset.saturating_sub(source.start)).min(text.end)
                })
        };
        at(start_line, source.start)..at(end_line, source.end)
    }

    /// Code samples are text even when they contain table or script tags.
    /// Split around them before HTML tokenization, so an unfinished tag in a
    /// sample cannot consume the real closing tags after it.
    fn table_tokens(&self, range: Range<usize>) -> impl Iterator<Item = HtmlToken> + '_ {
        let end = range.end;
        let mut at = range.start;
        let source = self.source_range(range.clone());
        let first = self
            .literals
            .partition_point(|literal| literal.end <= source.start);
        self.literals[first..]
            .iter()
            .take_while(move |literal| literal.start < source.end)
            .filter_map(move |literal| {
                let literal = self.text_range(literal.clone());
                let literal = literal.start.max(range.start)..literal.end.min(end);
                (literal.start < literal.end).then_some(literal)
            })
            .map(Some)
            .chain(std::iter::once(None))
            .flat_map(move |literal| {
                let base = at;
                let stop = literal.as_ref().map_or(end, |range| range.start);
                at = literal.as_ref().map_or(end, |range| range.end);
                html_tokens(&self.text[base..stop])
                    .map(move |token| match token {
                        HtmlToken::Tag(range) => {
                            HtmlToken::Tag(range.start + base..range.end + base)
                        }
                        HtmlToken::Text(range) => {
                            HtmlToken::Text(range.start + base..range.end + base)
                        }
                    })
                    .chain(literal.map(HtmlToken::Text))
            })
    }

    fn slice(&self, range: Range<usize>) -> Self {
        let mut result = Self::default();
        let first = self
            .lines
            .partition_point(|(text, _)| text.end <= range.start);
        let last = self
            .lines
            .partition_point(|(text, _)| text.start < range.end);
        for (text, _) in &self.lines[first..last] {
            let start = text.start.max(range.start);
            let end = text.end.min(range.end);
            if start < end {
                result.push(&self.text[start..end], self.source_range(start..end));
            }
        }
        let source = self.source_range(range);
        result.markdown.extend(
            self.markdown
                .iter()
                .filter(|fragment| {
                    fragment.source.start < source.end && fragment.source.end > source.start
                })
                .cloned(),
        );
        result.literals.extend(
            self.literals
                .iter()
                .filter(|literal| literal.start < source.end && literal.end > source.start)
                .cloned(),
        );
        result
    }

    fn append(&mut self, other: &Self) {
        let previous = self.lines.last().map_or(0, |(_, source)| source.end);
        if let Some((_, next)) = other.lines.first()
            && next.start > previous
            && !self.text.is_empty()
        {
            self.push("\n", previous..next.start);
        }
        for (text, source) in &other.lines {
            self.push(&other.text[text.clone()], source.clone());
        }
        self.markdown.extend(other.markdown.iter().cloned());
        self.literals.extend(other.literals.iter().cloned());
    }
}

impl TableEndScanner {
    /// Resume scanning instead of rescanning a growing table for every event.
    fn end(&mut self, block: &HtmlBlockBuffer) -> Option<usize> {
        let base = self.scanned;
        for token in block.table_tokens(base..block.text.len()) {
            let HtmlToken::Tag(range) = token else {
                continue;
            };
            let tag = &block.text[range.clone()];
            if !tag.ends_with('>') {
                self.scanned = range.start;
                return None;
            }
            self.scanned = range.end;
            let lower = tag.to_ascii_lowercase();
            if let Some(raw) = self.raw {
                if is_html_close_tag(&lower, raw) {
                    self.raw = None;
                }
                continue;
            }
            if let Some(raw) = raw_text_open_tag(&lower) {
                self.raw = Some(raw);
            } else if is_html_open_tag(&lower, "table") {
                self.depth += 1;
            } else if is_html_close_tag(&lower, "table") {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    return Some(range.end);
                }
            }
        }
        self.scanned = block.text.len();
        None
    }
}

struct CellSource {
    body: Range<usize>,
    is_header: bool,
    align: MarkdownTextAlign,
}

struct RowSource {
    source: Range<usize>,
    in_head: bool,
    cells: Vec<CellSource>,
}

fn close_cell(cell: &mut Option<CellSource>, row: &mut Option<RowSource>, at: usize) -> Option<()> {
    if let Some(mut cell) = cell.take() {
        cell.body.end = at;
        row.as_mut()?.cells.push(cell);
    }
    Some(())
}

fn close_row(row: &mut Option<RowSource>, rows: &mut Vec<RowSource>, at: usize) {
    if let Some(mut row) = row.take() {
        row.source.end = at;
        if !row.cells.is_empty() {
            rows.push(row);
        }
    }
}

fn inherited_html_align(tag: &str, parent: MarkdownTextAlign) -> MarkdownTextAlign {
    match html_align(tag) {
        MarkdownTextAlign::None => parent,
        align => align,
    }
}

/// Structural validation precedes rendering, so a rejected table stays intact.
fn table_rows(block: &HtmlBlockBuffer) -> Option<Vec<RowSource>> {
    let mut rows = Vec::new();
    let mut row = None;
    let mut cell = None;
    let mut in_head = false;
    let mut opened = false;
    let mut table_align = MarkdownTextAlign::None;
    let mut section_align = MarkdownTextAlign::None;
    let mut row_align = MarkdownTextAlign::None;
    for token in block.table_tokens(0..block.text.len()) {
        let range = match token {
            HtmlToken::Text(range) => {
                if cell.is_none() && !block.text[range].trim().is_empty() {
                    return None;
                }
                continue;
            }
            HtmlToken::Tag(range) => range,
        };
        let tag = &block.text[range.clone()];
        let lower = tag.to_ascii_lowercase();
        if is_html_open_tag(&lower, "table") {
            if opened {
                return None;
            }
            opened = true;
            table_align = html_align(tag);
            section_align = table_align;
        } else if is_html_close_tag(&lower, "table") {
            close_cell(&mut cell, &mut row, range.start)?;
            close_row(&mut row, &mut rows, range.end);
        } else if ["thead", "tbody", "tfoot"]
            .iter()
            .any(|name| is_html_open_tag(&lower, name) || is_html_close_tag(&lower, name))
        {
            close_cell(&mut cell, &mut row, range.start)?;
            close_row(&mut row, &mut rows, range.start);
            in_head = is_html_open_tag(&lower, "thead");
            section_align = inherited_html_align(tag, table_align);
        } else if is_html_open_tag(&lower, "tr") {
            close_cell(&mut cell, &mut row, range.start)?;
            close_row(&mut row, &mut rows, range.start);
            row_align = inherited_html_align(tag, section_align);
            row = Some(RowSource {
                source: range.start..range.end,
                in_head,
                cells: Vec::new(),
            });
        } else if is_html_close_tag(&lower, "tr") {
            close_cell(&mut cell, &mut row, range.start)?;
            close_row(&mut row, &mut rows, range.end);
        } else if is_html_open_tag(&lower, "td") || is_html_open_tag(&lower, "th") {
            row.as_ref()?;
            close_cell(&mut cell, &mut row, range.start)?;
            for attribute in ["colspan", "rowspan"] {
                if extract_html_attribute(tag, attribute).is_some_and(|value| value.trim() != "1") {
                    return None;
                }
            }
            cell = Some(CellSource {
                body: range.end..range.end,
                is_header: is_html_open_tag(&lower, "th"),
                align: inherited_html_align(tag, row_align),
            });
        } else if is_html_close_tag(&lower, "td") || is_html_close_tag(&lower, "th") {
            close_cell(&mut cell, &mut row, range.start)?;
        } else {
            let handling = classify_html_tag(tag);
            if handling == HtmlHandling::AppendLiteral
                || (cell.is_none() && handling != HtmlHandling::Ignore)
            {
                return None;
            }
        }
    }
    Some(rows)
}

impl Flattener<'_> {
    /// Keep the table whole across HTML blocks while recording the Markdown
    /// fragments pulldown recognizes between them.
    pub(super) fn capture_table_event(
        &mut self,
        event: &Event<'_>,
        range: &Range<usize>,
    ) -> Option<bool> {
        if let Event::End(tag) = event
            && self.captured_html_ends.last() == Some(tag)
        {
            self.captured_html_ends.pop();
            if self.pending_html_table.is_some() {
                self.capture_table_source(range.end)?;
            } else if *tag == TagEnd::Paragraph {
                self.end_html_flow(range.end)?;
            }
            return Some(true);
        }
        if self.pending_html_table.is_none()
            || self.html_block.is_some()
            || matches!(event, Event::Start(Tag::HtmlBlock))
        {
            return Some(false);
        }
        match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Paragraph => Some(HtmlMarkdownKind::Paragraph),
                    Tag::CodeBlock(_) => Some(HtmlMarkdownKind::CodeBlock(String::new())),
                    _ => None,
                };
                if let Some(kind) = kind {
                    if matches!(kind, HtmlMarkdownKind::CodeBlock(_)) {
                        self.pending_html_table
                            .as_mut()?
                            .block
                            .literals
                            .push(range.clone());
                    }
                    self.pending_html_table
                        .as_mut()?
                        .block
                        .markdown
                        .push(HtmlMarkdownFragment {
                            source: range.clone(),
                            kind,
                        });
                }
                self.captured_html_ends.push(tag.to_end());
            }
            Event::End(_) => {
                self.finish_pending_html_table()?;
                return Some(false);
            }
            Event::Code(_) => {
                self.pending_html_table
                    .as_mut()?
                    .block
                    .literals
                    .push(range.clone());
                self.capture_table_source(range.end)?;
            }
            Event::Text(text) => {
                if self.captured_html_ends.last() == Some(&TagEnd::CodeBlock)
                    && let Some(fragment) =
                        self.pending_html_table.as_mut()?.block.markdown.last_mut()
                    && let HtmlMarkdownKind::CodeBlock(code) = &mut fragment.kind
                {
                    code.push_str(text);
                }
                self.capture_table_source(range.end)?;
            }
            _ => self.capture_table_source(range.end)?,
        }
        Some(true)
    }

    fn capture_table_source(&mut self, end: usize) -> Option<()> {
        let start = self.pending_html_table.as_ref()?.block.lines.last()?.1.end;
        if end <= start {
            return Some(());
        }
        let mut block = HtmlBlockBuffer::default();
        let mut at = start;
        for line in self.source[start..end].split_inclusive('\n') {
            let mut skip = 0;
            if at == 0 || self.source.as_bytes().get(at - 1) == Some(&b'\n') {
                // Pulldown removes these prefixes from HTML events as well.
                for _ in 0..self.blockquote_level() {
                    let rest = &line[skip..];
                    let spaces = rest.len() - rest.trim_start_matches(' ').len();
                    if rest.as_bytes().get(spaces) != Some(&b'>') {
                        break;
                    }
                    skip += spaces + 1;
                    if line.as_bytes().get(skip) == Some(&b' ') {
                        skip += 1;
                    }
                }
            }
            block.push(&line[skip..], at + skip..at + line.len());
            at += line.len();
        }
        self.read_html_block(&block)
    }

    pub(super) fn finish_pending_html_table(&mut self) -> Option<()> {
        if let Some(pending) = self.pending_html_table.take() {
            self.fallback_table(&pending.block)?;
        }
        Some(())
    }

    fn fallback_table(&mut self, block: &HtmlBlockBuffer) -> Option<()> {
        // The buffer records event fragments for source mapping; several can
        // belong to one physical line. Only source newlines split the fallback.
        let mut start = 0;
        for line in block.text.split_inclusive('\n') {
            let end = start + line.len();
            self.push_fallback_rows(line, block.source_range(start..end))?;
            start = end;
        }
        Some(())
    }

    pub(super) fn read_html_block(&mut self, input: &HtmlBlockBuffer) -> Option<()> {
        let mut at = 0;
        let combined;
        let block = if let Some(mut pending) = self.pending_html_table.take() {
            pending.block.append(input);
            let Some(end) = pending.scanner.end(&pending.block) else {
                self.pending_html_table = Some(pending);
                return Some(());
            };
            self.emit_html_table(&pending.block.slice(0..end))?;
            at = end;
            combined = pending.block;
            &combined
        } else {
            input
        };
        if at == 0 && is_raw_text_html(&block.text) {
            return self.read_html_flow_block(block);
        }
        loop {
            let mut raw = None;
            let opening = block.table_tokens(at..block.text.len()).find_map(|token| {
                let HtmlToken::Tag(range) = token else {
                    return None;
                };
                let lower = block.text[range.clone()].to_ascii_lowercase();
                if let Some(name) = raw {
                    if is_html_close_tag(&lower, name) {
                        raw = None;
                    }
                    return None;
                }
                raw = raw_text_open_tag(&lower);
                is_html_open_tag(&lower, "table").then_some(range.start)
            });
            let Some(start) = opening else {
                if at < block.text.len() {
                    self.read_html_flow_block(&block.slice(at..block.text.len()))?;
                }
                break;
            };
            if start > at {
                self.read_html_flow_block(&block.slice(at..start))?;
            }
            self.end_html_row(block.source_range(start..start).start)?;
            let table = block.slice(start..block.text.len());
            let mut scanner = TableEndScanner::default();
            let Some(end) = scanner.end(&table) else {
                self.pending_html_table = Some(PendingHtmlTable {
                    block: table,
                    scanner,
                });
                break;
            };
            self.emit_html_table(&table.slice(0..end))?;
            at = start + end;
        }
        Some(())
    }

    fn emit_html_table(&mut self, block: &HtmlBlockBuffer) -> Option<()> {
        let Some(rows) = table_rows(block) else {
            return self.fallback_table(block);
        };
        let styles = self.styles.clone();
        let links = self.links.clone();
        let html_links = self.html_links;
        let floor = self.html_floor;
        self.table = Some(OpenTable {
            alignments: Vec::new(),
            row: None,
            cells: Vec::new(),
            starts_table: true,
        });
        for row in rows {
            self.clear_row();
            let is_header = row.in_head || row.cells.iter().all(|cell| cell.is_header);
            let source = block.source_range(row.source);
            self.table.as_mut()?.row = Some((source.start, is_header));
            for cell in row.cells {
                self.styles.clone_from(&styles);
                self.links.clone_from(&links);
                self.html_links = html_links;
                self.html_floor = floor;
                self.start_table_cell(cell.is_header, cell.align);
                self.read_html_table_cell(block, cell.body)?;
                self.trim_html_trailing_space();
                self.end_table_cell();
            }
            self.end_table_row(source.end)?;
        }
        self.table = None;
        self.styles = styles;
        self.links = links;
        self.html_links = html_links;
        self.html_floor = floor;
        Some(())
    }

    fn separate_html_cell_block(&mut self) {
        self.trim_html_trailing_space();
        if let Some(cell) = self.table.as_ref().and_then(|table| table.cells.last())
            && self.text.len() > cell.range.start
            && !self.text.ends_with('\n')
        {
            self.text.push('\n');
        }
    }

    fn read_html_table_cell(&mut self, block: &HtmlBlockBuffer, body: Range<usize>) -> Option<()> {
        let mut at = body.start;
        let mut after_block = false;
        let source = block.source_range(body.clone());
        let first = block
            .markdown
            .partition_point(|fragment| fragment.source.end <= source.start);
        for fragment in block.markdown[first..]
            .iter()
            .take_while(|fragment| fragment.source.start < source.end)
        {
            let range = block.text_range(fragment.source.clone());
            let range = range.start.max(body.start)..range.end.min(body.end);
            if range.start >= range.end {
                continue;
            }
            self.read_html_cell_text(block, at..range.start, after_block)?;
            self.separate_html_cell_block();
            match &fragment.kind {
                HtmlMarkdownKind::Paragraph => self.read_markdown_cell(block, range.clone())?,
                HtmlMarkdownKind::CodeBlock(code) => {
                    self.push_text(
                        code.trim_end_matches('\n'),
                        block.source_range(range.clone()),
                        true,
                    );
                }
            }
            at = range.end;
            after_block = true;
        }
        self.read_html_cell_text(block, at..body.end, after_block)
    }

    fn read_html_cell_text(
        &mut self,
        block: &HtmlBlockBuffer,
        range: Range<usize>,
        after_block: bool,
    ) -> Option<()> {
        if after_block {
            if block.text[range.clone()].trim().is_empty() {
                return Some(());
            }
            self.separate_html_cell_block();
        }
        for token in html_tokens(&block.text[range.clone()]) {
            let token = match token {
                HtmlToken::Tag(token) => {
                    let token = token.start + range.start..token.end + range.start;
                    self.apply_html(
                        classify_html_tag(&block.text[token.clone()]),
                        &block.text[token.clone()],
                        block.source_range(token),
                        false,
                    )?;
                    continue;
                }
                HtmlToken::Text(token) => token.start + range.start..token.end + range.start,
            };
            self.html_text(&block.text[token.clone()], block.source_range(token));
        }
        Some(())
    }

    fn read_markdown_cell(&mut self, block: &HtmlBlockBuffer, range: Range<usize>) -> Option<()> {
        for (event, offset) in
            Parser::new_ext(&block.text[range.clone()], markdown_parser_options())
                .into_offset_iter()
        {
            let source = block.source_range(offset.start + range.start..offset.end + range.start);
            match event {
                Event::Start(
                    tag @ (Tag::Emphasis
                    | Tag::Strong
                    | Tag::Strikethrough
                    | Tag::Link { .. }
                    | Tag::Image { .. }),
                ) => self.start(tag, source)?,
                Event::End(
                    tag @ (TagEnd::Emphasis
                    | TagEnd::Strong
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Image),
                ) => self.end(tag, source)?,
                Event::Text(text) => self.push_text(&text, source, false),
                Event::Code(text) => self.push_text(&text, source, true),
                Event::Html(html) | Event::InlineHtml(html) => self.html(&html, source, false)?,
                Event::SoftBreak | Event::HardBreak => self.soft_break(),
                Event::FootnoteReference(label) => {
                    self.push_text(&format!("[{label}]"), source, false)
                }
                _ => {}
            }
        }
        Some(())
    }
}
