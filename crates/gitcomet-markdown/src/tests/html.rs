use super::*;

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
fn html_release_notes_render_lists_quotes_and_original_source_lines() {
    let source = "<blockquote>\n<h2>Release notes</h2>\n<ul>\n<li>Fix <code>docs.rs</code> &amp; builds</li>\n<li><p>Second</p><ol start=\"3\"><li>Nested</li></ol></li>\n</ul>\n</blockquote>\n\nAfter\n";
    let doc = parse(source);
    assert_eq!(
        row_texts(&doc),
        vec![
            "Release notes",
            "Fix docs.rs & builds",
            "Second",
            "Nested",
            "After"
        ]
    );
    let first = &doc.rows[1];
    assert_eq!(
        first.kind,
        MarkdownPreviewRowKind::ListItem { number: None }
    );
    assert_eq!(first.indent_level, 1);
    assert_eq!(first.blockquote_level, 1);
    assert_eq!(first.source_line_range, 3..4);
    assert_eq!(
        doc.rows[3].kind,
        MarkdownPreviewRowKind::ListItem { number: Some(3) }
    );
    assert_eq!(doc.rows[3].indent_level, 2);
    assert_eq!(doc.rows[4].blockquote_level, 0);
    assert_eq!(doc.rows[4].indent_level, 0);
}

#[test]
fn html_preformatted_code_preserves_spaces_entities_and_literal_tags() {
    let doc = parse(
        "<blockquote><pre><code class=\"language-rust\">\n  let value = &lt;Type&gt;;\n\n  <unfinished\n</code></pre></blockquote>\n\nAfter\n",
    );
    assert_eq!(
        row_texts(&doc),
        vec!["  let value = <Type>;", "", "  <unfinished", "After"]
    );
    let code = code_rows(&doc);
    assert_eq!(code.len(), 3);
    assert_eq!(code[0].source_line_range, 1..2);
    assert_eq!(code[2].source_line_range, 3..4);
    assert!(code.iter().all(|row| row.blockquote_level == 1));
    assert!(code.iter().all(|row| {
        row.code_language == gitcomet_syntax::diff_syntax_language_for_code_fence_info("rust")
    }));
    assert_eq!(doc.rows[3].blockquote_level, 0);
}

#[test]
fn html_code_entity_newlines_keep_physical_source_lines_and_diff_hints() {
    let source = "<pre><code>one&#10;two\nthree\nfour</code></pre>\n";
    let doc = parse(source);
    assert_eq!(row_texts(&doc), vec!["one", "two", "three", "four"]);
    assert_eq!(
        doc.rows
            .iter()
            .map(|row| row.source_line_range.clone())
            .collect::<Vec<_>>(),
        vec![0..1, 0..1, 1..2, 2..3]
    );
    let changed = source.replace("three", "THREE");
    let diff = build_markdown_diff_preview(source, &changed).expect("diff");
    for document in [&diff.old, &diff.new] {
        assert_eq!(
            row_with_text(document, "two").change_hint,
            MarkdownChangeHint::None
        );
        assert_eq!(
            row_with_text(document, "four").change_hint,
            MarkdownChangeHint::None
        );
    }
    assert_ne!(
        row_with_text(&diff.old, "three").change_hint,
        MarkdownChangeHint::None
    );
    assert_ne!(
        row_with_text(&diff.new, "THREE").change_hint,
        MarkdownChangeHint::None
    );
}

#[test]
fn html_code_language_edits_mark_both_split_sides_and_inline_versions() {
    let old = "<pre><code class=\"language-rust\">\nlet x = 1;\n</code></pre>\n";
    let new = old.replace("language-rust", "language-python");
    let diff = build_markdown_diff_preview(old, &new).expect("diff");
    for document in [&diff.old, &diff.new] {
        let row = row_with_text(document, "let x = 1;");
        assert_ne!(row.change_hint, MarkdownChangeHint::None);
    }
    let rows = diff
        .inline
        .rows
        .iter()
        .filter(|row| row.text == "let x = 1;")
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].code_language, rows[1].code_language);
    assert!(
        rows.iter()
            .all(|row| row.change_hint != MarkdownChangeHint::None)
    );
}

#[test]
fn newline_separated_pre_code_wrappers_keep_code_indentation_language_and_source_lines() {
    for prefix in ["\n", "\r\n  ", " \n\t"] {
        let source = format!(
            "<pre>{prefix}<code class=\"language-rust\">\n  let x = 1;\n\n    x\n</code>\n</pre>\n"
        );
        let doc = parse(&source);
        assert_eq!(row_texts(&doc), vec!["  let x = 1;", "", "    x"]);
        assert_eq!(
            doc.rows
                .iter()
                .map(|row| row.source_line_range.clone())
                .collect::<Vec<_>>(),
            vec![2..3, 3..4, 4..5]
        );
        assert!(
            doc.rows
                .iter()
                .all(|row| row.code_language == Some(gitcomet_syntax::DiffSyntaxLanguage::Rust))
        );
        assert_eq!(
            doc.rows[0].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: false
            }
        );
        assert_eq!(
            doc.rows[2].kind,
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true
            }
        );
    }
}

#[test]
fn details_ancestry_survives_html_blocks_markdown_and_nested_sections() {
    let source = "<details open>\n<summary>Outer</summary>\n\n# Heading\n\n- Markdown item\n\n<details><summary>Inner</summary>\n<p>HTML body</p>\n</details>\n\nOuter body\n\n</details>\n\nAfter\n";
    let doc = parse(source);
    let outer = row_with_text(&doc, "Outer")
        .details
        .as_ref()
        .expect("outer");
    let inner = row_with_text(&doc, "Inner")
        .details
        .as_ref()
        .expect("inner");
    assert_eq!(outer.id, 0);
    assert!(outer.initially_open);
    assert!(outer.parent.is_none());
    assert_eq!(inner.id, source.find("<details>").expect("nested details"));
    assert!(!inner.initially_open);
    assert_eq!(inner.parent.as_deref(), Some(outer.as_ref()));
    for text in ["Heading", "Markdown item", "Outer body"] {
        assert_eq!(row_with_text(&doc, text).details.as_ref(), Some(outer));
    }
    assert_eq!(
        row_with_text(&doc, "HTML body").details.as_ref(),
        Some(inner)
    );
    assert!(row_with_text(&doc, "After").details.is_none());
    assert!(
        doc.rows
            .iter()
            .filter(|row| row.kind == MarkdownPreviewRowKind::Spacer)
            .all(|row| row.details.as_ref() == Some(outer))
    );
}

#[test]
fn dependabot_description_keeps_release_notes_commits_commands_and_links() {
    let source = include_str!("../fixtures/dependabot_release_notes.md");
    let doc = parse(source);
    assert!(
        doc.rows
            .iter()
            .all(|row| row.kind != MarkdownPreviewRowKind::PlainFallback)
    );
    assert_eq!(
        doc.rows
            .iter()
            .filter(|row| row.kind == MarkdownPreviewRowKind::DetailsSummary)
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>(),
        vec![
            "Release notes",
            "Commits",
            "Dependabot commands and options"
        ]
    );
    assert_eq!(
        doc.rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::ListItem { .. }))
            .count(),
        28
    );
    assert_eq!(
        doc.rows
            .iter()
            .filter(|row| matches!(row.kind, MarkdownPreviewRowKind::Heading { level: 2 }))
            .count(),
        11
    );
    let release = row_with_text(&doc, "Release notes")
        .details
        .as_ref()
        .expect("release");
    let commits = row_with_text(&doc, "Commits")
        .details
        .as_ref()
        .expect("commits");
    let commands = row_with_text(&doc, "Dependabot commands and options")
        .details
        .as_ref()
        .expect("commands");
    assert!(!release.initially_open && !commits.initially_open && !commands.initially_open);
    assert!(release.parent.is_none() && commits.parent.is_none() && commands.parent.is_none());
    let fixed = row_with_text(&doc, "Fixes docs.rs build errors.");
    assert_eq!(fixed.details.as_ref(), Some(release));
    assert_eq!(fixed.blockquote_level, 1);
    assert_eq!(spans_with_style(fixed, MarkdownInlineStyle::Code).len(), 1);
    let commit = row_with_text(&doc, "c14836d Cargo: version 0.103.14 -> 0.103.15");
    assert_eq!(commit.details.as_ref(), Some(commits));
    assert_eq!(commit.blockquote_level, 0);
    assert!(commit.inline_spans.iter().any(|span| {
        span.style == MarkdownInlineStyle::Code
            && span.link_url.as_deref()
                == Some("https://github.com/rustls/webpki/commit/c14836d8de33c0ad0ac7dcb28fe9aade20831a4d")
    }));
    let command = row_with_text(
        &doc,
        "@dependabot show <dependency name> ignore conditions will show all of the ignore conditions of the specified dependency",
    );
    assert_eq!(command.details.as_ref(), Some(commands));
    let badge = image_rows(&doc);
    assert_eq!(badge.len(), 1);
    assert!(badge[0].details.is_none());
    assert!(
        doc.rows
            .last()
            .expect("last")
            .inline_spans
            .iter()
            .any(|span| {
                span.link_url.as_deref()
                    == Some("https://github.com/example/repository/network/alerts")
            })
    );
}

#[test]
fn html_wrapper_presentation_changes_mark_unchanged_body_lines_in_both_diff_modes() {
    for (old, new, body) in [
        (
            "<ol start=\"3\">\n<li>First</li>\n<li>Second</li>\n</ol>\n",
            "<ol start=\"7\">\n<li>First</li>\n<li>Second</li>\n</ol>\n",
            "Second",
        ),
        (
            "<div>\nBody\n</div>\n",
            "<blockquote>\nBody\n</blockquote>\n",
            "Body",
        ),
        (
            "<details><summary>Label</summary>\n<p>Body</p>\n</details>\n",
            "<details open><summary>Label</summary>\n<p>Body</p>\n</details>\n",
            "Body",
        ),
    ] {
        let diff = build_markdown_diff_preview(old, new).expect("diff");
        assert_eq!(
            row_with_text(&diff.new, body).change_hint,
            MarkdownChangeHint::Modified,
            "{new}"
        );
        assert_eq!(
            diff.inline
                .rows
                .iter()
                .filter(|row| row.text == body)
                .count(),
            2,
            "{new}"
        );
    }
}

#[test]
fn missing_details_summary_does_not_take_the_body_image() {
    let doc = parse("<details>\n\n![Body picture](body.png)\n\n</details>\n");
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
    assert_eq!(doc.rows[0].text.as_ref(), "Details");
    assert!(doc.rows[0].inline_images.is_empty());
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Image);
    assert_eq!(doc.rows[1].text.as_ref(), "Body picture");
    assert_eq!(doc.rows[0].details, doc.rows[1].details);
}

#[test]
fn html_list_item_containers_end_without_closing_markdown_containers() {
    let doc = parse(
        "> - Markdown</li> text\n>   <ul><li><div align=\"center\">HTML</li></ul>\n>\n>   Continuation\n\nAfter\n",
    );
    let markdown = row_with_text(&doc, "Markdown text");
    assert_eq!(
        markdown.kind,
        MarkdownPreviewRowKind::ListItem { number: None }
    );
    assert_eq!(markdown.blockquote_level, 1);
    let html = row_with_text(&doc, "HTML");
    assert_eq!(html.kind, MarkdownPreviewRowKind::ListItem { number: None });
    assert_eq!(html.indent_level, 2);
    assert_eq!(html.blockquote_level, 1);
    let continuation = row_with_text(&doc, "Continuation");
    assert_eq!(
        continuation.kind,
        MarkdownPreviewRowKind::ListItem { number: None }
    );
    assert!(continuation.continues_item);
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);

    let doc = parse("<ul><li><div align=\"center\">First</li></ul>\n<p>After</p>\n");
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);
}

#[test]
fn moving_details_source_offsets_does_not_mark_unchanged_sections() {
    let old = "<details><summary>Outer</summary>\n<details open><summary>Inner</summary>\n<p>Body</p>\n</details>\n</details>\n";
    let new = format!("Inserted paragraph\n\n{old}");
    let diff = build_markdown_diff_preview(old, &new).expect("diff");
    assert_ne!(
        row_with_text(&diff.old, "Body")
            .details
            .as_ref()
            .expect("old")
            .id,
        row_with_text(&diff.new, "Body")
            .details
            .as_ref()
            .expect("new")
            .id,
    );
    for text in ["Outer", "Inner", "Body"] {
        assert_eq!(
            row_with_text(&diff.new, text).change_hint,
            MarkdownChangeHint::None
        );
        assert_eq!(
            diff.inline
                .rows
                .iter()
                .filter(|row| row.text == text)
                .count(),
            1
        );
    }
}

#[test]
fn unclosed_details_end_with_their_markdown_item_and_keep_formatting_inside() {
    let doc = parse("- <details>\n  <summary>Label</summary>\n  <b>Body\n\nAfter\n");
    assert!(row_with_text(&doc, "Body").details.is_some());
    assert!(!spans_with_style(row_with_text(&doc, "Body"), MarkdownInlineStyle::Bold).is_empty());
    let after = row_with_text(&doc, "After");
    assert!(after.details.is_none());
    assert!(after.inline_spans.is_empty());
    assert_eq!(after.kind, MarkdownPreviewRowKind::Paragraph);
}

#[test]
fn unclosed_details_end_with_html_owners_and_keep_formatting_inside() {
    for source in [
        "<ul>\n<li><details><summary>Label</summary>\n<b>Body\n</li>\n</ul>\n\nAfter\n",
        "<ul><li><details><summary>Label</summary><b>Body</ul>\n\nAfter\n",
        "<blockquote><details><summary>Label</summary><b>Body</blockquote>\n\nAfter\n",
    ] {
        let doc = parse(source);
        let body = row_with_text(&doc, "Body");
        assert!(body.details.is_some(), "{source}");
        assert!(
            !spans_with_style(body, MarkdownInlineStyle::Bold).is_empty(),
            "{source}"
        );
        let after = row_with_text(&doc, "After");
        assert!(after.details.is_none(), "{source}");
        assert!(after.inline_spans.is_empty(), "{source}");
        assert_eq!(after.kind, MarkdownPreviewRowKind::Paragraph, "{source}");
    }

    let doc = parse("<blockquote><details></blockquote>\n\nAfter\n");
    assert_eq!(
        row_with_text(&doc, "Details").kind,
        MarkdownPreviewRowKind::DetailsSummary
    );
    assert!(row_with_text(&doc, "After").details.is_none());

    let doc = parse(
        "<details><summary>Outer</summary><ul><li>Item</li></ul><p>After list</p></details>\n",
    );
    assert_eq!(
        row_with_text(&doc, "Outer").details,
        row_with_text(&doc, "After list").details
    );
}

#[test]
fn details_nesting_fits_the_previews_depth_and_refuses_deeper_input() {
    let allowed = format!(
        "{}Payload\n{}",
        "<details>\n".repeat(255),
        "</details>\n".repeat(255)
    );
    let doc = parse(&allowed);
    assert_eq!(doc.rows.len(), 256);
    assert_eq!(doc.rows.last().expect("body").text.as_ref(), "Payload");
    let too_deep = format!(
        "{}Payload\n{}",
        "<details>\n".repeat(256),
        "</details>\n".repeat(256)
    );
    assert!(parse_markdown(&too_deep).is_none());
}

#[test]
fn unclosed_details_default_summary_covers_images_tables_and_inert_fallback() {
    let doc = parse(
        "<details open=\"false\">\n\n![Body picture](body.png)\n\n<table><tr><td>Cell</td></tr></table>\n\n<section>Literal fallback</section>\n",
    );
    assert_eq!(doc.rows.len(), 4);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::DetailsSummary);
    assert_eq!(doc.rows[0].text.as_ref(), "Details");
    let details = doc.rows[0].details.as_ref().expect("details");
    assert!(details.initially_open);
    assert!(
        doc.rows
            .iter()
            .all(|row| row.details.as_ref() == Some(details))
    );
    assert_eq!(doc.rows[1].kind, MarkdownPreviewRowKind::Image);
    assert_eq!(
        doc.rows[2].kind,
        MarkdownPreviewRowKind::TableRow { is_header: false }
    );
    assert_eq!(doc.rows[3].kind, MarkdownPreviewRowKind::PlainFallback);
    assert_eq!(
        doc.rows[3].text.as_ref(),
        "<section>Literal fallback</section>"
    );
}

fn row_with_text<'a>(doc: &'a MarkdownPreviewDocument, text: &str) -> &'a MarkdownPreviewRow {
    doc.rows
        .iter()
        .find(|row| row.text.as_ref() == text)
        .unwrap_or_else(|| panic!("no row {text:?} in {:?}", row_texts(doc)))
}

fn assert_no_tags_shown(doc: &MarkdownPreviewDocument) {
    assert!(
        doc.rows.iter().all(|row| !row.text.contains('<')
            && !matches!(row.kind, MarkdownPreviewRowKind::PlainFallback)),
        "rows: {:?}",
        doc.rows
            .iter()
            .map(|row| (row.kind, row.text.as_ref()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn p_tags_are_not_shown() {
    let doc = parse("<p>Hello</p>\n");
    assert_eq!(row_texts(&doc), vec!["Hello"]);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].align, MarkdownTextAlign::None);
}

#[test]
fn a_centred_p_over_several_lines_is_one_centred_paragraph() {
    let doc = parse("<p align=\"center\">\n  A fast,\n  native client\n</p>\n");
    assert_no_tags_shown(&doc);
    assert_eq!(row_texts(&doc), vec!["A fast, native client"]);
    assert_eq!(doc.rows[0].kind, MarkdownPreviewRowKind::Paragraph);
    assert_eq!(doc.rows[0].align, MarkdownTextAlign::Center);
    // The row owns the tags around it, so a diff that only touches them
    // still marks it.
    assert_eq!(doc.rows[0].source_line_range, 0..4);
}

#[test]
fn a_centred_logo_is_a_centred_image_block() {
    let doc =
        parse("<p align=\"center\">\n  <img src=\"logo.png\" width=\"120\" alt=\"Logo\">\n</p>\n");
    assert_eq!(doc.rows.len(), 1, "rows: {:?}", row_texts(&doc));
    let images = image_rows(&doc);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].align, MarkdownTextAlign::Center);
    assert_eq!(images[0].text.as_ref(), "Logo");
    assert_eq!(images[0].image.as_ref().expect("image").width_px, Some(120));
}

#[test]
fn centred_badges_on_separate_lines_share_one_row() {
    let doc = parse(
        "<p align=\"center\">\n  <a href=\"https://a.example\"><img src=\"a.svg\" alt=\"A\"></a>&nbsp;\n  <a href=\"https://b.example\"><img src=\"b.svg\" alt=\"B\"></a>\n</p>\n",
    );
    assert_no_tags_shown(&doc);
    assert_eq!(doc.rows.len(), 1, "rows: {:?}", row_texts(&doc));
    let row = &doc.rows[0];
    assert_eq!(row.align, MarkdownTextAlign::Center);
    assert_eq!(row.text.as_ref(), "");
    let links: Vec<_> = row
        .inline_images
        .iter()
        .map(|inline| inline.link_url.as_ref().map(|url| url.as_ref()))
        .collect();
    assert_eq!(
        links,
        vec![Some("https://a.example"), Some("https://b.example")]
    );
}

#[test]
fn links_in_a_block_line_keep_the_text_after_them() {
    let doc = parse(
        "<p align=\"center\">\n  <a href=\"#install\">Install</a> •\n  <a href=\"#usage\">Usage</a>\n</p>\n",
    );
    assert_no_tags_shown(&doc);
    assert_eq!(row_texts(&doc), vec!["Install • Usage"]);
    let row = &doc.rows[0];
    let links: Vec<_> = spans_with_style(row, MarkdownInlineStyle::Link)
        .into_iter()
        .map(|span| {
            (
                &row.text[span.byte_range.clone()],
                span.link_url.as_ref().map(|url| url.as_ref()),
            )
        })
        .collect();
    assert_eq!(
        links,
        vec![("Install", Some("#install")), ("Usage", Some("#usage"))]
    );
}

#[test]
fn html_headings_become_aligned_heading_rows() {
    let doc = parse("<h1 align=\"center\">GitComet</h1>\n\nIntro\n");
    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Spacer,
            &MarkdownPreviewRowKind::Paragraph,
        ]
    );
    assert_eq!(doc.rows[0].text.as_ref(), "GitComet");
    assert_eq!(doc.rows[0].align, MarkdownTextAlign::Center);
    assert_eq!(doc.rows[2].align, MarkdownTextAlign::None);
    assert_eq!(markdown_preview_anchor_row(&doc, "gitcomet"), Some(0));

    let doc = parse("<h3 align=\"right\">\n  Sub\n</h3>\n");
    assert_eq!(row_texts(&doc), vec!["Sub"]);
    assert_eq!(
        doc.rows[0].kind,
        MarkdownPreviewRowKind::Heading { level: 3 }
    );
    assert_eq!(doc.rows[0].align, MarkdownTextAlign::Right);
}

#[test]
fn a_logo_and_a_br_in_an_html_heading_stay_one_heading() {
    let doc = parse(
        "<h1 align=\"center\">\n  <img src=\"logo.svg\" width=\"64\" alt=\"logo\"><br>\n  GitComet\n</h1>\n",
    );
    assert_eq!(doc.rows.len(), 1, "rows: {:?}", row_texts(&doc));
    let row = &doc.rows[0];
    assert_eq!(row.kind, MarkdownPreviewRowKind::Heading { level: 1 });
    assert_eq!(row.text.as_ref(), "GitComet");
    assert_eq!(row.inline_images.len(), 1);
    assert_eq!(row.align, MarkdownTextAlign::Center);
}

#[test]
fn a_centred_div_aligns_the_markdown_inside_it() {
    let doc = parse("<div align=\"center\">\n\n# Title\n\nSome **bold** text\n\n</div>\n\nAfter\n");
    assert_no_tags_shown(&doc);
    assert_eq!(
        row_with_text(&doc, "Title").align,
        MarkdownTextAlign::Center
    );
    let body = row_with_text(&doc, "Some bold text");
    assert_eq!(body.align, MarkdownTextAlign::Center);
    assert_eq!(spans_with_style(body, MarkdownInlineStyle::Bold).len(), 1);
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);
}

#[test]
fn align_values_and_the_center_tag() {
    let align = |source: &str| parse(source).rows[0].align;
    assert_eq!(
        align("<center>Middle</center>\n"),
        MarkdownTextAlign::Center
    );
    assert_eq!(
        align("<p align=\"RIGHT\">r</p>\n"),
        MarkdownTextAlign::Right
    );
    assert_eq!(align("<p align=right>r</p>\n"), MarkdownTextAlign::Right);
    assert_eq!(align("<p align='left'>l</p>\n"), MarkdownTextAlign::Left);
    assert_eq!(
        align("<p align=\"justify\">j</p>\n"),
        MarkdownTextAlign::None
    );

    // The innermost explicit `align` wins, as `text-align` inherits.
    let doc = parse("<div align=\"center\">\n\n<p align=\"left\">x</p>\n\ny\n\n</div>\n");
    assert_eq!(row_with_text(&doc, "x").align, MarkdownTextAlign::Left);
    assert_eq!(row_with_text(&doc, "y").align, MarkdownTextAlign::Center);
}

#[test]
fn stray_and_unclosed_containers() {
    let doc = parse("</p>\n\nText\n\n</div>\n");
    assert_eq!(row_texts(&doc), vec!["Text"]);

    // An unclosed `<div>` runs to the end of the document, as it does on
    // GitHub.
    let doc = parse("<div align=\"center\">\n\nOne\n\nTwo\n");
    assert_eq!(row_texts(&doc), vec!["One", "Two"]);
    assert!(
        doc.rows
            .iter()
            .all(|row| row.align == MarkdownTextAlign::Center)
    );
}

#[test]
fn a_br_splits_a_centred_p_into_centred_rows() {
    let doc = parse("<p align=\"center\">one<br>two</p>\n");
    assert_eq!(row_texts(&doc), vec!["one", "two"]);
    assert!(
        doc.rows
            .iter()
            .all(|row| row.align == MarkdownTextAlign::Center)
    );
}

#[test]
fn a_br_after_a_logo_puts_the_text_under_it() {
    let doc =
        parse("<p align=\"center\">\n  <img src=\"logo.png\" alt=\"logo\"><br>\n  Name\n</p>\n");
    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Image,
            &MarkdownPreviewRowKind::Paragraph
        ]
    );
    assert_eq!(row_texts(&doc), vec!["logo", "Name"]);
    assert!(
        doc.rows
            .iter()
            .all(|row| row.align == MarkdownTextAlign::Center)
    );
}

#[test]
fn entities_in_block_html_text_are_decoded() {
    let doc = parse("<p>Fish &amp; chips &lt;3 &#169; &#xA9; &bogus; x&nbsp;y</p>\n");
    assert_eq!(
        row_texts(&doc),
        vec!["Fish & chips <3 © © &bogus; x\u{a0}y"]
    );
}

#[test]
fn tags_and_comments_split_over_lines_and_crlf() {
    let doc =
        parse("<p align=\"center\">\n  <img src=\"a.png\"\n       width=\"40\" alt=\"A\">\n</p>\n");
    let images = image_rows(&doc);
    assert_eq!(doc.rows.len(), 1, "rows: {:?}", row_texts(&doc));
    assert_eq!(images[0].image.as_ref().expect("image").width_px, Some(40));
    assert_eq!(images[0].align, MarkdownTextAlign::Center);

    let doc = parse("<p align=\"center\">\n<!-- a\ncomment -->\nText\n</p>\n");
    assert_eq!(row_texts(&doc), vec!["Text"]);

    let doc = parse("<p align=\"center\">\r\n  A\r\n  B\r\n</p>\r\n");
    assert_eq!(row_texts(&doc), vec!["A B"]);
    assert_eq!(doc.rows[0].align, MarkdownTextAlign::Center);
}

#[test]
fn an_html_container_does_not_outlive_its_list_item_or_quote() {
    let doc = parse("- item\n  <div align=\"center\">x</div>\n- next\n\nAfter\n");
    assert_no_tags_shown(&doc);
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);

    let doc = parse("> <div align=\"center\">\n>\n> quoted\n\nAfter\n");
    assert_no_tags_shown(&doc);
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);
}

#[test]
fn inline_formatting_tags_become_styles() {
    let doc = parse(
        "<em>e</em> <code>c</code> <del>d</del> <kbd>K</kbd> <strong>s</strong> <i>i</i> <u>u</u>\n",
    );
    let row = &doc.rows[0];
    assert_eq!(row.text.as_ref(), "e c d K s i u");
    let styled: Vec<_> = row
        .inline_spans
        .iter()
        .map(|span| (&row.text[span.byte_range.clone()], span.style))
        .collect();
    assert_eq!(
        styled,
        vec![
            ("e", MarkdownInlineStyle::Italic),
            ("c", MarkdownInlineStyle::Code),
            ("d", MarkdownInlineStyle::Strikethrough),
            ("K", MarkdownInlineStyle::Code),
            ("s", MarkdownInlineStyle::Bold),
            ("i", MarkdownInlineStyle::Italic),
            ("u", MarkdownInlineStyle::Underline),
        ]
    );

    // A tag the preview has no meaning for is still shown as written.
    let doc = parse("a <custom-tag>b</custom-tag>\n");
    assert_eq!(doc.rows[0].text.as_ref(), "a <custom-tag>b</custom-tag>");
}

#[test]
fn formatting_tags_inside_a_centred_p_are_styled() {
    let doc = parse("<p align=\"center\">\n  <b>Bold</b> tagline\n</p>\n");
    assert_no_tags_shown(&doc);
    let row = &doc.rows[0];
    assert_eq!(row.text.as_ref(), "Bold tagline");
    assert_eq!(
        spans_with_style(row, MarkdownInlineStyle::Bold)
            .iter()
            .map(|span| &row.text[span.byte_range.clone()])
            .collect::<Vec<_>>(),
        vec!["Bold"]
    );
}

#[test]
fn a_changed_align_marks_the_row_in_a_diff() {
    let preview =
        build_markdown_diff_preview("<p>x</p>\n", "<p align=\"center\">x</p>\n").expect("parses");
    let new_row = preview
        .new
        .rows
        .iter()
        .find(|row| row.text.as_ref() == "x")
        .expect("new row");
    assert_eq!(new_row.align, MarkdownTextAlign::Center);
    assert_ne!(new_row.change_hint, MarkdownChangeHint::None);
    assert!(
        preview
            .inline
            .rows
            .iter()
            .all(|row| !row.text.contains('<'))
    );
}

#[test]
fn container_tags_inside_a_table_cell_or_heading_do_not_split_it() {
    let doc = parse("| A | <p>B</p><p>C</p> |\n|---|---|\n| c | <div align=\"center\">d</div> |\n");
    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::TableRow { is_header: true },
            &MarkdownPreviewRowKind::TableRow { is_header: false },
        ]
    );
    assert_eq!(row_texts(&doc), vec!["A\tB C", "c\td"]);

    let doc = parse("# Title <center>x</center>\n\n## Sub <h3>y</h3>\n");
    assert_eq!(row_texts(&doc), vec!["Title x", "Sub y"]);
    assert_eq!(
        row_kinds(&doc),
        vec![
            &MarkdownPreviewRowKind::Heading { level: 1 },
            &MarkdownPreviewRowKind::Heading { level: 2 },
        ]
    );
}

#[test]
fn a_container_closed_on_a_verbatim_line_still_closes() {
    // An unsupported element takes the line-by-line path; its last line
    // closes the div among other markup.
    let doc = parse("<div align=\"center\">\n<unknown>x\n</unknown></div>\n\nIntro text\n");
    assert_eq!(
        row_with_text(&doc, "Intro text").align,
        MarkdownTextAlign::None
    );
}

#[test]
fn an_unclosed_html_heading_ends_with_its_list_item() {
    let doc = parse("- Item <h3>Note\n- next\n\nPlain paragraph\n");
    assert_eq!(
        row_with_text(&doc, "Plain paragraph").kind,
        MarkdownPreviewRowKind::Paragraph
    );
}

#[test]
fn a_block_opened_inside_an_html_heading_keeps_the_heading() {
    let doc = parse("<h1 align=\"center\">GitComet<div>tagline</div></h1>\n");
    let heading = row_with_text(&doc, "GitComet");
    assert_eq!(heading.kind, MarkdownPreviewRowKind::Heading { level: 1 });
    assert_eq!(heading.align, MarkdownTextAlign::Center);
    assert!(markdown_preview_anchor_row(&doc, "gitcomet").is_some());
}

#[test]
fn markdown_and_html_formatting_close_their_own_style() {
    let doc = parse("**bold <i>it** rest</i> tail\n");
    let row = &doc.rows[0];
    assert_eq!(row.text.as_ref(), "bold it rest tail");
    let style_of = |word: &str| {
        let at = row.text.find(word).expect("word");
        row.inline_spans
            .iter()
            .find(|span| span.byte_range.contains(&at))
            .map(|span| span.style)
    };
    assert_eq!(style_of("bold"), Some(MarkdownInlineStyle::Bold));
    assert_eq!(style_of("rest"), Some(MarkdownInlineStyle::Italic));
    assert_eq!(style_of("tail"), None);
}

#[test]
fn wide_space_entities_keep_their_width() {
    assert_eq!(
        decode_html_entities("a&ensp;b&emsp;c&thinsp;d"),
        "a\u{2002}b\u{2003}c\u{2009}d"
    );
}

#[test]
fn an_align_only_edit_marks_the_rows_it_moves() {
    let preview = build_markdown_diff_preview(
        "<div align=\"center\">\n\nBody\n\n</div>\n",
        "<div>\n\nBody\n\n</div>\n",
    )
    .expect("parses");
    for side in [&preview.old, &preview.new] {
        assert_ne!(
            row_with_text(side, "Body").change_hint,
            MarkdownChangeHint::None,
            "rows: {:?}",
            side.rows
        );
    }
    let inline: Vec<_> = preview
        .inline
        .rows
        .iter()
        .filter(|row| row.text.as_ref() == "Body")
        .collect();
    assert!(
        inline
            .iter()
            .all(|row| row.change_hint != MarkdownChangeHint::None),
        "both versions are drawn, so both are marked: {inline:?}"
    );
}

// ── HTML containers: second review ──────────────────────────────────

#[test]
fn container_tags_in_comments_scripts_and_attributes_do_not_count() {
    let doc = parse("<table>\n<!--\n<div align=\"center\">\n-->\n</table>\n\nAfter\n");
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);

    let doc = parse("<script>\nvar s = \"<div align='center'>\";\n</script>\n\nAfter\n");
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);

    let doc = parse(
        "<div align=\"center\">\n\n<table><tr><td><img src=\"x.png\" alt=\"</div>\"></td></tr></table>\n\nStill centred\n\n</div>\n",
    );
    assert_eq!(
        row_with_text(&doc, "Still centred").align,
        MarkdownTextAlign::Center
    );
}

#[test]
fn a_container_wrapping_a_verbatim_line_still_aligns_it() {
    let doc = parse(
        "<p align=\"center\"><img src=\"logo.png\" width=\"120\"></p>\n<table><tr><td>x</td></tr></table>\n",
    );
    let images = image_rows(&doc);
    assert_eq!(images.len(), 1, "rows: {:?}", row_texts(&doc));
    assert_eq!(images[0].align, MarkdownTextAlign::Center);
}

#[test]
fn an_inline_html_heading_left_open_in_a_list_item_stays_in_it() {
    let doc = parse("- a <h2>T\n  > quote\n");
    assert_eq!(
        row_with_text(&doc, "T").kind,
        MarkdownPreviewRowKind::Heading { level: 2 }
    );
    assert_eq!(
        row_with_text(&doc, "quote").kind,
        MarkdownPreviewRowKind::BlockquoteLine
    );

    let doc = parse("- a <h2>T\n  - one\n\n  - two\n");
    assert!(matches!(
        row_with_text(&doc, "one").kind,
        MarkdownPreviewRowKind::ListItem { .. }
    ));

    let doc = parse("- Item <h3>Note\n- next\n");
    assert_eq!(
        row_with_text(&doc, "Note").kind,
        MarkdownPreviewRowKind::Heading { level: 3 }
    );
}

#[test]
fn hr_and_summary_do_not_send_a_block_back_to_raw_tags() {
    let doc = parse(
        "<p align=\"center\">\n<img src=\"a.png\">\n</p>\n<h1 align=\"center\">Project</h1>\n<hr>\n",
    );
    assert_no_tags_shown(&doc);
    assert_eq!(
        row_with_text(&doc, "Project").align,
        MarkdownTextAlign::Center
    );
    assert!(
        doc.rows
            .iter()
            .any(|row| row.kind == MarkdownPreviewRowKind::ThematicBreak),
        "rows: {:?}",
        row_texts(&doc)
    );

    let doc = parse("<details>\n<summary>Sum</summary>\n<p align=\"center\">x</p>\n</details>\n");
    assert_no_tags_shown(&doc);
    assert_eq!(
        row_with_text(&doc, "Sum").kind,
        MarkdownPreviewRowKind::DetailsSummary
    );
    assert_eq!(row_with_text(&doc, "x").align, MarkdownTextAlign::Center);
}

#[test]
fn a_link_written_as_its_own_html_block_links_what_it_wraps() {
    let doc = parse(
        "<a href=\"https://example.com\">\n\n![Logo](logo.png)\n\nRead the **docs**\n\n</a>\n\nAfter\n",
    );
    let logo = image_rows(&doc)[0];
    assert_eq!(
        logo.inline_spans
            .iter()
            .find_map(|span| span.link_url.as_ref())
            .map(|url| url.as_ref()),
        Some("https://example.com")
    );
    let docs = row_with_text(&doc, "Read the docs");
    assert!(
        docs.inline_spans
            .iter()
            .any(|span| span.link_url.as_deref() == Some("https://example.com")),
        "{docs:?}"
    );
    assert!(row_with_text(&doc, "After").inline_spans.is_empty());
}

#[test]
fn a_heading_gap_moving_past_an_aligned_heading_does_not_mark_it() {
    let preview = build_markdown_diff_preview(
        "<h1 align=\"center\">GitComet</h1>\n\nIntro\n",
        "<p align=\"center\"><img src=\"logo.png\"></p>\n\n<h1 align=\"center\">GitComet</h1>\n\nIntro\n",
    )
    .expect("parses");
    assert_eq!(
        row_with_text(&preview.new, "GitComet").change_hint,
        MarkdownChangeHint::None
    );
    assert_eq!(
        preview
            .inline
            .rows
            .iter()
            .filter(|row| row.text.as_ref() == "GitComet")
            .count(),
        1
    );
}

#[test]
fn adding_a_br_line_inside_block_html_marks_the_rows_it_splits() {
    let old = "<p align=\"center\">\n  <a href=\"https://a\"><img src=\"a.png\" alt=\"A\"></a>\n  <a href=\"https://b\"><img src=\"b.png\" alt=\"B\"></a>\n</p>\n";
    let new = "<p align=\"center\">\n  <a href=\"https://a\"><img src=\"a.png\" alt=\"A\"></a>\n  <br>\n  <a href=\"https://b\"><img src=\"b.png\" alt=\"B\"></a>\n</p>\n";
    let preview = build_markdown_diff_preview(old, new).expect("parses");
    assert!(
        preview
            .new
            .rows
            .iter()
            .any(|row| row.change_hint != MarkdownChangeHint::None),
        "rows: {:?}",
        preview.new.rows
    );
}

#[test]
fn a_p_or_heading_closes_the_one_left_open_before_it() {
    let doc = parse("<p align=\"center\">A\n<p>B\n");
    assert_eq!(row_with_text(&doc, "B").align, MarkdownTextAlign::None);

    let doc = parse("<h1 align=\"center\">A<h2>B</h2>\n");
    let b = row_with_text(&doc, "B");
    assert_eq!(b.kind, MarkdownPreviewRowKind::Heading { level: 2 });
    assert_eq!(b.align, MarkdownTextAlign::None);
}

#[test]
fn a_p_stays_open_across_raw_html_blocks() {
    let doc = parse(
        "<p align=\"center\">\n  <img src=\"a.png\" width=\"300\">\n\n  <img src=\"b.png\" width=\"300\">\n</p>\n\nAfter\n",
    );
    let images = image_rows(&doc);
    assert_eq!(images.len(), 2, "rows: {:?}", row_texts(&doc));
    assert!(
        images
            .iter()
            .all(|row| row.align == MarkdownTextAlign::Center)
    );
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);
}

#[test]
fn non_breaking_and_wide_spaces_survive_in_paragraphs() {
    let doc = parse("<p>a&emsp;b&nbsp;&nbsp;c</p>\n");
    assert_eq!(row_texts(&doc), vec!["a\u{2003}b\u{a0}\u{a0}c"]);

    let doc = parse("10&nbsp;MB  free\n");
    assert_eq!(row_texts(&doc), vec!["10\u{a0}MB free"]);
}

#[test]
fn an_abruptly_closed_empty_comment_ends_at_once() {
    let doc = parse(
        "<div align=\"center\">\n<!-->\n<img src=\"logo.png\" alt=\"logo\">\n<h1>Title</h1>\n</div>\n\nAfter\n",
    );
    assert_eq!(image_rows(&doc).len(), 1, "rows: {:?}", row_texts(&doc));
    assert_eq!(
        row_with_text(&doc, "Title").kind,
        MarkdownPreviewRowKind::Heading { level: 1 }
    );
    assert_eq!(row_with_text(&doc, "After").align, MarkdownTextAlign::None);
    assert_eq!(parse("<p>a<!--->b</p>\n").rows[0].text.as_ref(), "ab");
}

#[test]
fn numeric_character_references_follow_html() {
    assert_eq!(decode_html_entities("&#+65;"), "&#+65;");
    assert_eq!(decode_html_entities("&#146;"), "\u{2019}");
    assert_eq!(decode_html_entities("&#x80;"), "\u{20ac}");
}

#[test]
fn a_stray_html_close_tag_does_not_end_markdown_formatting() {
    let doc = parse("**a</b> b**\n");
    let row = &doc.rows[0];
    assert_eq!(row.text.as_ref(), "a b");
    let bold: usize = spans_with_style(row, MarkdownInlineStyle::Bold)
        .iter()
        .map(|span| span.byte_range.len())
        .sum();
    assert_eq!(bold, "a b".len(), "{:?}", row.inline_spans);
}

#[test]
fn summary_label_formatting_closes_its_own_style() {
    let (text, spans) = parse_inline_markdown_fragment("**a <i>b** c</i> d");
    assert_eq!(text, "a b c d");
    let style_of = |word: &str| {
        let at = text.find(word).expect("word");
        spans
            .iter()
            .find(|span| span.byte_range.contains(&at))
            .map(|span| span.style)
    };
    assert_eq!(style_of("c"), Some(MarkdownInlineStyle::Italic));
    assert_eq!(style_of("d"), None);
}

#[test]
fn rows_split_by_html_leave_no_edge_spaces() {
    let doc = parse("<h1 align=\"center\">Title<br></h1>\n");
    assert_eq!(row_texts(&doc), vec!["Title"]);

    let doc = parse("Some text <p>x</p> more\n");
    assert_eq!(row_texts(&doc), vec!["Some text", "x", "more"]);
}
