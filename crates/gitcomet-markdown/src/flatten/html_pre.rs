use super::*;

pub(super) struct PendingHtmlPre {
    block: HtmlBlockBuffer,
    scanned: usize,
}

pub(super) fn pre_end(html: &str, after: usize) -> Option<Range<usize>> {
    let lower = html[after..].to_ascii_lowercase();
    let start = lower
        .match_indices("</pre")
        .find(|(start, _)| is_html_close_tag(&lower[*start..], "pre"))?
        .0
        + after;
    let end = html[start..].find('>')? + start + 1;
    Some(start..end)
}

impl Flattener<'_> {
    pub(super) fn capture_pre_event(
        &mut self,
        event: &Event<'_>,
        range: &Range<usize>,
    ) -> Option<bool> {
        if let Event::End(tag) = event
            && self.captured_pre_ends.last() == Some(tag)
        {
            self.captured_pre_ends.pop();
            if self.pending_html_pre.is_some() {
                self.capture_pre_source(range.end)?;
            } else if *tag == TagEnd::Paragraph {
                self.end_html_flow(range.end)?;
            }
            return Some(true);
        }
        if self.pending_html_pre.is_none() {
            return Some(false);
        }
        match event {
            Event::Start(tag) => self.captured_pre_ends.push(tag.to_end()),
            Event::End(_) => {
                self.finish_pending_html_pre()?;
                return Some(false);
            }
            _ => self.capture_pre_source(range.end)?,
        }
        Some(true)
    }

    fn capture_pre_source(&mut self, end: usize) -> Option<()> {
        let start = self.pending_html_pre.as_ref()?.block.lines.last()?.1.end;
        if end > start {
            let block = self.source_buffer(start..end);
            self.read_html_pre(&block)?;
        }
        Some(())
    }

    pub(super) fn read_html_pre(&mut self, block: &HtmlBlockBuffer) -> Option<()> {
        let mut pending = if let Some(mut pending) = self.pending_html_pre.take() {
            pending.block.append(block);
            pending
        } else {
            self.split_row_at(block.source_range(0..0).start)?;
            PendingHtmlPre {
                block: block.slice(0..block.text.len()),
                scanned: 0,
            }
        };
        let Some(closing) = pre_end(&pending.block.text, pending.scanned) else {
            pending.scanned = pending
                .block
                .text
                .floor_char_boundary(pending.block.text.len().saturating_sub("</pre>".len()));
            self.pending_html_pre = Some(pending);
            return Some(());
        };
        let block = &pending.block;
        self.emit_html_pre(block, closing.start)?;
        if closing.end < block.text.len() {
            self.read_html_block(&block.slice(closing.end..block.text.len()))?;
        }
        Some(())
    }

    pub(super) fn finish_pending_html_pre(&mut self) -> Option<()> {
        if let Some(pending) = self.pending_html_pre.take() {
            self.emit_html_pre(&pending.block, pending.block.text.len())?;
        }
        Some(())
    }

    fn emit_html_pre(&mut self, block: &HtmlBlockBuffer, end: usize) -> Option<()> {
        let Some(HtmlToken::Tag(open)) = html_tokens(&block.text).next() else {
            return Some(());
        };
        let mut body = open.end..end;
        let mut language = None;
        let mut raw = &block.text[body.clone()];
        let after_whitespace = raw.trim_ascii_start();
        if let Some(HtmlToken::Tag(code)) = html_tokens(after_whitespace).next()
            && is_html_open_tag(&after_whitespace[code.clone()].to_ascii_lowercase(), "code")
        {
            language = extract_html_attribute(&after_whitespace[code.clone()], "class").and_then(
                |class| {
                    class.split_ascii_whitespace().find_map(|name| {
                        gitcomet_syntax::diff_syntax_language_for_code_fence_info(
                            name.strip_prefix("language-")?,
                        )
                    })
                },
            );
            body.start += raw.len() - after_whitespace.len() + code.end;
            raw = &block.text[body.clone()];
            let trimmed = raw.trim_ascii_end();
            if trimmed.to_ascii_lowercase().ends_with("</code>") {
                body.end = body.start + trimmed.len() - "</code>".len();
            }
        }
        raw = &block.text[body.clone()];
        body.start += if raw.starts_with("\r\n") {
            2
        } else {
            usize::from(raw.starts_with('\n'))
        };
        self.trim_next_start = false;
        self.push_html_code_rows(block, body, language)
    }

    fn push_html_code_rows(
        &mut self,
        block: &HtmlBlockBuffer,
        body: Range<usize>,
        language: Option<gitcomet_syntax::DiffSyntaxLanguage>,
    ) -> Option<()> {
        self.ensure_details_summary()?;
        let first_row = self.rows.len();
        let raw = &block.text[body.clone()];
        let mut at = body.start;
        for line in raw
            .split_inclusive('\n')
            .chain(raw.is_empty().then_some(""))
        {
            let end = at + line.len();
            let source = block.source_range(at..end);
            let source_line = byte_offset_to_line(source.start, self.line_starts);
            self.push_code_rows(
                &decode_html_entities(line),
                source_line,
                source_line,
                language,
            )?;
            at = end;
        }
        let last_row = self.rows.len().saturating_sub(first_row + 1);
        for (index, row) in self.rows[first_row..].iter_mut().enumerate() {
            row.kind = MarkdownPreviewRowKind::CodeLine {
                is_first: index == 0,
                is_last: index == last_row,
            };
        }
        Some(())
    }
}
