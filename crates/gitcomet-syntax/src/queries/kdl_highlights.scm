; KDL.
;
; Upstream's queries/highlights.scm from https://github.com/tree-sitter-grammars/tree-sitter-kdl (MIT),
; the release vendor/tree-sitter-kdl is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.
;
; Written against KDL 2.0.0, which still parses KDL 1 documents. Three things
; the v1 query did are impossible here:
;
;   * `annotation_type` is gone -- every annotation is now `(type)`, so the
;     builtin-vs-user split goes with it. Restoring it would need `#any-of?`,
;     and GitComet's highlighter does not evaluate text predicates, so the rule
;     would paint every annotation instead of the builtin ones.
;   * `node_children_comment` now wraps `node_children` rather than sitting
;     inside it, so `(node_children (node_children_comment))` never matches.
;   * `+` and `-` are no longer separate tokens.
;
; Every `node` pattern is anchored on the `name:` field on purpose. Matching a
; `node` child positionally instead (`(node (type))`) makes this query take
; seconds to compile, and `(node (node_field ...))` takes minutes -- see
; `kdl_highlights_query_compiles_promptly` in the syntax tests.

; Types

(node name: (identifier) @type)

(type) @type

; Properties

(prop key: (identifier) @property)

; Variables

(identifier) @variable

; Operators

"=" @operator

; Literals

(string) @string

[
  (escape)
  (escaped_whitespace)
] @string.escape

(number) @number

(number (decimal) @float)
(number (exponent) @float)

(boolean) @boolean

[
  "null"
  "#null"
] @constant.builtin

; Punctuation

["{" "}"] @punctuation.bracket

["(" ")"] @punctuation.bracket

[
  ";"
] @punctuation.delimiter

; Comments

[
  (single_line_comment)
  (multi_line_comment)
] @comment @spell

; The `/- kdl-version 2` marker a v2 document may open with.

(version) @comment

; `/-` comments out the node, field or child block that follows it.

[
  (node_comment)
  (node_field_comment)
  (node_children_comment)
] @comment
