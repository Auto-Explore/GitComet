; V.
;
; Vendored from tree-sitter-v 0.0.4's highlights.scm (MIT) rather than used
; through `tree_sitter_v::HIGHLIGHTS_QUERY`, because that query does not compile
; against the parser shipped beside it: it captures `(builtin_type)`, a node the
; grammar has no such thing as. `Query::new` rejects an unknown node type
; outright, so wiring the crate as-is panicked at spec-compile time -- which is
; the loud failure, at least, rather than a silent mis-colour.
;
; Deleting that one line is the whole fix. V's builtin types (`int`, `string`,
; `bool`) are `type_identifier` nodes, and the line above it already captures
; those.
;
; One edit, marked `GitComet:` below.

(ERROR) @error
(comment) @comment

(identifier) @variable
(module_identifier) @variable
(import_path) @variable

(parameter_declaration
  name: (identifier) @parameter)
(function_declaration
  name: (identifier) @function)
(function_declaration
  receiver: (parameter_list)
  name: (identifier) @method)

(call_expression
  function: (identifier) @function)
(call_expression
  function: (selector_expression
    field: (identifier) @method))

(type_identifier) @type
; GitComet: removed `(builtin_type) @type` -- no such node; see the header.
(pointer_type) @type
(array_type) @type

(field_identifier) @property
(selector_expression
  field: (identifier) @property)

(int_literal) @number
(interpreted_string_literal) @string
(rune_literal) @string
(escape_sequence) @string.escape

[
 "as"
 "asm"
 "assert"
 ;"atomic"
 ;"break"
 "const"
 ;"continue"
 "defer"
 "else"
 "enum"
 "fn"
 "for"
 "$for"
 "go"
 "goto"
 "if"
 "$if"
 "import"
 "in"
 "!in"
 "interface"
 "is"
 "!is"
 "lock"
 "match"
 "module"
 "mut"
 "or"
 "pub"
 "return"
 "rlock"
 "select"
 ;"shared"
 ;"static"
 "struct"
 "type"
 ;"union"
 "unsafe"
] @keyword

[
 (true)
 (false)
] @boolean

[
 "."
 ","
 ":"
 ";"
] @punctuation.delimiter

[
 "("
 ")"
 "{"
 "}"
 "["
 "]"
] @punctuation.bracket

(array) @punctuation.bracket

[
 "++"
 "--"

 "+"
 "-"
 "*"
 "/"
 "%"

 "~"
 "&"
 "|"
 "^"

 "!"
 "&&"
 "||"
 "!="

 "<<"
 ">>"

 "<"
 ">"
 "<="
 ">="

 "+="
 "-="
 "*="
 "/="
 "&="
 "|="
 "^="
 "<<="
 ">>="

 "="
 ":="
 "=="

 "?"
 "<-"
 "$"
 ".."
 "..."
] @operator