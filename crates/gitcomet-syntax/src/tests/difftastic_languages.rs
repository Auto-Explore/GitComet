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

#[test]
fn latex_commands_are_not_control_keywords_and_escaped_percent_is_not_a_comment() {
    let text = "\\section{Intro} % note\nCosts 5\\% more.\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Latex, text, line, fragment);
    assert!(!kinds(0, "\\section").contains(&K::KeywordControl));
    assert_eq!(kinds(0, "% note"), [K::Comment]);
    assert_eq!(kinds(1, "\\%"), [K::StringEscape]);
}

#[test]
fn apex_names_are_not_constants_unless_upper_case_and_soql_objects_are_types() {
    let text = "public class Invoice {\n    Integer total = MAX_TOTAL;\n    List<Account> rows = [SELECT Id FROM Account];\n}\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Apex, text, line, fragment);
    assert!(!kinds(1, "total").contains(&K::Constant));
    assert_eq!(kinds(1, "MAX_TOTAL"), [K::Constant]);
    assert_eq!(kinds(2, "SELECT"), [K::Keyword]);
    assert_eq!(kinds(2, "Account]"), [K::Type, K::PunctuationBracket]);
}

#[test]
fn qml_object_declarations_name_their_type() {
    let text = "import QtQuick\n\nRectangle {\n    width: 640\n}\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Qml, text, line, fragment);
    assert_eq!(kinds(2, "Rectangle"), [K::Type]);
    assert_eq!(kinds(3, "width"), [K::Property]);
}

#[test]
fn vhdl_block_comments_are_plain_comments() {
    let text = "/* block */\nentity counter is\nend entity;\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Vhdl, text, line, fragment);
    assert_eq!(kinds(0, "block"), [K::Comment]);
    assert_eq!(kinds(1, "entity"), [K::Keyword]);
}

#[test]
fn ada_loops_are_control_flow_and_packages_namespaces() {
    let text = "package body Queues is\n   procedure Drain is\n   begin\n      for I in 1 .. 10 loop\n         null;\n      end loop;\n   end Drain;\n   package Int_Vectors is new Ada.Containers.Vectors (Positive, Integer);\nend Queues;\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Ada, text, line, fragment);
    assert_eq!(kinds(0, "Queues"), [K::Namespace]);
    assert_eq!(kinds(3, "loop"), [K::KeywordControl]);
    assert_eq!(kinds(7, "Int_Vectors"), [K::Namespace]);
}

#[test]
fn common_lisp_definitions_quotes_and_function_references_keep_their_kinds() {
    let text = "(defun add (a b) (+ a b))\n(mapcar #'car '(1 2))\n(list 'sym t nil)\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::CommonLisp, text, line, fragment);
    assert_eq!(kinds(0, "add"), [K::Function]);
    assert!(kinds(1, "#'car").contains(&K::Function));
    assert_eq!(kinds(1, "1"), [K::Number]);
    assert_eq!(kinds(2, "sym"), [K::Constant]);
    assert_eq!(kinds(2, " t "), [K::Boolean]);
}

#[test]
fn racket_and_scheme_quoted_data_keep_their_own_colours() {
    for language in [DiffSyntaxLanguage::Racket, DiffSyntaxLanguage::Scheme] {
        let text = "(define xs '(0 1 2))\n";
        let kinds = |fragment| kinds(language, text, 0, fragment);
        assert_eq!(kinds("0"), [K::Number], "{language:?}");
        assert_eq!(kinds("'"), [K::Operator], "{language:?}");
    }
}

#[test]
fn scheme_let_bindings_are_not_functions() {
    let text = "(let ((x 1) (y 2))\n  (+ x y))\n";
    let kinds = kinds(DiffSyntaxLanguage::Scheme, text, 0, "x");
    assert!(!kinds.contains(&K::Function), "{kinds:?}");
}

#[test]
fn emacs_lisp_docstrings_are_strings_not_comments() {
    let text = "(defun greet (name)\n  \"Say hello to NAME.\"\n  (message name))\n";
    let kinds = kinds(DiffSyntaxLanguage::EmacsLisp, text, 1, "Say hello");
    assert!(kinds.contains(&K::String), "{kinds:?}");
    assert!(!kinds.contains(&K::Comment), "{kinds:?}");
}

#[test]
fn janet_definitions_parameters_and_literals_keep_their_kinds() {
    let text = "(defn add [a b] (+ a b))\n(def ok true)\n(get m :key)\n";
    let kinds = |line, fragment| kinds(DiffSyntaxLanguage::Janet, text, line, fragment);
    assert_eq!(kinds(0, "add"), [K::Function]);
    assert_eq!(
        kinds(0, "[a"),
        [K::PunctuationBracket, K::VariableParameter]
    );
    assert_eq!(kinds(1, "true"), [K::Boolean]);
    assert_eq!(kinds(2, ":key"), [K::Constant]);
}

#[test]
fn lisp_block_comments_are_heuristic_comments() {
    for language in [
        DiffSyntaxLanguage::CommonLisp,
        DiffSyntaxLanguage::Racket,
        DiffSyntaxLanguage::Scheme,
    ] {
        let tokens = heuristic_tokens("#| a block |# (car x)", language);
        assert_eq!(
            tokens
                .first()
                .map(|token| (token.range.clone(), token.kind)),
            Some((0..13, K::Comment)),
            "{language:?}: {tokens:?}"
        );
    }
}

#[test]
fn lisp_heuristic_keywords_are_whole_hyphenated_names() {
    for (language, text, expected) in [
        (
            DiffSyntaxLanguage::Racket,
            "(define (do-something xs)",
            vec!["define"],
        ),
        (DiffSyntaxLanguage::Racket, "(for-each display xs)", vec![]),
        (
            DiffSyntaxLanguage::CommonLisp,
            "(return-from scan (case-p l))",
            vec!["return-from"],
        ),
        (
            DiffSyntaxLanguage::Scheme,
            "(define-syntax swap!",
            vec!["define-syntax"],
        ),
        (
            DiffSyntaxLanguage::EmacsLisp,
            "(cl-defun demo-greet (name)",
            vec!["cl-defun"],
        ),
        (
            DiffSyntaxLanguage::Janet,
            "(defn- helper [x]",
            vec!["defn-"],
        ),
        (
            DiffSyntaxLanguage::Clojure,
            "(when-let [x (do-it)]",
            vec!["when-let"],
        ),
    ] {
        assert_eq!(
            heuristic_keywords(text, language),
            expected,
            "{language:?}: {text}"
        );
    }
}

#[test]
fn lisp_value_definitions_are_plain_and_dynamic_names_special() {
    for (language, text) in [
        (DiffSyntaxLanguage::Racket, "(define limit 10)\n"),
        (DiffSyntaxLanguage::Scheme, "(define limit 10)\n"),
        (DiffSyntaxLanguage::Janet, "(def limit 10)\n"),
    ] {
        assert_eq!(
            kinds(language, text, 0, "limit"),
            [K::Variable],
            "{language:?}"
        );
    }
    let janet = "(defdyn *depth*)\n";
    assert_eq!(
        kinds(DiffSyntaxLanguage::Janet, janet, 0, "*depth*"),
        [K::VariableSpecial]
    );
}

#[test]
fn racket_prefixed_requires_name_their_module() {
    let text = "(require racket/list (prefix-in s: racket/string))\n";
    let kinds = |fragment| kinds(DiffSyntaxLanguage::Racket, text, 0, fragment);
    assert_eq!(kinds("racket/list"), [K::Namespace]);
    assert_eq!(kinds("racket/string"), [K::Namespace]);
}
