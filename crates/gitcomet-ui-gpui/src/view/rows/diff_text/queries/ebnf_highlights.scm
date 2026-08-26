; EBNF.
;
; Upstream's queries/highlights.scm from https://github.com/RubixDev/ebnf (MIT),
; the release vendor/tree-sitter-ebnf is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.

;;;; Simple tokens ;;;;
(terminal) @string.grammar

(special_sequence) @string.special.grammar

(integer) @number

(comment) @comment.block

;;;; Identifiers ;;;;
(identifier) @variable.grammar

; Allow different highlighting for specific casings
((identifier) @variable.grammar.pascal
 (#match? @variable.grammar.pascal "^[A-Z]"))

((identifier) @variable.grammar.camel
 (#match? @variable.grammar.camel "^[a-z]"))

((identifier) @variable.grammar.upper
 (#match? @variable.grammar.upper "^[A-Z][A-Z0-9_]+$"))

((identifier) @variable.grammar.lower
 (#match? @variable.grammar.lower "^[a-z][a-z0-9_]+$"))

;;; Punctuation ;;;;
[
 ";"
 ","
] @punctuation.delimiter

[
 "|"
 "*"
 "-"
] @operator

"=" @keyword.operator

[
 "("
 ")"
 "["
 "]"
 "{"
 "}"
] @punctuation.bracket
