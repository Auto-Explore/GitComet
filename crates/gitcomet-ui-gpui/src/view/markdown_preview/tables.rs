use super::*;

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
