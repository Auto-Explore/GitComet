; Derived from tree-sitter-jinja-dialects' queries/highlights.scm
; (https://github.com/bennypowers/tree-sitter-jinja-dialects, MIT) at v0.1.1.
;
; The grammar parses the union of Jinja2, Nunjucks, Twig, Tera, Inja and Django
; template syntax, so one query serves every extension in the `Jinja` arm of
; diff_syntax_language_for_identifier.
;
; Local changes vs upstream:
;   - the single `@keyword` list is split. Control flow becomes
;     `@keyword.control`, which GitComet renders semibold (see
;     syntax_highlight_style in ../build.rs); declarations and modifiers stay
;     `@keyword`. Upstream has no reason to care -- the distinction only exists
;     because this theme draws the two differently.
;   - `@function.builtin` is kept rather than narrowed to `@function`: the
;     capture-name table trims dotted suffixes progressively, so it already
;     lands on Function, and keeping the upstream name means a future
;     `function.builtin` colour needs no change here.
;
; Note the delimiters are captured on the anonymous tokens (`{{`, `{%-`, `#}`)
; rather than on the enclosing `output`/`statement`/`comment` node. Capturing
; the container would paint the whole tag, and `@none` cannot be used to punch
; the interior back out -- it emits no token at all, so the outer capture simply
; wins. vue_highlights.scm captures `{{`/`}}` for the same reason.

(comment) @comment

(string) @string
(integer) @number
(float) @number
(boolean) @boolean
(none) @constant.builtin

(identifier) @variable

(member_expression
  property: (identifier) @property)

(call_expression
  function: (identifier) @function)

(filter_expression
  name: (identifier) @function.builtin)

(test_expression
  name: (identifier) @function.builtin)

(keyword_argument
  key: (identifier) @property)

(pair
  key: (identifier) @property)

["{{" "{{-" "{{~" "}}" "-}}" "~}}"
 "{%" "{%-" "{%~" "%}" "-%}" "~%}"
 "{#" "{#-" "{#~" "#}" "-#}" "~#}"] @punctuation.special

["(" ")" "[" "]" "{" "}"] @punctuation.bracket

["," "." "|" "~"] @punctuation.delimiter

["+" "-" "*" "/" "//" "%" "**"
 "==" "!=" "<" ">" "<=" ">=" "===" "!=="
 "??" "?." "?" ":"
 "b-and" "b-or" "b-xor"] @operator

; Control flow and the block terminators that pair with it.
["if" "elif" "elseif" "else" "endif"
 "for" "in" "endfor" "recursive"
 "is" "not" "and" "or"] @keyword.control

; Declarations, imports and tag modifiers.
["block" "endblock" "extends"
 "macro" "endmacro" "call" "endcall"
 "filter" "endfilter" "raw" "endraw"
 "set" "endset" "with" "endwith"
 "include" "import" "from" "as"
 "autoescape" "endautoescape"
 "do"
 "ignore" "missing" "scoped"
 "verbatim" "endverbatim"
 "context" "without"] @keyword
