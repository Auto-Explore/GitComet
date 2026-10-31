# Razor Grammar

This crate parses C# Razor views and Blazor components for GitComet syntax highlighting.

## Provenance and local changes

The MIT upstream is `alfpricot/tree-sitter-razor-modern` at
`0f2ac3c7eb05b3cf41ff2370c42d8d2d6f2b4319` (0.1.2). Its original license,
notice, and complete grammar test corpus are retained. The grammar imports
GitComet's sibling `tree-sitter-c-sharp/grammar.js`; the scanner is rebased on
that sibling's C# 14 scanner, including its lambda lookahead token and rollback
behavior. The C# copyright and MIT terms are in `../tree-sitter-c-sharp/LICENSE`.

Local patches cover:

- C# 14 declarations, raw strings, escaped identifiers, and Unicode identifiers.
- Implicit member, call, index, and null-conditional chains with Razor's
  whitespace and generic-angle-bracket boundaries.
- Structured directives, asynchronous control blocks, switch patterns,
  nested markup, legacy `@helper`, and inline `@<...>` templates.
- Blazor expression attributes and literal binding modifiers, with separate
  text leaves so nested Razor expressions retain their ownership.
- HTML void tags, prefixed Tag Helpers, qualified components, opt-out `!`,
  raw script/style bodies, HTML comments, line markup, and escaped `@@`.
- Unterminated Razor comments and visible tag-name terminals during incomplete
  tag recovery. Markup resembling C# comments stays markup.

Scanner identifier tables are extracted from the generated C# identifier lexer
by `regenerate.py`. Character classification uses those Unicode tables and
explicit whitespace ranges, without process locale or global mutable state.
Non-ASCII scalars cannot alias ASCII keywords when filling lookahead buffers.

## Ownership contract

The root parser owns Razor and C#. GitComet combines the source ranges of
`element`, `html_comment`, `html_doctype`, `attribute_value_unquoted`,
`html_text`, `razor_line_markup`, `razor_raw_markup`, and `escaped_at` into an
HTML injection after excluding each node's direct named children. Nested markup
is captured separately by the same query pattern, restoring its source ranges.
`escaped_at` owns exactly the second `@`; `razor_tag_opt_out` owns `!` and is
excluded from HTML. Anonymous markup tokens retain whitespace and delimiters.
Changing these node boundaries requires updating the engine's injection query
and exact byte-range regressions together.

Highlight and injection queries are owned by `crates/gitcomet-syntax`, so the
CLI metadata intentionally contains no query paths. Semantic component parameter
types and Tag Helper resolution require project metadata and are outside this
syntax parser. Legacy `@helper` parses for highlighting, although modern Razor
compilers reject it. Invalid implicit generic calls retain explicit ERROR trees
in the two original upstream tests; their inputs are unchanged.

## Regeneration and verification

Use tree-sitter CLI 0.26.5 with the generator patch documented in
`../README.md` under “Regenerating a grammar”. Python 3.10 or newer and Node.js
are required for regeneration. The script forces threshold 128 and rejects a
stock generator's larger table, derives the Unicode header, and removes private
Tree-sitter headers in favor of the shared vendor headers.

At the pinned grammar revision the retune changes `LARGE_STATE_COUNT` from
4,121 to 2,818 and generated C size from 34,000,664 to 32,457,530 bytes. The
12,041 states, 702 symbols, 294 tokens, 40 external tokens, and 250 production
IDs are unchanged. The script enforces the measured 2,818 large-state bound;
the threshold 128 is a row-size threshold, not a bound on the number of rows.

```sh
python3 regenerate.py /path/to/patched/tree-sitter
CFLAGS=-I"$(pwd)/../tree-sitter-headers" /path/to/patched/tree-sitter test --rebuild
cargo test --manifest-path Cargo.toml
```

The CLI corpus pins full trees. Rust tests additionally pin exact source spans
for transitions, nested language boundaries, Unicode, comments, and recovery.
The compiler-checked application fixtures and highlighting expectations live in
`fixtures/syntax_test/templates/razor` and `crates/gitcomet-syntax/src/tests`.
Building this crate uses the checked-in ABI 15 parser and does not run Node.js,
Python, the CLI, or .NET.
