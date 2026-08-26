; Perl.
;
; Authored here: `tree-sitter-perl` 1.1.2 ships a parser and node types but no
; highlights query at all. Perl was heuristic-only before this, which cost it more
; than colour -- with no tree there is no bracket matching and no click-to-highlight
; either, because both read the tree rather than the tokens.
;
; Sigils are the thing to get right. `$x`, `@x`, `%x` are three different variables
; that a reader tells apart by one character, and the grammar gives each its own
; node, so they can be coloured apart rather than all being "a variable".

[
  (comments)
  (pod_statement)
] @comment

; --- Strings, and Perl's several ways of writing one -------------------------
[
  (string_single_quoted)
  (string_double_quoted)
  (string_q_quoted)
  (string_qq_quoted)
  (word_list_qw)
  (heredoc_body_statement)
] @string

; Backticks and `qx//` run a command; they are strings that do something.
[
  (command_qx_quoted)
  (backtick_quoted)
] @string.special

(escape_sequence) @string.escape

[
  (heredoc_start_identifier)
  (heredoc_end_identifier)
] @label

; --- Regex -------------------------------------------------------------------
[
  (regex_pattern_content)
  (regex_pattern_qr)
  (substitution_pattern_s)
  (transliteration_tr_or_y)
] @string.regex

[
  (pattern_matcher)
  (pattern_matcher_m)
  (regex_option)
  (regex_option_for_substitution)
  (regex_option_for_transliteration)
] @punctuation.special

; --- Numbers -----------------------------------------------------------------
[
  (integer)
  (floating_point)
  (hexadecimal)
  (octal)
  (scientific_notation)
  (version)
] @number

[
  (true)
  (false)
] @boolean

; --- Variables, one colour per sigil -----------------------------------------
(scalar_variable) @variable
(array_variable) @variable
(hash_variable) @variable

; `$_`, `@ARGV`, `$0`, `%ENV` -- the ones Perl defines for you.
[
  (special_scalar_variable)
  (special_literal)
  (standard_input)
] @variable.builtin

(package_variable) @variable.special
(typeglob) @variable.special

; --- Names -------------------------------------------------------------------
(function_definition
  name: (identifier) @function)

(function_definition_without_sub
  name: (identifier) @function)

(method_invocation
  (identifier) @function.method)

[
  (package_name)
  (module_name)
] @namespace

(label) @label
(function_attribute) @attribute

; `=>` quotes its left side, so a bareword there is a key rather than a call.
(keywords_in_hash_key) @property

; --- Keywords ----------------------------------------------------------------
[
  "sub"
  "my"
  "our"
  "local"
  "state"
  "package"
  "use"
  "no"
  "require"
  "import"
  "constant"
  "feature"
  "parent"
  "prototype"
  "method"
  "func"
  "subs"
  "BEGIN"
  "END"
  "INIT"
  "CHECK"
  "UNITCHECK"
] @keyword

[
  "if"
  "elsif"
  "else"
  "unless"
  "while"
  "until"
  "for"
  "foreach"
  "when"
  "continue"
  "return"
  "last"
  "next"
  "redo"
  "goto"
] @keyword.control

(loop_control_keyword) @keyword.control

; Perl spells several operators as words.
[
  "and"
  "or"
  "not"
  "xor"
  "eq"
  "ne"
  "lt"
  "gt"
  "le"
  "ge"
  "cmp"
  "isa"
  "bless"
] @operator

[
  "="
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "<=>"
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
  "."
  ".."
  "..."
  "++"
  "--"
  "!"
  "&&"
  "||"
  "//"
  "?"
  ":"
  "=~"
  "!~"
  "->"
  "\\"
  "~"
] @operator

(fat_comma) @operator
(arrow_operator) @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ";"
  "::"
] @punctuation.delimiter

(normal_comma) @punctuation.delimiter

[
  (start_delimiter)
  (end_delimiter)
  (start_delimiter_qw)
  (end_delimiter_qw)
  (separator_delimiter)
] @punctuation.delimiter
