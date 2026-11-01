; Written for GitComet's Perl grammar (ganezdragon/tree-sitter-perl), which ships no
; highlights query. Difftastic reads only these captures: comments, strings and
; regexes (diffed word by word), and keywords. The keyword lists match GitComet's
; own Perl query for the same grammar.

[
  (comments)
  (pod_statement)
] @comment

[
  (string_single_quoted)
  (string_double_quoted)
  (string_q_quoted)
  (string_qq_quoted)
  (word_list_qw)
  (heredoc_body_statement)
  (command_qx_quoted)
  (backtick_quoted)
  (regex_pattern_content)
  (regex_pattern_qr)
  (substitution_pattern_s)
  (transliteration_tr_or_y)
] @string

[
  (true)
  (false)
] @boolean

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
