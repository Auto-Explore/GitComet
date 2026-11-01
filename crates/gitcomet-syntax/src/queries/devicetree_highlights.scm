; Devicetree (.dts, .dtsi, .dtso, .its).
;
; Based on tree-sitter-devicetree 0.14.1's queries/highlights.scm (MIT;
; github.com/joelspadin/tree-sitter-devicetree at 6557729); the rest is authored
; from the grammar's node-types.json. Edits:
;   * `(identifier) @variable` comes first: upstream had it after its label,
;     property-name and call rules, so it beat them. All-caps names (cpp macros
;     in cell lists) are `@constant`.
;   * cpp directives are `@preproc` (plus `#elifdef`, `#elifndef`, `defined`);
;     dtc directives stay `@keyword` (plus `/plugin/`, `/bits/`).
;   * operators are scoped to expressions, `=` is an operator, `<`/`>` are
;     brackets only in a cell list, and `,` `;` `:` `@` are delimiters.
;   * added: strings, escapes, numbers, byte strings, node names (`@module`),
;     macro names and parameters, /delete-node/ and /delete-property/ targets.
;   * a whole reference, `&uart0` or `&{/soc/i2c@0}`, is one `@constant`.

(identifier) @variable

((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]*$"))

[
  "/bits/"
  "/delete-node/"
  "/delete-property/"
  "/dts-v1/"
  "/incbin/"
  "/include/"
  "/memreserve/"
  "/omit-if-no-ref/"
  "/plugin/"
] @keyword

[
  "#define"
  "#elif"
  "#elifdef"
  "#elifndef"
  "#else"
  "#endif"
  "#if"
  "#ifdef"
  "#ifndef"
  "#include"
  "#undef"
  "defined"
] @preproc

[
  ","
  ";"
  ":"
  "@"
] @punctuation.delimiter

[
  "("
  ")"
  "{"
  "}"
] @punctuation.bracket

(integer_cells
  [
    "<"
    ">"
  ] @punctuation.bracket)

; After the punctuation, so `:` and `>` inside an expression read as operators.
(binary_expression
  operator: _ @operator)

(unary_expression
  operator: _ @operator)

(conditional_expression
  [
    "?"
    ":"
  ] @operator)

"=" @operator

[
  (string_literal)
  (system_lib_string)
] @string

(escape_sequence) @string.escape

[
  (integer_literal)
  (byte_string_literal)
] @number

(call_expression
  function: (identifier) @function)

(preproc_def
  name: (identifier) @constant)

(preproc_function_def
  name: (identifier) @function.special)

(preproc_params
  (identifier) @variable.parameter)

(node
  name: (identifier) @module)

(delete_node
  name: (identifier) @module)

(node
  label: (identifier) @label)

(property
  label: (identifier) @label)

(memory_reservation
  label: (identifier) @label)

(property
  name: (identifier) @property)

(delete_property
  name: (identifier) @property)

(unit_address) @tag

; A reference is one value. Its parts are captured too, because a child starting
; after `&` would otherwise keep its own capture and win last-wins.
(reference) @constant

(reference
  label: (identifier) @constant)

(path_node
  name: (identifier) @constant)

(reference
  "}" @constant)

(path_node
  "@" @constant)

(path_node
  address: (unit_address) @constant)

(comment) @comment
