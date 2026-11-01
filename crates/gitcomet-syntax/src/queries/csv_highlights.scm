; CSV.
;
; Upstream's queries/highlights.scm from https://github.com/amaanq/tree-sitter-csv (MIT),
; the release vendor/tree-sitter-csv is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.

(text) @string
(number) @number
(float) @float
(boolean) @boolean
"," @punctuation.delimiter
