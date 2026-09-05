; Protocol Buffers.
;
; Authored here for `tree-sitter-proto` 0.6.0. Keep every construct below in
; sync with that grammar when it is bumped; 0.5.0 added the proto2 `group`
; declaration and the edition-2023 `export`/`local` visibility modifiers, and
; 0.6.0 changed syntax versions from anonymous tokens to `string` nodes.
;
; The grammar spells the scalar types (`int32`, `string`, `bytes`, ...) and the
; declaration words as *anonymous* tokens, so most of this is a literal list
; rather than node patterns -- the same shape as queries/wat_highlights.scm.

(comment) @comment

(string) @string
(escape_sequence) @string.escape

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
  ; proto2's `group Foo = 1 { ... }`. Its name arrives as a `message_name`, so
  ; the rule above already types it; only the keyword itself needs naming.
  "group"
  ; Edition 2023 visibility, written ahead of `message` / `enum`.
  "export"
  "local"
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
