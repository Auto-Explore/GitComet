; Ruby.
;
; Vendored from tree-sitter-ruby 0.23.1's queries/highlights.scm (MIT) rather than
; used through `tree_sitter_ruby::HIGHLIGHTS_QUERY`, to drop one rule:
;
;   ((identifier) @function.method
;    (#is-not? local))
;
; `#is-not? local` is a locals-based predicate. It needs a `locals.scm` and scope
; tracking, which GitComet does not implement and `tree_sitter::Query` does not
; evaluate -- unevaluated predicates do not filter, so the pattern applied to
; *every* identifier in the file. Overlapping captures resolve last-wins and this
; sits just below the general `(identifier) @variable`, so it won everywhere:
; every local variable in every Ruby file rendered in the method colour.
;
; Dropping it is the whole fix. `(identifier) @variable` above then stands for
; ordinary names, and the specific rules further down -- `(call method:
; (identifier))`, `(method name:)`, `(alias)`, `(setter)` -- still colour real
; methods, which is what the deleted rule was reaching for without scope
; information.
;
; The same predicate appears in queries/javascript_highlights.scm and is harmless
; there: both uses sit beside an evaluated `#match?`/`#eq?` that already narrows
; the rule to five known names, so losing the locals check only means a local
; shadowing `document` keeps the builtin colour. The danger is a broad capture
; whose *only* predicate is unevaluated, which is what this one was.
;
; One edit, marked `GitComet:` below.

(identifier) @variable

; GitComet: removed
;   ((identifier) @function.method
;    (#is-not? local))
; See the header -- unevaluated, so it captured every identifier.

[
  "alias"
  "and"
  "begin"
  "break"
  "case"
  "class"
  "def"
  "do"
  "else"
  "elsif"
  "end"
  "ensure"
  "for"
  "if"
  "in"
  "module"
  "next"
  "or"
  "rescue"
  "retry"
  "return"
  "then"
  "unless"
  "until"
  "when"
  "while"
  "yield"
] @keyword

((identifier) @keyword
 (#match? @keyword "^(private|protected|public)$"))

(constant) @constructor

; Function calls

"defined?" @function.method.builtin

(call
  method: [(identifier) (constant)] @function.method)

((identifier) @function.method.builtin
 (#eq? @function.method.builtin "require"))

; Function definitions

(alias (identifier) @function.method)
(setter (identifier) @function.method)
(method name: [(identifier) (constant)] @function.method)
(singleton_method name: [(identifier) (constant)] @function.method)

; Identifiers

[
  (class_variable)
  (instance_variable)
] @property

((identifier) @constant.builtin
 (#match? @constant.builtin "^__(FILE|LINE|ENCODING)__$"))

(file) @constant.builtin
(line) @constant.builtin
(encoding) @constant.builtin

(hash_splat_nil
  "**" @operator) @constant.builtin

((constant) @constant
 (#match? @constant "^[A-Z\\d_]+$"))

[
  (self)
  (super)
] @variable.builtin

(block_parameter (identifier) @variable.parameter)
(block_parameters (identifier) @variable.parameter)
(destructured_parameter (identifier) @variable.parameter)
(hash_splat_parameter (identifier) @variable.parameter)
(lambda_parameters (identifier) @variable.parameter)
(method_parameters (identifier) @variable.parameter)
(splat_parameter (identifier) @variable.parameter)

(keyword_parameter name: (identifier) @variable.parameter)
(optional_parameter name: (identifier) @variable.parameter)

; Literals

[
  (string)
  (bare_string)
  (subshell)
  (heredoc_body)
  (heredoc_beginning)
] @string

[
  (simple_symbol)
  (delimited_symbol)
  (hash_key_symbol)
  (bare_symbol)
] @string.special.symbol

(regex) @string.special.regex
(escape_sequence) @escape

[
  (integer)
  (float)
] @number

[
  (nil)
  (true)
  (false)
] @constant.builtin

(interpolation
  "#{" @punctuation.special
  "}" @punctuation.special) @embedded

(comment) @comment

; Operators

[
"="
"=>"
"->"
] @operator

[
  ","
  ";"
  "."
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "%w("
  "%i("
] @punctuation.bracket
