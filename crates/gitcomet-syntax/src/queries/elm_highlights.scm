; Elm.
;
; From nvim-treesitter's runtime/queries/elm/highlights.scm at
; ca581c3436fc2b236847f5b739ebc263739e5ca6 (Apache-2.0). Not the crate's own
; queries/highlights.scm (tree-sitter-elm 5.9.4, MIT): that one uses TextMate
; scope names (`@storage.type.elm`, `@union.elm`, `@meta.import.elm`, `@char.elm`)
; that GitComet does not map, so types, constructors, imports and chars were plain.
;
; Edits, each marked `GitComet:` below:
;   * the doc-comment rule's `#lua-match?` -> `#match?`. Unevaluated, it made every
;     `{- -}` comment a doc comment.
;   * `(string_escape)` -> `@string.escape` in strings and chars; upstream gave it
;     the plain string/char colour.
;   * the `_` wildcard: `@character.special` -> `@punctuation.special`. GitComet
;     maps `character` to the string colour.

[
  (line_comment)
  (block_comment)
] @comment @spell

; GitComet: was `#lua-match? "^{[-]|[^|]"`, which the engine never evaluates.
((block_comment) @comment.documentation
  (#match? @comment.documentation "^\\{-\\|[^|]"))

; Keywords
;---------
[
  "if"
  "then"
  "else"
  (case)
  (of)
] @keyword.conditional

[
  "let"
  "in"
  (as)
  (port)
  (alias)
  (infix)
  (module)
  (type)
] @keyword

[
  (import)
  (exposing)
] @keyword.import

; Punctuation
;------------
(double_dot) @punctuation.special

[
  ","
  "|"
  (dot)
] @punctuation.delimiter

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

; Variables
;----------
(value_qid
  (lower_case_identifier) @variable)

(value_declaration
  (function_declaration_left
    (lower_case_identifier) @variable))

(type_annotation
  (lower_case_identifier) @variable)

(port_annotation
  (lower_case_identifier) @variable)

(anything_pattern
  (underscore) @punctuation.special) ; GitComet: was @character.special

(record_base_identifier
  (lower_case_identifier) @variable)

(lower_pattern
  (lower_case_identifier) @variable)

(exposed_value
  (lower_case_identifier) @variable)

(value_qid
  ((dot)
    (lower_case_identifier) @variable.member))

(field_access_expr
  ((dot)
    (lower_case_identifier) @variable.member))

(function_declaration_left
  (anything_pattern
    (underscore) @punctuation.special)) ; GitComet: was @character.special

(function_declaration_left
  (lower_pattern
    (lower_case_identifier) @variable.parameter))

; Functions
;----------
(value_declaration
  functionDeclarationLeft: (function_declaration_left
    (lower_case_identifier) @function
    (pattern)))

(value_declaration
  functionDeclarationLeft: (function_declaration_left
    (lower_case_identifier) @function
    pattern: (_)))

(value_declaration
  functionDeclarationLeft: (function_declaration_left
    (lower_case_identifier) @function)
  body: (anonymous_function_expr))

(type_annotation
  name: (lower_case_identifier) @function
  typeExpression: (type_expression
    (arrow)))

(port_annotation
  name: (lower_case_identifier) @function
  typeExpression: (type_expression
    (arrow)))

(function_call_expr
  target: (value_expr
    (value_qid
      (lower_case_identifier) @function.call)))

; Operators
;----------
[
  (operator_identifier)
  (eq)
  (colon)
  (arrow)
  (backslash)
  "::"
] @operator

; Modules
;--------
(module_declaration
  (upper_case_qid
    (upper_case_identifier) @module))

(import_clause
  (upper_case_qid
    (upper_case_identifier) @module))

(as_clause
  (upper_case_identifier) @module)

(value_expr
  (value_qid
    (upper_case_identifier) @module))

; Types
;------
(type_declaration
  (upper_case_identifier) @type)

(type_ref
  (upper_case_qid
    (upper_case_identifier) @type))

(type_variable
  (lower_case_identifier) @type)

(lower_type_name
  (lower_case_identifier) @type)

(exposed_type
  (upper_case_identifier) @type)

(type_alias_declaration
  (upper_case_identifier) @type.definition)

(field_type
  name: (lower_case_identifier) @property)

(field
  name: (lower_case_identifier) @property)

(type_declaration
  (union_variant
    (upper_case_identifier) @constructor))

(nullary_constructor_argument_pattern
  (upper_case_qid
    (upper_case_identifier) @constructor))

(union_pattern
  (upper_case_qid
    (upper_case_identifier) @constructor))

(value_expr
  (upper_case_qid
    (upper_case_identifier)) @constructor)

; Literals
;---------
(number_constant_expr
  (number_literal) @number)

(upper_case_qid
  ((upper_case_identifier) @boolean
    (#any-of? @boolean "True" "False")))

[
  (open_quote)
  (close_quote)
] @string

(string_constant_expr
  (string_escape) @string.escape) ; GitComet: was @string

(string_constant_expr
  (regular_string_part) @string)

[
  (open_char)
  (close_char)
] @character

(char_constant_expr
  (string_escape) @string.escape) ; GitComet: was @character

(char_constant_expr
  (regular_string_part) @character)
