use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkdownTableCell {
    pub(crate) text: String,
    pub(crate) spans: Vec<MarkdownInlineSpan>,
}

/// Lay out every table's cells: trim the tab the last cell left, record each
/// cell's range, give short rows empty cells, and measure the columns.
pub(crate) fn finish_table_blocks(rows: &mut [MarkdownPreviewRow]) {
    let mut start = 0usize;
    while start < rows.len() {
        if !matches!(rows[start].kind, MarkdownPreviewRowKind::TableRow { .. }) {
            start += 1;
            continue;
        }

        // A header row opens a table, so it also closes the one before it —
        // two tables that touch must keep their own columns.
        let mut end = start + 1;
        while end < rows.len()
            && matches!(
                rows[end].kind,
                MarkdownPreviewRowKind::TableRow { is_header: false }
            )
        {
            end += 1;
        }

        finish_table_block(&mut rows[start..end]);
        start = end;
    }
}

fn finish_table_block(rows: &mut [MarkdownPreviewRow]) {
    let mut row_cells = Vec::with_capacity(rows.len());
    for row in rows.iter_mut() {
        if let Some(text) = row.text.strip_suffix('\t') {
            row.text = SharedString::from(text.to_owned());
        }
        let text = row.text.as_ref();
        let mut cells = Vec::new();
        let mut cell_start = 0usize;
        for (byte_ix, _) in text.match_indices('\t') {
            cells.push(cell_start..byte_ix);
            cell_start = byte_ix + 1;
        }
        cells.push(cell_start..text.len());
        row_cells.push(cells);
    }

    let alignments = rows
        .first()
        .and_then(|row| row.table.as_ref())
        .map(|table| table.table.alignments.clone())
        .unwrap_or_default();
    let column_count = row_cells
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(alignments.len());
    let mut column_widths = vec![0usize; column_count];
    for (row, cells) in rows.iter().zip(&row_cells) {
        for (width, cell) in column_widths.iter_mut().zip(cells) {
            *width = (*width).max(row.text[cell.clone()].chars().count());
        }
    }
    let mut alignments = alignments;
    alignments.resize(column_count, MarkdownTableAlign::None);
    let table = Arc::new(MarkdownTableInfo {
        alignments,
        column_widths,
    });

    for (row, mut cells) in rows.iter_mut().zip(row_cells) {
        let end = row.text.len();
        cells.resize(column_count, end..end);
        row.table = Some(MarkdownTableRow {
            cells: Arc::from(cells),
            table: Arc::clone(&table),
        });
    }
}

/// A table row as the monospace row list draws it: cells padded to their
/// column's width and joined by ` | `, with the spans moved to match.
pub(crate) fn markdown_table_row_display(
    row: &MarkdownPreviewRow,
) -> Option<(String, Vec<MarkdownInlineSpan>)> {
    let table = row.table.as_ref()?;
    let cells = table
        .cells
        .iter()
        .map(|range| MarkdownTableCell {
            text: row.text[range.clone()].to_owned(),
            spans: row
                .inline_spans
                .iter()
                .filter_map(|span| {
                    let start = span.byte_range.start.max(range.start);
                    let end = span.byte_range.end.min(range.end);
                    (start < end).then(|| span.restyled((start - range.start)..(end - range.start)))
                })
                .collect(),
        })
        .collect();
    Some(build_aligned_table_row_text(
        cells,
        &table.table.column_widths,
    ))
}

pub(crate) fn build_aligned_table_row_text(
    cells: Vec<MarkdownTableCell>,
    column_widths: &[usize],
) -> (String, Vec<MarkdownInlineSpan>) {
    const TABLE_COLUMN_SEPARATOR: &str = " | ";

    let text_capacity = column_widths
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add)
        .saturating_add(
            cells
                .iter()
                .fold(0usize, |bytes, cell| bytes.saturating_add(cell.text.len())),
        )
        .saturating_add(
            TABLE_COLUMN_SEPARATOR
                .len()
                .saturating_mul(column_widths.len().saturating_sub(1)),
        );
    let span_capacity = cells
        .iter()
        .fold(0usize, |len, cell| len.saturating_add(cell.spans.len()));
    let mut text = String::with_capacity(text_capacity);
    let mut spans = Vec::with_capacity(span_capacity);
    let mut cells = cells.into_iter();

    for (ix, width) in column_widths.iter().copied().enumerate() {
        let cell = cells.next();
        let cell_width = cell
            .as_ref()
            .map(|cell| cell.text.chars().count())
            .unwrap_or(0);
        let cell_start = text.len();
        if let Some(cell) = cell {
            text.push_str(&cell.text);
            spans.extend(cell.spans.into_iter().map(|span| {
                span.restyled(
                    (cell_start + span.byte_range.start)..(cell_start + span.byte_range.end),
                )
            }));
        }

        if ix + 1 < column_widths.len() {
            let pad = width.saturating_sub(cell_width);
            for _ in 0..pad {
                text.push(' ');
            }
            text.push_str(TABLE_COLUMN_SEPARATOR);
        }
    }

    (text, spans)
}

pub(crate) fn normalize_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
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
        if ch.is_whitespace() {
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
