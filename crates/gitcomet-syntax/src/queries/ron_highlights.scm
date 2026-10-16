; RON.
;
; Upstream's queries/highlights.scm from https://github.com/amaanq/tree-sitter-ron (MIT),
; the release vendor/tree-sitter-ron is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.

; Structs
;------------

(enum_variant) @constant
(struct_entry (identifier) @property)
(struct_entry (enum_variant (identifier) @constant))
(struct_name (identifier)) @type

(unit_struct) @type.builtin


; Literals
;------------

(string) @string
(boolean) @boolean
(integer) @number
(float) @float
(char) @character


; Comments
;------------

[
  (line_comment)
  (block_comment)
] @comment @spell


; Punctuation
;------------

["{" "}"] @punctuation.bracket

["(" ")"] @punctuation.bracket

["[" "]"] @punctuation.bracket

[
  ","
  ":"
] @punctuation.delimiter

[
  "-"
] @operator

; Special
;------------

(escape_sequence) @string.escape
(ERROR) @error
