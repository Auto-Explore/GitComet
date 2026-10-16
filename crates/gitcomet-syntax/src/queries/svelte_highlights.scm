; Derived from nvim-treesitter's queries for tree-sitter-grammars/tree-sitter-svelte
; (`queries/svelte/highlights.scm`, MIT), which the crate also exports as
; `tree_sitter_svelte_ng::HIGHLIGHTS_QUERY`.
;
; That constant starts with `; inherits: html`, an nvim-treesitter directive that
; has no equivalent here: TreesitterQueryAsset takes a single query source. Used
; as-is it colours the block markers (`{#if}`, `{:else}`) and nothing else -- no
; tags, no attributes, no strings -- because those rules live in the html file it
; expects to be prepended. So the `--- html base ---` section below is a verbatim
; copy of queries/html_highlights.scm, exactly as vue_highlights.scm does it, and
; `svelte_highlights_query_embeds_the_html_base_verbatim` keeps it honest.
;
; Local changes vs upstream:
;   - `"const" @type.qualifier` is `@keyword` here. `type.qualifier` resolves
;     through syntax_kind_from_capture_name to SyntaxTokenKind::Type, so `{@const}`
;     came out in the type colour while every other block marker was a keyword.
;   - `(raw_text) @none` dropped. `none` emits no token in this engine, so the
;     rule is inert; the script and style bodies are coloured by the injections in
;     svelte_injections.scm instead.
;   - `(snippet_name) @function` added. A snippet declaration reads as a function
;     definition and upstream leaves the name unpainted.

; --- html base -------------------------------------------------------------

(tag_name) @tag
(erroneous_end_tag_name) @tag.error
(doctype) @constant
(attribute_name) @attribute
(attribute_value) @string
(comment) @comment

[
  "<"
  ">"
  "</"
  "/>"
] @punctuation.bracket

; --- svelte ----------------------------------------------------------------

[
  "as"
  "key"
  "html"
  "snippet"
  "render"
  "const"
] @keyword

[
  "if"
  "else"
  "then"
] @keyword.conditional

"each" @keyword.repeat

[
  "await"
  "then"
] @keyword.coroutine

"catch" @keyword.exception

"debug" @keyword.debug

(snippet_name) @function

[
  "{"
  "}"
] @punctuation.bracket

[
  "#"
  ":"
  "/"
  "@"
] @tag.delimiter
