; Derived from
; gpui-component/crates/ui/src/highlighter/languages/javascript/highlights.scm
; (Apache-2.0). Local additions preserve parameter highlighting and JSX-in-.js
; support in GitComet diffs.

; Variables
;----------

(identifier) @variable

; Properties
;-----------

(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @property
(private_property_identifier) @property

; Function and method definitions
;--------------------------------

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
;--------------------------

(call_expression
  function: (identifier) @function)

(call_expression
  function: (member_expression
    property: (property_identifier) @function.method))

(new_expression
  constructor: (identifier) @type)

; Parameters
;-----------

(arrow_function
  parameter: (identifier) @variable.parameter)

(catch_clause
  parameter: (identifier) @variable.parameter)

; Special identifiers
;--------------------

((identifier) @type
 (#match? @type "^[A-Z]"))

([
  (identifier)
  (shorthand_property_identifier)
  (shorthand_property_identifier_pattern)
] @constant
 (#match? @constant "^_*[A-Z_][A-Z\\d_]*$"))

((identifier) @variable.special
 (#match? @variable.special "^(arguments|module|console|window|document)$")
 (#is-not? local))

((identifier) @function.special
 (#eq? @function.special "require")
 (#is-not? local))

; Literals
;---------

(this) @variable.special

(super) @variable.special

[
  (true)
  (false)
] @boolean

[
  (null)
  (undefined)
] @constant.builtin

(comment) @comment

(hash_bang_line) @comment

[
  (string)
  (template_string)
] @string

(escape_sequence) @string.escape

(regex) @string.special

(number) @number

; Tokens
;-------

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

[
  "as"
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "default"
  "delete"
  "do"
  "else"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "get"
  "if"
  "import"
  "in"
  "instanceof"
  "let"
  "new"
  "of"
  "return"
  "set"
  "static"
  "switch"
  "target"
  "throw"
  "try"
  "typeof"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword

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
