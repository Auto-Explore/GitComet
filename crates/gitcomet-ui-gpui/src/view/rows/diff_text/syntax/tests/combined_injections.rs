use super::*;

// ---- `#set! injection.combined` ------------------------------------------

/// The inventory tripwire.
///
/// Combined injections change how a grammar's whole document is assembled, so
/// a grammar bump that quietly introduces the directive must not slip through
/// review. F#'s `xml_doc` rule arrived with the upstream
/// `tree_sitter_fsharp::INJECTIONS_QUERY` rather than being written here; the
/// rest are ours, and each one is a deliberate decision recorded beside it.
#[test]
fn combined_injection_declarations_are_exactly_the_known_set() {
    let mut declared = Vec::new();
    for lang in all_supported_languages() {
        let Some(spec) = tree_sitter_highlight_spec(lang) else {
            continue;
        };
        for (pattern_ix, combined) in spec.injection_combined_patterns.iter().enumerate() {
            if *combined {
                declared.push((lang, pattern_ix));
            }
        }
        assert_eq!(
            spec.has_combined_injections,
            spec.injection_combined_patterns.iter().any(|c| *c),
            "{lang:?} has a stale has_combined_injections flag"
        );
    }
    assert_eq!(
        declared,
        vec![
            // queries/jinja_injections.scm -- the HTML around the template tags.
            (DiffSyntaxLanguage::Jinja, 0),
            // queries/hcl_injections.scm -- shell in `user_data`, JSON in
            // `policy`. Combined for the same reason as Nix: a `${...}`
            // interpolation splits one heredoc into several `template_literal`
            // nodes, and those are one script. The third pattern there, for
            // bodies nested inside a `%{ if }`, is deliberately *not* combined.
            (DiffSyntaxLanguage::Hcl, 0),
            (DiffSyntaxLanguage::Hcl, 1),
            // queries/nix_injections.scm -- bash in script/hook attributes.
            (DiffSyntaxLanguage::Nix, 0),
            (DiffSyntaxLanguage::Nix, 1),
            (DiffSyntaxLanguage::Nix, 2),
            (DiffSyntaxLanguage::Nix, 3),
            // Upstream tree_sitter_fsharp::INJECTIONS_QUERY -- `xml_doc` lines.
            (DiffSyntaxLanguage::FSharp, 3),
        ],
        "the set of grammars declaring `#set! injection.combined` changed. Every entry \
             here parses all its matches as one document via set_included_ranges, so a new \
             one needs the gap-clipping and cache behaviour reviewed -- it is not a \
             drop-in.\nfound: {declared:?}"
    );
}

// The one-range cases are the point: a single included range is the shape
// every non-combined injection has, and both helpers have to leave it alone.
#[allow(clippy::single_range_in_vec_init)]
#[test]
fn merge_sorted_injection_ranges_normalises_for_set_included_ranges() {
    // Empty stays empty: an empty slice is tree-sitter's "whole document"
    // reset, which callers must detect rather than pass on.
    assert!(merge_sorted_injection_ranges(Vec::new()).is_empty());
    // Degenerate ranges are dropped, not kept as zero-width.
    assert!(merge_sorted_injection_ranges(vec![5..5]).is_empty());
    assert_eq!(merge_sorted_injection_ranges(vec![2..5]), vec![2..5]);
    // Unsorted input is sorted: set_included_ranges rejects descending ranges.
    assert_eq!(
        merge_sorted_injection_ranges(vec![10..12, 2..5]),
        vec![2..5, 10..12]
    );
    // Touching ranges coalesce, so the gap list carries no empty entries.
    assert_eq!(merge_sorted_injection_ranges(vec![2..5, 5..9]), vec![2..9]);
    // Overlapping ranges coalesce: set_included_ranges rejects overlap.
    assert_eq!(merge_sorted_injection_ranges(vec![2..7, 5..9]), vec![2..9]);
    // Fully contained range is absorbed rather than shortening the outer one.
    assert_eq!(
        merge_sorted_injection_ranges(vec![2..20, 5..9]),
        vec![2..20]
    );
}

// The one-range cases are the point: a single included range is the shape
// every non-combined injection has, and both helpers have to leave it alone.
#[allow(clippy::single_range_in_vec_init)]
#[test]
fn combined_injection_gaps_are_the_complement_within_the_window() {
    assert_eq!(combined_injection_gaps(0..100, &[]), vec![0..100]);
    assert!(combined_injection_gaps(0..100, &[0..100]).is_empty());
    assert_eq!(
        combined_injection_gaps(0..100, &[10..20, 30..40]),
        vec![0..10, 20..30, 40..100]
    );
    // Range flush against each edge produces no leading/trailing gap.
    assert_eq!(combined_injection_gaps(0..100, &[0..20]), vec![20..100]);
    assert_eq!(combined_injection_gaps(0..100, &[80..100]), vec![0..80]);
    // Ranges reaching outside the window are clipped to it, not extrapolated.
    assert_eq!(combined_injection_gaps(20..80, &[0..30]), vec![30..80]);
    assert_eq!(combined_injection_gaps(20..80, &[70..200]), vec![20..70]);
    assert!(combined_injection_gaps(20..80, &[0..200]).is_empty());
    // A range entirely outside contributes nothing and does not swallow the window.
    assert_eq!(combined_injection_gaps(20..80, &[200..300]), vec![20..80]);
}

/// Two halves of an HTML element split across a host-grammar tag must parse as
/// one element, and the injected grammar must not colour the host bytes
/// between them.
///
/// HTML stands in for the eventual template grammar here so the test needs no
/// new dependency. The ranges are the same shape a `(text) @injection.content`
/// rule produces on a real template.
#[test]
fn combined_injection_parses_disjoint_ranges_as_one_document() {
    let text = "<ul>\n{% for x in xs %}<li>hi</li>{% endfor %}\n</ul>\n";
    let input = treesitter_document_input_from_text(text);
    let bytes = text.as_bytes();
    let ranges = combined_test_ranges(text);

    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
    let tree = parse_combined_injection_tree(spec, bytes, input.line_starts.as_ref(), &ranges)
        .expect("combined parse should succeed");

    assert_eq!(
        tree.root_node().start_byte(),
        ranges[0].start,
        "a tree parsed with included_ranges reports document offsets"
    );
    assert!(
        !tree.root_node().has_error(),
        "the <ul> opened before `{{% for %}}` should close after `{{% endfor %}}` when the \
             three text runs are parsed as one document: {}",
        tree.root_node().to_sexp(),
    );
}

/// The other half: nodes straddling two included ranges report a byte range
/// covering the host bytes in between, so their captures have to be clipped.
#[test]
fn combined_injection_tokens_do_not_bleed_into_the_gaps() {
    let text = "<ul>\n{% for x in xs %}<li>hi</li>{% endfor %}\n</ul>\n";
    let input = treesitter_document_input_from_text(text);
    let bytes = text.as_bytes();
    let ranges = combined_test_ranges(text);
    let line_starts = input.line_starts.as_ref();

    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
    let tree = parse_combined_injection_tree(spec, bytes, line_starts, &ranges)
        .expect("combined parse should succeed");

    let line_count = line_starts.len();
    let mut tokens = collect_treesitter_document_line_tokens_for_line_window(
        &tree,
        spec,
        bytes,
        line_starts,
        0,
        line_count,
        treesitter_text_hash(text),
    );
    let window_end = line_region_end_byte(line_starts, bytes.len(), line_count - 1);
    for gap in combined_injection_gaps(0..window_end, &ranges) {
        subtract_absolute_range_from_document_tokens(line_starts, bytes, 0, &mut tokens, gap);
    }

    // Line 1 is `{% for x in xs %}<li>hi</li>{% endfor %}`. Only the `<li>hi</li>`
    // slice belongs to HTML; both template tags are host-grammar bytes.
    let line_start = line_starts[1];
    let html_start = text.find("<li>").expect("li") - line_start;
    let html_end = text.find("{% endfor %}").expect("endfor") - line_start;
    for token in &tokens[1] {
        assert!(
            token.range.start >= html_start && token.range.end <= html_end,
            "injected HTML token {:?} escaped its included range \
                 ({html_start}..{html_end}) into a `{{% … %}}` gap",
            token.range,
        );
    }
    assert!(
        !tokens[1].is_empty(),
        "clipping should not have removed the genuine <li> tokens as well"
    );
}

/// Byte ranges of the HTML runs in the combined-injection fixture, i.e. what a
/// template grammar's `(text)` nodes would capture.
fn combined_test_ranges(text: &str) -> Vec<Range<usize>> {
    let for_tag = text.find("{% for x in xs %}").expect("for tag");
    let li = text.find("<li>").expect("li");
    let endfor = text.find("{% endfor %}").expect("endfor");
    let after_endfor = endfor + "{% endfor %}".len();
    merge_sorted_injection_ranges(vec![0..for_tag, li..endfor, after_endfor..text.len()])
}

// ---- Combined-injection scoping -------------------------------------------

/// A template dense enough to exercise the per-window ceilings, `rows` lines of
/// `cells` cells each wrapped in a block so the body is one big text run.
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

/// An 8-column table row used to produce 513 ranges in one 64-line chunk, one
/// over the ceiling, and the whole chunk lost its HTML.
#[test]
fn dense_table_template_keeps_its_html_highlighting() {
    for cells in [4usize, 8, 16] {
        let text = dense_jinja_table(200, cells);
        let lines: Vec<&str> = text.lines().collect();
        let doc = prepare_test_document(DiffSyntaxLanguage::Jinja, &text);

        let kinds = token_kinds_for_line_fragment(doc, 100, lines[100], "<td>");
        assert!(
            kinds.contains(&SyntaxTokenKind::Tag),
            "a {cells}-cell table row lost its HTML highlighting: {kinds:?}"
        );
    }
}

/// The byte ceiling had the same defect at an ordinary file size: all the HTML
/// between two template tags is ONE `(text)` node, so a ~1800-line template
/// tripped the 128KB ceiling in every window.
#[test]
fn large_template_with_one_huge_text_run_keeps_its_html_highlighting() {
    let mut lines = vec!["{% block body %}".to_string()];
    for ix in 0..2_400 {
        lines.push(format!(
            "  <span class=\"cell\" data-row=\"{ix}\">value {ix} padded out</span>"
        ));
    }
    lines.push("{% endblock %}".to_string());
    let text = lines.join("\n");
    assert!(
        text.len() > TS_COMBINED_INJECTION_MAX_BYTES,
        "fixture must exceed the byte ceiling to be a regression test ({} bytes)",
        text.len()
    );

    let line_refs: Vec<&str> = text.lines().collect();
    let doc = prepare_test_document(DiffSyntaxLanguage::Jinja, &text);
    for line_ix in [1usize, 700, 1_500, 2_300] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, line_refs[line_ix], "span");
        assert!(
            kinds.contains(&SyntaxTokenKind::Tag),
            "line {line_ix} of a {}-byte template lost its HTML: {kinds:?}",
            text.len()
        );
    }
}

/// The property the whole optimisation rests on, and the reason for the margin:
/// a `<section` whose attributes run onto the next lines straddles the window
/// edge, and an exact clip cuts it in half. Asserted against an unclipped parse
/// so it stays honest if the margin is ever tuned.
#[test]
fn clipping_a_combined_layer_to_the_window_preserves_its_tokens() {
    let mut lines = vec!["{% block body %}".to_string()];
    for ix in 0..300 {
        if ix == 62 || ix == 126 {
            lines.push("  <section".to_string());
            lines.push("     id=\"straddle\"".to_string());
            lines.push("     class=\"wide\">body</section>".to_string());
        } else {
            lines.push(format!(
                "  <span class=\"c{ix}\" data-x='y'>row {ix}</span>"
            ));
        }
    }
    lines.push("{% endblock %}".to_string());
    let text = lines.join("\n") + "\n";

    let input = treesitter_document_input_from_text(&text);
    let bytes = text.as_bytes();
    let line_starts = input.line_starts.as_ref();
    let jinja = tree_sitter_highlight_spec(DiffSyntaxLanguage::Jinja).expect("jinja spec");
    let root = with_ts_parser_parse_result(&jinja.ts_language, |parser| {
        parse_treesitter_tree(parser, bytes, None, None)
    })
    .expect("root parse");

    let start_line_ix = 64usize;
    let end_line_ix = start_line_ix + TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
    let matches = collect_treesitter_injection_matches_for_line_window(
        &root,
        jinja,
        bytes,
        line_starts,
        start_line_ix,
        end_line_ix,
        treesitter_text_hash(&text),
    );
    let group = matches.combined.first().expect("one combined html group");
    assert_eq!(
        group.ranges.len(),
        1,
        "the fixture's body must be one text run, or it is not testing the hard case"
    );

    let window_start = line_starts[start_line_ix];
    let window_end = line_region_end_byte(line_starts, bytes.len(), end_line_ix - 1);
    let html = tree_sitter_highlight_spec(DiffSyntaxLanguage::Html).expect("html spec");
    let render = |ranges: &[Range<usize>]| -> Vec<Vec<SyntaxToken>> {
        let tree = parse_combined_injection_tree(html, bytes, line_starts, ranges)
            .expect("combined parse");
        let mut injected = collect_treesitter_document_line_tokens_for_line_window(
            &tree,
            html,
            bytes,
            line_starts,
            start_line_ix,
            end_line_ix,
            treesitter_text_hash(&text),
        );
        for gap in combined_injection_gaps(window_start..window_end, ranges) {
            subtract_absolute_range_from_document_tokens(
                line_starts,
                bytes,
                start_line_ix,
                &mut injected,
                gap,
            );
        }
        injected
    };

    let clip_region =
        combined_injection_clip_region(line_starts, bytes.len(), start_line_ix, end_line_ix);
    let clipped_ranges = clip_injection_ranges_to_region(&group.ranges, &clip_region);
    let clipped_bytes: usize = clipped_ranges.iter().map(|r| r.end - r.start).sum();
    let full_bytes: usize = group.ranges.iter().map(|r| r.end - r.start).sum();
    assert!(
        clipped_bytes < full_bytes,
        "the clip must actually shrink the parse ({clipped_bytes} vs {full_bytes})"
    );

    assert_eq!(
        render(&group.ranges),
        render(&clipped_ranges),
        "clipping to the window changed the tokens the window renders"
    );
}

/// The clip region is the window plus a margin on both sides, and the margin is
/// load-bearing rather than decorative -- see the constant.
#[test]
fn combined_injection_clip_region_pads_the_window_on_both_sides() {
    let text = dense_jinja_table(400, 2);
    let input = treesitter_document_input_from_text(&text);
    let line_starts = input.line_starts.as_ref();
    let len = text.len();

    let start_line_ix = 200usize;
    let end_line_ix = start_line_ix + TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS;
    let region = combined_injection_clip_region(line_starts, len, start_line_ix, end_line_ix);
    let window_start = line_starts[start_line_ix];
    let window_end = line_region_end_byte(line_starts, len, end_line_ix - 1);

    assert!(
        region.start < window_start && region.end > window_end,
        "clip region {region:?} must strictly contain the window \
             {window_start}..{window_end}"
    );
    assert_eq!(
        window_start - region.start,
        TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES,
        "leading margin"
    );

    // ... and still bounded, which is what makes the ceilings window-scoped.
    assert!(
        region.end - region.start
            < window_end - window_start + 2 * TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES + 1,
        "clip region must not grow past window + 2 * margin"
    );

    // At the top of the document the margin runs out rather than underflowing.
    let head = combined_injection_clip_region(line_starts, len, 0, 8);
    assert_eq!(head.start, 0, "no underflow at the start of the document");
}

/// A cut that touches nothing must leave the line's tokens exactly as they were,
/// and must not reallocate to do it.
#[test]
fn subtracting_a_non_overlapping_range_leaves_line_tokens_untouched() {
    let original = vec![
        SyntaxToken {
            range: 0..4,
            kind: SyntaxTokenKind::Tag,
        },
        SyntaxToken {
            range: 10..14,
            kind: SyntaxTokenKind::String,
        },
    ];

    // Entirely before, entirely after, and in the gap between the two tokens.
    for cut in [20..30usize, 4..10, 100..200] {
        let mut tokens = original.clone();
        subtract_relative_range_from_line_tokens(&mut tokens, cut.clone());
        assert_eq!(tokens, original, "cut {cut:?} must be a no-op");
    }

    // ... and a cut that does overlap still splits, so the fast path is not
    // swallowing real work.
    let mut tokens = original.clone();
    subtract_relative_range_from_line_tokens(&mut tokens, 2..12);
    assert_eq!(
        tokens,
        vec![
            SyntaxToken {
                range: 0..2,
                kind: SyntaxTokenKind::Tag,
            },
            SyntaxToken {
                range: 12..14,
                kind: SyntaxTokenKind::String,
            },
        ]
    );
}

/// Pins the ordering rather than a symptom: no in-tree grammar declares both
/// kinds over one span yet, but with combined applied first an overlapping
/// single would delete its tokens and repaint only part of the span.
#[test]
fn combined_injection_groups_are_applied_after_the_single_ones() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/view/rows/diff_text/syntax/prepared/query_tokens.rs"
    ))
    .expect("query_tokens.rs should be readable");
    let body_start = source
        .find("fn apply_injection_query_tokens_for_document")
        .expect("the function that applies both kinds of layer");
    let body = &source[body_start..];
    let body_end = body.find("\n}\n").expect("the end of the function");
    let body = &body[..body_end];

    let singles_at = body
        .find("for injection in &injections.singles")
        .expect("the singles loop");
    let combined_at = body
        .find("for group in &injections.combined")
        .expect("the combined loop");
    assert!(
        singles_at < combined_at,
        "the singles loop must run before the combined one, or a single's \
             subtraction erases combined tokens nothing repaints"
    );
}

/// F# XML doc comments are the one in-tree consumer of `injection.combined`.
///
/// `xml_doc` is a per-line token, so before combined support each `///` line
/// was its own XML document: `<summary>` on one line and `</summary>` on
/// another never met, and each cost an entry in the 32-slot injection cache.
#[test]
fn fsharp_xml_doc_comment_is_highlighted_as_one_xml_document() {
    let lines = [
        /* 0 */ "/// <summary>",
        /* 1 */ "/// Adds two numbers.",
        /* 2 */ "/// </summary>",
        /* 3 */ "let add x y = x + y",
    ];
    let doc = prepare_test_document(DiffSyntaxLanguage::FSharp, &lines.join("\n"));

    let closing = token_kinds_for_line_fragment(doc, 2, lines[2], "summary");
    assert!(
        closing.contains(&SyntaxTokenKind::Tag),
        "`</summary>` closes a tag opened two lines earlier, which only parses \
             when the three xml_doc lines are one document: {closing:?}"
    );

    // And the layer stays inside its own ranges: the following line is F#.
    let keyword = token_kinds_for_line_fragment(doc, 3, lines[3], "let");
    assert!(
        keyword.contains(&SyntaxTokenKind::Keyword),
        "the combined XML layer leaked past the doc comment onto `let`: {keyword:?}"
    );
}

/// Combined layers must not touch the per-node injection cache at all.
///
/// The 32-slot LRU is keyed by a single node's content hash, which a combined
/// layer does not have -- its identity is a *set* of ranges. Feeding it one
/// entry per constituent node is what F# used to do: 200 `///` lines meant 200
/// entries into a 32-slot cache, evicting everything
/// `vue_static_inline_styles_do_not_flood_the_injection_cache` depends on.
///
/// This is not a claim that combined parses are memoised elsewhere. They are
/// not: each of the N/64 chunks pays its own on first build, and clipping is
/// what keeps that cost proportional to the window.
#[test]
fn combined_injections_do_not_consume_the_per_node_injection_cache() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = vec!["/// <summary>".to_string()];
    for ix in 0..200 {
        lines.push(format!("/// line {ix}"));
    }
    lines.push("/// </summary>".to_string());
    lines.push("let add x y = x + y".to_string());
    let line_count = lines.len();

    let doc = prepare_test_document(DiffSyntaxLanguage::FSharp, &lines.join("\n"));
    for line_ix in 0..line_count {
        let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
    }

    let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
    assert_eq!(
        cached, 0,
        "a combined layer's identity is a set of ranges, not one node's content \
             hash, so it must not enter TS_INJECTION_CACHE (cap \
             {TS_INJECTION_CACHE_MAX_ENTRIES}); {line_count} lines of xml doc comment \
             created {cached} entries"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// The failure this would cause is invisible and global.
///
/// `TS_PARSER` is pooled and its included ranges are sticky; `with_ts_parser`
/// can skip `set_language` entirely on the fast path, so a combined parse that
/// forgot to clear them would truncate the *next* root parse on this thread —
/// for any language, with no error anywhere. Asserted behaviourally so it
/// survives tree-sitter API changes.
#[test]
fn combined_injection_parse_clears_the_pooled_parsers_included_ranges() {
    let fsharp = ["/// <summary>", "/// x", "/// </summary>", "let x = 1"];
    let _ = prepare_test_document(DiffSyntaxLanguage::FSharp, &fsharp.join("\n"));

    let mut rust_lines = Vec::new();
    for ix in 0..300 {
        rust_lines.push(format!("fn f{ix}() -> u32 {{ {ix} }}"));
    }
    let last_ix = rust_lines.len() - 1;
    let last_line = rust_lines[last_ix].clone();
    let doc = prepare_test_document(DiffSyntaxLanguage::Rust, &rust_lines.join("\n"));

    let kinds = token_kinds_for_line_fragment(doc, last_ix, &last_line, "fn");
    assert!(
        kinds.contains(&SyntaxTokenKind::Keyword),
        "the last line of a 300-line Rust document lost its tokens after a combined \
             injection ran on this thread -- the pooled parser's included ranges were not \
             cleared, so the root parse was truncated: {kinds:?}"
    );
}

/// A `(text)`-style combined rule fires once per node, so this is the query
/// most likely to overflow the in-progress match pool. Overflow is worse for a
/// combined layer than a single one: tree-sitter discards matches silently, and
/// a missing range changes the document the injected grammar assembles.
#[test]
fn fsharp_xml_doc_injection_stays_under_the_match_limit_on_a_long_doc_comment() {
    let mut lines = vec!["/// <summary>".to_string()];
    for ix in 0..200 {
        lines.push(format!("/// line {ix}"));
    }
    lines.push("/// </summary>".to_string());
    let text = lines.join("\n");

    let lang: tree_sitter::Language = tree_sitter_fsharp::LANGUAGE_FSHARP.into();
    let query = tree_sitter::Query::new(&lang, tree_sitter_fsharp::INJECTIONS_QUERY)
        .expect("fsharp injections.scm should compile");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).expect("fsharp grammar");
    let tree = parser.parse(&text, None).expect("doc comment should parse");

    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    let mut matched = 0usize;
    {
        let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
        tree_sitter::StreamingIterator::advance(&mut matches);
        while matches.get().is_some() {
            matched += 1;
            tree_sitter::StreamingIterator::advance(&mut matches);
        }
    }

    assert!(
        !cursor.did_exceed_match_limit(),
        "the F# injection query overflowed the {TS_QUERY_MATCH_LIMIT}-match in-progress \
             pool on a {}-line doc comment; a combined group that loses ranges assembles a \
             different document, so the whole group is dropped when this happens",
        lines.len(),
    );
    assert!(matched > 0, "the doc comment should produce matches at all");
}

#[test]
fn highlight_spec_exposes_ts_language() {
    let spec = tree_sitter_highlight_spec(DiffSyntaxLanguage::Rust)
        .expect("Rust highlight spec should exist");
    // Verify the ts_language field is usable for parsing
    with_ts_parser(&spec.ts_language, |_| ()).expect("should accept the spec's ts_language");
}

#[test]
#[ignore]
fn perf_treesitter_tokenization_smoke() {
    let text = "fn main() { let x = Some(123); println!(\"{x:?}\"); }";
    let start = Instant::now();
    for _ in 0..200_000 {
        let _ = syntax_tokens_for_line(text, DiffSyntaxLanguage::Rust, DiffSyntaxMode::Auto);
    }
    eprintln!("syntax_tokens_for_line (rust): {:?}", start.elapsed());
}
