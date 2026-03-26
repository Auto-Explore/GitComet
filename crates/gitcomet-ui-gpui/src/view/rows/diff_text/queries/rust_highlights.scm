; Derived from
; gpui-component/crates/ui/src/highlighter/languages/rust/highlights.scm
; (Apache-2.0). Local additions preserve GitComet's richer diff token classes.

(identifier) @variable

(metavariable) @variable

(type_identifier) @type

(fragment_specifier) @type

(primitive_type) @type.builtin

(self) @variable.special

(field_identifier) @property

(shorthand_field_identifier) @property

(trait_item
  name: (type_identifier) @type.interface)

(impl_item
  trait: (type_identifier) @type.interface)

(abstract_type
  trait: (type_identifier) @type.interface)

(dynamic_type
  trait: (type_identifier) @type.interface)

(trait_bounds
  (type_identifier) @type.interface)

; Identifier conventions
((identifier) @constant
  (#match? @constant "^_*[A-Z][A-Z\\d_]*$"))

((identifier) @type
  (#match? @type "^[A-Z]"))

((scoped_identifier
  path: (identifier) @type)
  (#match? @type "^[A-Z]"))

((scoped_identifier
  path: (scoped_identifier
    name: (identifier) @type))
  (#match? @type "^[A-Z]"))

((scoped_type_identifier
  path: (identifier) @type)
  (#match? @type "^[A-Z]"))

((scoped_type_identifier
  path: (scoped_identifier
    name: (identifier) @type))
  (#match? @type "^[A-Z]"))

(struct_pattern
  type: (scoped_type_identifier
    name: (type_identifier) @type))

(enum_variant
  name: (identifier) @type)

; Function calls
(call_expression
  function: [
    (identifier) @function
    (scoped_identifier
      name: (identifier) @function)
    (field_expression
      field: (field_identifier) @function.method)
  ])

(generic_function
  function: [
    (identifier) @function
    (scoped_identifier
      name: (identifier) @function)
    (field_expression
      field: (field_identifier) @function.method)
  ])

; Function definitions
(function_item
  name: (identifier) @function.definition)

(function_signature_item
  name: (identifier) @function.definition)

; Macros
(macro_invocation
  macro: [
    (identifier) @function.special
    (scoped_identifier
      name: (identifier) @function.special)
  ])

(macro_invocation
  "!" @function.special)

(macro_definition
  name: (identifier) @function.special.definition)

[
  (line_comment)
  (block_comment)
] @comment

[
  (line_comment
    (doc_comment))
  (block_comment
    (doc_comment))
] @comment.doc

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

(type_arguments
  "<" @punctuation.bracket
  ">" @punctuation.bracket)

(type_parameters
  "<" @punctuation.bracket
  ">" @punctuation.bracket)

[
  "."
  ";"
  ","
  "::"
] @punctuation.delimiter

"#" @punctuation.special

[
  "as"
  "async"
  "const"
  "default"
  "dyn"
  "enum"
  "extern"
  "fn"
  "impl"
  "let"
  "macro_rules!"
  "mod"
  "move"
  "pub"
  "raw"
  "ref"
  "static"
  "struct"
  "for"
  "trait"
  "type"
  "union"
  "unsafe"
  "use"
  "where"
  (crate)
  (mutable_specifier)
  (super)
] @keyword

[
  "await"
  "break"
  "continue"
  "else"
  "if"
  "in"
  "loop"
  "match"
  "return"
  "while"
  "yield"
] @keyword.control

(for_expression
  "for" @keyword.control)

[
  (string_literal)
  (raw_string_literal)
  (char_literal)
] @string

(escape_sequence) @string.escape

[
  (integer_literal)
  (float_literal)
] @number

(boolean_literal) @boolean

[
  "!="
  "%"
  "%="
  "&"
  "&="
  "&&"
  "*"
  "*="
  "+"
  "+="
  "-"
  "-="
  "->"
  ".."
  "..="
  "..."
  "/="
  ":"
  "<<"
  "<<="
  "<"
  "<="
  "="
  "=="
  "=>"
  ">"
  ">="
  ">>"
  ">>="
  "@"
  "^"
  "^="
  "|"
  "|="
  "||"
  "?"
] @operator

(unary_expression
  "!" @operator)

operator: "/" @operator

(lifetime
  "'" @lifetime
  (identifier) @lifetime)

(parameter
  (identifier) @variable.parameter)

(attribute_item) @attribute

(inner_attribute_item) @attribute
