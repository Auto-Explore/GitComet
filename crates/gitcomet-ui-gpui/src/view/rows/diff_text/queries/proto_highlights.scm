; Protocol Buffers.
;
; Authored here: `tree-sitter-proto` 0.4.0 ships a parser and node types but no
; highlights query at all.
;
; The grammar spells the scalar types (`int32`, `string`, `bytes`, ...) and the
; declaration words as *anonymous* tokens, so most of this is a literal list
; rather than node patterns -- the same shape as queries/wat_highlights.scm.

(comment) @comment

(string) @string
(escape_sequence) @string.escape

; `syntax = "proto3"` -- the grammar spells the two legal values as literal
; tokens, quotes included, rather than as `string` nodes, so the most prominent
; string in every `.proto` file needs naming separately.
[
  "\"proto2\""
  "\"proto3\""
] @string

[
  (int_lit)
  (float_lit)
  (hex_lit)
  (octal_lit)
  (decimal_lit)
] @number

[
  (true)
  (false)
] @boolean

; The names a schema declares. These are what a reader scans for.
(message_name (identifier) @type)
(enum_name (identifier) @type)
(service_name (identifier) @type)
(rpc_name (identifier) @function)

; A field's own name, and the number that is its wire identity.
(field (identifier) @property)
(oneof_field (identifier) @property)
(map_field (identifier) @property)
(enum_field (identifier) @constant)
(field_number) @number

; A reference to another message or enum.
(message_or_enum_type) @type

; `option java_package = "..."`, `[deprecated = true]`.
(option (full_ident) @property)
(field_option (full_ident) @property)

; Declaration keywords.
[
  "syntax"
  "edition"
  "package"
  "import"
  "public"
  "weak"
  "option"
  "message"
  "enum"
  "service"
  "rpc"
  "returns"
  "stream"
  "extend"
  "extensions"
  "oneof"
  "reserved"
  "to"
  "max"
] @keyword

; Field labels.
[
  "optional"
  "repeated"
  "required"
] @keyword.declaration

; The scalar types, which the grammar spells as literal tokens.
[
  "bool"
  "bytes"
  "double"
  "fixed32"
  "fixed64"
  "float"
  "int32"
  "int64"
  "map"
  "sfixed32"
  "sfixed64"
  "sint32"
  "sint64"
  "string"
  "uint32"
  "uint64"
] @type.builtin

"=" @operator

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "<"
  ">"
] @punctuation.bracket

[
  ","
  ";"
  "."
  ":"
] @punctuation.delimiter
