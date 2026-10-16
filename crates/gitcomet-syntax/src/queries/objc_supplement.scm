; Appended to `tree_sitter_objc::HIGHLIGHTS_QUERY`, which captures neither
; comments, strings nor numbers -- an Objective-C file rendered with just that
; query has unhighlighted comments and string literals.

(comment) @comment

(string_literal) @string
(char_literal) @string
(system_lib_string) @string
(number_literal) @number

(type_identifier) @type
(primitive_type) @type.builtin
(field_identifier) @property
(method_identifier) @function.method

[
  "{"
  "}"
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ";"
  ","
  ":"
  "."
  "->"
] @punctuation.delimiter
