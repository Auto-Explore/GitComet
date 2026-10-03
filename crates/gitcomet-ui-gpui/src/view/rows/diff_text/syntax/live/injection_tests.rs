use super::tests::{document_in, styles_at};
use super::*;

#[derive(Clone, Copy, Default)]
struct ParseControl {
    slots: Option<usize>,
    attempts: usize,
    seeded: usize,
}

thread_local! {
    static PARSE_CONTROL: std::cell::Cell<ParseControl> = const { std::cell::Cell::new(
        ParseControl { slots: None, attempts: 0, seeded: 0 }
    ) };
}

// Deterministically expire the shared budget after N layer attempts. Real
// wall-clock deadlines cannot reliably distinguish a root/first-layer
// success from a later-layer timeout on different test machines.
pub(super) fn take_parse_slot(seeded: bool) -> bool {
    PARSE_CONTROL.with(|control| {
        let mut state = control.get();
        state.attempts += 1;
        state.seeded += usize::from(seeded);
        let allowed = state.slots != Some(0);
        state.slots = state.slots.map(|slots| slots.saturating_sub(1));
        control.set(state);
        allowed
    })
}

fn with_parse_control<R>(slots: Option<usize>, run: impl FnOnce() -> R) -> (R, ParseControl) {
    struct Reset(ParseControl);
    impl Drop for Reset {
        fn drop(&mut self) {
            PARSE_CONTROL.with(|control| control.set(self.0));
        }
    }
    let _reset = Reset(PARSE_CONTROL.with(|control| {
        control.replace(ParseControl {
            slots,
            ..Default::default()
        })
    }));
    let result = run();
    (result, PARSE_CONTROL.with(|control| control.get()))
}

fn with_parse_slots<R>(slots: usize, run: impl FnOnce() -> R) -> R {
    with_parse_control(Some(slots), run).0
}

#[test]
fn newly_typed_markdown_syntax_is_published_while_other_layers_are_deferred() {
    let theme = AppTheme::gitcomet_dark();
    for paragraph in [0, 299] {
        for (opening, closing, probe) in [("`code", "`", "code"), ("**bold*", "*", "bold")] {
            let base = "Words around `existing` text.\n\n";
            let mut text = base.repeat(300);
            let paragraph_start = paragraph * base.len();
            text.replace_range(
                paragraph_start..paragraph_start + base.len(),
                &format!("Words {opening}\n\n"),
            );
            let at = paragraph_start + "Words ".len();
            let mut document = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
            let mut edit_at = at + opening.len();
            for step in 0..12 {
                let typed = if step == 0 { closing } else { "a" };
                text.insert_str(edit_at, typed);
                // Keep an unrelated layer missing to reproduce a document
                // whose background parse has not caught up with typing.
                let unrelated = if paragraph == 0 {
                    document.injections.len() - 1
                } else {
                    0
                };
                document.injections.remove(unrelated);
                with_parse_slots(1, || {
                    document.sync(
                        Rope::from_text(&text),
                        Arc::default(),
                        Some((edit_at..edit_at, edit_at..edit_at + typed.len())),
                        None,
                    );
                });
                assert!(document.background_reparse_request().is_some());
                let fresh = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
                let probe_at = if probe == "bold" {
                    at
                } else {
                    at + opening.find(probe).unwrap()
                };
                let expected = styles_at(
                    &fresh
                        .snapshot(theme)
                        .highlights_for_byte_range(at..at + opening.len() + step + closing.len()),
                    probe_at,
                );
                assert!(
                    expected.is_some(),
                    "fixture creates inline syntax: {opening:?}, edit {step}"
                );
                assert_eq!(
                    styles_at(
                        &document.snapshot(theme).highlights_for_byte_range(
                            at..at + opening.len() + step + closing.len()
                        ),
                        probe_at
                    ),
                    expected,
                    "paragraph {paragraph}, {opening:?}, edit {step}: finished inline syntax must be published before typing stops"
                );
                if probe == "bold" {
                    assert!(
                        document.injections.iter().any(|layer| layer
                            .ranges
                            .iter()
                            .any(|range| range.contains(&at))
                            && layer.tree.root_node().to_sexp().contains("strong_emphasis")),
                        "a completed strong-emphasis parse must also be published"
                    );
                }
                edit_at = if step == 0 {
                    at + opening.find(probe).unwrap() + 1
                } else {
                    edit_at + typed.len()
                };
            }
            let (version, tree, injections) =
                live_syntax_reparse(document.background_reparse_request().unwrap()).unwrap();
            assert!(document.adopt_background_tree(version, tree, injections));
            let fresh = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
            assert_eq!(
                document
                    .snapshot(theme)
                    .highlights_for_byte_range(0..text.len()),
                fresh
                    .snapshot(theme)
                    .highlights_for_byte_range(0..text.len())
            );
        }
    }
}

#[test]
fn insertion_at_an_injection_start_keeps_the_new_text_in_that_layer() {
    for (language, text, insertion) in [
        (DiffSyntaxLanguage::Html, "<script>123;</script>\n", "4"),
        (
            DiffSyntaxLanguage::Markdown,
            "`code` remains here\n",
            "é😀 ",
        ),
        (
            DiffSyntaxLanguage::Jinja,
            "{% if ready %}<script>123;</script>{% endif %}",
            "4",
        ),
    ] {
        let mut document = document_in(language, text, Vec::new());
        let target = document
            .injections
            .iter()
            .max_by_key(|layer| layer.depth)
            .unwrap();
        let start = target.ranges[0].start;
        let depth = target.depth;
        let spec = target.spec;
        let mut edited = text.to_owned();
        edited.insert_str(start, insertion);
        with_parse_slots(0, || {
            document.sync(
                Rope::from_text(&edited),
                Arc::default(),
                Some((start..start, start..start + insertion.len())),
                Some(Duration::ZERO),
            );
        });
        let layer = document
            .injections
            .iter()
            .find(|layer| layer.depth == depth && std::ptr::eq(layer.spec, spec))
            .unwrap();
        assert!(
            layer.ranges.iter().any(|range| range.contains(&start)),
            "{language:?}: insertion at {start} escaped its injection: {:?}",
            layer.ranges
        );
        let (version, tree, injections) =
            live_syntax_reparse(document.background_reparse_request().unwrap()).unwrap();
        assert!(document.adopt_background_tree(version, tree, injections));
        let fresh = document_in(language, &edited, Vec::new());
        assert_eq!(
            document
                .snapshot(AppTheme::gitcomet_dark())
                .highlights_for_byte_range(0..edited.len()),
            fresh
                .snapshot(AppTheme::gitcomet_dark())
                .highlights_for_byte_range(0..edited.len())
        );
    }
}

fn finish_reparse_and_compare_with_fresh(document: &mut LiveSyntaxDocument, text: &str) {
    let (version, tree, injections) =
        live_syntax_reparse(document.background_reparse_request().unwrap()).unwrap();
    assert!(document.adopt_background_tree(version, tree, injections));
    assert!(document.background_reparse_request().is_none());
    let fresh = document_in(document.language(), text, document.mask.to_vec());
    let theme = AppTheme::gitcomet_dark();
    assert_eq!(
        document
            .snapshot(theme)
            .highlights_for_byte_range(0..text.len()),
        fresh
            .snapshot(theme)
            .highlights_for_byte_range(0..text.len()),
    );
}

#[test]
fn deferred_paragraph_splits_and_joins_preserve_both_code_spans() {
    let theme = AppTheme::gitcomet_dark();
    for (text, replacement) in [
        ("Words `one` and more `two` text.\n\n", "\n\n"),
        ("Words `one`\n\n and more `two` text.\n\n", " "),
    ] {
        let mut document = document_in(DiffSyntaxLanguage::Markdown, text, Vec::new());
        let original = document
            .snapshot(theme)
            .highlights_for_byte_range(0..text.len());
        let at = text.find(" and").unwrap();
        let old = if replacement == " " {
            at - 2..at
        } else {
            at..at
        };
        let mut edited = text.to_owned();
        edited.replace_range(old.clone(), replacement);
        with_parse_slots(0, || {
            document.sync(
                Rope::from_text(&edited),
                Arc::default(),
                Some((old.clone(), old.start..old.start + replacement.len())),
                None,
            );
        });
        let highlights = document
            .snapshot(theme)
            .highlights_for_byte_range(0..edited.len());
        for probe in ["one", "two"] {
            let expected = styles_at(&original, text.find(probe).unwrap());
            assert!(expected.is_some());
            assert_eq!(
                styles_at(&highlights, edited.find(probe).unwrap()),
                expected,
                "{replacement:?}: {probe} lost its color at a changed paragraph boundary"
            );
        }
        finish_reparse_and_compare_with_fresh(&mut document, &edited);
    }
}

#[test]
fn deferred_injection_captures_do_not_color_a_new_host_boundary() {
    let text = "<script>/* comment */ const value = 1;</script><p>host</p>";
    let mut document = html_document(text);
    let at = text.find("comment").unwrap();
    let inserted = "</script><p>";
    let mut edited = text.to_owned();
    edited.insert_str(at, inserted);
    with_parse_slots(0, || {
        document.sync(
            Rope::from_text(&edited),
            Arc::default(),
            Some((at..at, at..at + inserted.len())),
            None,
        );
    });
    let theme = AppTheme::gitcomet_dark();
    let fresh = html_document(&edited);
    let actual = document
        .snapshot(theme)
        .highlights_for_byte_range(at..edited.len());
    let expected = fresh
        .snapshot(theme)
        .highlights_for_byte_range(at..edited.len());
    assert_eq!(
        actual, expected,
        "the old JavaScript comment must stop at the new HTML boundary"
    );
    finish_reparse_and_compare_with_fresh(&mut document, &edited);
}

#[test]
fn background_reparse_seeds_every_existing_injection() {
    let mut text = "Words `existing` text.\n\n".repeat(300);
    let mut document = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
    let at = text.rfind("existing").unwrap() + 2;
    text.insert(at, 'a');
    document.sync(
        Rope::from_text(&text),
        Arc::default(),
        Some((at..at, at..at + 1)),
        Some(Duration::ZERO),
    );
    let request = document.background_reparse_request().unwrap();
    let ((version, tree, injections), parsed) =
        with_parse_control(None, || live_syntax_reparse(request).unwrap());
    assert_eq!(
        parsed.attempts,
        document.injections.len(),
        "the background pass must visit every paragraph"
    );
    assert_eq!(
        parsed.seeded, parsed.attempts,
        "every existing layer should seed its background parse"
    );
    assert!(document.adopt_background_tree(version, tree, injections));
    let fresh = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
    let theme = AppTheme::gitcomet_dark();
    assert_eq!(
        document
            .snapshot(theme)
            .highlights_for_byte_range(0..text.len()),
        fresh
            .snapshot(theme)
            .highlights_for_byte_range(0..text.len())
    );
}

fn html_document(text: &str) -> LiveSyntaxDocument {
    LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Html,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("html live document should build")
}

/// Styles covering `needle`, if any.
fn styles_for<'a>(
    highlights: &'a [(Range<usize>, gpui::HighlightStyle)],
    text: &str,
    needle: &str,
) -> Vec<&'a gpui::HighlightStyle> {
    let at = text
        .find(needle)
        .expect("fixture should contain the needle");
    let span = at..at + needle.len();
    highlights
        .iter()
        .filter(|(range, _)| range.start < span.end && range.end > span.start)
        .map(|(_, style)| style)
        .collect()
}

fn fsharp_document(text: &str) -> LiveSyntaxDocument {
    LiveSyntaxDocument::new(
        DiffSyntaxLanguage::FSharp,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("fsharp live document should build")
}

/// F# XML doc comments are the tree's only `(#set! injection.combined)` rule.
/// `xml_doc` is a per-line token, so without combined support a three-line
/// doc comment produced three layers instead of one.
#[test]
fn combined_injection_produces_one_layer_covering_every_range() {
    let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
    let document = fsharp_document(text);

    assert_eq!(
        document.injections.len(),
        1,
        "three xml_doc lines belong to one combined layer, got {} layers",
        document.injections.len()
    );
    let layer = &document.injections[0];
    assert_eq!(
        layer.ranges.len(),
        3,
        "the combined layer should carry one range per xml_doc line: {:?}",
        layer.ranges
    );
    assert_eq!(
        layer.hull(),
        layer.ranges[0].start..layer.ranges[2].end,
        "`range` is the hull of `ranges`"
    );
    assert!(
        layer.hull().end <= text.find("let add").expect("let binding"),
        "the layer must not reach the F# code below it: {:?}",
        layer.hull()
    );
}

/// A node straddling two included ranges reports a byte range covering the
/// host bytes between them, so its captures have to be split. Here the XML
/// element spans all three `///` lines, and the `/// ` prefixes are F#.
#[test]
fn combined_layer_captures_are_clipped_to_its_ranges() {
    let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
    let document = fsharp_document(text);
    let layer = &document.injections[0];
    let ranges = layer.ranges.clone();

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());

    let gap_start = ranges[0].end;
    let gap_end = ranges[1].start;
    assert!(
        gap_start < gap_end,
        "fixture should have a real gap between the first two ranges"
    );
    for (range, _) in &highlights {
        assert!(
            range.start >= gap_end || range.end <= gap_start,
            "highlight {range:?} crosses into the host-grammar gap \
                 {gap_start}..{gap_end} between two combined ranges"
        );
    }
}

/// A combined layer may answer for a caret on its own boundary, but neither
/// end of the pair it returns may straddle a host-grammar gap.
///
/// The gaps here are one byte wide -- the newline between two `///` lines --
/// so a caret "in the gap" is always also on some range's edge, which the
/// membership check treats as inside the layer on purpose: a caret directly
/// after an injected region's last character still belongs to it. What must
/// never happen is a delimiter range that covers bytes the layer does not
/// own, because those bytes are host text the layer never parsed.
#[test]
fn syntax_pair_at_never_straddles_a_combined_layer_gap() {
    let text = "/// <summary>\n/// Adds.\n/// </summary>\nlet add x y = x + y\n";
    let document = fsharp_document(text);
    let ranges = document.injections[0].ranges.clone();
    let gap = ranges[0].end..ranges[1].start;
    assert!(gap.start < gap.end, "fixture should have a real gap");
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());

    let pair = snapshot
        .syntax_pair_at(gap.start)
        .expect("the caret sits just past `<summary>`, which pairs with `</summary>`");
    for span in [&pair.open, &pair.close] {
        assert!(
            ranges
                .iter()
                .any(|range| range.start <= span.start && span.end <= range.end),
            "delimiter {span:?} is not contained in any injected range {ranges:?}"
        );
    }

    // And a caret out in the host grammar is never answered from the layer.
    let host = text.find("let add").expect("host code");
    if let Some(pair) = snapshot.syntax_pair_at(host) {
        let in_layer = |offset: usize| {
            ranges
                .iter()
                .any(|range| range.start <= offset && offset <= range.end)
        };
        assert!(
            !(in_layer(pair.open.start) && in_layer(pair.close.start)),
            "a caret in host code was answered by the combined XML layer"
        );
    }
}

/// A caret strictly inside a wide gap belongs to the host grammar, and the
/// combined tree -- which has no nodes there -- must not answer for it.
///
/// The `///` fixture's gaps are one byte wide, so every offset in them is
/// also on some range's boundary, which membership counts as inside the
/// layer on purpose (a caret directly after an injected region's last
/// character still belongs to it). Interposing a plain `//` comment makes a
/// ten-byte gap with a real interior, which is the case this guards.
#[test]
fn syntax_pair_at_ignores_a_wide_combined_layer_gap() {
    let text = "/// <summary>\n// plain\n/// </summary>\nlet add x y = x + y\n";
    let document = fsharp_document(text);
    let ranges = document.injections[0].ranges.clone();
    let gap = ranges[0].end..ranges[1].start;
    assert!(
        gap.end - gap.start > 2,
        "fixture must have a gap with a strict interior, got {gap:?}"
    );
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());

    let inside = gap.start + (gap.end - gap.start) / 2;
    assert!(inside > gap.start && inside < gap.end);
    if let Some(pair) = snapshot.syntax_pair_at(inside) {
        let in_layer = |offset: usize| {
            ranges
                .iter()
                .any(|range| range.start <= offset && offset <= range.end)
        };
        assert!(
            !(in_layer(pair.open.start) && in_layer(pair.close.start)),
            "a caret at {inside}, strictly inside the host-grammar gap {gap:?}, \
                 was answered by the combined XML layer: {:?}/{:?}",
            pair.open,
            pair.close
        );
    }
}

/// Clipping must preserve an ordinary, fully parsed single-range layer.
#[test]
fn single_range_layers_are_unaffected_by_the_clip_parameter() {
    let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
    let document = html_document(text);
    let layer = &document.injections[0];
    assert_eq!(layer.ranges.len(), 1, "an ordinary injection has one range");
    assert_eq!(layer.ranges[0], layer.hull());

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());
    assert!(
        !styles_for(&highlights, text, "const").is_empty(),
        "clipping must not have eaten the injected JavaScript keyword"
    );
}

#[test]
fn script_bodies_are_highlighted_as_javascript() {
    let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
    let document = html_document(text);
    assert_eq!(
        document.injections.len(),
        1,
        "the script body should produce exactly one injected layer"
    );

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());

    // `const` is a JavaScript keyword. The HTML grammar sees the whole
    // script body as one `raw_text` node and has no keyword concept, so a
    // keyword-coloured run here can only come from the injected layer.
    let keyword = styles_for(&highlights, text, "const");
    assert!(
        !keyword.is_empty(),
        "expected the injected JavaScript layer to highlight `const`: {highlights:?}"
    );

    // And the enclosing HTML is still highlighted by the root layer.
    assert!(
        !styles_for(&highlights, text, "script").is_empty(),
        "the host grammar must keep highlighting its own tags"
    );
}

#[test]
fn injected_layers_cover_only_their_own_span() {
    let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
    let document = html_document(text);
    let layer = document
        .injections
        .first()
        .expect("expected one injected layer");

    let body_start = text.find("\nconst").expect("script body") + 1;
    assert!(
        layer.hull().start <= body_start && layer.hull().end >= body_start + "const".len(),
        "layer {:?} should cover the script body at {body_start}",
        layer.hull()
    );
    assert!(
        layer.hull().start > text.find("<script>").expect("open tag"),
        "the layer must not swallow the opening tag"
    );
    assert!(
        layer.hull().end <= text.find("</script>").expect("close tag"),
        "the layer must not swallow the closing tag"
    );
}

/// The layer's tree is parsed with `included_ranges`, so its node offsets
/// are already document coordinates. If that ever regressed to
/// injection-local offsets, highlights would land near the top of the file.
#[test]
fn injected_capture_offsets_are_document_coordinates() {
    let prefix = "<html>\n<body>\n<p>filler</p>\n".repeat(20);
    let text = format!("{prefix}<script>\nconst answer = 42;\n</script>\n</html>\n");
    let document = html_document(&text);
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());

    let keyword_at = text.find("const").expect("keyword");
    assert!(
        keyword_at > 200,
        "fixture should place the injection well into the document"
    );
    let covering = styles_for(&highlights, &text, "const");
    assert!(
        !covering.is_empty(),
        "the injected keyword should be highlighted at its real offset {keyword_at}"
    );
}

fn jinja_document(text: &str) -> LiveSyntaxDocument {
    LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Jinja,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("jinja live document should build")
}

/// The template's HTML is depth 1, so its script bodies are depth 2 -- which
/// the diff panes have always coloured and the editor used to leave bare.
#[test]
fn script_bodies_inside_a_template_are_highlighted_as_javascript() {
    let text = "{% block body %}\n<script>\nconst answer = 42;\n</script>\n{% endblock %}\n";
    let document = jinja_document(text);
    assert!(
        document.injections.iter().any(|layer| layer.depth == 2),
        "the script body should be a depth-2 layer under the HTML layer"
    );
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());
    assert!(
        !styles_for(&highlights, text, "const").is_empty(),
        "`const` inside a template's <script> should carry the JavaScript style"
    );
}

/// An ld+json body that spans a `{% if %}` is one `raw_text` to HTML, but the
/// nested JSON layer must only see the bytes the HTML layer itself owns.
#[test]
fn nested_layer_ranges_exclude_the_template_gaps() {
    let text = "<script type=\"application/ld+json\">\n{\"a\": 1{% if x %}, \"b\": 2{% endif %}}\n</script>\n";
    let document = jinja_document(text);
    let gap = text.find("{% if x %}").expect("gap");
    let gap = gap..gap + "{% if x %}".len();
    let json = document
        .injections
        .iter()
        .find(|layer| layer.depth == 2)
        .expect("the JSON body should be a depth-2 layer");
    assert!(
        json.ranges.len() >= 2,
        "the JSON layer must be split around the template tag: {:?}",
        json.ranges
    );
    assert!(
        json.ranges
            .iter()
            .all(|range| range.end <= gap.start || range.start >= gap.end),
        "no JSON range may cover the `{{% if %}}` bytes: {:?}",
        json.ranges
    );
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());
    let palette = syntax_highlight_palette(AppTheme::gitcomet_dark());
    let keyword = palette
        .style(SyntaxTokenKind::KeywordControl)
        .expect("keyword style");
    assert!(
        styles_for(&highlights, text, "if x").contains(&&keyword),
        "the template's `if` keeps the Jinja keyword style through the nested layer"
    );
}

/// Both engines stop at `TS_MAX_INJECTION_DEPTH`: a JSDoc comment inside a
/// script inside a template would be depth 3.
#[test]
fn nested_layers_stop_at_the_shared_depth_cap() {
    let text = "{% block body %}\n<script>\n/** @param {number} n */\nfunction f(n) {}\n</script>\n{% endblock %}\n";
    let document = jinja_document(text);
    assert!(
        document.injections.iter().any(|layer| layer.depth == 2),
        "the script body is reached"
    );
    assert!(
        document
            .injections
            .iter()
            .all(|layer| usize::from(layer.depth) <= TS_MAX_INJECTION_DEPTH),
        "no layer may exceed the cap"
    );
    assert!(
        !document.injections.iter().any(|layer| std::ptr::eq(
            layer.spec,
            tree_sitter_highlight_spec(DiffSyntaxLanguage::Jsdoc).expect("jsdoc spec")
        )),
        "a depth-3 JSDoc layer must not be parsed"
    );
}

/// Nested layers ride the same deadline as the first level, and a starved
/// one is reported so the background reparse restores it.
#[test]
fn nested_layers_the_budget_could_not_finish_are_reported_as_dropped() {
    let body = (0..4_000)
        .map(|ix| format!("const answer{ix} = {ix} + compute{ix}(1, 2, 3);"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("{{% block body %}}\n<script>\n{body}\n</script>\n{{% endblock %}}\n");
    let document = jinja_document(&text);
    let rope = Rope::from_text(&text);

    let (complete, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        None,
        Vec::new(),
        None,
    );
    assert!(!dropped, "nothing is dropped without a deadline");
    assert!(
        complete.iter().any(|layer| layer.depth == 2),
        "an unbudgeted pass reaches the script body"
    );

    let (_, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        Some(Duration::ZERO),
        Vec::new(),
        None,
    );
    assert!(dropped, "a starved nested layer must be reported");
}

#[test]
fn the_two_engines_agree_on_a_template_with_a_script_body() {
    let mut text = String::from("{% block body %}\n<ul class=\"list\">\n");
    for ix in 0..40 {
        text.push_str(&format!("  <li>{{{{ item{ix} | upper }}}}</li>\n"));
    }
    text.push_str("</ul>\n<script>\nconst answer = 42;\n</script>\n{% endblock %}\n");
    super::tests::assert_engines_agree(
        DiffSyntaxLanguage::Jinja,
        &text,
        &[
            ("ul", SyntaxTokenKind::Tag),
            ("const", SyntaxTokenKind::Keyword),
        ],
    );
}

fn dense_jinja_table(rows: usize, cells: usize) -> String {
    let mut lines = vec!["{% block body %}".to_string()];
    for row in 0..rows {
        let mut line = String::from("<tr>");
        for cell in 0..cells {
            line.push_str(&format!("<td>{{{{ r{row}.c{cell} }}}}</td>"));
        }
        line.push_str("</tr>");
        lines.push(line);
    }
    lines.push("{% endblock %}".to_string());
    lines.join("\n")
}

/// This path is not windowed, so the prepared path's ceilings must not reach it:
/// applied here they capped on document size, and a 600-line `.njk` lost all of
/// its HTML in the editor while the diff pane still highlighted it.
#[test]
fn combined_layer_survives_a_template_past_the_prepared_range_ceiling() {
    let text = dense_jinja_table(600, 4);
    let document = jinja_document(&text);

    // Pinned as a literal, not read from the prepared path's constant: the point
    // is that a document of this density used to be dropped.
    const CEILING_THAT_USED_TO_APPLY: usize = 512;
    let layer = document
        .injections
        .iter()
        .find(|layer| layer.ranges.len() > CEILING_THAT_USED_TO_APPLY)
        .unwrap_or_else(|| {
            panic!(
                "fixture must produce more than {CEILING_THAT_USED_TO_APPLY} combined \
                     ranges to be a regression test; layers: {:?}",
                document
                    .injections
                    .iter()
                    .map(|layer| layer.ranges.len())
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        layer.hull().end > 0,
        "the combined html layer must actually cover the template"
    );

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let probe = text.find("<td>").expect("a table cell");
    let highlights = snapshot.highlights_for_byte_range(probe..probe + "<td>".len());
    assert!(
        !highlights.is_empty(),
        "the editor dropped HTML highlighting that the diff pane keeps"
    );
}

/// Same defect on the byte ceiling: `live_syntax_document_supported` admits
/// documents up to 8MB, so a 200KB template is valid but used to lose all its
/// HTML to the 128KB ceiling.
#[test]
fn combined_layer_survives_a_template_past_the_prepared_byte_ceiling() {
    let mut lines = vec!["{% block body %}".to_string()];
    for ix in 0..3_000 {
        lines.push(format!(
            "  <span class=\"cell\" data-row=\"{ix}\">value {ix} padded out</span>"
        ));
    }
    lines.push("{% endblock %}".to_string());
    let text = lines.join("\n");
    assert!(
        text.len() > TS_COMBINED_INJECTION_MAX_BYTES,
        "fixture must exceed the byte ceiling ({} bytes)",
        text.len()
    );
    assert!(
        live_syntax_document_supported(DiffSyntaxLanguage::Jinja, text.len()),
        "and must still be a document the live path accepts"
    );

    let document = jinja_document(&text);
    assert!(
        document
            .injections
            .iter()
            .any(|layer| layer.ranges.len() > 1
                || layer.hull().end - layer.hull().start > TS_COMBINED_INJECTION_MAX_BYTES),
        "the combined html layer must cover a span past the prepared byte ceiling"
    );

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let probe = text.rfind("<span").expect("a span near the end");
    let highlights = snapshot.highlights_for_byte_range(probe..probe + "<span".len());
    assert!(
        !highlights.is_empty(),
        "a {}-byte template lost its HTML highlighting in the editor",
        text.len()
    );
}

/// A root parse must not inherit an injected layer's clipping. `TS_PARSER` is
/// pooled and its included ranges are sticky, so ranges left set would silently
/// truncate the next root parse on this thread, for any language.
#[test]
fn an_included_range_parse_leaves_the_pooled_parser_unclipped() {
    let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
    let rope = Rope::from_text("<div class=\"a\">text</div>\n");
    let head: Range<usize> = 0..5;
    let _ = parse_included_range(html, &rope, &[], std::slice::from_ref(&head), None, None);

    let text = "fn main() { let value = 1; }\n";
    let document = LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Rust,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("rust live document should build");
    assert_eq!(
        document.tree.root_node().end_byte(),
        text.len(),
        "the next root parse was truncated to the previous layer's ranges"
    );
}

/// The half of that contract a successful parse cannot exercise. Neither the
/// parser nor `masked_read` has a panic a test can inject, so the unwind path is
/// pinned on the guard directly; what binds it to `parse_included_range` is that
/// there is no other way for that function to set ranges.
#[test]
fn the_included_ranges_guard_clears_them_on_unwind() {
    let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
    let rope = Rope::from_text("<div class=\"a\">text</div>\n");

    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_ts_parser_parse_result(&html.ts_language, |parser| {
            let included = [tree_sitter::Range {
                start_byte: 0,
                end_byte: 5,
                start_point: rope_ts_point(&rope, 0),
                end_point: rope_ts_point(&rope, 5),
            }];
            let _guard = IncludedRangesGuard::set(parser, &included)?;
            panic!("simulated parse panic");
            #[allow(unreachable_code)]
            None::<tree_sitter::Tree>
        })
    }));
    assert!(unwound.is_err(), "the probe must actually unwind");

    let text = "fn main() { let value = 1; }\n";
    let document = LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Rust,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("rust live document should build");
    assert_eq!(
        document.tree.root_node().end_byte(),
        text.len(),
        "an unwinding parse left its included ranges set on the pooled parser"
    );
}

fn vue_document(text: &str) -> LiveSyntaxDocument {
    LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Vue,
        Rope::from_text(text),
        Vec::new().into(),
        None,
    )
    .expect("vue live document should build")
}

/// The editor uses this layer engine rather than `prepared.rs`, so Vue's
/// script/style/interpolation injections need covering here too.
#[test]
fn vue_sections_are_highlighted_by_their_own_grammars() {
    let text = concat!(
        "<template>\n",
        "  <p v-if=\"count > 10\">{{ count }}</p>\n",
        "</template>\n",
        "\n",
        "<script setup lang=\"ts\">\n",
        "const count = 42;\n",
        "</script>\n",
        "\n",
        "<style lang=\"scss\">\n",
        ".wrapper { color: red; }\n",
        "</style>\n",
    );
    let document = vue_document(text);
    assert!(
        !document.injections.is_empty(),
        "the SFC should produce injected layers"
    );

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..text.len());

    // `const` can only be coloured by the injected TypeScript layer: the Vue
    // grammar sees the whole script body as one `raw_text` node.
    assert!(
        !styles_for(&highlights, text, "const").is_empty(),
        "expected the injected TypeScript layer to highlight `const`: {highlights:?}"
    );
    // Likewise `color` comes from the injected CSS layer.
    assert!(
        !styles_for(&highlights, text, "color").is_empty(),
        "expected the injected CSS layer to highlight `color`: {highlights:?}"
    );
    // And the root Vue grammar still highlights the template markup.
    assert!(
        !styles_for(&highlights, text, "template").is_empty(),
        "the host grammar must keep highlighting its own tags"
    );
}

/// A document whose language has no injection query must be unaffected.
#[test]
fn languages_without_injections_build_no_layers() {
    let document = LiveSyntaxDocument::new(
        DiffSyntaxLanguage::Json,
        Rope::from_text("{\"a\": 1}\n"),
        Vec::new().into(),
        None,
    )
    .expect("json live document should build");
    assert!(document.injections.is_empty());
}

/// Editing inside the host must not leave the injected layer painting at
/// stale offsets — the failure mode that would look like highlighting
/// "sliding" away from the code.
#[test]
fn layers_are_rebuilt_after_an_edit_moves_them() {
    let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
    let mut document = html_document(text);

    let inserted = "<div>pushed down</div>\n";
    let after = format!("<html>\n{inserted}<script>\nconst answer = 42;\n</script>\n</html>\n");
    let at = "<html>\n".len();
    document.sync(
        Rope::from_text(&after),
        Vec::new().into(),
        Some((at..at, at..at + inserted.len())),
        None,
    );

    let layer = document
        .injections
        .first()
        .expect("expected the layer to survive the edit");
    let body_start = after.find("\nconst").expect("script body") + 1;
    assert!(
        layer.hull().start <= body_start && layer.hull().end >= body_start,
        "layer {:?} should have moved with the edit to cover {body_start}",
        layer.hull()
    );

    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    let highlights = snapshot.highlights_for_byte_range(0..after.len());
    assert!(
        !styles_for(&highlights, &after, "const").is_empty(),
        "the injected keyword should still be highlighted after the edit"
    );
}

/// A layer the budget could not finish has to be reported, not swallowed.
///
/// The budget is now shared across all layers, so a document with many
/// injections can exhaust it partway through. Both callers assign
/// `stale = dropped`, so if this flag were dropped on the floor
/// `background_reparse_request` would return `None` and those regions would
/// keep only the enclosing grammar until the user happened to type again.
///
/// Driven at the function rather than through `new`/`sync`: a budget small
/// enough to starve layers also starves the root parse, and one tuned to sit
/// between the two would be a timing race.
#[test]
fn layers_the_budget_could_not_finish_are_reported_as_dropped() {
    // Script bodies large enough that their parse reaches a progress
    // callback — tree-sitter polls periodically, so a handful of bytes
    // finishes before any deadline is consulted.
    let body = (0..4_000)
        .map(|ix| format!("const answer{ix} = {ix} + compute{ix}(1, 2, 3);"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut text = String::from("<html>\n");
    for _ in 0..4 {
        text.push_str("<script>\n");
        text.push_str(&body);
        text.push_str("\n</script>\n");
    }
    text.push_str("</html>\n");

    let document = html_document(&text);
    let rope = Rope::from_text(&text);

    let (complete, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        None,
        Vec::new(),
        None,
    );
    assert_eq!(
        complete.len(),
        4,
        "an unbudgeted pass should find every script body"
    );
    assert!(!dropped, "nothing is dropped when there is no deadline");
    assert!(
        document.background_reparse_request().is_none(),
        "a complete document owes no reparse"
    );

    // A deadline already in the past breaks every layer at its first
    // progress callback.
    let (starved, dropped) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        Some(Duration::ZERO),
        Vec::new(),
        None,
    );
    assert!(
        starved.len() < complete.len(),
        "an exhausted budget should not have finished every layer"
    );
    assert!(
        dropped,
        "layers skipped for want of budget must be reported so the caller \
             can mark the document stale"
    );

    // A root parse can succeed while its layers exhaust their separate
    // budget. Every existing layer must still paint through that handoff.
    let (retained, pending) = parse_injection_layers(
        &rope,
        document.spec,
        &document.tree,
        &[],
        Some(Duration::ZERO),
        complete.clone(),
        None,
    );
    assert!(pending);
    assert_eq!(retained.len(), complete.len());
    for (retained, complete) in retained.iter().zip(&complete) {
        assert_eq!(
            retained.tree.root_node().to_sexp(),
            complete.tree.root_node().to_sexp()
        );
        assert_eq!(retained.ranges, complete.ranges);
    }
}

#[test]
fn markdown_inline_colors_survive_repeated_edits_while_reparsing_is_deferred() {
    let theme = AppTheme::gitcomet_dark();
    for location in ["paragraph start", "prose", "code"] {
        let mut text = "Words around `inline_code` and **bold** text.\n\n".repeat(300);
        let mut document = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
        let code = text.find("inline_code").unwrap();
        let style = styles_at(
            &document.snapshot(theme).highlights_for_byte_range(0..100),
            code,
        )
        .expect("inline code must start highlighted");
        assert!(style.color.is_some());
        let start = match location {
            "paragraph start" => 0,
            "prose" => 2,
            _ => code + 3,
        };
        for (step, at) in (start..start + 20).enumerate() {
            text.insert(at, 'a');
            document.sync(
                Rope::from_text(&text),
                Arc::default(),
                Some((at..at, at..at + 1)),
                Some(Duration::ZERO),
            );
            assert!(document.background_reparse_request().is_some());
            if step % 2 == 1 {
                // Also exercise a finished root whose inline layers run
                // out of time, without relying on relative parse timings.
                document.tree = parse_masked_tree(
                    document.spec,
                    &document.rope,
                    &[],
                    Some(&document.tree),
                    None,
                )
                .unwrap();
                let (layers, pending) = parse_injection_layers(
                    &document.rope,
                    document.spec,
                    &document.tree,
                    &[],
                    Some(Duration::ZERO),
                    std::mem::take(&mut document.injections),
                    Some(at..at + 1),
                );
                assert!(pending);
                document.injections = layers;
            }
            let snapshot = document.snapshot(theme);
            let highlights = snapshot.highlights_for_byte_range(0..text.len());
            // Both the edited paragraph and the untouched paragraphs keep
            // their color while a held key outruns the background parse.
            for probe in [
                text.find("code`").unwrap(),
                text.rfind("inline_code").unwrap(),
            ] {
                assert_eq!(
                    styles_at(&highlights, probe),
                    Some(style),
                    "{location}, repeated edit {step}, byte {probe}"
                );
            }
            if location == "code" {
                assert_eq!(styles_at(&highlights, at), Some(style));
            }
        }
        let (version, tree, injections) =
            live_syntax_reparse(document.background_reparse_request().unwrap()).unwrap();
        assert!(document.adopt_background_tree(version, tree, injections));
        let fresh = document_in(DiffSyntaxLanguage::Markdown, &text, Vec::new());
        assert_eq!(
            document
                .snapshot(theme)
                .highlights_for_byte_range(0..text.len()),
            fresh
                .snapshot(theme)
                .highlights_for_byte_range(0..text.len()),
        );

        // Retaining a previous layer is temporary: removing the closing
        // backtick must remove the code color once parsing finishes.
        let closing = text.find("code`").unwrap() + 4;
        text.remove(closing);
        document.sync(
            Rope::from_text(&text),
            Arc::default(),
            Some((closing..closing + 1, closing..closing)),
            Some(Duration::ZERO),
        );
        let (version, tree, injections) =
            live_syntax_reparse(document.background_reparse_request().unwrap()).unwrap();
        assert!(document.adopt_background_tree(version, tree, injections));
        assert_ne!(
            styles_at(
                &document
                    .snapshot(theme)
                    .highlights_for_byte_range(0..closing),
                code
            ),
            Some(style),
            "a removed code span must stop using its previous color"
        );
    }
}

#[test]
fn deferred_injections_track_unicode_newlines_and_deletions() {
    let theme = AppTheme::gitcomet_dark();
    for (language, text) in [
        (
            DiffSyntaxLanguage::Markdown,
            "```js\nconst answer = 42;\n```\n",
        ),
        (
            DiffSyntaxLanguage::Html,
            "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n",
        ),
        (
            DiffSyntaxLanguage::Jinja,
            "{% if ready %}\n<script>\nconst answer = 42;\n</script>\n{% endif %}\n",
        ),
    ] {
        let mut document = document_in(language, text, Vec::new());
        let style = styles_at(
            &document
                .snapshot(theme)
                .highlights_for_byte_range(0..text.len()),
            text.find("const").unwrap(),
        )
        .expect("injected keyword starts highlighted");
        let prefix = "header é😀\n";
        let inserted = format!("{prefix}{text}");
        for (current, old, new) in [
            (inserted.as_str(), 0..0, 0..prefix.len()),
            (text, 0..prefix.len(), 0..0),
        ] {
            document.sync(
                Rope::from_text(current),
                Arc::default(),
                Some((old, new)),
                Some(Duration::ZERO),
            );
            assert!(document.background_reparse_request().is_some());
            assert_eq!(
                styles_at(
                    &document
                        .snapshot(theme)
                        .highlights_for_byte_range(0..current.len()),
                    current.find("const").unwrap(),
                ),
                Some(style),
                "{language:?}: the injected token must keep its color at its new position"
            );
        }
    }
}

/// A first parse may have no prior layers to retain. The background parse
/// must install those missing layers together with the completed root tree.
#[test]
fn adopting_a_background_tree_restores_the_injected_layers() {
    let text = "<html>\n<script>\nconst answer = 42;\n</script>\n</html>\n";
    let mut document = html_document(text);
    assert!(
        !document.injections.is_empty(),
        "fixture should start with an injected script layer"
    );

    // Stand where an initial parse leaves a document when only its root
    // tree fits in the budget.
    document.injections.clear();
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    assert!(
        styles_for(
            &snapshot.highlights_for_byte_range(0..text.len()),
            text,
            "const"
        )
        .is_empty(),
        "with the layers dropped the injected keyword is unhighlighted — the \
             state the background parse exists to repair"
    );

    // The background parse finishes and hands its tree back.
    document.stale = true;
    let request = document
        .background_reparse_request()
        .expect("a stale document owes a background reparse");
    let (version, tree, injections) =
        live_syntax_reparse(request).expect("an unbudgeted reparse should succeed");
    assert!(document.adopt_background_tree(version, tree, injections));

    assert!(
        !document.injections.is_empty(),
        "adopting the caught-up tree must rebuild the injected layers"
    );
    let snapshot = document.snapshot(AppTheme::gitcomet_dark());
    assert!(
        !styles_for(
            &snapshot.highlights_for_byte_range(0..text.len()),
            text,
            "const"
        )
        .is_empty(),
        "the injected keyword must be highlighted again after the background parse"
    );
}
