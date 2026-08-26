use super::*;

// ---- Batch: Groovy, Clojure, Elixir, Erlang, Haskell, Julia, OCaml, ------
// ---- Solidity, Assembly, Svelte ------------------------------------------

/// Every extension and fence alias the batch claims, plus the collisions the
/// mapping had to avoid. Path resolution is the only thing standing between a
/// wired-up grammar and a file that still renders as plain text, and nothing
/// else in the suite exercises these arms.
#[test]
fn batch_language_paths_and_fences_resolve() {
    let cases: &[(&str, DiffSyntaxLanguage)] = &[
        ("src/Demo.groovy", DiffSyntaxLanguage::Groovy),
        ("build.gradle", DiffSyntaxLanguage::Groovy),
        ("Jenkinsfile", DiffSyntaxLanguage::Groovy),
        ("src/demo/core.clj", DiffSyntaxLanguage::Clojure),
        ("src/demo/core.cljs", DiffSyntaxLanguage::Clojure),
        ("deps.edn", DiffSyntaxLanguage::Clojure),
        ("lib/demo/worker.ex", DiffSyntaxLanguage::Elixir),
        ("test/demo_test.exs", DiffSyntaxLanguage::Elixir),
        ("src/demo.erl", DiffSyntaxLanguage::Erlang),
        ("include/demo.hrl", DiffSyntaxLanguage::Erlang),
        ("rebar.config", DiffSyntaxLanguage::Erlang),
        ("src/Demo/Worker.hs", DiffSyntaxLanguage::Haskell),
        ("src/demo.jl", DiffSyntaxLanguage::Julia),
        ("lib/demo.ml", DiffSyntaxLanguage::OCaml),
        ("lib/demo.mli", DiffSyntaxLanguage::OCamlInterface),
        ("contracts/Demo.sol", DiffSyntaxLanguage::Solidity),
        ("src/boot.asm", DiffSyntaxLanguage::Assembly),
        ("src/memcpy.s", DiffSyntaxLanguage::Assembly),
        // `.S` is preprocessed assembly; the lowercasing in
        // `diff_syntax_language_for_path` is what makes it land here.
        ("src/entry.S", DiffSyntaxLanguage::Assembly),
        ("src/App.svelte", DiffSyntaxLanguage::Svelte),
    ];
    for (path, expected) in cases {
        assert_eq!(
            diff_syntax_language_for_path(path),
            Some(*expected),
            "{path} should resolve to {expected:?}"
        );
    }

    for (fence, expected) in [
        ("groovy", DiffSyntaxLanguage::Groovy),
        ("clojure", DiffSyntaxLanguage::Clojure),
        ("elixir", DiffSyntaxLanguage::Elixir),
        ("erlang", DiffSyntaxLanguage::Erlang),
        ("haskell", DiffSyntaxLanguage::Haskell),
        ("julia", DiffSyntaxLanguage::Julia),
        ("ocaml", DiffSyntaxLanguage::OCaml),
        ("solidity", DiffSyntaxLanguage::Solidity),
        ("asm", DiffSyntaxLanguage::Assembly),
        ("svelte", DiffSyntaxLanguage::Svelte),
    ] {
        assert_eq!(
            diff_syntax_language_for_code_fence_info(fence),
            Some(expected),
            "```{fence} should resolve to {expected:?}"
        );
    }
}

/// The three collisions the batch had to route around. Each one is a silent
/// regression if the identifier table is ever reordered: the file still
/// highlights, just as the wrong language.
#[test]
fn batch_language_extensions_do_not_steal_existing_ones() {
    // `.gradle.kts` is Kotlin. The extension pass sees `kts` and never reaches
    // the `gradle` arm.
    assert_eq!(
        diff_syntax_language_for_path("build.gradle.kts"),
        Some(DiffSyntaxLanguage::Kotlin),
    );

    // `.m` stays Objective-C: the new `.ml` arm is one character away from it,
    // and an extension table is edited by hand.
    assert_eq!(
        diff_syntax_language_for_path("src/Demo.m"),
        Some(DiffSyntaxLanguage::ObjectiveC),
    );

    // `.ml` is OCaml, and must not be confused with the `.m` above.
    assert_eq!(
        diff_syntax_language_for_path("lib/demo.ml"),
        Some(DiffSyntaxLanguage::OCaml),
    );
}

// ---- Elixir ---------------------------------------------------------------

const ELIXIR_FIXTURE: &[&str] = &[
    /*  0 */ "defmodule Demo.Worker do",
    /*  1 */ "  @moduledoc \"Runs jobs.\"",
    /*  2 */ "",
    /*  3 */ "  def run(%{id: id} = job) when is_integer(id) do",
    /*  4 */ "    :ok",
    /*  5 */ "  end",
    /*  6 */ "end",
];

#[test]
fn prepared_elixir_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Elixir, &ELIXIR_FIXTURE.join("\n"));

    for (line_ix, fragment, expected) in [
        (0usize, "defmodule", SyntaxTokenKind::Keyword),
        (0, "Demo.Worker", SyntaxTokenKind::Namespace),
        (3, "when", SyntaxTokenKind::Keyword),
        (5, "end", SyntaxTokenKind::Keyword),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, ELIXIR_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }

    // Atoms are the shape that separates Elixir from a curly-brace language.
    let atom = token_kinds_for_line_fragment(doc, 4, ELIXIR_FIXTURE[4], ":ok");
    assert!(
        atom.contains(&SyntaxTokenKind::StringSpecial),
        "`:ok` is an atom, not an identifier: {atom:?}"
    );
}

// ---- Erlang ---------------------------------------------------------------

const ERLANG_FIXTURE: &[&str] = &[
    /*  0 */ "-module(demo).",
    /*  1 */ "-export([run/1]).",
    /*  2 */ "",
    /*  3 */ "%% Adds one.",
    /*  4 */ "run(X) when is_integer(X) ->",
    /*  5 */ "    Y = X + 1,",
    /*  6 */ "    {ok, Y}.",
];

#[test]
fn prepared_erlang_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Erlang, &ERLANG_FIXTURE.join("\n"));

    let directive = token_kinds_for_line_fragment(doc, 0, ERLANG_FIXTURE[0], "module");
    assert!(
        directive.contains(&SyntaxTokenKind::Keyword),
        "`-module` is a directive: {directive:?}"
    );

    // `%` is Erlang's line comment and nothing else in the tree uses it.
    let comment = token_kinds_for_line_fragment(doc, 3, ERLANG_FIXTURE[3], "Adds one");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`%%` starts an Erlang comment: {comment:?}"
    );

    let guard = token_kinds_for_line_fragment(doc, 4, ERLANG_FIXTURE[4], "when");
    assert!(
        guard.contains(&SyntaxTokenKind::Keyword),
        "`when` introduces a guard: {guard:?}"
    );

    let number = token_kinds_for_line_fragment(doc, 5, ERLANG_FIXTURE[5], "1");
    assert!(
        number.contains(&SyntaxTokenKind::Number),
        "`1` is a number: {number:?}"
    );
}

// ---- Haskell --------------------------------------------------------------

const HASKELL_FIXTURE: &[&str] = &[
    /*  0 */ "module Demo.Worker (run) where",
    /*  1 */ "",
    /*  2 */ "import Data.List (foldl')",
    /*  3 */ "",
    /*  4 */ "-- | Adds one.",
    /*  5 */ "run :: Int -> Int",
    /*  6 */ "run x = foldl' (+) 0 [x, 1]",
];

#[test]
fn prepared_haskell_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Haskell, &HASKELL_FIXTURE.join("\n"));

    for (line_ix, fragment, expected) in [
        (0usize, "module", SyntaxTokenKind::Keyword),
        (0, "where", SyntaxTokenKind::Keyword),
        (2, "import", SyntaxTokenKind::Keyword),
        (5, "Int", SyntaxTokenKind::Type),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, HASKELL_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }

    // Haddock's `-- |` is a documentation comment, not a plain one.
    let haddock = token_kinds_for_line_fragment(doc, 4, HASKELL_FIXTURE[4], "Adds one");
    assert!(
        haddock.contains(&SyntaxTokenKind::CommentDoc),
        "`-- |` opens a Haddock comment: {haddock:?}"
    );
}

// ---- Julia ----------------------------------------------------------------

const JULIA_FIXTURE: &[&str] = &[
    /*  0 */ "module Demo",
    /*  1 */ "",
    /*  2 */ "# Adds one.",
    /*  3 */ "function run(x::Int)::Int",
    /*  4 */ "    y = x + 1",
    /*  5 */ "    return y",
    /*  6 */ "end",
    /*  7 */ "",
    /*  8 */ "end",
];

#[test]
fn prepared_julia_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Julia, &JULIA_FIXTURE.join("\n"));

    let comment = token_kinds_for_line_fragment(doc, 2, JULIA_FIXTURE[2], "Adds one");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`#` is Julia's line comment: {comment:?}"
    );

    for (line_ix, fragment, expected) in [
        (3usize, "function", SyntaxTokenKind::Keyword),
        (3, "Int", SyntaxTokenKind::TypeBuiltin),
        (5, "return", SyntaxTokenKind::Keyword),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, JULIA_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }
}

// ---- OCaml ----------------------------------------------------------------

const OCAML_FIXTURE: &[&str] = &[
    /*  0 */ "(* Adds one. *)",
    /*  1 */ "let run (x : int) : int =",
    /*  2 */ "  let y = x + 1 in",
    /*  3 */ "  y",
];

const OCAML_INTERFACE_FIXTURE: &[&str] = &[
    /*  0 */ "(* Adds one. *)",
    /*  1 */ "val run : int -> int",
    /*  2 */ "",
    /*  3 */ "type t = { id : int }",
];

/// Both halves of the `.ml`/`.mli` pair, because they are separate grammars
/// sharing one query file. A change that compiles against the implementation
/// grammar can still fail against the interface one -- that is exactly why
/// `(shebang)` had to come out of the vendored copy.
#[test]
fn prepared_ocaml_documents_highlight_both_halves_of_the_pair() {
    let ml = prepare_test_document(DiffSyntaxLanguage::OCaml, &OCAML_FIXTURE.join("\n"));

    let comment = token_kinds_for_line_fragment(ml, 0, OCAML_FIXTURE[0], "Adds one");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`(* *)` is OCaml's only comment form: {comment:?}"
    );
    for (line_ix, fragment, expected) in [
        (1usize, "let", SyntaxTokenKind::Keyword),
        (1, "int", SyntaxTokenKind::TypeBuiltin),
        (2, "in", SyntaxTokenKind::Keyword),
    ] {
        let kinds = token_kinds_for_line_fragment(ml, line_ix, OCAML_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?} in a .ml: {kinds:?}"
        );
    }

    let mli = prepare_test_document(
        DiffSyntaxLanguage::OCamlInterface,
        &OCAML_INTERFACE_FIXTURE.join("\n"),
    );
    for (line_ix, fragment, expected) in [
        (1usize, "val", SyntaxTokenKind::Keyword),
        (3, "type", SyntaxTokenKind::Keyword),
        (3, "id", SyntaxTokenKind::Property),
        (3, "int", SyntaxTokenKind::TypeBuiltin),
    ] {
        let kinds =
            token_kinds_for_line_fragment(mli, line_ix, OCAML_INTERFACE_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?} in a .mli: {kinds:?}"
        );
    }
}

/// The reason queries/ocaml_highlights.scm exists rather than a reference to
/// `tree_sitter_ocaml::HIGHLIGHTS_QUERY`: upstream names `(shebang)`, which the
/// interface grammar has no rule for, and one unknown node type fails the whole
/// query rather than the pattern that names it.
#[test]
fn ocaml_query_serves_both_grammars_and_upstream_does_not() {
    for language in [
        tree_sitter_ocaml::LANGUAGE_OCAML,
        tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE,
    ] {
        tree_sitter::Query::new(&language.into(), OCAML_HIGHLIGHTS_QUERY)
            .expect("the vendored query should compile against both OCaml grammars");
    }

    assert!(
        tree_sitter::Query::new(
            &tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
            tree_sitter_ocaml::HIGHLIGHTS_QUERY,
        )
        .is_err(),
        "upstream's query now compiles against the interface grammar -- drop the \
             vendored copy and use `tree_sitter_ocaml::HIGHLIGHTS_QUERY` for both."
    );
}

// ---- Groovy ---------------------------------------------------------------

const GROOVY_FIXTURE: &[&str] = &[
    /*  0 */ "// Build config.",
    /*  1 */ "plugins {",
    /*  2 */ "    id 'java'",
    /*  3 */ "}",
    /*  4 */ "",
    /*  5 */ "class Demo {",
    /*  6 */ "    static int run(int x) {",
    /*  7 */ "        return x + 1",
    /*  8 */ "    }",
    /*  9 */ "}",
];

#[test]
fn prepared_groovy_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Groovy, &GROOVY_FIXTURE.join("\n"));

    let comment = token_kinds_for_line_fragment(doc, 0, GROOVY_FIXTURE[0], "Build config");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`//` is a Groovy line comment: {comment:?}"
    );

    // Single quotes are a plain string in Groovy, unlike the Haskell/OCaml/
    // Clojure arms added alongside it.
    let string = token_kinds_for_line_fragment(doc, 2, GROOVY_FIXTURE[2], "'java'");
    assert!(
        string.contains(&SyntaxTokenKind::String),
        "`'java'` is a string: {string:?}"
    );

    for (line_ix, fragment, expected) in [
        (5usize, "class", SyntaxTokenKind::Keyword),
        (6, "static", SyntaxTokenKind::Keyword),
        (6, "int", SyntaxTokenKind::TypeBuiltin),
        (7, "return", SyntaxTokenKind::KeywordControl),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, GROOVY_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }
}

// ---- Clojure --------------------------------------------------------------

const CLOJURE_FIXTURE: &[&str] = &[
    /*  0 */ "(ns demo.worker)",
    /*  1 */ "",
    /*  2 */ ";; Adds one.",
    /*  3 */ "(defn run [x]",
    /*  4 */ "  (let [y (+ x 1)]",
    /*  5 */ "    {:id y :name \"demo\"}))",
];

#[test]
fn prepared_clojure_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Clojure, &CLOJURE_FIXTURE.join("\n"));

    let comment = token_kinds_for_line_fragment(doc, 2, CLOJURE_FIXTURE[2], "Adds one");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "`;;` is Clojure's line comment: {comment:?}"
    );

    // The head-position rules in queries/clojure_highlights.scm. Upstream's
    // six-pattern query has none of this: without them a Clojure file is
    // literals and nothing else.
    for (line_ix, fragment, expected) in [
        (0usize, "ns", SyntaxTokenKind::Keyword),
        (3, "defn", SyntaxTokenKind::Keyword),
        (3, "run", SyntaxTokenKind::Function),
        (4, "let", SyntaxTokenKind::Keyword),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, CLOJURE_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }

    let keyword_literal = token_kinds_for_line_fragment(doc, 5, CLOJURE_FIXTURE[5], ":id");
    assert!(
        keyword_literal.contains(&SyntaxTokenKind::Constant),
        "`:id` is a keyword literal: {keyword_literal:?}"
    );
    let string = token_kinds_for_line_fragment(doc, 5, CLOJURE_FIXTURE[5], "\"demo\"");
    assert!(
        string.contains(&SyntaxTokenKind::String),
        "`\"demo\"` is a string: {string:?}"
    );
}

/// The quoting literals span the whole quoted form, so capturing the *node*
/// paints `'(alpha beta)` end to end. Upstream captures the one-character
/// marker instead. Capturing the node is cheap to reintroduce and invisible
/// without a test.
#[test]
fn clojure_quoted_form_paints_only_its_marker() {
    let line = "(def syms '(alpha beta))";
    let doc = prepare_test_document(DiffSyntaxLanguage::Clojure, line);

    let marker = token_kinds_for_line_fragment(doc, 0, line, "'");
    assert!(
        marker.contains(&SyntaxTokenKind::Operator),
        "the quote marker should be an operator: {marker:?}"
    );

    let quoted = token_kinds_for_line_fragment(doc, 0, line, "alpha");
    assert!(
        !quoted.contains(&SyntaxTokenKind::Operator)
            && !quoted.contains(&SyntaxTokenKind::PunctuationSpecial),
        "the quoted form itself should not take the marker's colour: {quoted:?}"
    );
}

/// queries/clojure_highlights.scm opens with a verbatim copy of the upstream
/// query and adds to it. A grammar bump that changes upstream leaves the copy
/// stale and silently diverging, which is the one failure mode a compile check
/// cannot see.
#[test]
fn clojure_highlights_query_embeds_the_upstream_base_verbatim() {
    let upstream = query_rule_lines(tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY);
    let vendored = query_rule_lines(CLOJURE_HIGHLIGHTS_QUERY);
    assert!(
        !upstream.is_empty(),
        "the upstream Clojure query should not be comment-only"
    );
    assert!(
        vendored
            .windows(upstream.len())
            .any(|window| window == upstream.as_slice()),
        "clojure_highlights.scm must contain tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY \
             as a contiguous, in-order block. Mirror the upstream change into the \
             `--- upstream ---` section.\nexpected block:\n{upstream:#?}\nvendored:\n{vendored:#?}"
    );
}

// ---- Solidity -------------------------------------------------------------

const SOLIDITY_FIXTURE: &[&str] = &[
    /*  0 */ "// SPDX-License-Identifier: MIT",
    /*  1 */ "pragma solidity ^0.8.0;",
    /*  2 */ "",
    /*  3 */ "contract Demo {",
    /*  4 */ "    uint256 public total;",
    /*  5 */ "",
    /*  6 */ "    function add(uint256 x) public returns (uint256) {",
    /*  7 */ "        total += x;",
    /*  8 */ "        return total;",
    /*  9 */ "    }",
    /* 10 */ "}",
];

#[test]
fn prepared_solidity_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Solidity, &SOLIDITY_FIXTURE.join("\n"));

    let comment = token_kinds_for_line_fragment(doc, 0, SOLIDITY_FIXTURE[0], "SPDX");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "the SPDX header is a comment: {comment:?}"
    );

    for (line_ix, fragment, expected) in [
        (1usize, "pragma", SyntaxTokenKind::Keyword),
        (3, "contract", SyntaxTokenKind::Keyword),
        (4, "uint256", SyntaxTokenKind::Type),
        (6, "function", SyntaxTokenKind::Keyword),
        (6, "returns", SyntaxTokenKind::Keyword),
        (8, "return", SyntaxTokenKind::Keyword),
    ] {
        let kinds =
            token_kinds_for_line_fragment(doc, line_ix, SOLIDITY_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }
}

/// If a grammar bump ships a query that compiles as-is, the vendored copy and
/// this test can both go.
#[test]
fn solidity_upstream_query_still_needs_the_vendored_fix() {
    assert!(
        tree_sitter::Query::new(
            &tree_sitter_solidity::LANGUAGE.into(),
            tree_sitter_solidity::HIGHLIGHT_QUERY,
        )
        .is_err(),
        "tree_sitter_solidity::HIGHLIGHT_QUERY now compiles -- drop \
             queries/solidity_highlights.scm and use the crate constant."
    );
}

// ---- Assembly -------------------------------------------------------------

const ASSEMBLY_FIXTURE: &[&str] = &[
    /*  0 */ "section .text",
    /*  1 */ "global run",
    /*  2 */ "run:",
    /*  3 */ "    mov eax, 1 ; seed",
    /*  4 */ "    add eax, edi",
    /*  5 */ "    ret",
];

#[test]
fn prepared_assembly_document_highlights_core_syntax() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Assembly, &ASSEMBLY_FIXTURE.join("\n"));

    for (line_ix, fragment, expected) in [
        (2usize, "run", SyntaxTokenKind::Label),
        (3, "mov", SyntaxTokenKind::Function),
        (3, "eax", SyntaxTokenKind::VariableBuiltin),
        (3, "1", SyntaxTokenKind::Number),
        (5, "ret", SyntaxTokenKind::Function),
    ] {
        let kinds =
            token_kinds_for_line_fragment(doc, line_ix, ASSEMBLY_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }

    // Trailing comments are the only comment position this grammar accepts;
    // see `assembly_standalone_comment_lines_fall_back_to_the_heuristic`.
    let comment = token_kinds_for_line_fragment(doc, 3, ASSEMBLY_FIXTURE[3], "; seed");
    assert!(
        comment.contains(&SyntaxTokenKind::Comment),
        "a trailing `;` comment should be greyed out: {comment:?}"
    );
}

/// A documented limitation, not a bug in the wiring: tree-sitter-asm only
/// admits a comment after an instruction, so a comment on its own line -- which
/// is most comments in real assembly -- puts the tree into error recovery.
///
/// Recovery is survivable (the instructions around it still highlight) and the
/// heuristic path, which this repo also runs for short lines and oversized
/// diffs, gets it right. The test pins both halves so a grammar bump that fixes
/// the parse shows up here rather than going unnoticed.
#[test]
fn assembly_standalone_comment_lines_fall_back_to_the_heuristic() {
    let source = "; set the return value\n    mov eax, 1\n    ret\n";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_asm::LANGUAGE.into())
        .expect("asm grammar should load");
    let tree = parser.parse(source, None).expect("asm should parse");
    assert!(
        tree.root_node().has_error(),
        "tree-sitter-asm now parses a standalone comment line -- update this test and \
             the note on the Assembly arm of heuristic_comment_config"
    );

    // The instructions after the bad line still highlight.
    let doc = prepare_test_document(DiffSyntaxLanguage::Assembly, source);
    let kinds = token_kinds_for_line_fragment(doc, 1, "    mov eax, 1", "mov");
    assert!(
        kinds.contains(&SyntaxTokenKind::Function),
        "error recovery should leave the following instructions intact: {kinds:?}"
    );

    // And the heuristic, which does not care what the grammar thinks, greys the
    // whole comment line.
    let heuristic = heuristic_tokens("; set the return value", DiffSyntaxLanguage::Assembly);
    assert!(
        heuristic
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "`;` is the heuristic's assembly line comment: {heuristic:?}"
    );
}

/// The identifier scanner starts on `_` or a letter, never `.`, so a GAS
/// directive reaches `is_keyword` as its bare tail, so a table spelling its
/// entries `".text"` and `".globl"` looks complete and matches nothing.
/// Nothing else in the suite would notice.
#[test]
fn assembly_gas_dot_directives_reach_the_keyword_table() {
    for (line, expected) in [
        ("    .text", "text"),
        ("    .data", "data"),
        ("    .bss", "bss"),
        ("    .globl main", "globl"),
        ("    .global main", "global"),
        ("    .align 4", "align"),
        ("    .long 1", "long"),
        ("    .quad 2", "quad"),
        ("    .short 3", "short"),
        ("    .byte 1", "byte"),
        ("    .word 1", "word"),
        ("    .ascii \"hi\"", "ascii"),
        ("    .section .rodata", "section"),
        // The NASM/MASM spellings, which carry no dot to begin with.
        ("section .text", "section"),
        ("global main", "global"),
        ("extern printf", "extern"),
    ] {
        let found = heuristic_keywords(line, DiffSyntaxLanguage::Assembly);
        assert!(
            found.contains(&expected),
            "{line:?} should yield the `{expected}` directive keyword: {found:?}"
        );
    }
}

/// `#` is an ARM immediate (`mov r0, #1`), not a comment. Giving the Assembly
/// arm `hash_comment: true` would grey out the operand of every such
/// instruction, which is why it shares nothing with the Python/Ruby arm.
#[test]
fn assembly_hash_immediate_is_not_a_comment() {
    let line = "    mov r0, #1";
    let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Assembly);
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "an ARM immediate was greyed out as a comment: {tokens:?}"
    );
}

// ---- Svelte ---------------------------------------------------------------

const SVELTE_FIXTURE: &[&str] = &[
    /*  0 */ "<script lang=\"ts\">",
    /*  1 */ "  let count: number = 0;",
    /*  2 */ "</script>",
    /*  3 */ "",
    /*  4 */ "{#if count > 0}",
    /*  5 */ "  <button class=\"btn\">{count}</button>",
    /*  6 */ "{:else}",
    /*  7 */ "  <p>none</p>",
    /*  8 */ "{/if}",
    /*  9 */ "",
    /* 10 */ "<style>",
    /* 11 */ "  .btn { color: red; }",
    /* 12 */ "</style>",
];

#[test]
fn prepared_svelte_document_highlights_markup_and_block_tags() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &SVELTE_FIXTURE.join("\n"));

    // The html base embedded in queries/svelte_highlights.scm. Without it the
    // upstream query colours the block markers and leaves the markup plain.
    for (line_ix, fragment, expected) in [
        (0usize, "script", SyntaxTokenKind::Tag),
        (0, "lang", SyntaxTokenKind::Attribute),
        (5, "button", SyntaxTokenKind::Tag),
        (5, "class", SyntaxTokenKind::Attribute),
        (5, "btn", SyntaxTokenKind::String),
    ] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, SVELTE_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&expected),
            "`{fragment}` should be {expected:?}: {kinds:?}"
        );
    }

    // The svelte half: `{#if}` / `{:else}` / `{/if}`.
    for (line_ix, fragment) in [(4usize, "if"), (6, "else"), (8, "if")] {
        let kinds = token_kinds_for_line_fragment(doc, line_ix, SVELTE_FIXTURE[line_ix], fragment);
        assert!(
            kinds.contains(&SyntaxTokenKind::Keyword),
            "the `{{{fragment}}}` block marker should be a keyword: {kinds:?}"
        );
    }
}

/// The script and style bodies are the bulk of a `.svelte` file and neither is
/// reachable from the highlights query -- they arrive as injections or not at
/// all. The `lang="ts"` veto is what keeps the default javascript rule from
/// firing over the same `raw_text`; see the note in svelte_injections.scm.
#[test]
fn svelte_script_and_style_blocks_inject_their_languages() {
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &SVELTE_FIXTURE.join("\n"));

    let script = token_kinds_for_line_fragment(doc, 1, SVELTE_FIXTURE[1], "let");
    assert!(
        script.contains(&SyntaxTokenKind::Keyword),
        "`<script lang=\"ts\">` should inject TypeScript: {script:?}"
    );

    let style = token_kinds_for_line_fragment(doc, 11, SVELTE_FIXTURE[11], "color");
    assert!(
        style.contains(&SyntaxTokenKind::Property),
        "`<style>` should inject CSS: {style:?}"
    );
}

/// The `lang` veto in svelte_injections.scm, which is the whole reason the two
/// default rules carry a `#not-match?`. Without it a `<script lang="ts">` body
/// matches the default javascript rule *and* the typescript one over the same
/// `raw_text`; live.rs keeps both layers and interleaves their captures at the
/// same depth, so the block comes out coloured by whichever wrote last.
#[test]
fn svelte_script_with_lang_injects_exactly_one_language() {
    let text = "<script lang=\"ts\">\nconst x: number = 1;\n</script>\n";

    let lang: tree_sitter::Language = tree_sitter_svelte_ng::LANGUAGE.into();
    let query = tree_sitter::Query::new(&lang, SVELTE_INJECTIONS_QUERY)
        .expect("vendored Svelte injections.scm should compile");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang)
        .expect("Svelte grammar should load");
    let tree = parser.parse(text, None).expect("script should parse");

    let mut cursor = tree_sitter::QueryCursor::new();
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    let mut patterns = Vec::new();
    {
        let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
        tree_sitter::StreamingIterator::advance(&mut matches);
        while let Some(m) = matches.get() {
            patterns.push(m.pattern_index);
            tree_sitter::StreamingIterator::advance(&mut matches);
        }
    }
    assert_eq!(
        patterns.len(),
        1,
        "a `<script lang=\"ts\">` must match exactly one injection pattern, \
             matched {patterns:?}"
    );

    // ...and it must be the TypeScript one, so the annotation is typed.
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
    let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
        .expect("script body should have prepared tokens");
    assert!(
        tokens
            .iter()
            .any(|t| t.kind == SyntaxTokenKind::Type || t.kind == SyntaxTokenKind::TypeBuiltin),
        "`lang=\"ts\"` should inject TypeScript, so `: number` should be typed: {tokens:?}"
    );
}

/// The trap vue_injections.scm documents: the default rule is vetoed by
/// `#not-match? "\\slang\\s*="` whenever *any* `lang` is present, so
/// enumerating the servable values with `#any-of?` means every unlisted one --
/// `lang="js"`, `lang="css"`, and the unquoted `lang=ts` -- falls into a gap
/// and the whole block renders with no highlighting at all.
///
/// Forwarding the value as `@injection.language` is what closes it. Asserting
/// `is_some()` would be vacuous here: the broken version returned
/// `Some(vec![])`, not `None`.
#[test]
fn svelte_lang_values_outside_the_default_still_inject() {
    for (open_tag, body, expected) in [
        // `js` and `css` name grammars we have but are not the default value
        // for their element -- the exact pair the enumerated version dropped.
        (
            "<script lang=\"js\">",
            "const x = 1;",
            SyntaxTokenKind::Keyword,
        ),
        (
            "<style lang=\"css\">",
            "  .b { color: red; }",
            SyntaxTokenKind::Property,
        ),
        // Resolved through the alias table rather than by name.
        (
            "<script lang=\"mts\">",
            "const x = 1;",
            SyntaxTokenKind::Keyword,
        ),
        (
            "<style lang=\"pcss\">",
            "  .b { color: red; }",
            SyntaxTokenKind::Property,
        ),
        // The unquoted form the grammar permits.
        ("<script lang=ts>", "const x = 1;", SyntaxTokenKind::Keyword),
    ] {
        let close = if open_tag.starts_with("<script") {
            "</script>"
        } else {
            "</style>"
        };
        let text = format!("{open_tag}\n{body}\n{close}\n");
        let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &text);
        let tokens = syntax_tokens_for_prepared_document_line(doc, 1)
            .expect("the block body should have prepared tokens");
        assert!(
            tokens.iter().any(|token| token.kind == expected),
            "`{open_tag}` should still inject: expected {expected:?}, got {tokens:?}"
        );
    }
}

/// The other half of the same trade-off: a `lang` no grammar here can serve
/// injects nothing, and that must not disturb the host grammar. Same contract
/// as `vue_unknown_lang_attribute_does_not_silently_disable_highlighting`.
#[test]
fn svelte_unservable_lang_leaves_the_markup_alone() {
    let text = "<script lang=\"coffee\">\nx = 1\n</script>\n";
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
    let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
        .expect("the opening tag should have prepared tokens");
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Tag),
        "an unservable lang must not disturb the host grammar: {tokens:?}"
    );
}

/// The bug in `tree_sitter_svelte_ng::INJECTIONS_QUERY` that svelte_injections.scm
/// exists to avoid: its bare `(raw_text)` catch-all matches the body of `<style>`
/// too, so a stylesheet gets parsed as JavaScript.
#[test]
fn svelte_style_block_injects_css_not_javascript() {
    let text = "<style>\n  .btn { color: red; }\n</style>\n";
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, text);
    let kinds = token_kinds_for_line_fragment(doc, 1, "  .btn { color: red; }", "color");
    assert!(
        kinds.contains(&SyntaxTokenKind::Property),
        "`color` is a CSS property, which the JavaScript grammar would never \
             produce: {kinds:?}"
    );
}

/// The counterpart to `vue_static_inline_styles_do_not_flood_the_injection_cache`.
/// A `.svelte` template injects per *expression*, not per file, so an ordinary
/// list render emits one layer per row. Without the bare-identifier guard in
/// svelte_injections.scm a 30-row list produced 30 cache entries against a cap
/// of 32, evicting everything else on its own.
#[test]
fn svelte_bare_identifier_expressions_do_not_flood_the_injection_cache() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());

    let mut lines = vec!["<ul>".to_string()];
    for ix in 0..30 {
        lines.push(format!("  <li>{{row{ix}}}</li>"));
    }
    lines.push("</ul>".to_string());
    let line_count = lines.len();

    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, &lines.join("\n"));
    for line_ix in 0..line_count {
        let _ = syntax_tokens_for_prepared_document_line(doc, line_ix);
    }

    let cached = TS_INJECTION_CACHE.with(|cache| cache.borrow().len());
    assert_eq!(
        cached, 0,
        "a bare identifier gains nothing from a JavaScript parse, but {cached} cache \
             entries were created from {line_count} lines (cap is \
             {TS_INJECTION_CACHE_MAX_ENTRIES})"
    );

    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}

/// ...and the guard must not be so broad that real expressions stop injecting.
#[test]
fn svelte_non_trivial_expressions_still_inject() {
    let line = "  <p>{count > 0 ? \"many\" : \"none\"}</p>";
    let doc = prepare_test_document(DiffSyntaxLanguage::Svelte, line);

    let kinds = token_kinds_for_line_fragment(doc, 0, line, "\"many\"");
    assert!(
        kinds.contains(&SyntaxTokenKind::String),
        "a real expression should still be parsed as JavaScript, so the string \
             literal inside it is a string: {kinds:?}"
    );
}

/// The same tripwire vue_highlights.scm carries, for the same reason: the
/// Svelte grammar is html-shaped, the base rules have to be present in the
/// file, and rule order decides which capture wins.
#[test]
fn svelte_highlights_query_embeds_the_html_base_verbatim() {
    let html_rules = query_rule_lines(HTML_HIGHLIGHTS_QUERY);
    let svelte_rules = query_rule_lines(SVELTE_HIGHLIGHTS_QUERY);
    assert!(
        !html_rules.is_empty(),
        "html_highlights.scm should not be comment-only"
    );
    assert!(
        svelte_rules
            .windows(html_rules.len())
            .any(|window| window == html_rules.as_slice()),
        "svelte_highlights.scm must contain queries/html_highlights.scm as a contiguous, \
             in-order block. Mirror the change into the `--- html base ---` section.\n\
             expected block:\n{html_rules:#?}\nsvelte rules:\n{svelte_rules:#?}"
    );
}

// ---- Heuristic fallback for the batch --------------------------------------

/// Three of the new languages spell something other than a string with `'`:
/// Haskell primes identifiers, OCaml opens type variables, Clojure quotes
/// forms. Left as `HeuristicSingleQuote::String` each one runs a string from
/// the tick to the end of the line -- the Nix bug, three more times.
#[test]
fn batch_apostrophes_do_not_open_a_string() {
    for (line, language) in [
        ("run xs = foldl' (+) 0 xs", DiffSyntaxLanguage::Haskell),
        ("let ids : 'a list = []", DiffSyntaxLanguage::OCaml),
        (
            "val map : ('a -> 'b) -> 'a list -> 'b list",
            DiffSyntaxLanguage::OCamlInterface,
        ),
        ("(def syms '(alpha beta))", DiffSyntaxLanguage::Clojure),
    ] {
        assert!(
            heuristic_string_spans(line, language).is_empty(),
            "an apostrophe opened a string in {language:?} line {line:?}: {:?}",
            heuristic_tokens(line, language)
        );
    }

    // Double quotes still work everywhere.
    assert_eq!(
        heuristic_string_spans("  name = \"demo\"", DiffSyntaxLanguage::Haskell),
        vec!["\"demo\""]
    );
}

/// Julia is the one language in the batch where `'` is both: `A'` is the
/// adjoint operator and `'c'` is a character literal. ValuePositionOnly tells
/// them apart by what precedes the tick.
#[test]
fn julia_adjoint_is_not_a_string_but_a_char_literal_is() {
    assert!(
        heuristic_string_spans("    b = A' * x", DiffSyntaxLanguage::Julia).is_empty(),
        "the adjoint operator opened a string: {:?}",
        heuristic_tokens("    b = A' * x", DiffSyntaxLanguage::Julia)
    );
    assert_eq!(
        heuristic_string_spans("    c = 'x'", DiffSyntaxLanguage::Julia),
        vec!["'x'"],
        "a character literal in value position is still a string"
    );
}

/// Every comment form the batch introduced. The heuristic runs in production
/// for lines past MAX_TREESITTER_LINE_BYTES and in HeuristicOnly mode, and
/// these arms are reached by nothing else.
#[test]
fn batch_heuristic_comment_forms_are_covered() {
    for (line, language) in [
        ("-- | Adds one.", DiffSyntaxLanguage::Haskell),
        ("{- block -}", DiffSyntaxLanguage::Haskell),
        ("%% Adds one.", DiffSyntaxLanguage::Erlang),
        (";; Adds one.", DiffSyntaxLanguage::Clojure),
        ("(* Adds one. *)", DiffSyntaxLanguage::OCaml),
        ("(* Adds one. *)", DiffSyntaxLanguage::OCamlInterface),
        ("# Adds one.", DiffSyntaxLanguage::Elixir),
        ("# Adds one.", DiffSyntaxLanguage::Julia),
        ("// Adds one.", DiffSyntaxLanguage::Groovy),
        ("/* Adds one. */", DiffSyntaxLanguage::Solidity),
        ("    ret ; done", DiffSyntaxLanguage::Assembly),
        ("<!-- Adds one. -->", DiffSyntaxLanguage::Svelte),
    ] {
        let tokens = heuristic_tokens(line, language);
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{language:?} should treat {line:?} as a comment: {tokens:?}"
        );
    }

    // Haskell's `--` must not swallow an operator section: `x -- y` is a
    // comment, but the heuristic has no way to know that `--` in
    // `f -->> g` is not one either. Pin the ordinary case only.
    let subtraction = heuristic_tokens("    y = x - 1", DiffSyntaxLanguage::Haskell);
    assert!(
        !subtraction
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "a single `-` is not a Haskell comment: {subtraction:?}"
    );
}

/// Per the Haskell report a run of dashes is a comment only when it is *not*
/// followed by a symbol character; otherwise the whole run is an operator.
/// `line_comment: Some("--")` cannot express that, so it greyed `a --> b` from
/// the dashes to the end of the line -- the worst failure mode this path has,
/// because it hides code rather than mis-colouring it.
#[test]
fn haskell_operator_sections_starting_with_dashes_are_not_comments() {
    for line in [
        "  step = a --> b",
        "  merged = xs --| ys",
        "  shifted = a --< b",
        "  chained = f --. g",
    ] {
        let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Haskell);
        assert!(
            !tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "an operator section was greyed out as a comment in {line:?}: {tokens:?}"
        );
    }

    // ...while every genuine comment still is one, including the `---` run that
    // the naive "any symbol after `--`" rule would have broken.
    for line in [
        "-- plain",
        "--- ruled off",
        "  x = 1 -- trailing",
        "-- | haddock",
    ] {
        let tokens = heuristic_tokens(line, DiffSyntaxLanguage::Haskell);
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{line:?} is a Haskell comment: {tokens:?}"
        );
    }
}

/// Neighbouring entries in four of the keyword tables, each gap visible as two
/// adjacent lines highlighting differently.
#[test]
fn batch_keyword_tables_cover_their_neighbours() {
    for (line, language, expected) in [
        // `let ... in` is one form; highlighting half of it looked like a bug.
        ("  let x = 1 in x + 1", DiffSyntaxLanguage::Haskell, "in"),
        // Erlang's word-spelled operators are reserved words.
        ("  Y = X div 2,", DiffSyntaxLanguage::Erlang, "div"),
        ("  Z = X band 255,", DiffSyntaxLanguage::Erlang, "band"),
        ("  ok = not Flag,", DiffSyntaxLanguage::Erlang, "not"),
        // Groovy had `boolean` but none of the other primitives.
        ("    int x = 1", DiffSyntaxLanguage::Groovy, "int"),
        ("    double d = 1.0", DiffSyntaxLanguage::Groovy, "double"),
        ("    char c = 'x'", DiffSyntaxLanguage::Groovy, "char"),
        // Solidity's two block forms.
        (
            "        assembly { let p := 1 }",
            DiffSyntaxLanguage::Solidity,
            "assembly",
        ),
        (
            "        unchecked { x += 1; }",
            DiffSyntaxLanguage::Solidity,
            "unchecked",
        ),
    ] {
        let found = heuristic_keywords(line, language);
        assert!(
            found.contains(&expected),
            "{language:?} should treat `{expected}` in {line:?} as a keyword: {found:?}"
        );
    }
}

/// The other half of the Solidity fix: sized types are *uniformly* absent now.
/// Listing `uint256` alone meant `uint256 total;` highlighted and `uint8 flags;`
/// two lines below it did not.
#[test]
fn solidity_sized_types_are_uniformly_absent_from_the_keyword_table() {
    for line in [
        "    uint8 a;",
        "    uint256 b;",
        "    int128 c;",
        "    bytes32 d;",
    ] {
        let found = heuristic_keywords(line, DiffSyntaxLanguage::Solidity);
        assert!(
            found.is_empty(),
            "sized types should all behave alike on the heuristic path, but {line:?} \
                 yielded {found:?}"
        );
    }

    // The base names still resolve, so the arm is not simply dead.
    assert!(heuristic_keywords("    uint x;", DiffSyntaxLanguage::Solidity).contains(&"uint"));
    assert!(
        heuristic_keywords("    address owner;", DiffSyntaxLanguage::Solidity).contains(&"address")
    );
}

/// The eleven keyword tables the batch added to `is_keyword`, none of which any
/// other test reaches: every other test in this section goes through
/// `prepare_test_document`, i.e. tree-sitter.
#[test]
fn batch_heuristic_keyword_tables_are_covered() {
    for (line, language, expected) in [
        (
            "class Demo extends Base {",
            DiffSyntaxLanguage::Groovy,
            "class",
        ),
        ("(defn run [x] x)", DiffSyntaxLanguage::Clojure, "defn"),
        ("defmodule Demo do", DiffSyntaxLanguage::Elixir, "defmodule"),
        (
            "run(X) when is_integer(X) ->",
            DiffSyntaxLanguage::Erlang,
            "when",
        ),
        (
            "newtype Wrapper = Wrapper Int",
            DiffSyntaxLanguage::Haskell,
            "newtype",
        ),
        ("mutable struct Point", DiffSyntaxLanguage::Julia, "struct"),
        ("let rec loop n =", DiffSyntaxLanguage::OCaml, "rec"),
        (
            "val run : int -> int",
            DiffSyntaxLanguage::OCamlInterface,
            "val",
        ),
        (
            "contract Demo is Base {",
            DiffSyntaxLanguage::Solidity,
            "contract",
        ),
        ("section .text", DiffSyntaxLanguage::Assembly, "section"),
        ("{#each items as item}", DiffSyntaxLanguage::Svelte, "each"),
    ] {
        let found = heuristic_keywords(line, language);
        assert!(
            found.contains(&expected),
            "{language:?} should treat `{expected}` in {line:?} as a keyword: {found:?}"
        );
    }
}
