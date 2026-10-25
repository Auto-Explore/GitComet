use super::*;

fn parse(source: &str) -> MarkdownPreviewDocument {
    parse_markdown(source).expect("table preview parses")
}

fn cells(row: &MarkdownPreviewRow) -> Vec<&str> {
    row.table
        .as_ref()
        .unwrap()
        .cells
        .iter()
        .map(|cell| &row.text[cell.range.clone()])
        .collect()
}

#[test]
fn html_tables_share_markdown_rows_and_header_styles() {
    let html = parse(
        "<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>",
    );
    let markdown = parse("| A | B |\n|---|---|\n| 1 | 2 |\n");
    for (actual, expected) in html.rows.iter().zip(&markdown.rows) {
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(cells(actual), cells(expected));
    }
    assert_eq!(html.rows.len(), 2);
    assert_eq!(
        markdown_document_blocks(&html),
        vec![MarkdownBlock::Table(0..2)]
    );
}

#[test]
fn auth_token_readme_tables_render_with_markdown_code_samples() {
    let doc = parse(include_str!("fixtures/auth_token_tables.md"));
    let tables = markdown_document_blocks(&doc)
        .into_iter()
        .filter_map(|block| match block {
            MarkdownBlock::Table(rows) => Some(rows),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 2, "both README tables render");
    assert!(
        doc.rows
            .iter()
            .all(|row| row.kind != MarkdownPreviewRowKind::PlainFallback)
    );
    let browser = &doc.rows[tables[0].start];
    assert_eq!(cells(browser)[0], "Browsers");
    assert!(cells(browser)[1].contains("<script type=\"module\">\n  import"));
    assert!(!browser.text.contains("```"));
    assert!(browser.inline_spans.iter().any(|span| {
        span.style == MarkdownInlineStyle::Code
            && browser.text[span.byte_range.clone()].starts_with("<script")
    }));
    assert!(browser.inline_spans.iter().any(|span| {
        span.link_url.as_deref() == Some("https://esm.sh")
            && &browser.text[span.byte_range.clone()] == "esm.sh"
    }));
    assert_eq!(cells(&doc.rows[tables[0].end - 1])[0], "Node");
    assert_eq!(tables[1].len(), 4);
    assert_eq!(
        cells(&doc.rows[tables[1].start]),
        vec!["name", "type", "description"]
    );
    assert!(
        tables
            .iter()
            .flat_map(|rows| &doc.rows[rows.clone()])
            .all(|row| {
                row.table
                    .as_ref()
                    .unwrap()
                    .cells
                    .iter()
                    .all(|cell| cell.align == MarkdownTextAlign::Left)
            })
    );
    assert!(doc.rows.iter().any(|row| {
        matches!(row.kind, MarkdownPreviewRowKind::CodeLine { .. })
            && row.text.starts_with("const auth =")
    }));
    assert_eq!(doc.rows.last().unwrap().text.as_ref(), "MIT");
}

#[test]
fn html_table_boundaries_do_not_depend_on_headers() {
    let doc = parse(
        "<table><tr><td>one</td></tr></table><table><thead><tr><th>a</th><th>b</th></tr><tr><th>c</th><th>d</th></tr></thead><tr><td>x</td></tr></table>",
    );
    assert_eq!(
        markdown_document_blocks(&doc),
        vec![MarkdownBlock::Table(0..1), MarkdownBlock::Table(1..4)]
    );
    assert_eq!(cells(&doc.rows[0]), vec!["one"]);
    assert_eq!(cells(&doc.rows[3]), vec!["x", ""]);
    assert_eq!(
        doc.rows
            .iter()
            .map(|row| row.table.as_ref().unwrap().starts_table)
            .collect::<Vec<_>>(),
        vec![true, true, false, false]
    );
}

#[test]
fn html_cells_keep_inline_formatting_links_entities_and_alignment() {
    let doc = parse(
        "<TABLE>\n<TR><TH>Label</TH><TD\n ALIGN='right'><b>Bold</b> &amp; <a href='docs/a.md'>link</a><br>tail</TD></TR>\n</TABLE>",
    );
    assert_eq!(cells(&doc.rows[0]), vec!["Label", "Bold & link tail"]);
    let row = &doc.rows[0];
    assert!(row.table.as_ref().unwrap().cells[0].is_header);
    assert_eq!(
        row.table.as_ref().unwrap().cells[1].align,
        MarkdownTextAlign::Right
    );
    assert!(
        row.inline_spans
            .iter()
            .any(|span| span.style == MarkdownInlineStyle::Bold
                && &row.text[span.byte_range.clone()] == "Bold")
    );
    assert!(
        row.inline_spans
            .iter()
            .any(|span| span.link_url.as_deref() == Some("docs/a.md")
                && &row.text[span.byte_range.clone()] == "link")
    );
    assert_eq!(row.source_line_range, 1..3);
}

#[test]
fn html_tables_continue_across_blank_lines_and_optional_end_tags() {
    let doc = parse("<table>\n<tr><td>A<td>B\n\n<tr><td>one<td>two\n</table>\n\nAfter\n");
    assert_eq!(cells(&doc.rows[0]), vec!["A", "B"]);
    assert_eq!(cells(&doc.rows[1]), vec!["one", "two"]);
    assert_eq!(doc.rows[2].text.as_ref(), "After");
    assert_eq!(doc.rows[2].kind, MarkdownPreviewRowKind::Paragraph);
}

#[test]
fn markdown_between_html_cell_blocks_keeps_inline_formatting() {
    let doc = parse("<table><tr><td>\n\n**literal** and `code`\n\n</td></tr></table>\n");
    assert_eq!(cells(&doc.rows[0]), vec!["literal and code"]);
    assert!(
        doc.rows[0]
            .inline_spans
            .iter()
            .any(|span| span.style == MarkdownInlineStyle::Bold)
    );
    assert!(
        doc.rows[0]
            .inline_spans
            .iter()
            .any(|span| span.style == MarkdownInlineStyle::Code)
    );
}

#[test]
fn html_cell_text_in_the_same_html_block_stays_literal() {
    let doc = parse("<table><tr><td>**literal** and `code`</td></tr></table>\n");
    assert_eq!(cells(&doc.rows[0]), vec!["**literal** and `code`"]);
    assert!(doc.rows[0].inline_spans.is_empty());
}

#[test]
fn html_table_code_samples_cannot_change_table_structure_or_create_images() {
    for fence in ["```", "~~~"] {
        for sample in [
            "</table>",
            "<table><tr><td>sample</td></tr></table>",
            "<img src='sample.png' alt='sample'>",
            "<table",
        ] {
            let doc = parse(&format!(
                "<table><tr><td>\n\n{fence}html\n{sample}\n{fence}\n\n</td></tr></table>\n\nAfter\n"
            ));
            assert_eq!(cells(&doc.rows[0]), vec![sample]);
            assert!(doc.rows[0].inline_images.is_empty());
            assert_eq!(doc.rows[1].text.as_ref(), "After");
            assert!(doc.rows[1].table.is_none());
        }
    }
    let doc =
        parse("<table><tr><td>\n\nbefore `<table><script>` after\n\n</td></tr></table>\n\nAfter\n");
    assert_eq!(cells(&doc.rows[0]), vec!["before <table><script> after"]);
    assert_eq!(doc.rows[1].text.as_ref(), "After");

    let quoted = parse(
        "> <table><tr><td>\n>\n> ```html\n> </table>\n> <img src='sample.png'>\n> ```\n>\n> </td></tr></table>\n\nAfter\n",
    );
    assert_eq!(
        cells(&quoted.rows[0]),
        vec!["</table>\n<img src='sample.png'>"]
    );
    assert_eq!(quoted.rows[0].blockquote_level, 1);
    assert!(quoted.rows[0].inline_images.is_empty());
    assert_eq!(quoted.rows[1].text.as_ref(), "After");
}

#[test]
fn html_table_alignment_inherits_through_sections_and_rows() {
    let doc = parse(
        "<table align=right><thead align=left><tr><th>A</th><th>B</th></tr></thead><tbody align=center><tr align=left><th>C</th><td align=right>D</td></tr><tr><td>E</td><td>F</td></tr></tbody><tfoot><tr><td>G</td><td>H</td></tr></tfoot></table>",
    );
    assert_eq!(
        doc.rows
            .iter()
            .map(|row| row
                .table
                .as_ref()
                .unwrap()
                .cells
                .iter()
                .map(|cell| cell.align)
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![
            vec![MarkdownTextAlign::Left, MarkdownTextAlign::Left],
            vec![MarkdownTextAlign::Left, MarkdownTextAlign::Right],
            vec![MarkdownTextAlign::Center, MarkdownTextAlign::Center],
            vec![MarkdownTextAlign::Right, MarkdownTextAlign::Right],
        ]
    );
}

#[test]
fn html_table_containers_and_formatting_do_not_leak() {
    let doc = parse(
        "<div align=center><table><tr><td><b>one</td><td>two</td></tr></table></div>\n\nAfter\n",
    );
    assert_eq!(cells(&doc.rows[0]), vec!["one", "two"]);
    assert!(
        doc.rows[0]
            .inline_spans
            .iter()
            .all(|span| span.byte_range.end <= 3)
    );
    assert_eq!(doc.rows[1].align, MarkdownTextAlign::None);
    assert!(doc.rows[1].inline_spans.is_empty());
    let quoted =
        parse("> <table>\n> <tr><td>A</td></tr>\n>\n> <tr><td>B</td></tr>\n> </table>\n\nAfter\n");
    assert!(quoted.rows[..2].iter().all(|row| row.blockquote_level == 1));
    assert_eq!(quoted.rows[2].blockquote_level, 0);
}

#[test]
fn unsupported_tables_fall_back_intact_without_losing_surrounding_text() {
    for source in [
        "<table><tr><td colspan='2'>merged</td></tr></table>",
        "<table><tr><td rowspan='0'>merged</td></tr></table>",
        "<table><tr><td><table><tr><td>nested</td></tr></table></td></tr></table>",
        "<table><caption>caption</caption><tr><td>x</td></tr></table>",
        "<table><tr><td><script>var x = '<table>';</script></td></tr></table>",
    ] {
        let doc = parse(&format!("Before\n\n{source}\n\nAfter\n"));
        assert_eq!(doc.rows[0].text.as_ref(), "Before");
        assert_eq!(doc.rows[1].text.as_ref(), source);
        assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::PlainFallback);
        assert_eq!(doc.rows.last().unwrap().text.as_ref(), "After");
        assert!(doc.rows.iter().all(|row| row.table.is_none()));
    }
}

#[test]
fn table_markup_in_code_comments_and_attributes_is_not_a_table() {
    for source in [
        "```html\n<table><tr><td>literal</td></tr></table>\n```",
        "<!-- <table><tr><td>hidden</td></tr></table> -->",
        "<script>var x = '<table><tr><td>literal</td></tr></table>';</script>",
        "<p title='<table><tr><td>literal</td></tr></table>'>words</p>",
    ] {
        assert!(
            parse(source).rows.iter().all(|row| row.table.is_none()),
            "{source}"
        );
    }
}

#[test]
fn images_keep_cell_ownership_copy_text_and_content_order() {
    for source in [
        "| A | B | C |\n|---|---|---|\n| ![](a.png) | before ![café](b.png) after <img src='c.png' alt='last'> | ![](d.png) |\n",
        "<table><tr><th>A</th><th>B</th><th>C</th></tr><tr><td><img src='a.png'></td><td>before <img src='b.png' alt='café'> after <img src='c.png' alt='last'></td><td><img src='d.png'></td></tr></table>",
    ] {
        let doc = parse(source);
        let row = &doc.rows[1];
        assert_eq!(cells(row), vec!["", "before café after last", ""]);
        assert_eq!(row.text.as_ref(), "\tbefore café after last\t");
        assert_eq!(row.inline_images.len(), 4);
        let table = row.table.as_ref().unwrap();
        let images = |column: usize| {
            table.cells[column]
                .content
                .iter()
                .filter_map(|part| match part {
                    MarkdownTableCellPart::Image { index, range } => {
                        assert_eq!(
                            &row.text[range.clone()],
                            row.inline_images[*index].alt.as_ref()
                        );
                        Some(*index)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(images(0), vec![0]);
        assert_eq!(images(1), vec![1, 2]);
        assert_eq!(images(2), vec![3]);
        assert_eq!(table.cells[1].content.len(), 4);
        assert!(doc.rows.iter().all(|row| row.image.is_none()));
    }
}

#[test]
fn padded_cells_do_not_duplicate_the_last_image() {
    let doc = parse("<table><tr><th>A<th>B<th>C<tr><td><img src='x.png'></table>");
    let row = &doc.rows[1];
    let cells = &row.table.as_ref().unwrap().cells;
    assert_eq!(cells.len(), 3);
    assert!(cells[1..].iter().all(|cell| {
        cell.content
            .iter()
            .all(|part| matches!(part, MarkdownTableCellPart::Text(_)))
    }));
}

#[test]
fn html_table_images_keep_dimensions_links_and_source_offsets() {
    let source = "<table><tr><td><a href='doc.md'><img src='x.png' alt='X' width='120' height='60'></a></td></tr></table>";
    let doc = parse(source);
    let image = &doc.rows[0].inline_images[0];
    assert_eq!(image.image.width_px, Some(120));
    assert_eq!(image.image.height_px, Some(60));
    assert_eq!(image.link_url.as_deref(), Some("doc.md"));
    assert_eq!(image.source_byte, source.find("<img").unwrap());
}

#[test]
fn header_and_alignment_edits_are_visible_in_both_diff_modes() {
    for (old, new) in [
        (
            "<table>\n<thead>\n<tr><td>A</td></tr>\n</thead>\n</table>\n",
            "<table>\n<tbody>\n<tr><td>A</td></tr>\n</tbody>\n</table>\n",
        ),
        (
            "<table>\n<tr><td align=left>A</td></tr>\n</table>",
            "<table>\n<tr><td align=right>A</td></tr>\n</table>",
        ),
        (
            "<table>\n<tbody align=left>\n<tr><td>A</td></tr>\n</tbody>\n</table>",
            "<table>\n<tbody align=right>\n<tr><td>A</td></tr>\n</tbody>\n</table>",
        ),
    ] {
        let diff = build_markdown_diff_preview(old, new).unwrap();
        for doc in [&diff.old, &diff.new, &diff.inline] {
            assert!(
                doc.rows
                    .iter()
                    .filter(|row| row.table.is_some())
                    .all(|row| row.change_hint != MarkdownChangeHint::None)
            );
        }
        assert_eq!(
            diff.inline
                .rows
                .iter()
                .filter(|row| row.table.is_some())
                .count(),
            2
        );
    }
}

#[test]
fn markdown_column_alignment_edits_are_visible_in_both_diff_modes() {
    let diff =
        build_markdown_diff_preview("| A |\n|:---|\n| body |\n", "| A |\n|---:|\n| body |\n")
            .unwrap();
    for doc in [&diff.old, &diff.new, &diff.inline] {
        let body_rows = doc
            .rows
            .iter()
            .filter(|row| row.text.as_ref() == "body")
            .collect::<Vec<_>>();
        assert!(!body_rows.is_empty());
        assert!(
            body_rows
                .iter()
                .all(|row| row.change_hint != MarkdownChangeHint::None)
        );
    }
    assert_eq!(
        diff.inline
            .rows
            .iter()
            .filter(|row| row.text.as_ref() == "body")
            .count(),
        2,
        "the inline diff shows both column alignments"
    );
}

#[test]
fn empty_and_unclosed_tables_do_not_leave_parser_state_open() {
    assert!(parse("<table></table>").rows.is_empty());
    let doc = parse("- <table><tr><td>unfinished\n\nAfter\n");
    assert!(
        doc.rows
            .iter()
            .any(|row| row.kind == MarkdownPreviewRowKind::PlainFallback)
    );
    assert_eq!(doc.rows.last().unwrap().text.as_ref(), "After");
    assert!(doc.rows.last().unwrap().table.is_none());
}

#[test]
fn html_tables_obey_the_preview_row_limit() {
    let source = format!(
        "<table>{}</table>",
        "<tr><td>x</td></tr>".repeat(MAX_PREVIEW_ROWS + 1)
    );
    assert!(source.len() < MAX_PREVIEW_SOURCE_BYTES);
    assert!(parse_markdown(&source).is_none());
}

fn assert_image_keeps_suffix(suffix: &str) {
    let doc = parse(&format!(
        "<div><table><tr><td><img src='x' alt=' icon'>{suffix}</td></tr></table></div>"
    ));
    let row = &doc.rows[0];
    assert_eq!(row.text.as_ref(), format!("icon{suffix}"));
    let parts = &row.table.as_ref().unwrap().cells[0].content;
    let [
        MarkdownTableCellPart::Image { index, range },
        MarkdownTableCellPart::Text(text),
    ] = parts.as_slice()
    else {
        panic!("the image and its following text remain separate: {parts:?}");
    };
    assert_eq!(&row.text[text.clone()], suffix);
    assert_eq!(&row.text[range.clone()], "icon");
    assert_eq!(row.inline_images[*index].alt.as_ref(), "icon");
}

#[test]
fn table_image_ranges_preserve_utf8_after_trimmed_alt_text() {
    assert_image_keeps_suffix("é");
}

#[test]
fn table_image_ranges_preserve_ascii_after_trimmed_alt_text() {
    assert_image_keeps_suffix("suffix");
}

#[test]
fn table_fallback_coalesces_inline_fragments_on_one_source_line() {
    let table = "<table><tr><td colspan='2'>x</td></tr></table>";
    let doc = parse(&format!("Before {table} After"));
    let fallback: Vec<_> = doc
        .rows
        .iter()
        .filter(|row| row.kind == MarkdownPreviewRowKind::PlainFallback)
        .collect();
    assert_eq!(
        fallback
            .iter()
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>(),
        vec![table]
    );
    assert_eq!(fallback[0].source_line_range, 0..1);
    assert_eq!(doc.rows.first().unwrap().text.trim(), "Before");
    assert_eq!(doc.rows.last().unwrap().text.trim(), "After");
}

#[test]
fn table_fallback_coalesces_content_captured_between_html_blocks() {
    let source =
        "<table><tr><td colspan='2'>\n\n**bold** and <b>raw</b> text\n\n</td></tr></table>";
    let doc = parse(source);
    assert_eq!(
        doc.rows
            .iter()
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>(),
        source.lines().collect::<Vec<_>>()
    );
    for (line, row) in doc.rows.iter().enumerate() {
        assert_eq!(row.kind, MarkdownPreviewRowKind::PlainFallback);
        assert_eq!(row.source_line_range, line..line + 1);
    }
}

#[test]
fn table_span_validation_accepts_whitespace_around_equals() {
    for attribute in [
        "rowspan = \"2\"",
        "colspan = '2'",
        "COLSPAN\t=\n\"2\"",
        "rowspan = 2",
        "colspan= '2'",
        "colspan\r\n=\t2",
    ] {
        let source = format!("<table><tr><td {attribute}>merged</td><td>tail</td></tr></table>");
        let doc = parse(&source);
        assert!(
            doc.rows.iter().all(|row| row.table.is_none()),
            "{attribute}"
        );
        assert_eq!(
            doc.rows
                .iter()
                .map(|row| row.text.as_ref())
                .collect::<Vec<_>>(),
            source.lines().collect::<Vec<_>>()
        );
    }
}

#[test]
fn table_span_validation_ignores_quoted_text_and_accepts_unit_spans() {
    let doc = parse(
        "<table><tr><td title=\" rowspan='2' colspan = '3' \" data-colspan='2' colspan = '1' rowspan\t=\t1>x</td><td>y</td></tr></table>",
    );
    assert_eq!(cells(&doc.rows[0]), vec!["x", "y"]);
}
