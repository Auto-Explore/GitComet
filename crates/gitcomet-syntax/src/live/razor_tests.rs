use super::tests::document_in;
use super::*;

#[derive(Clone, Copy)]
struct KindPalette;

impl SyntaxPalette for KindPalette {
    type Style = SyntaxTokenKind;

    fn style(&self, kind: SyntaxTokenKind) -> Option<Self::Style> {
        Some(kind)
    }
}

fn prepared_document(text: &str) -> PreparedSyntaxDocument {
    let input = treesitter_document_input_from_text(text);
    inject_prepared_document_data(
        prepare_treesitter_document_in_background_text_with_reuse(
            DiffSyntaxLanguage::Razor,
            DiffSyntaxMode::Auto,
            SharedString::from(text.to_owned()),
            input.line_starts,
            None,
            None,
        )
        .unwrap(),
    )
}

#[test]
fn razor_live_and_prepared_match_every_byte_across_rope_chunks_and_nested_layers() {
    let mut text = String::from("\u{feff}@model DemoModel\r\n<div>\r\n");
    for _ in 0..100 {
        text.push_str("<p title=\"prefix @Model.Name suffix\">café @Model?.Title / plain</p>\r\n");
    }
    text.push_str("@if (Model.Ready) { <span>@Model.Name</span> }\r\n<script>const message = `hello @Model.Name`; const n = 1 < 2;</script>\r\n<style>@@media (min-width: 1px) { p { color: @Model.Color; } }</style>\r\n<script type=\"application/json\">{\"name\":\"@Model.Name\"}</script>\r\n</div>");
    assert!(text.len() > gitcomet_text::rope::MAX_CHUNK_BYTES);
    let document = document_in(DiffSyntaxLanguage::Razor, &text, Vec::new());
    let prepared = prepared_document(&text);
    let mut live_bytes = vec![None; text.len()];
    for (range, kind) in document
        .snapshot(KindPalette)
        .highlights_for_byte_range(0..text.len())
    {
        assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        live_bytes[range].fill(Some(kind));
    }
    let mut prepared_bytes = vec![None; text.len()];
    let input = treesitter_document_input_from_text(&text);
    for (line_ix, &start) in input.line_starts.iter().enumerate() {
        for token in syntax_tokens_for_prepared_document_line(prepared, line_ix)
            .unwrap()
            .iter()
        {
            prepared_bytes[start + token.range.start..start + token.range.end]
                .fill(Some(token.kind));
        }
    }
    for byte in 0..text.len() {
        if matches!(text.as_bytes()[byte], b'\r' | b'\n') {
            continue;
        }
        assert_eq!(live_bytes[byte], prepared_bytes[byte], "byte {byte}");
    }
    let html = document
        .injections
        .iter()
        .find(|layer| layer.depth == 1)
        .unwrap();
    let state = prepared_document_tree_state(prepared).unwrap();
    assert_eq!(html.ranges, state.combined_layers().unwrap()[0].ranges);
    for language in [
        DiffSyntaxLanguage::JavaScript,
        DiffSyntaxLanguage::Css,
        DiffSyntaxLanguage::Json,
    ] {
        let spec = tree_sitter_highlight_spec(language).unwrap();
        let layers: Vec<_> = document
            .injections
            .iter()
            .filter(|layer| std::ptr::eq(layer.spec, spec))
            .collect();
        assert!(!layers.is_empty(), "missing {language:?}");
        for layer in layers {
            assert_eq!(layer.depth, 2);
            for range in &layer.ranges {
                assert!(
                    html.ranges
                        .iter()
                        .any(|parent| parent.start <= range.start && range.end <= parent.end)
                );
                for (start, matched) in text.match_indices("@Model") {
                    assert!(
                        range.end <= start || range.start >= start + matched.len(),
                        "{language:?} owns Razor at {start}"
                    );
                }
            }
        }
    }
}

#[test]
fn razor_incremental_delimiter_and_unicode_edits_match_cold_parses() {
    let mut text = String::from(
        "<div title=\"before @Model?.Name after\">\n<script>const value = \"@Model.Name\";</script>\n@if (ok) { <span>@Model.Name</span> }\n</div>",
    );
    let mut document = document_in(DiffSyntaxLanguage::Razor, &text, Vec::new());
    for (needle, replacement) in [
        ("@Model?.Name", "@(Model.Name)"),
        ("after\"", "after"),
        ("after>", "after\">"),
        ("\"@Model.Name\"", "\"Model.Name\""),
        ("<span>", "<span>😀 café "),
        ("@(Model.Name)", "@Model?.Name"),
        ("</script>", "</script"),
        ("</script\n", "</script>\n"),
    ] {
        let start = text.find(needle).unwrap();
        let old = start..start + needle.len();
        text.replace_range(old.clone(), replacement);
        assert_eq!(
            document.sync(
                Rope::from_text(&text),
                Arc::default(),
                Some((old, start..start + replacement.len())),
                None
            ),
            LiveSyntaxSyncOutcome::Reparsed
        );
        let cold = document_in(DiffSyntaxLanguage::Razor, &text, Vec::new());
        assert_eq!(
            document.tree.root_node().to_sexp(),
            cold.tree.root_node().to_sexp(),
            "after {needle:?} -> {replacement:?}"
        );
        assert_eq!(
            document
                .snapshot(KindPalette)
                .highlights_for_byte_range(0..text.len()),
            cold.snapshot(KindPalette)
                .highlights_for_byte_range(0..text.len()),
            "after {needle:?} -> {replacement:?}"
        );
        let ranges = |document: &LiveSyntaxDocument| {
            document
                .injections
                .iter()
                .map(|layer| (layer.depth, layer.ranges.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(ranges(&document), ranges(&cold));
    }
}

#[test]
fn razor_pairs_choose_markup_csharp_and_embedded_javascript_at_their_own_bytes() {
    let text =
        "@{ var n = (1 + 2); }\n<div><span>@n</span></div>\n<script>if (ready) { run(); }</script>";
    let document = document_in(DiffSyntaxLanguage::Razor, text, Vec::new());
    let snapshot = document.snapshot(KindPalette);
    for (needle, open, close, kind) in [
        ("1 + 2", "(", ")", SyntaxPairKind::Bracket),
        ("span", "<span>", "</span>", SyntaxPairKind::Tag),
        ("run();", "{", "}", SyntaxPairKind::Bracket),
    ] {
        let pair = snapshot.syntax_pair_at(text.find(needle).unwrap()).unwrap();
        assert_eq!(pair.kind, kind);
        assert_eq!(&text[pair.open], open);
        assert_eq!(&text[pair.close], close);
    }
}

#[test]
fn razor_expired_nested_discovery_reports_incomplete_and_rebuilds_without_a_deadline() {
    let text = "<script>const label = '@Model.Name';</script><style>p { color: @color; }</style>";
    let document = document_in(DiffSyntaxLanguage::Razor, text, Vec::new());
    let rope = Rope::from_text(text);
    let (expired, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        Some(parse_now()),
        Vec::new(),
        None,
    );
    assert!(expired.is_empty());
    assert!(dropped);
    let (rebuilt, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        None,
        Vec::new(),
        None,
    );
    assert!(!dropped);
    assert_eq!(rebuilt.len(), 3);
    assert_eq!(
        rebuilt.iter().map(|layer| layer.depth).collect::<Vec<_>>(),
        vec![1, 2, 2]
    );
}

#[test]
fn razor_long_unicode_attribute_runs_end_on_character_boundaries() {
    let text = format!(
        "<div title=\"{} @Model.Name\">text</div>",
        "日本語".repeat(4_000)
    );
    assert!(text.len() > TS_MAX_BYTES_TO_QUERY * 2);
    let document = document_in(DiffSyntaxLanguage::Razor, &text, Vec::new());
    let runs = document
        .snapshot(KindPalette)
        .highlights_for_byte_range(0..text.len());
    assert!(!runs.is_empty());
    let mut end = 0;
    for (range, _) in runs {
        assert!(range.start >= end && range.start < range.end);
        assert!(
            text.get(range.clone()).is_some(),
            "{range:?} cuts a UTF-8 character"
        );
        end = range.end;
    }
}
