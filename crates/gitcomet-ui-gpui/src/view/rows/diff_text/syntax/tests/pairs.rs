use super::*;

/// The display<->raw conversion in isolation. This is the one piece of the
/// feature that silently produces a plausible-but-wrong answer when it is
/// wrong, so it is pinned on its own rather than only through a document.
#[test]
fn display_and_raw_offsets_round_trip_across_tabs() {
    // Two tabs then `x`: display columns 0..8 are the tabs, `x` is at 8.
    let line = "\t\tx = 1";
    assert_eq!(display_offset_for_raw_offset(line, 0), 0);
    assert_eq!(
        display_offset_for_raw_offset(line, 1),
        4,
        "one tab is 4 wide"
    );
    assert_eq!(display_offset_for_raw_offset(line, 2), 8);
    assert_eq!(
        display_offset_for_raw_offset(line, line.find('x').expect("x")),
        8
    );

    assert_eq!(raw_offset_for_display_offset(line, 0), 0);
    assert_eq!(raw_offset_for_display_offset(line, 4), 1);
    assert_eq!(raw_offset_for_display_offset(line, 8), 2);
    // A click landing inside an expanded tab resolves to that tab.
    assert_eq!(raw_offset_for_display_offset(line, 2), 0);
    assert_eq!(raw_offset_for_display_offset(line, 6), 1);

    // Round trip on every real byte boundary.
    for (raw, _) in line.char_indices() {
        assert_eq!(
            raw_offset_for_display_offset(line, display_offset_for_raw_offset(line, raw)),
            raw,
            "raw offset {raw} did not survive the round trip"
        );
    }

    // A tab-free line is the identity, including past the end.
    let plain = "let x = 1;";
    for raw in 0..=plain.len() {
        assert_eq!(display_offset_for_raw_offset(plain, raw), raw);
        assert_eq!(raw_offset_for_display_offset(plain, raw), raw);
    }
    assert_eq!(raw_offset_for_display_offset(plain, 999), plain.len());

    // The click variant accepts the end caret boundary: hit testing can
    // produce it from the right half of the final glyph. The view rejects
    // trailing blank-space clicks from their pixel position instead.
    assert_eq!(
        clicked_raw_offset_for_display_offset(plain, plain.len() - 1),
        Some(plain.len() - 1),
        "the last character is still clickable"
    );
    assert_eq!(
        clicked_raw_offset_for_display_offset(plain, plain.len()),
        Some(plain.len())
    );
    assert_eq!(clicked_raw_offset_for_display_offset(plain, 999), None);
    // `\t\tx = 1` is 13 display columns wide, not 7 bytes.
    assert_eq!(clicked_raw_offset_for_display_offset(line, 8), Some(2));
    assert_eq!(clicked_raw_offset_for_display_offset(line, 12), Some(6));
    assert_eq!(
        clicked_raw_offset_for_display_offset(line, 13),
        Some(line.len())
    );
    assert_eq!(
        clicked_raw_offset_for_display_offset(line, line.len()),
        Some(1),
        "a raw length is not a display column: 7 still lands inside the second tab"
    );
}

/// A `line_starts` that no longer describes its text answers `None` rather
/// than panicking.
///
/// The click path reads a source-backed diff side back from disk and pairs
/// it with the line index taken when the diff was built. A worktree file
/// that shrank in between leaves starts past the end of the text, and the
/// lookup used to slice with them before checking. `line_starts_describe`
/// keeps that pairing from reaching here at all; this is the second wall.
#[test]
fn prepared_syntax_click_survives_a_line_index_from_a_longer_text() {
    let indexed = "fn a() {}\nfn b() {}\nfn c() {}\n";
    let shrunk = "fn a() {}\n";
    let stale_starts = treesitter_document_input_from_text(indexed).line_starts;
    // No start after the final newline: unlike the diff indexer's, this
    // index has one entry per line of text.
    assert_eq!(stale_starts.as_ref(), &[0, 10, 20]);

    let document = match prepare_treesitter_document_with_budget_reuse_text(
        DiffSyntaxLanguage::Rust,
        DiffSyntaxMode::Auto,
        SharedString::from(shrunk.to_owned()),
        Arc::clone(&stale_starts),
        DiffSyntaxBudget {
            foreground_parse: Duration::from_millis(200),
        },
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(doc) => doc,
        other => panic!("test document should parse successfully, got {other:?}"),
    };

    // Every line the index claims, at columns inside and past the text.
    for line_ix in 0..=stale_starts.len() {
        for display_offset in [0usize, 3, 9, 999] {
            let _ =
                prepared_document_syntax_pair_at_display_offset(document, line_ix, display_offset);
            let _ =
                prepared_document_occurrences_at_display_offset(document, line_ix, display_offset);
        }
    }

    // The lines the shrunken text no longer has answer nothing at all.
    assert!(prepared_document_syntax_pair_at_display_offset(document, 2, 0).is_none());
    assert!(prepared_document_occurrences_at_display_offset(document, 2, 0).is_empty());
}

/// A click in the blank area right of a short line names nothing.
///
/// The row hitbox spans the full width of the pane and clamps a point past
/// the text to the line's last column, so without a separate "was this
/// inside the line" answer, clicking empty space washed every use of the
/// last name on the line and lit the enclosing braces with it.
#[test]
fn prepared_syntax_click_past_the_end_of_a_line_highlights_nothing() {
    let text = "fn main() {\n    let total = 1;\n    let sum = total;\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

    let line = text.lines().nth(2).expect("the third line");
    assert_eq!(line, "    let sum = total;");
    let on_name = line.find("total").expect("the name");
    assert!(
        !prepared_document_occurrences_at_display_offset(document, 2, on_name).is_empty(),
        "clicking the name itself still lights its uses"
    );

    for past in [line.len() + 1, line.len() + 200] {
        assert!(
            prepared_document_occurrences_at_display_offset(document, 2, past).is_empty(),
            "column {past} is past the line's last character"
        );
        assert!(
            prepared_document_syntax_pair_at_display_offset(document, 2, past).is_none(),
            "column {past} is past the line's last character"
        );
    }

    // The closing brace on its own line: both caret boundaries produced by
    // its left and right halves pair, while a boundary beyond it does not.
    assert!(prepared_document_syntax_pair_at_display_offset(document, 3, 0).is_some());
    assert!(prepared_document_syntax_pair_at_display_offset(document, 3, 1).is_some());
    assert!(prepared_document_syntax_pair_at_display_offset(document, 3, 2).is_none());
}

#[test]
fn prepared_syntax_end_caret_selects_a_final_closing_parenthesis() {
    let text = "fn main() {\n    run()\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
    let line = "    run()";

    let pair = prepared_document_syntax_pair_at_display_offset(document, 1, line.len())
        .expect("the right half of the final `)` should still select its pair");
    assert_eq!(pair.kind, SyntaxPairKind::Bracket);
    assert_eq!(pair.open[0].display_range, 7..8);
    assert_eq!(pair.close[0].display_range, 8..9);

    let occurrences_text = "let total = 1;\ntotal";
    let occurrences_document = prepare_test_document(DiffSyntaxLanguage::Rust, occurrences_text);
    assert_eq!(
        prepared_document_occurrences_at_display_offset(occurrences_document, 1, "total".len(),),
        vec![
            PreparedSyntaxPairSpan {
                line_ix: 0,
                display_range: 4..9,
            },
            PreparedSyntaxPairSpan {
                line_ix: 1,
                display_range: 0..5,
            },
        ],
        "the right half of a final name should still light its occurrences"
    );
}

#[test]
fn prepared_syntax_pair_reports_display_columns_on_tab_indented_lines() {
    // Every body line is tab-indented, so a raw-offset answer would be three
    // columns per indent level to the left of where the row was painted.
    let text = "fn main() {\n\tif a {\n\t\tb();\n\t}\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

    // Line 1 is "\tif a {": display columns 0..4 are the tab, so the `{` is
    // painted at column 9 and its raw offset within the line is 6.
    let hit = prepared_document_syntax_pair_at_display_offset(document, 1, 9)
        .expect("the `if` block braces pair");
    assert_eq!(hit.kind, SyntaxPairKind::Bracket);
    assert_eq!(hit.open.len(), 1);
    assert_eq!(hit.open[0].line_ix, 1);
    assert_eq!(
        hit.open[0].display_range,
        9..10,
        "the open brace is reported where the canvas painted it, not at raw offset 6"
    );
    assert_eq!(hit.close.len(), 1);
    assert_eq!(hit.close[0].line_ix, 3);
    assert_eq!(
        hit.close[0].display_range,
        4..5,
        "the closing brace sits after one expanded tab"
    );
}

#[test]
fn prepared_syntax_pair_projects_each_end_onto_its_own_line() {
    let text = "<div class=\"card\">\n  <span>hi</span>\n</div>\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Html, text);

    // Caret on the inner element's text: an inner pair, both ends on line 1.
    let inner = prepared_document_syntax_pair_at_display_offset(document, 1, 9)
        .expect("the span element pairs");
    assert_eq!(inner.kind, SyntaxPairKind::Tag);
    assert_eq!(inner.open[0].line_ix, 1);
    assert_eq!(inner.close[0].line_ix, 1);
    // `  <span>hi</span>`: two spaces, then the tags at columns 2 and 10.
    assert_eq!(inner.open[0].display_range, 2..8);
    assert_eq!(inner.close[0].display_range, 10..17);

    // Caret on the outer element name: ends on lines 0 and 2, whole tags.
    let outer = prepared_document_syntax_pair_at_display_offset(document, 0, 2)
        .expect("the div element pairs");
    assert_eq!(outer.kind, SyntaxPairKind::Tag);
    assert_eq!(outer.open[0].line_ix, 0);
    assert_eq!(
        outer.open[0].display_range,
        0..18,
        "the whole start tag, attributes included"
    );
    assert_eq!(outer.close[0].line_ix, 2);
    assert_eq!(outer.close[0].display_range, 0..6);
}

/// A start tag split across lines is one delimiter but several rows, so it
/// reports one span per row rather than washing only its first line.
#[test]
fn prepared_syntax_pair_spans_every_line_a_tag_covers() {
    let text = "<div\n  class=\"card\"\n  id=\"x\">\ntext\n</div>\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Html, text);

    let hit = prepared_document_syntax_pair_at_display_offset(document, 3, 1)
        .expect("the caret in the element content pairs its tags");
    assert_eq!(hit.kind, SyntaxPairKind::Tag);
    assert_eq!(
        hit.open
            .iter()
            .map(|span| (span.line_ix, span.display_range.clone()))
            .collect::<Vec<_>>(),
        vec![(0, 0..4), (1, 0..14), (2, 0..9)],
        "every line of the start tag is washed, not just the first"
    );
    assert_eq!(hit.close.len(), 1);
    assert_eq!(hit.close[0].line_ix, 4);
}

/// PowerShell spells `$(` and `@(` as opening tokens of their own, both
/// closing at a plain `)`.
///
/// The pair tables are keyed by node kind, and the close side used to be
/// resolved by taking the table's *first* row whose close matched -- always
/// `(` -- so a caret anywhere inside a subexpression or an array literal
/// matched nothing while every other wired language paired its parens.
#[test]
fn powershell_subexpression_and_array_parens_pair() {
    let text = "$x = @(1, 2)\n$y = $(Get-Thing)\n";
    let document = prepare_test_document(DiffSyntaxLanguage::PowerShell, text);

    // `$x = @(1, 2)`: the opener is the two-byte `@(` at columns 5..7.
    let array = prepared_document_syntax_pair_at_display_offset(document, 0, 7)
        .expect("a caret inside `@(1, 2)` pairs the array literal's parens");
    assert_eq!(array.kind, SyntaxPairKind::Bracket);
    assert_eq!(array.open.len(), 1);
    assert_eq!(array.open[0].display_range, 5..7, "the whole `@(` opener");
    assert_eq!(array.close.len(), 1);
    assert_eq!(array.close[0].display_range, 11..12);

    // `$y = $(Get-Thing)`: same shape with the subexpression opener.
    let sub = prepared_document_syntax_pair_at_display_offset(document, 1, 8)
        .expect("a caret inside `$(Get-Thing)` pairs the subexpression's parens");
    assert_eq!(sub.kind, SyntaxPairKind::Bracket);
    assert_eq!(sub.open[0].display_range, 5..7, "the whole `$(` opener");
    assert_eq!(sub.close[0].display_range, 16..17);

    // And a plain paren in the same language still pairs with itself.
    let plain = prepare_test_document(DiffSyntaxLanguage::PowerShell, "$z = (1 + 2)\n");
    let hit = prepared_document_syntax_pair_at_display_offset(plain, 0, 6)
        .expect("a plain grouping paren still pairs");
    assert_eq!(hit.open[0].display_range, 5..6);
    assert_eq!(hit.close[0].display_range, 11..12);
}

/// A `)` that one opener has already claimed must not also be offered to an
/// opener spelled differently.
///
/// The open side counted depth on its own token kind only, while the close
/// side used `closes_open` -- so in the many-opens-one-close case the two
/// ends of the same delimiter disagreed. Recovery is what puts the openers
/// side by side: well-formed PowerShell keeps `$( ... )` inside its own
/// `sub_expression`, but a half-typed line flattens them into one ERROR
/// node, which is exactly the state the editor sees while you type.
#[test]
fn powershell_open_side_does_not_claim_a_paren_another_opener_closed() {
    // Recovers as one ERROR whose children are `$(`, `(`, `)`, a name --
    // the `)` belongs to the `(`, and `$(` is left unclosed.
    let document = prepare_test_document(DiffSyntaxLanguage::PowerShell, "$( ( ) a\n");

    let from_close = prepared_document_syntax_pair_at_display_offset(document, 0, 5)
        .expect("the `)` pairs with the `(` that opened it");
    assert_eq!(from_close.open[0].display_range, 3..4);
    assert_eq!(from_close.close[0].display_range, 5..6);

    assert!(
        prepared_document_syntax_pair_at_display_offset(document, 0, 1).is_none(),
        "the unclosed `$(` has no partner here, and must not borrow the `(`'s"
    );
}

/// A wide node whose children are all named still pairs, where the grammar
/// spells its delimiters as named nodes.
///
/// The caret-move path skips walking a node's children when they are all
/// named and the grammar has no named delimiter -- two O(1) counts instead
/// of stepping tens of thousands of siblings. HTML is the case that must not
/// take that shortcut: an element's children are `start_tag`, its content,
/// and `end_tag`, every one of them named, so the skip firing here would
/// silently stop tag matching in any element with a lot of children.
#[test]
fn wide_all_named_node_still_pairs_where_delimiters_are_named() {
    let text = wide_html_element_with_marker(false);
    let document = prepare_test_document(DiffSyntaxLanguage::Html, &text);

    let marker_line = marker_line_in(&text);
    let hit = prepared_document_syntax_pair_at_display_offset(document, marker_line, 3)
        .expect("text directly inside a wide element still pairs the element's tags");
    assert_eq!(hit.kind, SyntaxPairKind::Tag);
    assert_eq!(hit.open[0].line_ix, 0, "the `<div>` start tag");
    assert_eq!(hit.open[0].display_range, 0..5);
    assert_eq!(
        hit.close[0].line_ix,
        text.lines().count() - 1,
        "the `</div>` end tag"
    );
}

/// A generated flat array must answer from its structural boundary pair,
/// both when the caret is directly on the opener and when it is among the
/// many values. The fixture exceeds the bounded fallback scan, so either
/// answer proves the first/last-child path handled it.
#[test]
fn wide_flat_json_array_pairs_without_fallback_scanning() {
    let elements = 5_000usize;
    let text = format!(
        "[{}]",
        std::iter::repeat_n("0", elements)
            .collect::<Vec<_>>()
            .join(",")
    );
    let document = prepare_test_document(DiffSyntaxLanguage::Json, &text);
    let expected = [(0, 0..1), (0, text.len() - 1..text.len())];

    for column in [0, text.len() / 2] {
        let hit = prepared_document_syntax_pair_at_display_offset(document, 0, column)
            .expect("the wide array's boundary brackets should pair");
        assert_eq!(
            hit.open
                .iter()
                .chain(hit.close.iter())
                .map(|span| (span.line_ix, span.display_range.clone()))
                .collect::<Vec<_>>(),
            expected,
            "caret column {column}"
        );
    }
}

/// Named delimiters elsewhere in a grammar must not force a wide root to
/// scan ordinary named children. Python is the regression case because its
/// strings use named `string_start`/`string_end` nodes while module children
/// are statements.
#[test]
fn wide_python_module_without_direct_delimiters_returns_none() {
    let statements = 5_000usize;
    let text = (0..statements)
        .map(|ix| format!("value_{ix} = {ix}\n"))
        .collect::<String>();
    let document = prepare_test_document(DiffSyntaxLanguage::Python, &text);
    assert!(
        prepared_document_syntax_pair_at_display_offset(document, statements - 1, 2).is_none(),
        "ordinary module statements are not direct named delimiters"
    );
}

/// A parse error *anywhere* inside a wide element must not stop its tags
/// from pairing.
///
/// The boundary shortcut skips nodes with a parse error because recovery can
/// flatten several adjacent pairs, where first and last need not be
/// partners. `has_error` is the wrong question for that: it is true for the
/// whole subtree, so a stray `<` in one paragraph of a hundred marks the
/// enclosing `<div>` too, even though the div's own first and last children
/// are still its `start_tag` and `end_tag`. With the shortcut skipped, the
/// all-named bail below then answers `None` for an element whose children --
/// `start_tag`, content, `end_tag` -- are all named.
#[test]
fn wide_html_element_pairs_its_tags_despite_a_parse_error_inside() {
    let text = wide_html_element_with_marker(true);
    let document = prepare_test_document(DiffSyntaxLanguage::Html, &text);

    let marker_line = marker_line_in(&text);
    let hit = prepared_document_syntax_pair_at_display_offset(document, marker_line, 3)
        .expect("a parse error in a sibling must not unpair the enclosing element");
    assert_eq!(hit.kind, SyntaxPairKind::Tag);
    assert_eq!(hit.open[0].line_ix, 0, "the `<div>` start tag");
    assert_eq!(hit.open[0].display_range, 0..5);
    assert_eq!(
        hit.close[0].line_ix,
        text.lines().count() - 1,
        "the `</div>` end tag"
    );
}

/// One bad element must not unpair a generated array's brackets.
///
/// Clicking the `[`: the boundary shortcut is skipped because the array's
/// *subtree* has an error, and the fallback is then refused outright for
/// being wider than the sibling cap, so the click answers nothing. The
/// array's own first and last children are still `[` and `]`.
#[test]
fn wide_malformed_json_array_pairs_from_its_opening_bracket() {
    let text = malformed_wide_json_array(5_000);
    let document = prepare_test_document(DiffSyntaxLanguage::Json, &text);

    let hit = prepared_document_syntax_pair_at_display_offset(document, 0, 0)
        .expect("clicking `[` must pair the array's own brackets");
    assert_eq!(hit.open[0].display_range, 0..1);
    assert_eq!(hit.close[0].display_range, text.len() - 1..text.len());
}

/// The same array, with the caret among its values rather than on a bracket.
///
/// This is the enclosing path rather than the delimiter path, and it fails
/// differently: the fallback scan runs but is cut off at the sibling cap, so
/// it walks thousands of children and still never reaches the `]`.
#[test]
fn wide_malformed_json_array_pairs_from_inside() {
    let text = malformed_wide_json_array(5_000);
    let document = prepare_test_document(DiffSyntaxLanguage::Json, &text);

    let column = text.len() / 4;
    let hit = prepared_document_syntax_pair_at_display_offset(document, 0, column)
        .expect("a caret among the values must pair the enclosing array");
    assert_eq!(hit.open[0].display_range, 0..1);
    assert_eq!(hit.close[0].display_range, text.len() - 1..text.len());
}

/// A wide `<div>` with a hundred `<p>` children and a `MARKER` line at
/// child fifty, used by the named-delimiter pairing tests. `sibling_error`
/// adds a bare `<` in prose at child seventy so the subtree carries a parse
/// error while the div's own boundary tags stay intact.
fn wide_html_element_with_marker(sibling_error: bool) -> String {
    let mut text = String::from("<div>\n");
    for ix in 0..100 {
        text.push_str(&format!("<p>item {ix}</p>\n"));
        if ix == 50 {
            text.push_str("MARKER\n");
        }
        if sibling_error && ix == 70 {
            // A bare `<` in prose: ordinary content, and a parse error.
            text.push_str("< \n");
        }
    }
    text.push_str("</div>\n");
    text
}

/// Zero-based line index of the first `MARKER` line.
fn marker_line_in(text: &str) -> usize {
    text[..text.find("MARKER").expect("marker")]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
}

/// A flat array of `elements` values with a single unparseable one in the
/// middle, so the array node carries a subtree error while its own boundary
/// children stay `[` and `]`.
fn malformed_wide_json_array(elements: usize) -> String {
    let mut values = vec!["0"; elements];
    values[elements / 2] = "@@";
    format!("[{}]", values.join(","))
}

/// A delimiter its grammar wraps in a named node of its own must pair from
/// both ends.
///
/// HCL's `[` and `]` are a `tuple_start` and a `tuple_end`, and only those
/// wrappers are siblings; the bare tokens are one level deeper. Clicking the
/// `[` reached the pair through the enclosing walk, but clicking the `]`
/// searched from the token, found no sibling `tuple_start`, and fell through
/// to whatever delimiter sat one column to its left -- in
/// `[for p in ports : tostring(p)]`, the `)` of the call.
#[test]
fn wrapped_delimiters_pair_from_the_closing_end_too() {
    let text = "locals {\n  upper = [for p in var.ports : tostring(p)]\n}\n";
    let document = prepare_test_document(DiffSyntaxLanguage::Hcl, text);
    let line = text.lines().nth(1).expect("the tuple line");
    let open_col = line.find('[').expect("the opening bracket");
    let close_col = line.find(']').expect("the closing bracket");

    let from_open = prepared_document_syntax_pair_at_display_offset(document, 1, open_col)
        .expect("clicking `[` pairs the tuple");
    assert_eq!(from_open.open[0].display_range, open_col..open_col + 1);
    assert_eq!(from_open.close[0].display_range, close_col..close_col + 1);

    let from_close = prepared_document_syntax_pair_at_display_offset(document, 1, close_col)
        .expect("clicking `]` must pair the same tuple, not the call's `)`");
    assert_eq!(
        (from_close.open, from_close.close),
        (from_open.open, from_open.close),
        "a pair must name the same two ends whichever end is clicked"
    );
}
