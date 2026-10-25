; Appended to `tree_sitter_sequel::HIGHLIGHTS_QUERY`.
;
; That query tags every `(literal)` `@string` and then tries to take numbers
; back with `(#match? @number "^[-+]?%d+$")` -- a Lua pattern. tree-sitter
; evaluates `#match?` as a regex, where `%d` means a literal `%` followed by
; `d`, so the pattern never matches and `1` renders as a string. Same regex,
; written the way the engine actually reads it.

((literal) @number
  (#match? @number "^[-+]?[0-9]+$"))

((literal) @number
  (#match? @number "^[-+]?[0-9]*\\.[0-9]+$"))
