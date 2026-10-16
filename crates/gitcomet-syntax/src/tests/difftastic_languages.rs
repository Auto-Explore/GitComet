//! Languages wired so the highlighter covers every language Difftastic
//! diffs. Each test pins a rule a plausible query edit would break.

use super::*;
use SyntaxTokenKind as K;

/// The kinds of every token over `fragment` on line `line` of `text`.
fn kinds(language: DiffSyntaxLanguage, text: &str, line: usize, fragment: &str) -> Vec<K> {
    let document = prepare_test_document(language, text);
    let line_text = text.lines().nth(line).expect("line in sample");
    token_kinds_for_line_fragment(document, line, line_text, fragment)
}

#[test]
fn smali_marks_only_upper_case_fields_constant_and_jumps_as_control() {
    let text = [
        ".class public LFoo;",
        ".super Ljava/lang/Object;",
        ".method public run()V",
        "    .registers 2",
        "    iget v0, p0, LFoo;->count:I",
        "    sget v1, LFoo;->MAX:I",
        "    if-eqz v0, :done",
        "    add-int v0, v0, v1",
        "    return-void",
        ".end method",
    ]
    .join("\n");
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Smali, &text, line, fragment);
    assert!(!kinds(4, "count").contains(&K::Constant));
    assert_eq!(kinds(5, "MAX"), [K::Constant]);
    assert_eq!(kinds(6, "if-eqz"), [K::KeywordControl]);
    assert_eq!(kinds(7, "add-int"), [K::Keyword]);
    assert_eq!(kinds(8, "return-void"), [K::KeywordControl]);
}

#[test]
fn elm_doc_comments_differ_from_block_comments() {
    let text = "{- plain -}\n{-| docs -}\nmain = 1\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Elm, text, line, fragment);
    assert_eq!(kinds(0, "plain"), [K::Comment]);
    assert_eq!(kinds(1, "docs"), [K::CommentDoc]);
}

#[test]
fn devicetree_properties_labels_and_references_keep_their_kinds() {
    let text = "/ {\n    uart0: serial@40003000 {\n        compatible = \"acme,uart\";\n        clocks = <&clk0>, <&{/soc/i2c@0}>;\n    };\n};\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::DeviceTree, text, line, fragment);
    assert_eq!(kinds(1, "uart0"), [K::Label]);
    assert_eq!(kinds(2, "compatible"), [K::Property]);
    assert_eq!(kinds(2, "\"acme,uart\""), [K::String]);
    for reference in ["&clk0", "&{/soc/i2c@0}"] {
        let reference_kinds = kinds(3, reference);
        assert!(
            !reference_kinds.is_empty() && reference_kinds.iter().all(|kind| *kind == K::Constant),
            "{reference}: {reference_kinds:?}"
        );
    }
}

#[test]
fn fish_options_and_expansions_match_bash() {
    let text = "if [ -n \"$x\" ]\n    set -l y $x\nend\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Fish, text, line, fragment);
    assert_eq!(kinds(0, "["), [K::PunctuationBracket]);
    assert_eq!(kinds(0, "-n"), [K::Operator]);
    assert_eq!(kinds(1, "-l"), [K::Constant]);
    assert_eq!(kinds(1, "$x"), [K::PunctuationSpecial, K::Variable]);
}

#[test]
fn newick_leaves_support_values_and_lengths_keep_their_kinds() {
    let text = "(A:0.1,B:0.2)95:0.3;\n";
    let kinds = |fragment| kinds(DiffSyntaxLanguage::Newick, text, 0, fragment);
    assert_eq!(kinds("A"), [K::Type]);
    assert_eq!(kinds("95"), [K::Number]);
    assert_eq!(kinds("0.1"), [K::Number]);
}
