; Derived from nvim-treesitter's queries for
; tree-sitter-grammars/tree-sitter-vue (`queries/vue/highlights.scm`, MIT).
;
; The Vue grammar inherits html, so `<template>` is parsed natively here rather
; than through an injection. That means the html base rules have to be present
; in this file: the `--- html base ---` section below is a verbatim copy of
; queries/html_highlights.scm -- itself derived from
; gpui-component/crates/ui/src/highlighter/languages/html/highlights.scm
; (Apache-2.0) -- because TreesitterQueryAsset takes a single query source and
; nvim-treesitter's `; inherits:` directive has no equivalent here. The copy is
; kept honest by `vue_highlights_query_embeds_the_html_base_verbatim`.
;
; Local changes vs upstream:
;   - dropped `(#set! @_template bo.commentstring ...)`, a Neovim-only directive.
;   - directive expression values are captured as @variable rather than
;     @punctuation.special/@none, so the html `(attribute_value) @string` rule
;     does not colour the first injected token as a string. Both the quoted and
;     the unquoted form are covered; the grammar allows `v-if=ok`.
;   - `:` and `.` use @punctuation.special rather than upstream's
;     @character.special. Both resolve to the same SyntaxTokenKind, but
;     `character.special` is a capture F# and Swift also use, and teaching the
;     shared table about it would have recoloured their wildcards.
;   - `(directive_name)` uses @attribute rather than upstream's @tag.attribute,
;     for the same reason: `tag.attribute` is Scala's XML-literal capture.
;   - `#` (the v-slot shorthand) is captured alongside the other sigils.
;     Upstream leaves it out, which renders it invisible next to a highlighted
;     `:` or `@` on the same tag.
;   - the interpolation delimiters `{{` and `}}` are captured instead of the
;     whole `(interpolation)` node. Capturing the node paints the expression
;     inside it too: upstream relies on a companion `(raw_text) @none` rule to
;     punch it back out, but `none` emits no token in this engine, so the outer
;     capture simply wins and `{{ msg }}` renders entirely in the sigil colour.

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

; --- vue -------------------------------------------------------------------

[
  "["
  "]"
] @punctuation.bracket

; Directive sigils and interpolation delimiters.
[
  ":"
  "."
  "@"
  "#"
  "{{"
  "}}"
] @punctuation.special

(dynamic_directive_inner_value) @variable

(directive_name) @attribute

; Accessing a component object's field
(":"
  .
  (directive_value) @variable.member)

("."
  .
  (directive_value) @property)

; @click is like onclick for HTML
("@"
  .
  (directive_value) @function.method)

; Used in v-slot, declaring position the element should be put in
("#"
  .
  (directive_value) @variable)

; Override the html `(attribute_value) @string` rule for directive expressions.
; The TypeScript injection highlights the expression content; without this
; override the html rule colours the first injected token as a string. The
; second arm covers the unquoted form, which the grammar permits and which
; would otherwise be the one case that still renders as a string.
(directive_attribute
  (quoted_attribute_value
    (attribute_value) @variable))

(directive_attribute
  (attribute_value) @variable)

(directive_modifier) @function.method
