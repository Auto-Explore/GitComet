; Vendored JavaScript highlights query for tree-sitter-javascript.
; Based on upstream tree-sitter-javascript queries/highlights.scm and
; queries/highlights-jsx.scm, with keyword classification enhancements
; from Zed's JavaScript query assets.

; Variables
(identifier) @variable

; Properties
(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @property
(private_property_identifier) @property

; Function and method definitions
(function_expression
  name: (identifier) @function)

(function_declaration
  name: (identifier) @function)

(method_definition
  name: (property_identifier) @function.method)

(method_definition
  name: (property_identifier) @constructor
  (#eq? @constructor "constructor"))

(pair
  key: (property_identifier) @function.method
  value: [(function_expression) (arrow_function)])

(assignment_expression
  left: (member_expression
    property: (property_identifier) @function.method)
  right: [(function_expression) (arrow_function)])

(variable_declarator
  name: (identifier) @function
  value: [(function_expression) (arrow_function)])

(assignment_expression
  left: (identifier) @function
  right: [(function_expression) (arrow_function)])

; Function and method calls
(call_expression
  function: (identifier) @function)

(call_expression
  function: (member_expression
    property: (property_identifier) @function.method))

(new_expression
  constructor: (identifier) @type)

; Parameters
(arrow_function
  parameter: (identifier) @variable.parameter)

(catch_clause
  parameter: (identifier) @variable.parameter)

; Special identifiers
([
  (identifier)
  (shorthand_property_identifier)
  (shorthand_property_identifier_pattern)
] @constant
  (#match? @constant "^[A-Z_][A-Z\\d_]+$"))

((identifier) @constructor
  (#match? @constructor "^[A-Z]"))

; Literals
(this) @variable.special

(super) @variable.special

[
  (null)
  (undefined)
] @constant.builtin

[
  (true)
  (false)
] @boolean

(comment) @comment

(hash_bang_line) @comment

[
  (string)
  (template_string)
] @string

(escape_sequence) @string.escape

(regex) @string.regex

(number) @number

; Tokens
[
  ";"
  (optional_chain)
  "."
  ","
  ":"
] @punctuation.delimiter

[
  "-"
  "--"
  "-="
  "+"
  "++"
  "+="
  "*"
  "*="
  "**"
  "**="
  "/"
  "/="
  "%"
  "%="
  "<"
  "<="
  "<<"
  "<<="
  "="
  "=="
  "==="
  "!"
  "!="
  "!=="
  "=>"
  ">"
  ">="
  ">>"
  ">>="
  ">>>"
  ">>>="
  "~"
  "^"
  "&"
  "|"
  "^="
  "&="
  "|="
  "&&"
  "||"
  "??"
  "&&="
  "||="
  "??="
  "..."
] @operator

(regex
  "/" @string.regex)

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

(ternary_expression
  [
    "?"
    ":"
  ] @operator)

; Keywords — split into declaration / import / control for richer styling
[
  "as"
  "async"
  "await"
  "debugger"
  "default"
  "delete"
  "extends"
  "get"
  "in"
  "instanceof"
  "new"
  "of"
  "set"
  "static"
  "target"
  "typeof"
  "void"
  "with"
] @keyword

[
  "const"
  "let"
  "var"
  "function"
  "class"
] @keyword.declaration

[
  "export"
  "from"
  "import"
] @keyword.import

[
  "break"
  "case"
  "catch"
  "continue"
  "do"
  "else"
  "finally"
  "for"
  "if"
  "return"
  "switch"
  "throw"
  "try"
  "while"
  "yield"
] @keyword.control

(switch_default
  "default" @keyword.control)

(template_substitution
  "${" @punctuation.special
  "}" @punctuation.special) @embedded

; JSX elements
(jsx_opening_element
  (identifier) @tag
  (#match? @tag "^[a-z][^.]*$"))

(jsx_closing_element
  (identifier) @tag
  (#match? @tag "^[a-z][^.]*$"))

(jsx_self_closing_element
  (identifier) @tag
  (#match? @tag "^[a-z][^.]*$"))

(jsx_opening_element
  (identifier) @type)

(jsx_closing_element
  (identifier) @type)

(jsx_self_closing_element
  (identifier) @type)

(jsx_attribute
  (property_identifier) @attribute)

(jsx_opening_element
  (["<" ">"]) @punctuation.bracket)

(jsx_closing_element
  (["</" ">"]) @punctuation.bracket)

(jsx_self_closing_element
  (["<" "/>"]) @punctuation.bracket)
