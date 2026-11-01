; Appended to `tree_sitter_php::HIGHLIGHTS_QUERY`, which captures no punctuation.

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
  "->"
  "::"
] @punctuation.delimiter
