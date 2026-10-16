; .gitignore, .dockerignore, .npmignore.
;
; Authored here: vendor/tree-sitter-gitignore ships no query at all.
;
; The pattern text itself is deliberately left uncoloured. A `.gitignore` is a
; list of paths, and painting every path as a string would say nothing -- what a
; reader is scanning for is the handful of characters that make a line behave
; differently from a literal path: the `!` that re-includes, the `/` that anchors,
; and the glob metacharacters. Those are what is captured.

(comment) @comment

; `!vendor/keep.txt` -- re-includes a path an earlier pattern excluded, and is the
; one character in the file that reverses a line's meaning.
;
; `@keyword.control`, not `@operator`. The theme resolves `operator` to
; `foreground.secondary`, the same muted grey as punctuation and comments, so a
; `!` came out *less* prominent than the path beside it -- backwards for the one
; character that inverts the rule. `keyword.control` is the accent colour plus
; semibold (see `syntax_highlight_style` in rows/diff_text/build.rs), which is
; what "this line means the opposite" should look like.
(negation) @keyword.control

; `*`, `**`, `?`, `[a-z]`.
[
  (wildcard_chars)
  (wildcard_chars_allow_slash)
  (wildcard_char_single)
] @string.regex

(bracket_expr) @string.regex
(bracket_negation) @operator
(bracket_range) @string.regex
(bracket_char_class) @constant.builtin

; A leading or trailing `/` anchors the pattern; an interior one just separates.
(directory_separator) @punctuation.delimiter

; `\#not-a-comment`, `foo\ ` -- the escape is what stops git reading the next
; character as syntax.
[
  (pattern_char_escaped)
  (directory_separator_escaped)
  (bracket_char_escaped)
] @string.escape
