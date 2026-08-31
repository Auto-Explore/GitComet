use super::*;

// ---- Regression net for the batch ------------------------------------------

/// Every language the batch added, with a snippet that exercises its comment
/// form, its string form and one keyword.
///
/// Shared by the invariant sweeps below so a new language is added in one place
/// and picked up by all of them.
fn batch_language_samples() -> Vec<(DiffSyntaxLanguage, &'static str)> {
    Vec::from([
        (
            DiffSyntaxLanguage::Groovy,
            "class D { def s = 'x' } // note",
        ),
        (DiffSyntaxLanguage::Clojure, "(defn f [x] \"s\") ;; note"),
        (DiffSyntaxLanguage::Elixir, "def f(x), do: \"s\" # note"),
        (DiffSyntaxLanguage::Erlang, "f(X) -> \"s\". % note"),
        (DiffSyntaxLanguage::Haskell, "f x = \"s\" -- note"),
        (DiffSyntaxLanguage::Julia, "f(x) = \"s\" # note"),
        (DiffSyntaxLanguage::OCaml, "let f x = \"s\" (* note *)"),
        (
            DiffSyntaxLanguage::OCamlInterface,
            "val f : int -> int (* note *)",
        ),
        (
            DiffSyntaxLanguage::Solidity,
            "function f() { s = \"x\"; } // note",
        ),
        (DiffSyntaxLanguage::Assembly, "    mov eax, 1 ; note"),
        (
            DiffSyntaxLanguage::Svelte,
            "<p class=\"c\">x</p> <!-- note -->",
        ),
    ])
}

/// The `potential_open_state_lead` fast-skip decides which bytes are even worth
/// examining, and a language whose comment lead is missing from it has its
/// comments run past entirely on the streamed path. Haskell's `-` had to be
/// added there when `line_comment` became None for it.
///
/// The long body is not padding. Below the checkpoint threshold the streamed
/// entry point hands the visible region straight to the per-line tokenizer, so a
/// short-line version of this test exercises the scanner not at all and passes
/// with the fast-skip entry deleted.
///
/// Each case puts the comment opener *before* the slice, so the token can only
/// be right if the scanner resumed in the comment state.
#[test]
fn streamed_slices_resume_inside_batch_line_comments() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;

    for (language, opener) in [
        (DiffSyntaxLanguage::Haskell, "-- "),
        (DiffSyntaxLanguage::Erlang, "% "),
        (DiffSyntaxLanguage::Clojure, "; "),
        (DiffSyntaxLanguage::Assembly, "    ret ; "),
        (DiffSyntaxLanguage::Groovy, "// "),
        (DiffSyntaxLanguage::Elixir, "# "),
    ] {
        reset_streamed_heuristic_line_cache();

        let body = "note ".repeat(CHECKPOINT_SPACING / 5 + 64);
        let text = format!("{opener}{body}");
        let slice_start = opener.len() + CHECKPOINT_SPACING;
        let slice_end = slice_start + 128;
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text.clone()));
        let (slice_text, resolved) = raw_text
            .slice_text_resolved(slice_start..slice_end)
            .expect("ASCII slice should resolve");

        let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
            &raw_text,
            language,
            slice_start..slice_end,
            resolved,
        )
        .expect("streamed slice should be supported");
        assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{language:?}: a slice {CHECKPOINT_SPACING} bytes into a `{opener}` comment \
                 must still be a comment, got {tokens:?}"
        );
    }
}

/// The two block-comment kinds the batch touched: Haskell's `{- -}` is a new
/// `HeuristicBlockCommentKind`, and OCaml reuses the F# `(* *)` spec. Both are
/// resumed from a checkpoint here, which is the only place the start/end byte
/// tables are consulted rather than the per-line `starts_with`.
#[test]
fn streamed_slices_resume_inside_haskell_and_ocaml_block_comments() {
    const CHECKPOINT_SPACING: usize = 32 * 1024;

    for (language, open, close) in [
        (DiffSyntaxLanguage::Haskell, "{-", "-}"),
        (DiffSyntaxLanguage::OCaml, "(*", "*)"),
        (DiffSyntaxLanguage::OCamlInterface, "(*", "*)"),
    ] {
        reset_streamed_heuristic_line_cache();

        let body = "b".repeat(CHECKPOINT_SPACING + 192);
        let text = format!("{open}{body}{close} let x = 1");
        let slice_start = open.len() + CHECKPOINT_SPACING;
        let slice_end = slice_start + 96;
        let raw_text = gitcomet_core::file_diff::FileDiffLineText::shared(Arc::from(text.clone()));
        let (slice_text, resolved) = raw_text
            .slice_text_resolved(slice_start..slice_end)
            .expect("ASCII slice should resolve");

        let tokens = syntax_tokens_for_streamed_line_slice_heuristic(
            &raw_text,
            language,
            slice_start..slice_end,
            resolved,
        )
        .expect("streamed slice should be supported");
        assert_token_ranges_are_utf8_safe(slice_text.as_ref(), &tokens);

        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{language:?}: a slice inside a `{open} {close}` block must still be a \
                 comment, got {tokens:?}"
        );

        // …and the block has to *end*. Asserting only the line above passes even
        // with `heuristic_block_comment_end_bytes` corrupted: an unterminated
        // comment swallows the rest of the line, so the slice above stays inside
        // it either way.
        let tail_start = text.find(close).expect("close should be present") + close.len();
        let (tail_text, tail_resolved) = raw_text
            .slice_text_resolved(tail_start..text.len())
            .expect("tail slice should resolve");
        let tail = syntax_tokens_for_streamed_line_slice_heuristic(
            &raw_text,
            language,
            tail_start..text.len(),
            tail_resolved,
        )
        .expect("streamed tail slice should be supported");
        assert_token_ranges_are_utf8_safe(tail_text.as_ref(), &tail);
        assert!(
            !tail
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{language:?}: `{close}` must close the block, but the code after it is \
                 still a comment: {tail:?}"
        );
    }
}

/// Token ranges are used to slice the line for rendering, so an out-of-bounds or
/// mid-codepoint range panics rather than mis-colouring. The batch added eleven
/// languages to a hand-written scanner; these are the inputs that break scanners.
#[test]
fn batch_languages_emit_well_formed_tokens_on_hostile_input() {
    let hostile = [
        "",
        " ",
        "\t",
        // Unterminated everything.
        "\"unterminated",
        "'unterminated",
        "`unterminated",
        "/* unterminated",
        "{- unterminated",
        "(* unterminated",
        "<!-- unterminated",
        // Bare openers at end of line, where a lookahead can run past the end.
        "-",
        "--",
        "/",
        "//",
        "{",
        "(",
        "#",
        ";",
        "%",
        "\\",
        "\"",
        "'",
        // Multi-byte, including a comment opener immediately before one.
        "-- ✨ é 日本語",
        "x = \"日本語\" -- ✨",
        "'é'",
        "«»‹›",
        // Adjacent delimiters.
        "\"\"''``",
        "/*/*/*",
        "{-{-{-",
        "(*(*(*",
        "-->--|--<",
        "REM",
        "rem\tx",
    ];

    for (language, _) in batch_language_samples() {
        for line in hostile {
            let tokens = syntax_tokens_for_line(line, language, DiffSyntaxMode::HeuristicOnly);
            assert_token_ranges_are_utf8_safe(line, &tokens);

            // Ranges must also be ordered and non-overlapping: the renderer walks
            // them with a single forward cursor.
            let mut previous_end = 0usize;
            for token in tokens.iter() {
                assert!(
                    token.range.start >= previous_end,
                    "{language:?} emitted overlapping or unsorted tokens for {line:?}: \
                         {tokens:?}"
                );
                previous_end = token.range.end;
            }
        }
    }
}

/// `heuristic_comment_range` now delegates to `line_comment_start_len`, which
/// tests `is_ascii_whitespace()` where the old copy compared against a literal
/// `"rem "`. Visual Basic is the only caller that notices, and it is not a
/// language the batch touched -- exactly the kind of bystander a refactor
/// breaks quietly.
#[test]
fn visual_basic_rem_comment_survives_the_shared_comment_decision() {
    for line in ["REM note", "rem note", "Rem note", "REM\tnote"] {
        let tokens = heuristic_tokens(line, DiffSyntaxLanguage::VisualBasic);
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxTokenKind::Comment),
            "{line:?} is a Visual Basic REM comment: {tokens:?}"
        );
    }

    // `REM` still needs a delimiter: `REMARK` is an identifier.
    let tokens = heuristic_tokens("REMARK = 1", DiffSyntaxLanguage::VisualBasic);
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxTokenKind::Comment),
        "`REMARK` is not a REM comment: {tokens:?}"
    );
}

/// A completeness sweep rather than a behaviour check: every language that
/// claims a grammar must actually produce tokens for a line of itself. A
/// mis-wired grammar, a query that compiles but matches nothing, or an enum
/// variant wired to the wrong `LANGUAGE` constant all show up here as silence.
#[test]
fn every_batch_language_produces_treesitter_tokens() {
    for (language, sample) in batch_language_samples() {
        assert!(
            tree_sitter_grammar(language).is_some(),
            "{language:?} should have a grammar"
        );

        let doc = prepare_test_document(language, sample);
        let tokens = syntax_tokens_for_prepared_document_line(doc, 0)
            .unwrap_or_else(|| panic!("{language:?} should produce prepared tokens"));
        assert!(
            !tokens.is_empty(),
            "{language:?} produced no tokens for {sample:?} -- the grammar is wired but \
                 its query matches nothing"
        );
        assert_token_ranges_are_utf8_safe(sample, &tokens);
    }
}
