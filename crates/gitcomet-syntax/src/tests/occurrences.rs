use super::*;

fn occurrences_in(
    language: DiffSyntaxLanguage,
    text: &str,
    offset: usize,
) -> Option<SyntaxOccurrences> {
    let input = treesitter_document_input_from_text(text);
    let spec = tree_sitter_highlight_spec(language)?;
    let tree = with_ts_parser_parse_result(&spec.ts_language, |parser| {
        parse_treesitter_tree(parser, input.text.as_bytes(), None, None)
    })?;
    syntax_occurrences_in_tree(&tree, text, offset)
}

/// The clicked name lights everywhere the grammar also tokenised it.
#[test]
fn occurrences_find_every_use_of_the_clicked_name() {
    let text = "fn main() {\n    let values = vec![1];\n    for v in values {}\n}\n";
    let click = text.find("values").expect("declaration");
    let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

    assert_eq!(found.token, click..click + "values".len());
    assert_eq!(
        found
            .ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["values", "values"],
    );
    assert_eq!(found.ranges[1].start, text.rfind("values").expect("use"));
}

/// A name inside a comment or a string is content, not a use of the symbol.
#[test]
fn occurrences_skip_comments_and_string_bodies() {
    let text = concat!(
        "fn main() {\n",
        "    let total = 1;\n",
        "    // total is not a use\n",
        "    let label = \"total\";\n",
        "    let sum = total + 1;\n",
        "}\n",
    );
    let click = text.find("total").expect("declaration");
    let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

    assert_eq!(found.ranges.len(), 2, "declaration and the later use only");
    for range in &found.ranges {
        assert_eq!(&text[range.clone()], "total");
        assert!(
            !text[..range.start].ends_with("// ") && !text[..range.start].ends_with('"'),
            "matched inside a comment or string: {range:?}"
        );
    }
    assert_eq!(found.ranges[1].start, text.rfind("total").expect("use"));
}

/// A longer word that merely contains the name is not a use of it.
#[test]
fn occurrences_respect_word_boundaries() {
    let text = "fn main() {\n    let sum = 1;\n    let summary = 2;\n    let x = sum;\n}\n";
    let click = text.find("sum").expect("declaration");
    let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");

    assert_eq!(found.ranges.len(), 2, "`summary` must not match `sum`");
    assert!(
        found
            .ranges
            .iter()
            .all(|range| &text[range.clone()] == "sum"),
    );
}

/// A name whose first character is multi-byte must still light up.
///
/// The scan used to step one byte past each hit, which lands inside the
/// leading character of such a name; the slice then failed and the `?` threw
/// away every match found so far, including the clicked one, so the whole
/// highlight silently vanished.
#[test]
fn occurrences_handle_non_ascii_names() {
    let text = "x = 1\ncafé = 2\ny = café\n";
    let click = text.find("café").expect("name");
    let found = occurrences_in(DiffSyntaxLanguage::Python, text, click).expect("a name");
    assert_eq!(
        found
            .ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["café", "café"],
    );
    assert_eq!(found.ranges[1].start, text.rfind("café").expect("use"));

    // And a name that is entirely multi-byte.
    let cjk = "日本語 = 1\nz = 日本語\n";
    let at = cjk.find("日本語").expect("name");
    let found = occurrences_in(DiffSyntaxLanguage::Python, cjk, at).expect("a name");
    assert_eq!(found.ranges.len(), 2, "got {:?}", found.ranges);
}

/// The candidate budget bounds tree descents, so hits the cheap
/// word-boundary test throws away must not spend it.
///
/// `uuid` contains `id`. Counting every raw substring hit exhausted the
/// 4096-candidate budget on thousands of `uuid`s without a single descent,
/// and the real uses of `id` further down the file were never reached --
/// while `MAX_OCCURRENCES`, the cap the budget protects, was nowhere near.
#[test]
fn occurrence_candidates_are_counted_after_the_word_boundary_test() {
    use std::fmt::Write as _;

    let mut text = String::from("fn main() {\n");
    for ix in 0..4_200 {
        writeln!(text, "    let uuid{ix} = {ix};").expect("write");
    }
    text.push_str("    let id = 1;\n    let n = id;\n}\n");

    let click = text.find("let id = 1").expect("the declaration") + "let ".len();
    let found = occurrences_in(DiffSyntaxLanguage::Rust, &text, click).expect("a name");
    assert_eq!(
        found
            .ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["id", "id"],
        "both real uses, found past 4200 word-internal `id`s inside `uuid`",
    );
}

/// Clicking punctuation, whitespace or a literal is not clicking a name.
#[test]
fn occurrences_are_none_off_a_name() {
    let text = "fn main() {\n    let n = 1234;\n}\n";
    for probe in ["{", " 1234", ";"] {
        let at = text.find(probe).expect("probe");
        assert_eq!(
            occurrences_in(DiffSyntaxLanguage::Rust, text, at + 1).map(|found| found.token),
            None,
            "clicking {probe:?} should not name anything",
        );
    }
}

/// Some grammars make literals and prose named leaves, so `is_named` and a
/// word-shaped body alone do not make them symbol occurrences.
#[test]
fn occurrences_reject_named_literal_and_prose_leaves() {
    let rust = "fn main() { let a = true; let b = true; let c = false; let d = false; }\n";
    for literal in ["true", "false"] {
        let click = rust.find(literal).expect("boolean literal");
        assert!(
            occurrences_in(DiffSyntaxLanguage::Rust, rust, click).is_none(),
            "Rust {literal:?} is a boolean literal, not a name"
        );
    }

    for (language, text) in [
        (DiffSyntaxLanguage::Html, "<p>plain</p><p>plain</p>"),
        (
            DiffSyntaxLanguage::Xml,
            "<root><item>plain</item><item>plain</item></root>",
        ),
    ] {
        let click = text.find("plain").expect("prose leaf");
        assert!(
            occurrences_in(language, text, click).is_none(),
            "{language:?} prose is content, not a name"
        );
    }
}

/// Fields and calls are names too, which is the point of asking the tree
/// rather than scanning for words.
#[test]
fn occurrences_cover_calls_and_fields() {
    let text = concat!(
        "fn main() {\n",
        "    let item = Item { width: 1 };\n",
        "    let w = item.width;\n",
        "    resize(item.width);\n",
        "}\n",
    );
    let click = text.find("width").expect("field declaration");
    let found = occurrences_in(DiffSyntaxLanguage::Rust, text, click).expect("a name");
    assert_eq!(
        found.ranges.len(),
        3,
        "the shorthand field and both reads, got {:?}",
        found
            .ranges
            .iter()
            .map(|r| &text[r.clone()])
            .collect::<Vec<_>>()
    );
}
