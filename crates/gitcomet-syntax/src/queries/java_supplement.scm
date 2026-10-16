; Appended to `tree_sitter_java::HIGHLIGHTS_QUERY`, which captures no
; punctuation at all.

(field_access field: (identifier) @property)

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
  "."
  "::"
] @punctuation.delimiter
