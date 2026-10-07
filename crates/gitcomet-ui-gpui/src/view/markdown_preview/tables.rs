use super::*;

/// Finish each table independently, including headerless HTML tables.
pub(crate) fn finish_table_blocks(rows: &mut [MarkdownPreviewRow]) {
    let mut start = 0;
    while start < rows.len() {
        if rows[start].table.is_none() {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < rows.len()
            && rows[end]
                .table
                .as_ref()
                .is_some_and(|table| !table.starts_table)
        {
            end += 1;
        }
        finish_table_block(&mut rows[start..end]);
        start = end;
    }
}

fn finish_table_block(rows: &mut [MarkdownPreviewRow]) {
    let columns = rows
        .iter()
        .filter_map(|row| row.table.as_ref())
        .map(|table| table.cells.len())
        .max()
        .unwrap_or(0);
    for row in rows {
        let table = row.table.as_mut().unwrap();
        let mut cells = table.cells.to_vec();
        let mut images = row.inline_images.iter().enumerate().peekable();
        for cell in &mut cells {
            let mut parts = Vec::new();
            let mut at = cell.range.start;
            while let Some(&(index, image)) = images.peek() {
                if image.byte_offset > cell.range.end {
                    break;
                }
                images.next();
                let start = image.byte_offset;
                let end = (start + image.alt.len()).min(cell.range.end);
                if at < start {
                    parts.push(MarkdownTableCellPart::Text(at..start));
                }
                parts.push(MarkdownTableCellPart::Image {
                    index,
                    range: start..end,
                });
                at = end;
            }
            if parts.is_empty() {
                continue;
            }
            if at < cell.range.end {
                parts.push(MarkdownTableCellPart::Text(at..cell.range.end));
            }
            cell.content = parts;
        }
        cells.resize_with(columns, || MarkdownTableCell {
            align: if matches!(
                row.kind,
                MarkdownPreviewRowKind::TableRow { is_header: true }
            ) {
                MarkdownTextAlign::Center
            } else {
                MarkdownTextAlign::None
            },
            ..MarkdownTableCell::new(row.text.len()..row.text.len())
        });
        table.cells = Arc::from(cells);
    }
}

pub(crate) fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        // HTML collapses only ASCII whitespace: a no-break or em space keeps
        // its width and its place.
        if ch.is_ascii_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result
}

pub(crate) fn normalize_whitespace_with_spans(
    text: &str,
    inline_spans: &[MarkdownInlineSpan],
) -> (String, Vec<MarkdownInlineSpan>) {
    if inline_spans.is_empty() {
        return (normalize_whitespace(text), Vec::new());
    }

    let mut normalized = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];
    let mut prev_ws = false;
    let mut normalized_len = 0usize;

    for (byte_ix, ch) in text.char_indices() {
        byte_map[byte_ix] = normalized_len;
        if ch.is_ascii_whitespace() {
            if !prev_ws {
                normalized.push(' ');
                normalized_len += 1;
            }
            prev_ws = true;
        } else {
            normalized.push(ch);
            normalized_len += ch.len_utf8();
            prev_ws = false;
        }
        byte_map[byte_ix + ch.len_utf8()] = normalized_len;
    }

    let remapped_spans = inline_spans
        .iter()
        .filter_map(|span| {
            debug_assert!(text.is_char_boundary(span.byte_range.start));
            debug_assert!(text.is_char_boundary(span.byte_range.end));
            let start = *byte_map.get(span.byte_range.start)?;
            let end = *byte_map.get(span.byte_range.end)?;
            (start < end).then(|| span.restyled(start..end))
        })
        .collect();

    (normalized, remapped_spans)
}
