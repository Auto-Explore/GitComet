; fish.
;
; From tree-sitter-fish 3.6.0's queries/highlights.scm (github.com/ram02z/
; tree-sitter-fish at aa074a0; the crate declares MIT, the repository's LICENSE
; is the Unlicense), reordered, with additions and one changed capture; each edit is marked
; `GitComet:` below.
;
;   * The command-name and function-name rules move above the `test`/`[` rules.
;     Upstream had them last, so last-wins painted the `[` of `[ -n "$x" ]` as a
;     function instead of the bracket its own rule asks for.
;   * Control flow (`if`, `else`, `switch`, `case`, `for`, `in`, `while`, `end`,
;     `return`, `break`, `continue`) is `@keyword.control`, as in
;     queries/bash_highlights.scm; the `end` of a `function` or `begin` block stays
;     `@keyword` like its opener.
;   * As in queries/bash_highlights.scm: a `-x`/`--long` option of a command or
;     `function` is `@constant` (a bare `-` or `--` is not), an expansion is a
;     `$` (`@punctuation.special`) and a plain name, and a first-line `#!`
;     comment is `@keyword.directive`.

[(double_quote_string) (single_quote_string)] @string
(escape_sequence) @string.escape

(comment) @comment

; GitComet: added, the shebang rule from queries/bash_highlights.scm.
((program
  .
  (comment) @keyword.directive)
  (#match? @keyword.directive "^#![ \t]*/"))

[(integer) (float)] @number

[
  "&&"
  "||"
  "|"
  "&|"
  "2>|"
  "&"
  ".."
  (direction)
  (stream_redirect)
] @operator

; GitComet: moved up from the end, where they overrode the `[` rule below.
(function_definition name: [(word) (concatenation)] @function)
(command name: (word) @function)

; GitComet: added. Options are `@constant`, as in queries/bash_highlights.scm;
; above the `test` rules, which colour a test's own `-d`/`-n` operators.
(command
  argument: (word) @constant
  (#match? @constant "^--?[A-Za-z]"))

(function_definition
  option: (word) @constant
  (#match? @constant "^--?[A-Za-z]"))

; match operators of test command
(command
  name: (word) @function (#match? @function "^test$")
  argument: (word) @operator (#match? @operator "^(!?=|-[a-zA-Z]+)$"))

; match operators of [ command
(command
  name: (word) @punctuation.bracket (#match? @punctuation.bracket "^\\[$")
  argument: (word) @operator (#match? @operator "^(!?=|-[a-zA-Z]+)$"))

; GitComet: was `(variable_expansion) @constant`; a `$` and a plain name, as in
; queries/bash_highlights.scm.
(variable_expansion
  "$" @punctuation.special)

(variable_name) @variable

[
 "["
 "]"
 "{"
 "}"
 "("
 ")"
] @punctuation.bracket

"," @punctuation.delimiter

[
 "begin"
 "function"
 "not"
 "!"
 "and"
 "or"
] @keyword

; GitComet: control flow split out of the list above.
[
 "switch"
 "case"
 "in"
 "if"
 "else"
 "end"
 "while"
 "for"
 "return"
 (break)
 (continue)
] @keyword.control

; GitComet: added, so a block's `end` matches its opener.
(function_definition "end" @keyword)
(begin_statement "end" @keyword)
