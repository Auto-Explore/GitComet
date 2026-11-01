; Apex (Salesforce), with the SOQL and SOSL it embeds in `[...]`.
;
; From nvim-treesitter (Apache-2.0) at ca581c3436fc2b236847f5b739ebc263739e5ca6:
; runtime/queries/{soql,sosl,apex}/highlights.scm concatenated in that order.
; nvim's apex file starts `; inherits: soql`, which nvim resolves by prepending
; the soql file; sosl is a GitComet addition because the Apex grammar embeds
; SOSL too (`[FIND ... RETURNING ...]`) and nvim's apex query leaves it plain.
; Not the crate's apex/queries/highlights.scm (tree-sitter-sfapex 3.0.1, MIT): it
; uses VS Code semantic-token names (`@class`, `@enumMember`, `@decorator`, ...)
; that GitComet does not map, and colours no SOQL. Edits, marked `GitComet:`:
;
;   * apex's `(identifier) @variable` / `(type_identifier) @type` defaults moved
;     to the top. Appended after the soql part, they overrode every SOQL field,
;     alias and object-name rule.
;   * The SCREAM_CASE `#lua-match?` -> `#match?`, and the SOQL function-name rule
;     re-asserted after it.
;   * `decimal_floating_point_literal` added to the numbers.
;   * Deleted the rule that coloured every local declaration as a property; a
;     FROM object takes `@type` rather than `@keyword.modifier`; `obj.field`
;     colours `obj` as a type only when it is capitalised, like its siblings.

; GitComet: apex's default identifier colours, moved up from the apex part.
(identifier) @variable

(type_identifier) @type

; ---- soql ------------------------------------------------------------------
(field_identifier
  (identifier) @variable.member)

(field_identifier
  (dotted_identifier
    (identifier) @variable.member))

(type_of_clause
  (identifier) @variable.member)

(when_expression
  (identifier) @type)

(when_expression
  (field_list
    (identifier) @variable.member))

(when_expression
  (field_list
    (dotted_identifier
      (identifier) @variable.member)))

(else_expression
  (field_list
    (identifier) @variable.member))

(else_expression
  (field_list
    (dotted_identifier
      (identifier) @variable.member)))

(alias_expression
  (identifier) @label)

; GitComet: was `(storage_identifier) @keyword.modifier`, which the identifier
; default overrode anyway. The object after FROM is an SObject type, as the
; RETURNING targets in the sosl part are.
(storage_identifier
  (identifier) @type)

(_
  function_name: (identifier) @function)

(date_literal) @string.special

[
  ","
  "."
  ":"
  "("
  ")"
] @punctuation.delimiter

[
  "AND"
  "OR"
  "NOT"
  "LIKE"
  "NOT_IN"
  "INCLUDES"
  "EXCLUDES"
] @keyword.operator

[
  "="
  "!="
  "<="
  ">="
] @operator

(value_comparison_operator
  [
    "<"
    ">"
  ] @operator)

(set_comparison_operator
  "IN" @keyword.operator)

[
  (int)
  (decimal)
  (currency_literal)
] @number

(string_literal) @string

[
  (date)
  (date_time)
] @string.special

[
  "TRUE"
  "FALSE"
] @boolean

(null_literal) @constant.builtin

[
  "ABOVE"
  "ABOVE_OR_BELOW"
  "ALL"
  "AS"
  "ASC"
  "AT"
  "BELOW"
  "CUSTOM"
  "DATA_CATEGORY"
  "DESC"
  "END"
  "FIELDS"
  "FOR"
  "FROM"
  "GROUP_BY"
  "HAVING"
  "LIMIT"
  "NULLS_FIRST"
  "NULLS_LAST"
  "OFFSET"
  "ORDER_BY"
  "REFERENCE"
  "SELECT"
  "STANDARD"
  "TRACKING"
  "TYPEOF"
  "UPDATE"
  "USING"
  "SCOPE"
  "LOOKUP"
  "BIND"
  "VIEW"
  "VIEWSTAT"
  "WITH"
] @keyword

[
  "WHERE"
  "WHEN"
  "ELSE"
  "THEN"
] @keyword.conditional

; Using Scope
[
  "delegated"
  "everything"
  "mine"
  "mine_and_my_groups"
  "my_territory"
  "my_team_territory"
  "team"
] @keyword

; With
[
  "maxDescriptorPerRecord"
  "RecordVisibilityContext"
  "Security_Enforced"
  "supportsDomains"
  "supportsDelegates"
  "System_Mode"
  "User_Mode"
  "UserId"
] @keyword

; ---- sosl ------------------------------------------------------------------
(find_clause
  (term) @string)

(sobject_return
  (identifier) @type)

(with_type
  (_
    "=" @operator))

[
  "ALL"
  "DIVISION"
  "EMAIL"
  "FIND"
  "ListView"
  "HIGHLIGHT"
  "IN"
  "METADATA"
  "NAME"
  "NETWORK"
  "PHONE"
  "PricebookId"
  "RETURNING"
  "SIDEBAR"
  "SNIPPET"
  "SPELL_CORRECTION"
  "target_length"
  "USING"
] @keyword

; ---- apex ------------------------------------------------------------------
; Apex + SOQL
[
  "["
  "]"
  "{"
  "}"
  "("
  ")"
] @punctuation.bracket

[
  ","
  "."
  ":"
  "?"
  ";"
] @punctuation.delimiter

; Methods
(method_declaration
  name: (identifier) @function.method)

(method_invocation
  name: (identifier) @function.method.call)

(super) @function.builtin

; Annotations
(annotation
  name: (identifier) @attribute)

; Types
(interface_declaration
  name: (identifier) @type)

(class_declaration
  name: (identifier) @type)

(class_declaration
  (superclass) @type)

(enum_declaration
  name: (identifier) @type)

(enum_constant
  name: (identifier) @constant)

(type_arguments
  "<" @punctuation.delimiter)

(type_arguments
  ">" @punctuation.delimiter)

; GitComet: added the `^[A-Z]` test its two siblings below have; without it
; every `record.Field` coloured `record` as a type.
((field_access
  object: (identifier) @type)
  (#match? @type "^[A-Z]"))

(field_access
  field: (identifier) @property)

((scoped_identifier
  scope: (identifier) @type)
  (#match? @type "^[A-Z]"))

((method_invocation
  object: (identifier) @type)
  (#match? @type "^[A-Z]"))

(method_declaration
  (formal_parameters
    (formal_parameter
      name: (identifier) @variable.parameter)))

(constructor_declaration
  name: (identifier) @constructor)

(dml_type) @function.builtin

(assignment_operator) @operator

(update_operator) @operator

(trigger_declaration
  name: (identifier) @type
  object: (identifier) @type
  (trigger_event) @keyword
  (","
    (trigger_event) @keyword)*)

[
  "@"
  "="
  "!="
  "<="
  ">="
] @operator

(binary_expression
  operator: [
    ">"
    "<"
    "=="
    "==="
    "!=="
    "&&"
    "||"
    "+"
    "-"
    "*"
    "/"
    "&"
    "|"
    "^"
    "%"
    "<<"
    ">>"
    ">>>"
  ] @operator)

(unary_expression
  operator: [
    "+"
    "-"
    "!"
    "~"
  ]) @operator

"=>" @operator

[
  (boolean_type)
  (void_type)
] @type.builtin

; Fields
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @variable.member))

(field_access
  field: (identifier) @variable.member)

; GitComet: deleted `(variable_declarator (identifier) @property)`, which gave
; every local variable declaration the field colour.

(field_declaration
  (modifiers
    (modifier
      [
        (final)
        (static)
      ])
    (modifier
      [
        (final)
        (static)
      ]))
  (variable_declarator
    name: (identifier) @constant))

; GitComet: was #lua-match?, never evaluated, so every identifier was @constant.
((identifier) @constant
  (#match? @constant "^[A-Z][A-Z0-9_]+$")) ; SCREAM SNAKE CASE

; GitComet: re-asserted from the SOQL part, which the rule above overrides for
; `SUM(...)`, `COUNT(...)` and every other upper-case SOQL function.
(_
  function_name: (identifier) @function)

(this) @variable.builtin

; Literals
[
  (int)
  (decimal)
  (decimal_floating_point_literal) ; GitComet: Apex `0.5`, missing upstream
  (currency_literal)
] @number

[
  (string_literal)
  (multi_line_string_literal) ; GitComet: sfapex 3.0's `'''` strings
] @string

[
  (line_comment)
  (block_comment)
] @comment

(null_literal) @constant.builtin

; ;; Keywords
[
  "abstract"
  "final"
  "private"
  "protected"
  "public"
  "static"
] @keyword.modifier

[
  "if"
  "else"
  "switch"
] @keyword.conditional

[
  "for"
  "while"
  "do"
  "break"
] @keyword.repeat

"return" @keyword.return

[
  "throw"
  "finally"
  "try"
  "catch"
] @keyword.exception

"new" @keyword.operator

[
  (abstract)
  (all_rows_clause)
  "continue"
  "extends"
  (final)
  "get"
  (global)
  "implements"
  "instanceof"
  "on"
  (override)
  (private)
  (protected)
  (public)
  "set"
  (static)
  (testMethod)
  (webservice)
  (transient)
  "trigger"
  (virtual)
  "when"
  (with_sharing)
  (without_sharing)
  (inherited_sharing)
] @keyword

[
  "interface"
  "class"
  "enum"
] @keyword.type

"System.runAs" @function.builtin
