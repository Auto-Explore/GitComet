use super::*;

fn parse(src: &str) -> MarkdownPreviewDocument {
    parse_markdown(src).expect("parse should succeed")
}

#[test]
fn parse_does_not_reserve_rows_beyond_the_row_cap() {
    // Rows are per markdown *block*, so the source line count is unrelated
    // to how many there will be -- and `MAX_PREVIEW_SOURCE_BYTES` admits a
    // 1 MiB file of short lines. Reserving one row per line there costs
    // hundreds of megabytes for a document the parser caps far below it.
    let source = "\n".repeat(50_000);
    assert!(source.len() <= MAX_PREVIEW_SOURCE_BYTES);

    let doc = parse_markdown(&source).expect("blank lines parse");

    assert!(
        doc.rows.capacity() <= MAX_PREVIEW_ROWS,
        "reserved {} rows for a document capped at {MAX_PREVIEW_ROWS}",
        doc.rows.capacity()
    );
}

fn thematic_break_rows(count: usize) -> String {
    "---\n".repeat(count)
}

fn row_kinds(doc: &MarkdownPreviewDocument) -> Vec<&MarkdownPreviewRowKind> {
    doc.rows.iter().map(|r| &r.kind).collect()
}

fn row_texts(doc: &MarkdownPreviewDocument) -> Vec<&str> {
    doc.rows.iter().map(|r| r.text.as_ref()).collect()
}

fn code_rows(doc: &MarkdownPreviewDocument) -> Vec<&MarkdownPreviewRow> {
    doc.rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::CodeLine { .. }))
        .collect()
}

fn spans_with_style(
    row: &MarkdownPreviewRow,
    style: MarkdownInlineStyle,
) -> Vec<&MarkdownInlineSpan> {
    row.inline_spans
        .iter()
        .filter(|s| s.style == style)
        .collect()
}

// ── Heading tests ───────────────────────────────────────────────────

#[test]
fn heading_levels_are_preserved() {
    let doc = parse("# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n");
    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Heading { level: 2 },
            &MarkdownPreviewRowKind::Heading { level: 3 },
            &MarkdownPreviewRowKind::Heading { level: 4 },
            &MarkdownPreviewRowKind::Heading { level: 5 },
            &MarkdownPreviewRowKind::Heading { level: 6 },
        ]
    );
    assert_eq!(row_texts(&doc), vec!["H1", "H2", "H3", "H4", "H5", "H6"]);
}

#[test]
fn top_level_heading_inserts_section_spacer_before_following_content() {
    let doc = parse("# Title\n\nParagraph\n");

    assert_eq!(doc.rows.len(), 3);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(doc.rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(doc.rows[2].kind, MarkdownPreviewRowKind::Paragraph);
}

#[test]
fn content_before_top_level_heading_gets_section_spacer_before_heading() {
    let doc = parse("Paragraph\n\n# Title\n");

    assert_eq!(doc.rows.len(), 3);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(
        doc.rows[2].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
}

#[test]
fn consecutive_headings_do_not_insert_spacers_between_heading_rows() {
    let doc = parse("# Title\n## Subtitle\n");

    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Heading { level: 2 },
        ]
    );
}

#[test]
fn middle_heading_uses_one_section_spacer_not_two() {
    // The break above a section is a full blank row; the gap below the
    // heading comes from its own insets, so a second blank row here would
    // read as a hole in the document.
    let doc = parse("Intro\n\n# Title\n\nBody\n");

    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Paragraph,
            &MarkdownPreviewRowKind::Spacer,
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Paragraph,
        ]
    );
}

#[test]
fn heading_section_spacers_are_not_doubled_or_trailing() {
    // Consecutive headings share one break, and a heading that ends the
    // document gets no dangling spacer under it.
    assert_eq!(
        row_kinds(&parse("Intro\n\n# Title\n## Subtitle\n\nBody\n")),
        vec![
            &MarkdownPreviewRowKind::Paragraph,
            &MarkdownPreviewRowKind::Spacer,
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Heading { level: 2 },
            &MarkdownPreviewRowKind::Spacer,
            &MarkdownPreviewRowKind::Paragraph,
        ]
    );
    assert_eq!(
        row_kinds(&parse("Body\n\n# Title\n")),
        vec![
            &MarkdownPreviewRowKind::Paragraph,
            &MarkdownPreviewRowKind::Spacer,
            &MarkdownPreviewRowKind::Heading { level: 1 },
        ]
    );
}

// ── Paragraph tests ─────────────────────────────────────────────────

#[test]
fn paragraph_produces_one_row() {
    let doc = parse("Hello world.\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].text.as_ref(), "Hello world.");
}

#[test]
fn multiline_paragraph_normalizes_whitespace() {
    let doc = parse("Line one\nLine two\nLine three\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].text.as_ref(), "Line one Line two Line three");
}

#[test]
fn hard_breaks_split_paragraph_rows() {
    let doc = parse("This example  \nWill span two lines\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].text.as_ref(), "This example");
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
}

#[test]
fn backslash_hard_breaks_split_paragraph_rows() {
    let doc = parse("This example\\\nWill span two lines\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].text.as_ref(), "This example");
    assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
}

#[test]
fn html_br_splits_paragraph_rows() {
    let doc = parse("This example<br/>\nWill span two lines\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].text.as_ref(), "This example");
    assert_eq!(doc.rows[1].text.as_ref(), "Will span two lines");
}

#[test]
fn whitespace_normalization_preserves_inline_span_offsets() {
    let doc = parse("Prefix  **bold**\nnext line\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].text.as_ref(), "Prefix bold next line");

    let bold_span = doc.rows[0]
        .inline_spans
        .iter()
        .find(|span| span.style == MarkdownInlineStyle::Bold)
        .expect("expected bold span");
    assert_eq!(
        &doc.rows[0].text.as_ref()[bold_span.byte_range.clone()],
        "bold"
    );
}

// ── List tests ──────────────────────────────────────────────────────

#[test]
fn unordered_list_items_become_rows() {
    let doc = parse("- alpha\n- beta\n- gamma\n");
    assert_eq!(doc.rows.len(), 3);
    for row in &doc.rows {
        assert_eq!(row.kind, MarkdownPreviewRowKind::ListItem { number: None });
    }
    assert_eq!(row_texts(&doc), vec!["alpha", "beta", "gamma"]);
}

#[test]
fn ordered_list_items_preserve_numbers() {
    let doc = parse("3. first\n4. second\n5. third\n");
    assert_eq!(doc.rows.len(), 3);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::ListItem { number: Some(3) }
    );
    assert_eq!(
        doc.rows[1].kind,
        MarkdownPreviewRowKind::ListItem { number: Some(4) }
    );
    assert_eq!(
        doc.rows[2].kind,
        MarkdownPreviewRowKind::ListItem { number: Some(5) }
    );
}

#[test]
fn loose_list_items_still_render_as_list_rows() {
    let doc = parse("- first\n\n- second\n");
    assert_eq!(doc.rows.len(), 2);
    for row in &doc.rows {
        assert_eq!(row.kind, MarkdownPreviewRowKind::ListItem { number: None });
    }
}

#[test]
fn nested_list_increases_indent() {
    let doc = parse("- outer\n  - inner\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].indent_level, 1);
    assert_eq!(doc.rows[1].indent_level, 2);
}

// ── Blockquote tests ────────────────────────────────────────────────

#[test]
fn blockquote_produces_blockquote_row() {
    let doc = parse("> quoted text\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
    assert_eq!(doc.rows[0].text.as_ref(), "quoted text");
}

#[test]
fn multiline_blockquote_produces_one_row_per_logical_quote_line() {
    let doc = parse("> first line\n> second line\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
    assert_eq!(doc.rows[0].text.as_ref(), "first line");
    assert_eq!(doc.rows[0].source_line_range, 0..1);
    assert_eq!(doc.rows[0].blockquote_level, 1);
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::BlockquoteLine);
    assert_eq!(doc.rows[1].text.as_ref(), "second line");
    assert_eq!(doc.rows[1].source_line_range, 1..2);
    assert_eq!(doc.rows[1].blockquote_level, 1);
}

#[test]
fn nested_blockquotes_preserve_quote_depth_per_row() {
    let doc = parse("> outer\n>> inner\n>>> deepest\n");
    assert_eq!(doc.rows.len(), 3);
    assert_eq!(doc.rows[0].blockquote_level, 1);
    assert_eq!(doc.rows[1].blockquote_level, 2);
    assert_eq!(doc.rows[2].blockquote_level, 3);
}

#[test]
fn list_items_inside_blockquotes_keep_quote_depth() {
    let doc = parse("> - first\n>> 3. second\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::ListItem { number: None }
    );
    assert_eq!(doc.rows[0].blockquote_level, 1);
    assert_eq!(
        doc.rows[1].kind,
        MarkdownPreviewRowKind::ListItem { number: Some(3) }
    );
    assert_eq!(doc.rows[1].blockquote_level, 2);
}

#[test]
fn code_block_inside_blockquote_keeps_quote_depth() {
    let doc = parse("> ```\n> code\n> ```\n");
    let cr = code_rows(&doc);
    assert_eq!(cr.len(), 1);
    assert_eq!(cr[0].text.as_ref(), "code");
    assert_eq!(cr[0].blockquote_level, 1);
}

#[test]
fn gfm_alert_blockquotes_capture_alert_kind_and_hide_marker_line() {
    let doc = parse("> [!NOTE]\n> Line 1.\n> Line 2.\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::BlockquoteLine);
    assert_eq!(doc.rows[0].text.as_ref(), "Line 1.");
    assert_eq!(doc.rows[0].alert_kind, Some(MarkdownAlertKind::Note));
    assert!(doc.rows[0].starts_alert);
    assert_eq!(doc.rows[1].text.as_ref(), "Line 2.");
    assert_eq!(doc.rows[1].alert_kind, Some(MarkdownAlertKind::Note));
    assert!(!doc.rows[1].starts_alert);
}

#[test]
fn nested_alert_blockquotes_stay_scoped_to_inner_quote_rows() {
    let doc = parse("> outer\n>\n> > [!WARNING]\n> > inner\n>\n> outer again\n");
    assert_eq!(doc.rows.len(), 3);

    assert_eq!(doc.rows[0].text.as_ref(), "outer");
    assert_eq!(doc.rows[0].blockquote_level, 1);
    assert_eq!(doc.rows[0].alert_kind, None);
    assert!(!doc.rows[0].starts_alert);

    assert_eq!(doc.rows[1].text.as_ref(), "inner");
    assert_eq!(doc.rows[1].blockquote_level, 2);
    assert_eq!(doc.rows[1].alert_kind, Some(MarkdownAlertKind::Warning));
    assert!(doc.rows[1].starts_alert);

    assert_eq!(doc.rows[2].text.as_ref(), "outer again");
    assert_eq!(doc.rows[2].blockquote_level, 1);
    assert_eq!(doc.rows[2].alert_kind, None);
    assert!(!doc.rows[2].starts_alert);
}

// ── Code block tests ────────────────────────────────────────────────

#[test]
fn fenced_code_block_one_row_per_line() {
    let doc = parse("```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 3);
    assert_eq!(code_rows[0].text.as_ref(), "fn main() {");
    assert_eq!(code_rows[1].text.as_ref(), "    println!(\"hi\");");
    assert_eq!(code_rows[2].text.as_ref(), "}");
    assert_eq!(
        code_rows[0].code_language,
        Some(crate::view::rows::DiffSyntaxLanguage::Rust)
    );
}

#[test]
fn code_block_first_last_flags() {
    let doc = parse("```\na\nb\nc\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 3);
    assert!(matches!(
        code_rows[0].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: false
        }
    ));
    assert!(matches!(
        code_rows[1].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: false,
            is_last: false
        }
    ));
    assert!(matches!(
        code_rows[2].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: false,
            is_last: true
        }
    ));
}

#[test]
fn single_line_code_block_is_both_first_and_last() {
    let doc = parse("```\nonly\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 1);
    assert_eq!(code_rows[0].text.as_ref(), "only");
    assert!(matches!(
        code_rows[0].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: true
        }
    ));
}

#[test]
fn indented_code_block_rows_keep_actual_source_line_ranges() {
    let doc = parse("    old\n    keep\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 2);
    assert_eq!(code_rows[0].text.as_ref(), "old");
    assert_eq!(code_rows[0].source_line_range, 0..1);
    assert_eq!(code_rows[1].text.as_ref(), "keep");
    assert_eq!(code_rows[1].source_line_range, 1..2);
}

#[test]
fn fenced_code_block_preserves_trailing_blank_line() {
    let doc = parse("```\na\n\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 2);
    assert_eq!(code_rows[0].text.as_ref(), "a");
    assert_eq!(code_rows[0].source_line_range, 1..2);
    assert_eq!(code_rows[1].text.as_ref(), "");
    assert_eq!(code_rows[1].source_line_range, 2..3);
    assert!(matches!(
        code_rows[1].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: false,
            is_last: true
        }
    ));
}

#[test]
fn empty_fenced_code_block_produces_single_empty_row() {
    let doc = parse("```\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 1);
    assert_eq!(code_rows[0].text.as_ref(), "");
    assert!(matches!(
        code_rows[0].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: true
        }
    ));
    assert_eq!(code_rows[0].code_language, None);
}

#[test]
fn fenced_code_block_language_aliases_are_resolved() {
    let doc = parse("```language-typescript\nconst x = 1;\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 1);
    assert_eq!(
        code_rows[0].code_language,
        Some(crate::view::rows::DiffSyntaxLanguage::TypeScript)
    );
}

#[test]
fn wide_fenced_code_blocks_set_horizontal_scroll_hints() {
    let long_line = "scroll_hint_token_".repeat(6);
    let doc = parse(&format!("```text\n{long_line}\nshort\n```\n"));
    let code_rows = code_rows(&doc);

    assert_eq!(code_rows.len(), 2);
    assert!(
        code_rows
            .iter()
            .all(|row| row.code_block_horizontal_scroll_hint)
    );
}

// ── Thematic break ──────────────────────────────────────────────────

#[test]
fn thematic_break_produces_row() {
    let doc = parse("---\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::ThematicBreak);
}

// ── Task list ───────────────────────────────────────────────────────

#[test]
fn task_list_markers_are_prepended() {
    let doc = parse("- [x] done\n- [ ] todo\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].text.as_ref(), "[x] done");
    assert_eq!(doc.rows[1].text.as_ref(), "[ ] todo");
}

#[test]
fn footnote_references_and_definitions_are_preserved() {
    let doc = parse("Here is a simple footnote[^1].\n\n[^1]: My reference.\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].text.as_ref(), "Here is a simple footnote[1].");
    let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
    assert_eq!(links.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
        "[1]"
    );

    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[1].text.as_ref(), "My reference.");
    assert_eq!(
        doc.rows[1]
            .footnote_label
            .as_ref()
            .map(SharedString::as_ref),
        Some("1")
    );
    assert_eq!(doc.rows[1].indent_level, 1);
}

#[test]
fn footnote_definition_emits_label_only_for_first_rendered_row() {
    let doc = parse("Reference[^1].\n\n[^1]: First paragraph.\n\n    Second paragraph.\n");
    assert_eq!(doc.rows.len(), 3);

    assert_eq!(doc.rows[1].text.as_ref(), "First paragraph.");
    assert_eq!(
        doc.rows[1]
            .footnote_label
            .as_ref()
            .map(SharedString::as_ref),
        Some("1")
    );
    assert_eq!(doc.rows[1].indent_level, 1);

    assert_eq!(doc.rows[2].text.as_ref(), "Second paragraph.");
    assert_eq!(doc.rows[2].footnote_label, None);
    assert_eq!(doc.rows[2].indent_level, 1);
}

// ── Table ───────────────────────────────────────────────────────────

#[test]
fn table_rows_are_flattened() {
    let doc = parse("| A | B |\n|---|---|\n| 1 | 2 |\n");
    let table_rows: Vec<_> = doc
        .rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    assert!(table_rows.len() >= 2);
    assert!(matches!(
        table_rows[0].kind,
        MarkdownPreviewRowKind::TableRow { is_header: true }
    ));
    assert_eq!(table_rows[0].text.as_ref(), "A | B");
    assert_eq!(table_rows[1].text.as_ref(), "1 | 2");
}

#[test]
fn table_rows_align_columns_across_block() {
    let doc = parse("| Name | Age |\n|---|---|\n| Alexander | 3 |\n| Bo | 27 |\n");
    let table_rows: Vec<_> = doc
        .rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    assert_eq!(table_rows.len(), 3);

    let header_sep = table_rows[0]
        .text
        .find('|')
        .expect("header row should contain a column separator");
    let first_row_sep = table_rows[1]
        .text
        .find('|')
        .expect("body row should contain a column separator");
    let second_row_sep = table_rows[2]
        .text
        .find('|')
        .expect("body row should contain a column separator");

    assert_eq!(header_sep, first_row_sep);
    assert_eq!(first_row_sep, second_row_sep);
}

#[test]
fn table_alignment_without_inline_spans_keeps_rows_plain() {
    let doc = parse("| Name | Age |\n|---|---|\n| Alexander | 3 |\n| Bo | 27 |\n");
    let table_rows: Vec<_> = doc
        .rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();

    assert!(table_rows.iter().all(|row| row.inline_spans.is_empty()));
    assert_eq!(table_rows[0].text.as_ref(), "Name      | Age");
    assert_eq!(table_rows[1].text.as_ref(), "Alexander | 3");
    assert_eq!(table_rows[2].text.as_ref(), "Bo        | 27");
}

#[test]
fn table_alignment_preserves_inline_spans_after_padding_cells() {
    let doc = parse(
        "| A | **Header Bold** |\n| --- | --- |\n| A much longer first column | [link](https://example.com) |\n",
    );
    let table_rows: Vec<_> = doc
        .rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    assert_eq!(table_rows.len(), 2);

    let header_sep = table_rows[0]
        .text
        .find('|')
        .expect("header row should contain a column separator");
    let body_sep = table_rows[1]
        .text
        .find('|')
        .expect("body row should contain a column separator");
    assert_eq!(header_sep, body_sep);

    let header_bold = spans_with_style(table_rows[0], MarkdownInlineStyle::Bold);
    assert_eq!(header_bold.len(), 1);
    assert_eq!(
        &table_rows[0].text.as_ref()[header_bold[0].byte_range.clone()],
        "Header Bold"
    );

    let body_links = spans_with_style(table_rows[1], MarkdownInlineStyle::Link);
    assert_eq!(body_links.len(), 1);
    assert_eq!(
        &table_rows[1].text.as_ref()[body_links[0].byte_range.clone()],
        "link"
    );
}

#[test]
fn table_alignment_handles_inline_spans_in_earlier_cells() {
    let doc =
        parse("| **Header Bold** | B |\n| --- | --- |\n| [link](https://example.com) | plain |\n");
    let table_rows: Vec<_> = doc
        .rows
        .iter()
        .filter(|r| matches!(r.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    assert_eq!(table_rows.len(), 2);

    let header_bold = spans_with_style(table_rows[0], MarkdownInlineStyle::Bold);
    assert_eq!(header_bold.len(), 1);
    assert_eq!(
        &table_rows[0].text.as_ref()[header_bold[0].byte_range.clone()],
        "Header Bold"
    );

    let body_links = spans_with_style(table_rows[1], MarkdownInlineStyle::Link);
    assert_eq!(body_links.len(), 1);
    assert_eq!(
        &table_rows[1].text.as_ref()[body_links[0].byte_range.clone()],
        "link"
    );
}

#[test]
fn row_width_cache_does_not_affect_preview_row_equality() {
    let cached = parse("Paragraph\n").rows.remove(0);
    let fresh = parse("Paragraph\n").rows.remove(0);

    cached.measured_width_px.get_or_init(1, || 123);

    assert_eq!(cached, fresh);
}

#[test]
fn cloned_row_preserves_cached_width_measurement() {
    let cached = parse("Paragraph\n").rows.remove(0);
    cached.measured_width_px.get_or_init(1, || 123);

    let cloned = cached.clone();

    assert_eq!(cloned.measured_width_px.get_or_init(1, || 999), 123);
}

// ── Inline spans ────────────────────────────────────────────────────

#[test]
fn bold_text_produces_bold_span() {
    let doc = parse("Some **bold** text\n");
    assert_eq!(doc.rows.len(), 1);
    let bold = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Bold);
    assert_eq!(bold.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[bold[0].byte_range.clone()],
        "bold"
    );
}

#[test]
fn italic_text_produces_italic_span() {
    let doc = parse("Some *italic* text\n");
    assert_eq!(
        spans_with_style(&doc.rows[0], MarkdownInlineStyle::Italic).len(),
        1
    );
}

#[test]
fn inline_code_produces_code_span() {
    let doc = parse("Use `code` here\n");
    let code = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Code);
    assert_eq!(code.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[code[0].byte_range.clone()],
        "code"
    );
}

#[test]
fn strikethrough_produces_span() {
    let doc = parse("Some ~~struck~~ text\n");
    assert_eq!(
        spans_with_style(&doc.rows[0], MarkdownInlineStyle::Strikethrough).len(),
        1
    );
}

#[test]
fn link_produces_link_span() {
    let doc = parse("[click](http://example.com)\n");
    let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
    assert_eq!(links.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
        "click"
    );
}

#[test]
fn inline_html_links_preserve_text_and_link_span() {
    let doc = parse("Built with <a href=\"https://pages.github.com/\">GitHub Pages</a>.\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].text.as_ref(), "Built with GitHub Pages.");
    let links = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Link);
    assert_eq!(links.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[links[0].byte_range.clone()],
        "GitHub Pages"
    );
}

#[test]
fn bold_italic_produces_bold_italic_span() {
    let doc = parse("***both***\n");
    assert_eq!(
        spans_with_style(&doc.rows[0], MarkdownInlineStyle::BoldItalic).len(),
        1
    );
}

#[test]
fn underline_html_produces_underline_span() {
    let doc = parse("This is an <ins>underlined</ins> text\n");
    assert_eq!(doc.rows[0].text.as_ref(), "This is an underlined text");
    let underline = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Underline);
    assert_eq!(underline.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[underline[0].byte_range.clone()],
        "underlined"
    );
}

#[test]
fn subscript_and_superscript_tags_are_stripped_from_preview_text() {
    let doc = parse("This is a <sub>subscript</sub> and <sup>superscript</sup> text\n");
    assert_eq!(
        doc.rows[0].text.as_ref(),
        "This is a subscript and superscript text"
    );
}

#[test]
fn details_summary_renders_as_structured_preview_rows() {
    let doc = parse(
        "<details open>\n<summary>**Quick start**</summary>\n\nInstall the package.\n</details>\n",
    );

    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
    assert_eq!(doc.rows[0].text.as_ref(), "Quick start");
    let summary_bold = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Bold);
    assert_eq!(summary_bold.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[summary_bold[0].byte_range.clone()],
        "Quick start"
    );

    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[1].text.as_ref(), "Install the package.");
}

#[test]
fn details_summary_on_same_html_line_ignores_wrapper_tags() {
    let doc = parse(
        "<details><summary><strong>Examples</strong> and `usage`</summary>\n\nBody text.\n</details>\n",
    );

    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
    assert_eq!(doc.rows[0].text.as_ref(), "Examples and usage");
    let summary_code = spans_with_style(&doc.rows[0], MarkdownInlineStyle::Code);
    assert_eq!(summary_code.len(), 1);
    assert_eq!(
        &doc.rows[0].text.as_ref()[summary_code[0].byte_range.clone()],
        "usage"
    );
    assert_eq!(doc.rows[1].text.as_ref(), "Body text.");
}

#[test]
fn escaped_markdown_characters_remain_literal() {
    let doc = parse("Let's rename \\*our-new-project\\* to \\*our-old-project\\*.\n");
    assert_eq!(
        doc.rows[0].text.as_ref(),
        "Let's rename *our-new-project* to *our-old-project*."
    );
    assert!(doc.rows[0].inline_spans.is_empty());
}

#[test]
fn excessive_inline_spans_degrade_to_plain_text() {
    // Build a paragraph with more than MAX_INLINE_SPANS_PER_ROW inline
    // code spans so the cap fires and all styling is dropped.
    let mut src = String::new();
    for i in 0..MAX_INLINE_SPANS_PER_ROW + 10 {
        if i > 0 {
            src.push(' ');
        }
        src.push_str(&format!("`s{i}`"));
    }
    src.push('\n');

    let doc = parse(&src);
    assert_eq!(doc.rows.len(), 1);
    assert!(
        doc.rows[0].inline_spans.is_empty(),
        "expected all spans to be dropped when exceeding MAX_INLINE_SPANS_PER_ROW, got {}",
        doc.rows[0].inline_spans.len()
    );
}

#[test]
fn normalize_whitespace_with_spans_handles_multibyte_utf8() {
    // Emoji and accented characters with inline bold around a non-ASCII word.
    let doc = parse("café  **résumé**\nnext\n");
    assert_eq!(doc.rows.len(), 1);
    // Whitespace should be collapsed and span should point at the bold text.
    assert_eq!(doc.rows[0].text.as_ref(), "café résumé next");
    let bold_span = doc.rows[0]
        .inline_spans
        .iter()
        .find(|s| s.style == MarkdownInlineStyle::Bold)
        .expect("expected bold span");
    assert_eq!(
        &doc.rows[0].text.as_ref()[bold_span.byte_range.clone()],
        "résumé"
    );
}

// ── Source line range tests ──────────────────────────────────────────

#[test]
fn source_line_ranges_are_plausible() {
    let doc = parse("# Heading\n\nParagraph\n");
    assert!(!doc.rows[0].source_line_range.is_empty());
    assert!(doc.rows[0].source_line_range.start < 5);
}

// ── Change hint annotation tests ────────────────────────────────────

#[test]
fn change_hints_mark_changed_rows() {
    let old_src = "# Title\n\nOld paragraph\n";
    let new_src = "# Title\n\nNew paragraph\n";
    let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

    // Line 2 (0-based) is changed in both.
    let old_mask = vec![false, false, true];
    let new_mask = vec![false, false, true];
    annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

    // Title row should be unchanged.
    assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_doc.rows[0].change_hint, MarkdownChangeHint::None);

    // Paragraph row should be marked.
    let old_para = old_doc
        .rows
        .iter()
        .find(|r| r.text.as_ref() == "Old paragraph")
        .unwrap();
    assert_eq!(old_para.change_hint, MarkdownChangeHint::Removed);
    let new_para = new_doc
        .rows
        .iter()
        .find(|r| r.text.as_ref() == "New paragraph")
        .unwrap();
    assert_eq!(new_para.change_hint, MarkdownChangeHint::Added);
}

#[test]
fn heading_spacing_rows_do_not_receive_change_hints() {
    let preview =
        build_markdown_diff_preview("# Title\n\nOld paragraph\n", "# Title\n\nNew paragraph\n")
            .unwrap();

    let old_spacer = preview
        .old
        .rows
        .iter()
        .find(|row| matches!(row.kind, MarkdownPreviewRowKind::Spacer))
        .expect("expected heading spacer row on old side");
    let new_spacer = preview
        .new
        .rows
        .iter()
        .find(|row| matches!(row.kind, MarkdownPreviewRowKind::Spacer))
        .expect("expected heading spacer row on new side");

    assert_eq!(old_spacer.change_hint, MarkdownChangeHint::None);
    assert_eq!(new_spacer.change_hint, MarkdownChangeHint::None);
}

#[test]
fn partial_change_ranges_use_modified_hint() {
    let (mut old_doc, mut new_doc) =
        parse_markdown_diff("line one\nline two\n", "line one\nline two\n").unwrap();

    let old_mask = vec![false, true];
    let new_mask = vec![false, true];
    annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

    assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::Modified);
    assert_eq!(new_doc.rows[0].change_hint, MarkdownChangeHint::Modified);
}

#[test]
fn list_item_change_hints_follow_changed_lines() {
    let old_src = "- keep\n- remove me\n";
    let new_src = "- keep\n- add me\n";
    let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

    let old_mask = vec![false, true];
    let new_mask = vec![false, true];
    annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

    assert_eq!(old_doc.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(old_doc.rows[1].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(new_doc.rows[1].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn changed_code_lines_are_marked_individually() {
    let old_src = "```\nold\nkeep\n```\n";
    let new_src = "```\nnew\nkeep\n```\n";
    let (mut old_doc, mut new_doc) = parse_markdown_diff(old_src, new_src).unwrap();

    let old_mask = vec![false, true, false, false];
    let new_mask = vec![false, true, false, false];
    annotate_change_hints(&mut old_doc, &mut new_doc, &old_mask, &new_mask);

    let old_code_rows = code_rows(&old_doc);
    let new_code_rows = code_rows(&new_doc);
    assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
    assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);
}

#[test]
fn changed_indented_code_lines_are_marked_individually() {
    let preview =
        build_markdown_diff_preview("    old\n    keep\n", "    new\n    keep\n").unwrap();

    let old_code_rows = code_rows(&preview.old);
    let new_code_rows = code_rows(&preview.new);

    assert_eq!(old_code_rows[0].source_line_range, 0..1);
    assert_eq!(old_code_rows[1].source_line_range, 1..2);
    assert_eq!(new_code_rows[0].source_line_range, 0..1);
    assert_eq!(new_code_rows[1].source_line_range, 1..2);
    assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
    assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);
}

#[test]
fn changed_trailing_blank_code_line_is_marked_individually() {
    let preview = build_markdown_diff_preview("```\na\n\n```\n", "```\na\nb\n```\n").unwrap();

    let old_code_rows = code_rows(&preview.old);
    let new_code_rows = code_rows(&preview.new);

    assert_eq!(old_code_rows.len(), 2);
    assert_eq!(new_code_rows.len(), 2);
    assert_eq!(old_code_rows[1].text.as_ref(), "");
    assert_eq!(new_code_rows[1].text.as_ref(), "b");
    assert_eq!(old_code_rows[1].source_line_range, 2..3);
    assert_eq!(new_code_rows[1].source_line_range, 2..3);
    assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn build_markdown_diff_preview_applies_change_hints() {
    let preview = build_markdown_diff_preview("- old item\n", "- new item\n").unwrap();

    assert_eq!(preview.old.rows.len(), 1);
    assert_eq!(preview.new.rows.len(), 1);
    assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn diff_preview_inserts_spacer_rows_for_added_markdown_blocks() {
    let preview = build_markdown_diff_preview("- keep\n", "- keep\n- add me\n").unwrap();

    assert_eq!(preview.old.rows.len(), 2);
    assert_eq!(preview.new.rows.len(), 2);
    assert_eq!(preview.old.rows[0].text.as_ref(), "keep");
    assert_eq!(preview.new.rows[0].text.as_ref(), "keep");
    assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(
        preview.new.rows[1].kind,
        MarkdownPreviewRowKind::ListItem { number: None }
    );
    assert_eq!(preview.new.rows[1].text.as_ref(), "add me");
    assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn diff_preview_inserts_spacer_rows_for_removed_markdown_blocks() {
    let preview = build_markdown_diff_preview("keep\n\nremove me\n", "keep\n").unwrap();

    assert_eq!(preview.old.rows.len(), 2);
    assert_eq!(preview.new.rows.len(), 2);
    assert_eq!(preview.old.rows[0].text.as_ref(), "keep");
    assert_eq!(preview.new.rows[0].text.as_ref(), "keep");
    assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(preview.old.rows[1].text.as_ref(), "remove me");
    assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(preview.new.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::None);
}

#[test]
fn diff_preview_builds_inline_document_for_changed_rows() {
    let preview = build_markdown_diff_preview("keep\n\nremove me\n", "keep\n\nadd me\n").unwrap();

    assert_eq!(preview.inline.rows.len(), 3);
    assert_eq!(preview.inline.rows[0].text.as_ref(), "keep");
    assert_eq!(preview.inline.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(preview.inline.rows[1].text.as_ref(), "remove me");
    assert_eq!(
        preview.inline.rows[1].change_hint,
        MarkdownChangeHint::Removed
    );
    assert_eq!(preview.inline.rows[2].text.as_ref(), "add me");
    assert_eq!(
        preview.inline.rows[2].change_hint,
        MarkdownChangeHint::Added
    );
}

#[test]
fn diff_preview_inline_document_merges_unchanged_rows_after_insertions() {
    let preview = build_markdown_diff_preview("- keep\n", "- add\n- keep\n").unwrap();

    assert_eq!(preview.inline.rows.len(), 2);
    assert_eq!(preview.inline.rows[0].text.as_ref(), "add");
    assert_eq!(
        preview.inline.rows[0].change_hint,
        MarkdownChangeHint::Added
    );
    assert_eq!(preview.inline.rows[1].text.as_ref(), "keep");
    assert_eq!(preview.inline.rows[1].change_hint, MarkdownChangeHint::None);
}

#[test]
fn diff_preview_aligns_added_code_lines_with_spacer_rows() {
    let preview = build_markdown_diff_preview("```\nkeep\n```\n", "```\nkeep\nadd\n```\n").unwrap();

    assert_eq!(preview.old.rows.len(), 2);
    assert_eq!(preview.new.rows.len(), 2);
    assert!(matches!(
        preview.old.rows[0].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: true
        }
    ));
    assert!(matches!(
        preview.new.rows[0].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: true,
            is_last: false
        }
    ));
    assert_eq!(preview.old.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert!(matches!(
        preview.new.rows[1].kind,
        MarkdownPreviewRowKind::CodeLine {
            is_first: false,
            is_last: true
        }
    ));
    assert_eq!(preview.new.rows[1].text.as_ref(), "add");
    assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn diff_preview_marks_last_line_change_with_trailing_newline() {
    // The diff engine and mask sizing both use str::lines(), which strips
    // trailing newlines. Verify that a change on the very last line is still
    // detected and annotated correctly regardless of trailing newline.
    let preview =
        build_markdown_diff_preview("# Same\n\nold last\n", "# Same\n\nnew last\n").unwrap();

    let old_last = preview.old.rows.last().unwrap();
    let new_last = preview.new.rows.last().unwrap();
    assert_eq!(old_last.text.as_ref(), "old last");
    assert_eq!(new_last.text.as_ref(), "new last");
    assert_eq!(old_last.change_hint, MarkdownChangeHint::Removed);
    assert_eq!(new_last.change_hint, MarkdownChangeHint::Added);
}

#[test]
fn diff_preview_marks_last_line_change_without_trailing_newline() {
    let preview = build_markdown_diff_preview("# Same\n\nold last", "# Same\n\nnew last").unwrap();

    let old_last = preview.old.rows.last().unwrap();
    let new_last = preview.new.rows.last().unwrap();
    assert_eq!(old_last.text.as_ref(), "old last");
    assert_eq!(new_last.text.as_ref(), "new last");
    assert_eq!(old_last.change_hint, MarkdownChangeHint::Removed);
    assert_eq!(new_last.change_hint, MarkdownChangeHint::Added);
}

#[test]
fn multiline_blockquote_change_hints_follow_changed_quote_lines() {
    let preview =
        build_markdown_diff_preview("> keep\n> remove me\n", "> keep\n> add me\n").unwrap();

    assert_eq!(preview.old.rows.len(), 2);
    assert_eq!(preview.new.rows.len(), 2);
    assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(preview.old.rows[1].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(preview.new.rows[1].change_hint, MarkdownChangeHint::Added);
}

#[test]
fn mixed_markdown_blocks_keep_change_hints_scoped_to_changed_rows() {
    let old_src = concat!(
        "# Title\n",
        "\n",
        "- keep\n",
        "- old item\n",
        "\n",
        "```rust\n",
        "let old_value = 1;\n",
        "let stable = 2;\n",
        "```\n",
        "\n",
        "| Name | Count |\n",
        "| --- | --- |\n",
        "| keep | 1 |\n",
        "| old | 2 |\n",
    );
    let new_src = concat!(
        "# Title\n",
        "\n",
        "- keep\n",
        "- new item\n",
        "\n",
        "```rust\n",
        "let new_value = 1;\n",
        "let stable = 2;\n",
        "```\n",
        "\n",
        "| Name | Count |\n",
        "| --- | --- |\n",
        "| keep | 1 |\n",
        "| new | 3 |\n",
    );

    let preview = build_markdown_diff_preview(old_src, new_src).unwrap();

    assert_eq!(
        preview.old.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
    assert_eq!(preview.old.rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(preview.new.rows[0].change_hint, MarkdownChangeHint::None);

    let old_list_rows: Vec<_> = preview
        .old
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
        .collect();
    let new_list_rows: Vec<_> = preview
        .new
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
        .collect();
    assert_eq!(old_list_rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_list_rows[0].change_hint, MarkdownChangeHint::None);
    assert_ne!(old_list_rows[1].change_hint, MarkdownChangeHint::None);
    assert_ne!(new_list_rows[1].change_hint, MarkdownChangeHint::None);

    let old_code_rows = code_rows(&preview.old);
    let new_code_rows = code_rows(&preview.new);
    assert_eq!(old_code_rows[0].change_hint, MarkdownChangeHint::Removed);
    assert_eq!(old_code_rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_code_rows[0].change_hint, MarkdownChangeHint::Added);
    assert_eq!(new_code_rows[1].change_hint, MarkdownChangeHint::None);

    let old_table_rows: Vec<_> = preview
        .old
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    let new_table_rows: Vec<_> = preview
        .new
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::TableRow { .. }))
        .collect();
    assert_eq!(old_table_rows.len(), 3);
    assert_eq!(new_table_rows.len(), 3);
    assert_eq!(old_table_rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_table_rows[0].change_hint, MarkdownChangeHint::None);
    assert_eq!(old_table_rows[1].change_hint, MarkdownChangeHint::None);
    assert_eq!(new_table_rows[1].change_hint, MarkdownChangeHint::None);
    assert_ne!(old_table_rows[2].change_hint, MarkdownChangeHint::None);
    assert_ne!(new_table_rows[2].change_hint, MarkdownChangeHint::None);
}

// ── plan_changed_line_masks ──────────────────────────────────────────

#[test]
fn plan_changed_line_masks_from_plan_rows() {
    use gitcomet_core::file_diff::{FileDiffPlan, FileDiffPlanRun};

    let plan = FileDiffPlan {
        runs: vec![
            FileDiffPlanRun::Context {
                old_start: 0,
                new_start: 0,
                len: 1,
            },
            FileDiffPlanRun::Remove {
                old_start: 1,
                len: 1,
            },
            FileDiffPlanRun::Add {
                new_start: 1,
                len: 1,
            },
        ],
        row_count: 3,
        inline_row_count: 3,
        eof_newline: None,
    };

    let (old_mask, new_mask) = gitcomet_core::file_diff::plan_changed_line_masks(&plan, 3, 3);
    assert!(!old_mask[0]); // context line
    assert!(old_mask[1]); // removed line
    assert!(!new_mask[0]); // context line
    assert!(new_mask[1]); // added line
}

// ── Limit tests ─────────────────────────────────────────────────────

#[test]
fn parse_returns_none_for_oversized_source() {
    let huge = "x".repeat(MAX_PREVIEW_SOURCE_BYTES + 1);
    assert!(parse_markdown(&huge).is_none());
}

#[test]
fn parse_returns_none_when_rendered_rows_exceed_limit() {
    let too_many_rows = thematic_break_rows(MAX_PREVIEW_ROWS + 1);
    assert!(too_many_rows.len() < MAX_PREVIEW_SOURCE_BYTES);
    assert!(parse_markdown(&too_many_rows).is_none());
}

#[test]
fn parse_diff_returns_none_for_oversized_combined() {
    let big = "x".repeat(MAX_DIFF_PREVIEW_SOURCE_BYTES / 2 + 1);
    assert!(parse_markdown_diff(&big, &big).is_none());
}

#[test]
fn parse_diff_returns_none_when_one_side_exceeds_rendered_row_limit() {
    let too_many_rows = thematic_break_rows(MAX_PREVIEW_ROWS + 1);
    assert!(too_many_rows.len() < MAX_DIFF_PREVIEW_SOURCE_BYTES);
    assert!(parse_markdown_diff(&too_many_rows, "# ok\n").is_none());
}

#[test]
fn parse_diff_allows_single_side_over_single_preview_limit_within_combined_cap() {
    let old = "x".repeat(MAX_PREVIEW_SOURCE_BYTES + 1);
    let new = "y".repeat(MAX_DIFF_PREVIEW_SOURCE_BYTES - old.len());

    assert!(parse_markdown(&old).is_none());

    let (old_doc, new_doc) =
        parse_markdown_diff(&old, &new).expect("combined diff under 2 MiB should parse");
    assert_eq!(old_doc.rows.len(), 1);
    assert_eq!(new_doc.rows.len(), 1);
}

// ── Empty input ─────────────────────────────────────────────────────

#[test]
fn empty_source_produces_empty_document() {
    let doc = parse("");
    assert!(doc.rows.is_empty());
}

// ── Mixed document ──────────────────────────────────────────────────

#[test]
fn mixed_document_produces_correct_row_sequence() {
    let src = "\
# Title

A paragraph with **bold** text.

- item one
- item two

```
code line
```

---
";
    let doc = parse(src);

    // Should have: Heading, Spacer, Paragraph, ListItem, ListItem, CodeLine, ThematicBreak
    assert!(
        doc.rows.len() >= 7,
        "expected at least 7 rows, got {}",
        doc.rows.len()
    );
    assert!(matches!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    ));
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(doc.rows[2].kind, MarkdownPreviewRowKind::Paragraph);
}

// ── Internal helpers ────────────────────────────────────────────────

#[test]
fn build_line_starts_correct() {
    let src = "abc\ndef\nghi";
    let starts = build_line_starts(src);
    assert_eq!(starts, vec![0, 4, 8]);
}

#[test]
fn byte_offset_to_line_maps_correctly() {
    let starts = vec![0, 4, 8];
    assert_eq!(byte_offset_to_line(0, &starts), 0);
    assert_eq!(byte_offset_to_line(3, &starts), 0);
    assert_eq!(byte_offset_to_line(4, &starts), 1);
    assert_eq!(byte_offset_to_line(7, &starts), 1);
    assert_eq!(byte_offset_to_line(8, &starts), 2);
}

#[test]
fn normalize_whitespace_collapses_runs() {
    assert_eq!(normalize_whitespace("a  b\tc\n d"), "a b c d");
    assert_eq!(normalize_whitespace("  leading"), " leading");
    assert_eq!(normalize_whitespace(""), "");
}

#[test]
fn unsupported_html_degrades_cleanly() {
    let doc = parse("<div>block html</div>\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::PlainFallback);
    assert_eq!(doc.rows[0].text.as_ref(), "<div>block html</div>");
}

#[test]
fn inline_html_is_preserved_inside_paragraphs() {
    let doc = parse("Text with <b>html</b> inline\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].text.as_ref(), "Text with <b>html</b> inline");
}

#[test]
fn html_comments_are_hidden_from_preview() {
    let doc = parse("Visible <!-- hidden --> text\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].text.as_ref(), "Visible text");
}

#[test]
fn block_html_comments_do_not_create_rows() {
    let doc = parse("<!-- hidden -->\nVisible\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].text.as_ref(), "Visible");
}

#[test]
fn custom_anchor_tags_are_hidden_from_preview() {
    let doc = parse("# Section Heading\n\n<a name=\"my-custom-anchor-point\"></a>\nVisible\n");
    assert_eq!(doc.rows.len(), 3);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
    assert_eq!(doc.rows[0].text.as_ref(), "Section Heading");
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(doc.rows[2].text.as_ref(), "Visible");
}

#[test]
fn custom_anchor_id_tags_are_hidden_from_preview() {
    let doc = parse("# Section Heading\n\n<a id=\"jump-target\"></a>\nVisible\n");
    assert_eq!(doc.rows.len(), 3);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
    assert_eq!(doc.rows[0].text.as_ref(), "Section Heading");
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Spacer);
    assert_eq!(doc.rows[2].text.as_ref(), "Visible");
}

#[test]
fn markdown_images_preserve_alt_text() {
    // A remote image is still laid out as a block; it just cannot be
    // fetched, so every band keeps the alt text to describe itself.
    let doc = parse("![Octocat smiling](https://example.com/octocat.svg)\n");
    assert_eq!(
        doc.rows.len(),
        usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS)
    );
    assert!(doc.rows.iter().all(|row| row.kind.is_image()));
    assert!(
        doc.rows
            .iter()
            .all(|row| row.text.as_ref() == "Octocat smiling")
    );
    assert_eq!(
        doc.rows[0]
            .image
            .as_ref()
            .map(|image| image.source.as_ref()),
        Some("https://example.com/octocat.svg")
    );
}

#[test]
fn html_img_tags_preserve_alt_text() {
    let doc = parse("<img alt=\"Octocat smiling\" src=\"https://example.com/octocat.svg\" />\n");
    assert_eq!(
        doc.rows.len(),
        usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS)
    );
    assert!(doc.rows.iter().all(|row| row.kind.is_image()));
    assert!(
        doc.rows
            .iter()
            .all(|row| row.text.as_ref() == "Octocat smiling")
    );
}

#[test]
fn html_img_tags_without_a_source_still_fall_back_to_alt_text() {
    let doc = parse("<img alt=\"Octocat smiling\" />\n");
    assert_eq!(doc.rows.len(), 1);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].text.as_ref(), "Octocat smiling");
}

#[test]
fn picture_elements_render_their_nested_img() {
    // A `<picture>` carries themed `<source>` alternatives around a plain
    // `<img>` fallback; the fallback is the one to draw, and its `src` must
    // not be confused with a `<source srcset=…>` beside it.
    let doc = parse(
        "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"dark.svg\" />\n  <img alt=\"Octocat smiling\" src=\"light.svg\" />\n</picture>\n",
    );

    let images = image_rows(&doc);
    assert_eq!(images.len(), usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS));
    assert_eq!(
        images[0].image.as_ref().map(|image| image.source.as_ref()),
        Some("light.svg")
    );
    assert_eq!(images[0].text.as_ref(), "Octocat smiling");
}

// ── Modify-kind mask coverage ────────────────────────────────────────

#[test]
fn plan_changed_line_masks_handles_modify_kind() {
    use gitcomet_core::file_diff::{FileDiffPlan, FileDiffPlanRun};

    let plan = FileDiffPlan {
        runs: vec![FileDiffPlanRun::Modify {
            old_start: 0,
            new_start: 0,
            len: 1,
        }],
        row_count: 1,
        inline_row_count: 2,
        eof_newline: None,
    };

    let (old_mask, new_mask) = gitcomet_core::file_diff::plan_changed_line_masks(&plan, 2, 2);
    assert!(old_mask[0]); // modify marks old side
    assert!(!old_mask[1]);
    assert!(new_mask[0]); // modify marks new side
    assert!(!new_mask[1]);
}

// ── Identical content diff produces no change hints ──────────────────

#[test]
fn identical_content_diff_produces_no_change_hints() {
    let src = "# Title\n\nSame paragraph\n\n- item one\n";
    let preview = build_markdown_diff_preview(src, src).unwrap();

    for row in &preview.old.rows {
        assert_eq!(
            row.change_hint,
            MarkdownChangeHint::None,
            "old row {:?} should be unchanged",
            row.text
        );
    }
    for row in &preview.new.rows {
        assert_eq!(
            row.change_hint,
            MarkdownChangeHint::None,
            "new row {:?} should be unchanged",
            row.text
        );
    }
}

#[test]
fn markdown_diff_scrollbar_markers_show_added_rows_for_one_sided_preview() {
    let preview = build_markdown_diff_preview("", "# New\n").unwrap();

    assert_eq!(
        scrollbar_markers_for_diff_preview(&preview),
        vec![crate::view::components::ScrollbarMarker {
            start: 0.0,
            end: 1.0,
            kind: crate::view::components::ScrollbarMarkerKind::Add,
        }]
    );
}

#[test]
fn markdown_diff_scrollbar_markers_show_removed_rows_for_one_sided_preview() {
    let preview = build_markdown_diff_preview("# Gone\n", "").unwrap();

    assert_eq!(
        scrollbar_markers_for_diff_preview(&preview),
        vec![crate::view::components::ScrollbarMarker {
            start: 0.0,
            end: 1.0,
            kind: crate::view::components::ScrollbarMarkerKind::Remove,
        }]
    );
}

#[test]
fn markdown_diff_scrollbar_markers_merge_replacements_into_modify_markers() {
    let preview = build_markdown_diff_preview("old\n", "new\n").unwrap();

    assert_eq!(
        scrollbar_markers_for_diff_preview(&preview),
        vec![crate::view::components::ScrollbarMarker {
            start: 0.0,
            end: 1.0,
            kind: crate::view::components::ScrollbarMarkerKind::Modify,
        }]
    );
}

#[test]
fn markdown_diff_scrollbar_markers_split_disjoint_change_regions() {
    let preview = build_markdown_diff_preview(
        "- old one\n- keep two\n- keep three\n- keep four\n- old five\n",
        "- new one\n- keep two\n- keep three\n- keep four\n- new five\n",
    )
    .unwrap();

    assert_eq!(
        scrollbar_markers_for_diff_preview(&preview),
        vec![
            crate::view::components::ScrollbarMarker {
                start: 0.0,
                end: 0.2,
                kind: crate::view::components::ScrollbarMarkerKind::Modify,
            },
            crate::view::components::ScrollbarMarker {
                start: 0.8,
                end: 1.0,
                kind: crate::view::components::ScrollbarMarkerKind::Modify,
            },
        ]
    );
}

// ── Code span inside code block is not styled ────────────────────────

#[test]
fn code_block_lines_have_no_inline_spans() {
    let doc = parse("```\n**not bold** `not code`\n```\n");
    let code_rows = code_rows(&doc);
    assert_eq!(code_rows.len(), 1);
    assert!(
        code_rows[0].inline_spans.is_empty(),
        "inline spans inside code blocks should be empty"
    );
}

// ── Deeply nested list preserves indent levels ───────────────────────

#[test]
fn deeply_nested_lists_increment_indent() {
    let doc = parse("- a\n  - b\n    - c\n");
    assert!(doc.rows.len() >= 3);
    assert!(
        doc.rows[0].indent_level < doc.rows[1].indent_level,
        "second level should be more indented"
    );
    assert!(
        doc.rows[1].indent_level < doc.rows[2].indent_level,
        "third level should be more indented"
    );
}

// ── Edge case: line_range_change_hint with empty mask ────────────────

#[test]
fn line_range_change_hint_with_empty_mask_is_none() {
    assert_eq!(
        line_range_change_hint(&(0..3), &[], true),
        MarkdownChangeHint::None
    );
}

#[test]
fn line_range_change_hint_with_empty_range_is_none() {
    assert_eq!(
        line_range_change_hint(&(2..2), &[true, true, true], true),
        MarkdownChangeHint::None
    );
}

// ── source_line_range helper ────────────────────────────────────────

#[test]
fn source_line_range_computes_correct_range() {
    let starts = build_line_starts("abc\ndef\nghi\n");
    // "abc\n" starts at 0 (line 0), "def\n" starts at 4 (line 1),
    // "ghi\n" starts at 8 (line 2)
    assert_eq!(source_line_range(0, 4, &starts), 0..1);
    assert_eq!(source_line_range(0, 8, &starts), 0..2);
    assert_eq!(source_line_range(4, 12, &starts), 1..3);
}

#[test]
fn source_line_range_handles_empty_range() {
    let starts = build_line_starts("abc\n");
    assert_eq!(source_line_range(0, 0, &starts), 0..1);
}

// ── Error message helpers ───────────────────────────────────────────

#[test]
fn single_preview_unavailable_reason_reports_size_for_oversized() {
    let reason = single_preview_unavailable_reason(MAX_PREVIEW_SOURCE_BYTES + 1);
    assert!(
        reason.contains("1 MiB"),
        "should mention size limit: {reason}"
    );
}

#[test]
fn single_preview_unavailable_reason_reports_rows_for_normal_size() {
    let reason = single_preview_unavailable_reason(100);
    assert!(
        reason.contains("row limit"),
        "should mention row limit: {reason}"
    );
}

#[test]
fn diff_preview_unavailable_reason_reports_size_for_oversized() {
    let reason = diff_preview_unavailable_reason(MAX_DIFF_PREVIEW_SOURCE_BYTES + 1);
    assert!(
        reason.contains("2 MiB"),
        "should mention size limit: {reason}"
    );
}

#[test]
fn diff_preview_unavailable_reason_reports_rows_for_normal_size() {
    let reason = diff_preview_unavailable_reason(100);
    assert!(
        reason.contains("row limit"),
        "should mention row limit: {reason}"
    );
}

// ── Word wrap ───────────────────────────────────────────────────────

#[test]
fn wrap_plan_keeps_one_visual_row_per_unwrapped_source_row() {
    let doc = parse("# Title\n\nParagraph one.\n\nParagraph two.\n");
    let plan = build_markdown_preview_wrap_plan(&doc, |_| Vec::new()).expect("plan fits");

    assert_eq!(plan.len(), doc.rows.len());
    for (visual_ix, row) in doc.rows.iter().enumerate() {
        let visual = plan.get(visual_ix).expect("visual row");
        assert_eq!(visual.row_ix, visual_ix);
        assert_eq!(visual.wrap_ix, 0);
        assert_eq!(visual.byte_range, 0..row.text.len());
        assert!(!visual.is_continuation());
    }
}

#[test]
fn wrap_plan_expands_split_rows_and_maps_source_rows_to_their_first_visual_row() {
    let doc = parse("First paragraph.\n\nSecond paragraph.\n");
    // Split every row with text into two halves at a char boundary.
    let plan = build_markdown_preview_wrap_plan(&doc, |row| {
        let len = row.text.len();
        if len < 4 {
            return Vec::new();
        }
        let mut mid = len / 2;
        while mid > 0 && !row.text.is_char_boundary(mid) {
            mid -= 1;
        }
        vec![0..mid, mid..len]
    })
    .expect("plan fits");

    let split_rows = doc.rows.iter().filter(|row| row.text.len() >= 4).count();
    assert_eq!(plan.len(), doc.rows.len() + split_rows);

    for row_ix in 0..doc.rows.len() {
        let visual_ix = plan.visual_ix_for_row(row_ix);
        let visual = plan.get(visual_ix).expect("first visual row");
        assert_eq!(visual.row_ix, row_ix);
        assert_eq!(visual.wrap_ix, 0);
        assert!(!visual.is_continuation());
    }

    let continuations = (0..plan.len())
        .filter_map(|ix| plan.get(ix))
        .filter(|visual| visual.is_continuation())
        .count();
    assert_eq!(continuations, split_rows);
}

#[test]
fn wrap_plan_slices_cover_the_whole_row_text() {
    let doc = parse("A paragraph with several words in it.\n");
    let plan = build_markdown_preview_wrap_plan(&doc, |row| {
        let len = row.text.len();
        if len >= 8 {
            vec![0..4, 4..len]
        } else {
            Vec::new()
        }
    })
    .expect("plan fits");

    let mut covered: Vec<(usize, Range<usize>)> = Vec::new();
    for ix in 0..plan.len() {
        let visual = plan.get(ix).expect("visual row");
        covered.push((visual.row_ix, visual.byte_range.clone()));
    }
    for (row_ix, row) in doc.rows.iter().enumerate() {
        let mut cursor = 0usize;
        for (_, range) in covered.iter().filter(|(ix, _)| *ix == row_ix) {
            assert_eq!(range.start, cursor, "slices must be contiguous");
            cursor = range.end;
        }
        assert_eq!(cursor, row.text.len(), "slices must cover the row text");
    }
}

#[test]
fn wrap_plan_reports_overflow_instead_of_truncating_the_document() {
    // Wrapping every row into many visual rows blows past the cap. The
    // builder must report that rather than hand back a plan whose tail
    // rows are missing, which would make them unreachable in the list.
    let paragraph = "w".repeat(900);
    let source = format!("{paragraph}\n\n").repeat(200);
    let doc = parse(&source);
    // One visual row per byte overshoots MAX_PREVIEW_WRAPPED_ROWS, which
    // a pane only a few pixels wide would do for real.
    let plan = build_markdown_preview_wrap_plan(&doc, |row| {
        let len = row.text.len();
        (0..len).map(|ix| ix..ix + 1).collect()
    });
    assert!(
        plan.is_none(),
        "an oversized wrapped document must fall back to unwrapped rendering"
    );
}

#[test]
fn split_wrap_plans_keep_both_columns_row_aligned() {
    let old = "# Title\n\nlong old paragraph that wraps\n\nshared tail\n";
    let new = "# Title\n\nshort\n\nshared tail\n";
    let preview = build_markdown_diff_preview(old, new).expect("diff preview should build");

    // Wrap only rows longer than 10 bytes, into two halves.
    let (old_plan, new_plan) =
        build_markdown_preview_split_wrap_plans(&preview.old, &preview.new, |row| {
            let len = row.text.len();
            if len <= 10 {
                return Vec::new();
            }
            let mut mid = len / 2;
            while mid > 0 && !row.text.is_char_boundary(mid) {
                mid -= 1;
            }
            vec![0..mid, mid..len]
        })
        .expect("split plans should fit");

    assert_eq!(
        old_plan.len(),
        new_plan.len(),
        "split columns must render the same number of visual rows"
    );
    for visual_ix in 0..old_plan.len() {
        let old_visual = old_plan.get(visual_ix).expect("old visual row");
        let new_visual = new_plan.get(visual_ix).expect("new visual row");
        assert_eq!(
            (old_visual.row_ix, old_visual.wrap_ix),
            (new_visual.row_ix, new_visual.wrap_ix),
            "visual row {visual_ix} must show the same source row on both sides"
        );
    }
}

#[test]
fn split_wrap_plans_pad_the_short_side_with_empty_continuations() {
    // The narrow column has to hold a blank row opposite each extra
    // wrapped row on the wide side, or the two lists drift apart.
    let old = "# Title\n\nlong paragraph on the old side\n";
    let new = "# Title\n\nshort\n";
    let preview = build_markdown_diff_preview(old, new).expect("diff preview should build");

    let (old_plan, new_plan) =
        build_markdown_preview_split_wrap_plans(&preview.old, &preview.new, |row| {
            let len = row.text.len();
            if len <= 10 {
                return Vec::new();
            }
            vec![0..5, 5..len]
        })
        .expect("split plans should fit");

    assert_eq!(old_plan.len(), new_plan.len());
    let padded: Vec<_> = (0..new_plan.len())
        .filter_map(|ix| new_plan.get(ix))
        .filter(|visual| visual.is_continuation())
        .collect();
    assert!(
        !padded.is_empty(),
        "the short column should gain padding rows"
    );
    for visual in padded {
        assert!(
            visual.byte_range.is_empty(),
            "a padding row paints nothing: {visual:?}"
        );
    }
}

#[test]
fn visual_row_text_slice_returns_the_painted_portion() {
    let doc = parse("first second third\n");
    let row = doc
        .rows
        .iter()
        .find(|row| row.kind == MarkdownPreviewRowKind::Paragraph)
        .expect("paragraph row");

    let visual = |wrap_ix, byte_range| MarkdownPreviewVisualRow {
        row_ix: 0,
        wrap_ix,
        byte_range,
    };

    assert_eq!(
        visual(0, 0..row.text.len()).text_slice(row).as_ref(),
        "first second third"
    );
    assert_eq!(visual(1, 6..12).text_slice(row).as_ref(), "second");
    // A padding row and an out-of-range slice both paint nothing.
    assert_eq!(
        visual(2, row.text.len()..row.text.len())
            .text_slice(row)
            .as_ref(),
        ""
    );
    assert_eq!(visual(3, 1..2).text_slice(row).as_ref(), "i");
}

// ── Flowing document blocks ─────────────────────────────────────────

#[test]
fn blocks_group_the_lines_of_one_construct_together() {
    let doc = parse(
        "# Title\n\nA paragraph.\n\n- one\n- two\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\n> quoted\n> lines\n\n| a | b |\n| --- | --- |\n| c | d |\n\n---\n",
    );
    let blocks = markdown_document_blocks(&doc);

    let shapes: Vec<String> = blocks
        .iter()
        .map(|block| match block {
            MarkdownBlock::Heading { level, .. } => format!("h{level}"),
            MarkdownBlock::Paragraph(_) => "p".to_string(),
            MarkdownBlock::List(rows) => format!("list({})", rows.len()),
            MarkdownBlock::Blockquote(rows) => format!("quote({})", rows.len()),
            MarkdownBlock::Code(rows) => format!("code({})", rows.len()),
            MarkdownBlock::Table(rows) => format!("table({})", rows.len()),
            MarkdownBlock::Image(_) => "img".to_string(),
            MarkdownBlock::ThematicBreak(_) => "hr".to_string(),
        })
        .collect();

    // The table is header + body: the `| --- |` line is alignment
    // metadata and never becomes a row of its own.
    assert_eq!(
        shapes,
        vec![
            "h1", "p", "list(2)", "code(2)", "quote(2)", "table(2)", "hr"
        ]
    );
}

#[test]
fn spacer_rows_do_not_become_blocks() {
    // Spacers open a gap in the fixed row grid; the flowing layout uses a
    // margin instead, so carrying them through would double the gap.
    let doc = parse("Intro\n\n# Title\n\nBody\n");
    assert!(
        doc.rows
            .iter()
            .any(|row| row.kind == MarkdownPreviewRowKind::Spacer)
    );

    let blocks = markdown_document_blocks(&doc);
    assert_eq!(blocks.len(), 3, "paragraph, heading, paragraph: {blocks:?}");
}

#[test]
fn two_tables_that_touch_stay_separate() {
    // Folding them together padded both to the widest table's columns and
    // drew them as one grid.
    let doc = parse(
        "| a | b |\n| --- | --- |\n| c | d |\n\n| wiiiiiiiiiide | x |\n| --- | --- |\n| e | f |\n",
    );

    let tables: Vec<Range<usize>> = markdown_document_blocks(&doc)
        .into_iter()
        .filter_map(|block| match block {
            MarkdownBlock::Table(rows) => Some(rows),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 2, "rows: {:?}", row_texts(&doc));

    let width_of = |rows: &Range<usize>| doc.rows[rows.start].text.chars().count();
    assert_ne!(
        width_of(&tables[0]),
        width_of(&tables[1]),
        "each table is padded to its own columns, not the other's: {:?}",
        row_texts(&doc)
    );
}

#[test]
fn two_alerts_that_touch_stay_separate_blocks() {
    // Folding them together labelled the second alert with the first one's
    // kind and drew a single bar down both.
    let doc = parse("> [!NOTE]\n> first\n\n> [!WARNING]\n> second\n");
    let quotes: Vec<Range<usize>> = markdown_document_blocks(&doc)
        .into_iter()
        .filter_map(|block| match block {
            MarkdownBlock::Blockquote(rows) => Some(rows),
            _ => None,
        })
        .collect();

    assert_eq!(
        quotes.len(),
        2,
        "blocks: {:?}",
        markdown_document_blocks(&doc)
    );
    let kinds: Vec<Option<MarkdownAlertKind>> = quotes
        .iter()
        .map(|rows| doc.rows[rows.start].alert_kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            Some(MarkdownAlertKind::Note),
            Some(MarkdownAlertKind::Warning)
        ]
    );
    assert!(
        quotes.iter().all(|rows| doc.rows[rows.start].starts_alert),
        "each block must begin at the row that opens its alert"
    );
}

#[test]
fn an_image_block_collapses_its_bands_into_one() {
    let doc = parse("![shot](a.png)\n");
    assert_eq!(
        doc.rows.len(),
        usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS),
        "the row model still carries one row per band"
    );

    let blocks = markdown_document_blocks(&doc);
    assert_eq!(blocks.len(), 1);
    assert!(matches!(blocks[0], MarkdownBlock::Image(_)));
}

#[test]
fn adjacent_images_share_one_line() {
    // Two pictures written on consecutive lines are one paragraph, so they
    // belong on one line — the shape a row of badges takes.
    let doc = parse("![one](a.png)\n![two](b.png)\n");

    let sources: Vec<&str> = doc
        .rows
        .iter()
        .flat_map(|row| row.inline_images.iter())
        .map(|inline| inline.image.source.as_ref())
        .collect();
    assert_eq!(sources, vec!["a.png", "b.png"]);
    assert!(
        image_rows(&doc).is_empty(),
        "neither picture is alone on its line, so neither becomes a block"
    );
}

#[test]
fn a_list_item_holding_only_a_picture_keeps_it_on_that_item() {
    // A tight list item emits no paragraph, so the item closes with an
    // empty text buffer. Skipping the row there carried the badge onto the
    // next item, or dropped it when the list ended the document.
    let doc = parse("- ![one](a.png)\n- second item\n");

    let items: Vec<(&str, Vec<&str>)> = doc
        .rows
        .iter()
        .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
        .map(|row| {
            (
                row.text.as_ref(),
                row.inline_images
                    .iter()
                    .map(|inline| inline.image.source.as_ref())
                    .collect(),
            )
        })
        .collect();

    assert_eq!(
        items,
        vec![("", vec!["a.png"]), ("second item", vec![])],
        "rows: {:?}",
        row_texts(&doc)
    );
}

#[test]
fn a_picture_in_the_last_list_item_is_not_dropped() {
    let doc = parse("- text item\n- ![only](b.png)\n");

    let sources: Vec<&str> = doc
        .rows
        .iter()
        .flat_map(|row| row.inline_images.iter())
        .map(|inline| inline.image.source.as_ref())
        .collect();
    assert_eq!(sources, vec!["b.png"], "rows: {:?}", row_texts(&doc));
}

#[test]
fn a_picture_before_a_nested_list_stays_with_its_parent_item() {
    let doc = parse("- ![parent](a.png)\n  - nested\n");

    let parent = doc
        .rows
        .iter()
        .find(|row| !row.inline_images.is_empty())
        .expect("the parent item keeps its picture");
    assert_eq!(parent.image, None);
    assert_eq!(parent.indent_level, 1, "rows: {:?}", row_texts(&doc));
    assert!(
        doc.rows
            .iter()
            .any(|row| row.text.as_ref() == "nested" && row.indent_level == 2),
        "the nested item is still its own row: {:?}",
        row_texts(&doc)
    );
}

#[test]
fn a_picture_in_a_table_cell_stays_in_its_column_as_text() {
    // A table row paints as one string whose columns are aligned by
    // padding, so a picture recorded against the row would draw at its
    // leading or trailing edge — out of its column.
    let doc = parse("| ![icon](i.png) | Enabled |\n| --- | --- |\n| b | c |\n");

    let header = doc
        .rows
        .iter()
        .find(|row| {
            matches!(
                row.kind,
                MarkdownPreviewRowKind::TableRow { is_header: true }
            )
        })
        .expect("the header row survives");
    assert!(
        header.text.contains("icon"),
        "the picture's description holds its cell: {:?}",
        header.text
    );
    assert!(
        doc.rows.iter().all(|row| row.inline_images.is_empty()),
        "no picture escapes the table to render beside it"
    );
    assert!(
        image_rows(&doc).is_empty(),
        "and none becomes a block either"
    );
}

#[test]
fn an_html_picture_in_a_table_cell_also_stays_in_its_column() {
    // The `<img>` producer records pictures separately from the markdown
    // one, so it needs the same guard.
    let doc = parse("| <img alt=\"icon\" src=\"i.png\" /> | Enabled |\n| --- | --- |\n| b | c |\n");

    let header = doc
        .rows
        .iter()
        .find(|row| {
            matches!(
                row.kind,
                MarkdownPreviewRowKind::TableRow { is_header: true }
            )
        })
        .expect("the header row survives");
    assert!(
        header.text.contains("icon"),
        "the tag's description holds its cell: {:?}",
        header.text
    );
    assert!(
        doc.rows.iter().all(|row| row.inline_images.is_empty()),
        "no picture escapes the table to render beside it"
    );
}

#[test]
fn source_lines_are_found_for_byte_offsets() {
    // "a\nbb\n\nc" — line starts at 0, 2, 5, 6.
    let line_starts = [0usize, 2, 5, 6];

    assert_eq!(source_line_for_byte(0, &line_starts), 0);
    assert_eq!(source_line_for_byte(1, &line_starts), 0);
    assert_eq!(source_line_for_byte(2, &line_starts), 1);
    assert_eq!(source_line_for_byte(4, &line_starts), 1);
    assert_eq!(source_line_for_byte(5, &line_starts), 2);
    assert_eq!(source_line_for_byte(6, &line_starts), 3);
    // Past the end still resolves to the last line rather than panicking.
    assert_eq!(source_line_for_byte(9_999, &line_starts), 3);
    // And an empty table cannot underflow.
    assert_eq!(source_line_for_byte(3, &[]), 0);
}

#[test]
fn a_picture_alone_on_its_line_becomes_a_block() {
    let doc = parse("![only](a.png)\n");

    assert_eq!(
        image_rows(&doc).len(),
        usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS),
        "rows: {:?}",
        row_texts(&doc)
    );
    assert!(
        doc.rows.iter().all(|row| row.inline_images.is_empty()),
        "a block picture is not also inline"
    );
}

#[test]
fn every_row_that_paints_reaches_a_block() {
    // Nothing but spacers may be dropped, or the flowing preview would
    // silently lose content the diff preview still shows.
    let doc = parse(
        "# T\n\ntext\n\n- a\n\n```\ncode\n```\n\n> q\n\n| x |\n| --- |\n\n![i](a.png)\n\n---\n",
    );
    let painted = doc
        .rows
        .iter()
        .filter(|row| row.kind != MarkdownPreviewRowKind::Spacer)
        .count();
    let covered: usize = markdown_document_blocks(&doc)
        .iter()
        .map(|block| block.row_range().len())
        .sum();

    assert_eq!(
        covered,
        painted,
        "blocks: {:?}",
        markdown_document_blocks(&doc)
    );
}

// ── Images ──────────────────────────────────────────────────────────

fn image_rows(doc: &MarkdownPreviewDocument) -> Vec<&MarkdownPreviewRow> {
    doc.rows.iter().filter(|row| row.kind.is_image()).collect()
}

#[test]
fn an_image_becomes_a_block_of_rows_carrying_its_source_and_alt() {
    let doc = parse("![A screenshot](docs/shot.png)\n");
    let rows = image_rows(&doc);

    assert_eq!(rows.len(), usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS));
    for (expected_ix, row) in rows.iter().enumerate() {
        assert_eq!(
            row.kind,
            MarkdownPreviewRowKind::Image {
                slice_ix: expected_ix as u8,
                slice_count: MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS,
            }
        );
        assert_eq!(
            row.image.as_ref().map(|image| image.source.as_ref()),
            Some("docs/shot.png")
        );
        // The alt stays as row text so copy and the unavailable-image
        // fallback have something to show.
        assert_eq!(row.text.as_ref(), "A screenshot");
    }
}

#[test]
fn a_picture_written_mid_sentence_stays_on_the_sentence_row() {
    let doc = parse("Before ![shot](a.png) after\n");
    let kinds = row_kinds(&doc);

    assert_eq!(kinds.first(), Some(&&MarkdownPreviewRowKind::Paragraph));
    assert_eq!(
        doc.rows[0].text.as_ref(),
        "Before after",
        "the sentence stays one row: {:?}",
        row_texts(&doc)
    );
    assert!(
        image_rows(&doc).is_empty(),
        "a picture sharing its line with text is not a block"
    );

    let inline = &doc.rows[0].inline_images;
    assert_eq!(inline.len(), 1);
    assert_eq!(inline[0].image.source.as_ref(), "a.png");
    assert_eq!(inline[0].alt.as_ref(), "shot");
    assert_eq!(
        inline[0].byte_offset,
        "Before ".len(),
        "the picture records where in the text it was written"
    );
}

#[test]
fn image_alt_text_is_not_also_rendered_as_paragraph_text() {
    let doc = parse("![only alt](a.png)\n");
    assert!(
        !doc.rows
            .iter()
            .any(|row| !row.kind.is_image() && row.text.contains("only alt")),
        "alt text belongs to the image block only: {:?}",
        row_texts(&doc)
    );
}

#[test]
fn a_logo_in_a_heading_shares_the_heading_line() {
    // A logo in a heading, written as HTML because markdown cannot size an
    // image — the shape GitComet's own README uses. Putting it on a line of
    // its own left the heading text stranded underneath.
    let doc = parse(
        "## <img alt=\"GitComet logo\" src=\"assets/gitcomet_logo.svg\" width=\"26\" /> GitComet\n",
    );

    let heading = doc
        .rows
        .iter()
        .find(|row| matches!(row.kind, MarkdownPreviewRowKind::Heading { .. }))
        .expect("the heading survives");
    assert_eq!(heading.text.as_ref(), "GitComet");
    assert_eq!(heading.inline_images.len(), 1);

    let inline = &heading.inline_images[0];
    assert_eq!(inline.image.source.as_ref(), "assets/gitcomet_logo.svg");
    assert_eq!(inline.alt.as_ref(), "GitComet logo");
    assert_eq!(
        inline.byte_offset, 0,
        "the logo is written before the heading text"
    );
    // The tag declares `width="26"`, which is the size it is drawn at.
    assert_eq!(inline.image.width_px, Some(26));
    assert_eq!(inline.image.height_px, None);
    assert!(
        image_rows(&doc).is_empty(),
        "a logo beside a heading is not a block of its own"
    );
}

#[test]
fn a_block_html_image_stands_on_its_own_line() {
    // `<img>` at the top level has no paragraph around it, so it has to
    // close its own row — and being alone there, it is a block.
    let doc = parse("<img alt=\"demo\" src=\"assets/demo.gif\" />\n");

    let images = image_rows(&doc);
    assert_eq!(
        images.len(),
        usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS),
        "rows: {:?}",
        row_texts(&doc)
    );
    assert_eq!(
        images[0].image.as_ref().map(|image| image.source.as_ref()),
        Some("assets/demo.gif")
    );
    assert_eq!(images[0].text.as_ref(), "demo");
}

#[test]
fn image_block_rows_follow_the_declared_size() {
    let sized = |width_px, height_px| {
        MarkdownImage {
            source: "a.png".into(),
            width_px,
            height_px,
        }
        .block_rows()
    };

    // Undeclared falls back to the default block.
    assert_eq!(sized(None, None), MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS);
    // A declared height is authoritative, even against a wide width.
    assert_eq!(sized(Some(400), Some(20)), 1);
    assert_eq!(sized(Some(400), Some(60)), 3);
    // Width alone bounds the height, so a small logo stays small.
    assert_eq!(sized(Some(26), None), 1);
    assert_eq!(sized(Some(28), None), 1);
    assert_eq!(sized(Some(29), None), 2);
    // Anything large is capped at the default block.
    assert_eq!(sized(Some(4000), None), MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS);
    // A zero or unparseable size is treated as undeclared.
    assert_eq!(sized(Some(0), None), MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS);
}

#[test]
fn non_pixel_size_attributes_are_ignored() {
    // A percentage is relative to a container the fixed row grid does not
    // have, so it falls back to the default block rather than guessing.
    let doc = parse("<img alt=\"wide\" src=\"a.png\" width=\"100%\" />\n");
    let images = image_rows(&doc);
    assert_eq!(images.len(), usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS));
    assert_eq!(images[0].image.as_ref().expect("image").width_px, None);

    // An explicit `px` suffix is accepted.
    let doc = parse("<img alt=\"logo\" src=\"a.png\" width=\"26px\" />\n");
    assert_eq!(
        image_rows(&doc)[0].image.as_ref().expect("image").width_px,
        Some(26)
    );
}

#[test]
fn linked_badge_images_keep_both_the_picture_and_the_link() {
    // `[![alt](badge)](target)` — the standard badge shape. The image is a
    // block; the link it sits in is still recorded.
    let doc = parse(
        "[![Build Status](https://github.com/o/r/badge.svg?branch=main)](https://github.com/o/r/actions)\n",
    );

    let images = image_rows(&doc);
    assert_eq!(images.len(), usize::from(MARKDOWN_PREVIEW_IMAGE_BLOCK_ROWS));
    assert_eq!(
        images[0].image.as_ref().map(|image| image.source.as_ref()),
        Some("https://github.com/o/r/badge.svg?branch=main")
    );
    assert_eq!(images[0].text.as_ref(), "Build Status");
}

#[test]
fn a_row_of_badges_stays_on_one_line_and_keeps_its_links() {
    // Badges are written one per source line but form a single paragraph,
    // and each is a picture wrapped in a link.
    let doc = parse(
        "[![One](https://img.shields.io/badge/one.svg)](https://a.example)\n[![Two](https://img.shields.io/badge/two.svg)](https://b.example)\n",
    );

    let badges: Vec<&MarkdownInlineImage> = doc
        .rows
        .iter()
        .flat_map(|row| row.inline_images.iter())
        .collect();

    assert_eq!(
        badges
            .iter()
            .map(|badge| badge.image.source.as_ref())
            .collect::<Vec<_>>(),
        vec![
            "https://img.shields.io/badge/one.svg",
            "https://img.shields.io/badge/two.svg"
        ]
    );
    assert_eq!(
        badges
            .iter()
            .map(|badge| badge.link_url.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("https://a.example"), Some("https://b.example")],
        "clicking a badge has to reach the link it stands for"
    );
    assert_eq!(
        doc.rows
            .iter()
            .filter(|row| !row.inline_images.is_empty())
            .count(),
        1,
        "both badges belong to the same line: {:?}",
        row_texts(&doc)
    );
}

// ── Links ───────────────────────────────────────────────────────────

fn link_spans(row: &MarkdownPreviewRow) -> Vec<(&str, &str)> {
    row.inline_spans
        .iter()
        .filter_map(|span| {
            let url = span.link_url.as_ref()?;
            let text = row.text.get(span.byte_range.clone())?;
            Some((text, url.as_ref()))
        })
        .collect()
}

#[test]
fn inline_links_keep_their_destination() {
    let doc = parse("See [the docs](https://example.com/docs) for details.\n");
    let row = &doc.rows[0];

    assert_eq!(row.text.as_ref(), "See the docs for details.");
    assert_eq!(
        link_spans(row),
        vec![("the docs", "https://example.com/docs")]
    );
}

#[test]
fn autolinks_and_styled_link_text_stay_clickable() {
    // A bold span inside a link resolves to Bold, but it is still part of
    // the link and must carry the destination.
    let doc = parse("<https://example.com/bare> and [**bold**](https://example.com/b)\n");
    let row = &doc.rows[0];

    let spans = link_spans(row);
    assert!(
        spans
            .iter()
            .any(|(text, url)| *text == "https://example.com/bare"
                && *url == "https://example.com/bare"),
        "autolink should be clickable: {spans:?}"
    );
    assert!(
        spans
            .iter()
            .any(|(text, url)| *text == "bold" && *url == "https://example.com/b"),
        "bold link text should be clickable: {spans:?}"
    );
}

#[test]
fn only_web_destinations_are_offered() {
    // Relative paths, anchors, and non-web schemes still render as links
    // but have nothing to open in a browser.
    let doc = parse(
        "[rel](./other.md) [anchor](#section) [mail](mailto:a@b.c) [js](javascript:alert(1)) [ok](https://example.com)\n",
    );
    let row = &doc.rows[0];

    assert_eq!(link_spans(row), vec![("ok", "https://example.com")]);
}

#[test]
fn footnote_references_are_not_web_links() {
    let doc = parse("text[^1]\n\n[^1]: note\n");
    for row in &doc.rows {
        assert!(
            link_spans(row).is_empty(),
            "footnotes point inside the document: {:?}",
            row.text
        );
    }
}

#[test]
fn link_destinations_survive_whitespace_normalisation_and_table_alignment() {
    // Both rewrite row text and remap span offsets; the URL has to ride
    // along or the remapped span becomes unclickable.
    let doc = parse("a   [spaced   link](https://example.com/x)   b\n");
    let paragraph = &doc.rows[0];
    assert_eq!(paragraph.text.as_ref(), "a spaced link b");
    assert_eq!(
        link_spans(paragraph),
        vec![("spaced link", "https://example.com/x")]
    );

    let table = parse("| a | b |\n| --- | --- |\n| [x](https://example.com/y) | wide cell |\n");
    let body = table
        .rows
        .iter()
        .find(|row| row.text.contains('x'))
        .expect("table body row");
    assert_eq!(link_spans(body), vec![("x", "https://example.com/y")]);
}

// ── Inline span integrity ───────────────────────────────────────────

/// Inline spans become `gpui` text runs, and `gpui` shapes a line by
/// splitting the text at each run boundary. A span that lands inside a
/// multi-byte character aborts the process in `str::split_at`, so the
/// parser must never emit one — see [`crate::text_runs`] for the guard on
/// the render side.
fn assert_rows_span_aligned(source: &str, doc: &MarkdownPreviewDocument) {
    for (row_ix, row) in doc.rows.iter().enumerate() {
        let text = row.text.as_ref();
        let mut prev_end = 0usize;
        for span in row.inline_spans.iter() {
            assert!(
                span.byte_range.start <= span.byte_range.end,
                "src {source:?} row {row_ix} text {text:?} span {span:?} inverted"
            );
            assert!(
                span.byte_range.end <= text.len(),
                "src {source:?} row {row_ix} text {text:?} span {span:?} out of bounds"
            );
            assert!(
                text.is_char_boundary(span.byte_range.start),
                "src {source:?} row {row_ix} text {text:?} span {span:?} start not boundary"
            );
            assert!(
                text.is_char_boundary(span.byte_range.end),
                "src {source:?} row {row_ix} text {text:?} span {span:?} end not boundary"
            );
            assert!(
                span.byte_range.start >= prev_end,
                "src {source:?} row {row_ix} text {text:?} span {span:?} overlaps previous"
            );
            prev_end = span.byte_range.end;
        }
    }
}

#[test]
fn inline_spans_stay_on_char_boundaries_for_multibyte_markdown() {
    const FRAGMENTS: &[&str] = &[
        "plain — text",
        "**bold — run**",
        "*em — run*",
        "~~strike —~~",
        "`code — span`",
        "[link — text](https://example.com)",
        "text with é中😀 mix",
        "# Heading — one",
        "## Heading **—** two",
        "- list — item",
        "- [ ] task — item",
        "- [x] done **—** item",
        "1. ordered — item",
        "> quote — line",
        "> [!NOTE]\n> alert — body",
        "| a — | b |\n| --- | --- |\n| **c—** | d |",
        "| ——— | short |\n| --- | --- |\n| x | *y—z* |",
        "```rust\nlet x = \"—\";\n```",
        "---",
        "<b>html — bold</b>",
        "<details><summary>sum — **mary**</summary></details>",
        "<img alt=\"alt — text\" src=\"x.png\">",
        "text[^1] — ref\n\n[^1]: note — body",
        "line one —  \nline two —",
        "a—b   c—d",
        "  —indented — paragraph",
        "—",
        "**—**",
        "*—*text—",
        "| — |\n| --- |\n| **—** |",
    ];

    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    for _ in 0..20_000 {
        let count = 1 + (next() % 5) as usize;
        let mut source = String::new();
        for _ in 0..count {
            let fragment = FRAGMENTS[(next() % FRAGMENTS.len() as u64) as usize];
            source.push_str(fragment);
            source.push_str(if next() % 2 == 0 { "\n\n" } else { "\n" });
        }
        let Some(doc) = parse_markdown(&source) else {
            continue;
        };
        assert_rows_span_aligned(&source, &doc);
    }
}

#[test]
fn inline_spans_stay_on_char_boundaries_for_random_markdown_soup() {
    const ALPHABET: &[&str] = &[
        "—",
        "é",
        "中",
        "😀",
        "…",
        "\u{a0}",
        "a",
        "b",
        "x",
        " ",
        "  ",
        "\t",
        "\n",
        "\n\n",
        "#",
        "##",
        "*",
        "**",
        "_",
        "__",
        "~~",
        "`",
        "```",
        "[",
        "]",
        "(",
        ")",
        "<",
        ">",
        "|",
        "-",
        "- ",
        "1. ",
        "!",
        "\\",
        "&amp;",
        "<b>",
        "</b>",
        "<i>",
        "</i>",
        "<br>",
        "<summary>",
        "</summary>",
        "<details>",
        "</details>",
        "[^1]",
        "[^1]: ",
        "---",
        "> ",
        "[!NOTE]",
        "[ ] ",
        "[x] ",
        ":",
        "\"",
        "'",
        "/",
        "=",
        "img ",
        "alt=",
        "http://x",
    ];

    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    for _ in 0..60_000 {
        let token_count = 4 + (next() % 60) as usize;
        let mut source = String::with_capacity(token_count * 3);
        for _ in 0..token_count {
            source.push_str(ALPHABET[(next() % ALPHABET.len() as u64) as usize]);
        }
        let Some(doc) = parse_markdown(&source) else {
            continue;
        };
        assert_rows_span_aligned(&source, &doc);
    }
}
