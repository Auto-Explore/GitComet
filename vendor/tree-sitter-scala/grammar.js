const PREC = {
  comment: 1,
  control: 1,
  stable_type_id: 2,
  type: 2,
  while: 2,
  assign: 3,
  case: 3,
  stable_id: 4,
  unit: 4,
  postfix: 5,
  colon_call: 5,
  // Infix operator precedence (SLS 6.12.3), decided by the operator's first
  // character. Operators ending in `=` (except <=, >=, !=, or starting with
  // `=`) drop to assignment precedence below every other operator.
  iassign: 6,
  iname: 7, // alphanumeric operators
  ior: 8, // |
  ixor: 9, // ^
  iand: 10, // &
  ieq: 11, // = !
  irel: 12, // < >
  icolon: 13, // :
  iadd: 14, // + -
  imul: 15, // * / %
  iother: 16, // remaining symbolic characters
  infix: 12, // flat level for patterns and types, independent of the ladder
  prefix: 17,
  compound: 17,
  call: 18,
  field: 18,
  macro: 20,
  binding: 20,
};

// \p{Sm} minus its six ASCII members (+ < = > | ~). The per-class operator
// tokens below partition the old operator_identifier token by first character,
// so their first-character sets must be disjoint and a regex class cannot
// subtract from \p{Sm}. Astral code points appear as literal characters.
// Verified byte-equal against Node v26's \p{Sm} minus ASCII.
const SM_NONASCII =
  "\\u00ac\\u00b1\\u00d7\\u00f7\\u03f6\\u0606-\\u0608\\u2044\\u2052\\u207a-\\u207c\\u208a-\\u208c\\u2118\\u2140-\\u2144\\u214b\\u2190-\\u2194\\u219a-\\u219b\\u21a0\\u21a3\\u21a6\\u21ae\\u21ce-\\u21cf\\u21d2\\u21d4\\u21f4-\\u22ff\\u2320-\\u2321\\u237c\\u239b-\\u23b3\\u23dc-\\u23e1\\u25b7\\u25c1\\u25f8-\\u25ff\\u266f\\u27c0-\\u27c4\\u27c7-\\u27e5\\u27f0-\\u27ff\\u2900-\\u2982\\u2999-\\u29d7\\u29dc-\\u29fb\\u29fe-\\u2aff\\u2b30-\\u2b44\\u2b47-\\u2b4c\\ufb29\\ufe62\\ufe64-\\ufe66\\uff0b\\uff1c-\\uff1e\\uff5c\\uff5e\\uffe2\\uffe9-\\uffec" +
  "\u{10d8e}-\u{10d8f}\u{1cef0}\u{1d6c1}\u{1d6db}\u{1d6fb}\u{1d715}\u{1d735}\u{1d74f}\u{1d76f}\u{1d789}\u{1d7a9}\u{1d7c3}\u{1eef0}-\u{1eef1}\u{1f8d0}-\u{1f8d8}";

// Regex class bodies over SLS 1.1 opchar. OP is the full set. OP_END_LEFT
// drops `:` and `=`, the endings that change an operator's precedence class.
// OP_NO_COLON only drops `:`, for operators starting with `=` whose `=`
// ending does not demote them to assignment.
const OP = "\\-!#%&*+/\\\\:<=>?@\\u005e\\u007c~\\p{Sm}\\p{So}";
const OP_END_LEFT =
  "\\-!#%&*+/\\\\<>?@\\u005e\\u007c~" + SM_NONASCII + "\\p{So}";
const OP_NO_COLON = "\\-!#%&*+/\\\\<=>?@\\u005e\\u007c~\\p{Sm}\\p{So}";

// SLS 1.1: an operator ends where a comment begins, so no `/` in one may be
// followed by `/` or `*`. Every `/` is bound to the character after it, which
// also means an operator of two or more characters cannot end in `/`.
// Each class body holds one `/` and one `*`, so removing the first is enough.
const noSlash = cls => cls.replace("/", "");
const noSlashStar = cls => noSlash(cls).replace("*", "");
// One character of a class, or a `/` bound to a character that cannot open a
// comment. A tail character out of the same class reads the same way.
const opStep = cls => `(?:[${noSlash(cls)}]|/[${noSlashStar(cls)}])`;
const OP_NSS = noSlashStar(OP);
const OP_NSS_END_LEFT = noSlashStar(OP_END_LEFT);
const OP_STEP = opStep(OP);
const OP_RUN = `${OP_STEP}*`;
// A run before a tail that is neither `/` nor `*` may itself end in `/`.
const OP_RUN_SLASH = `${OP_RUN}/?`;
// The same with at least one character, for the `<=`, `>=` and `!=` forms
// that need something between the first character and the final `=`.
const OP_RUN1_SLASH = `(?:${OP_STEP}+/?|/)`;
const OP_TAIL_LEFT = opStep(OP_END_LEFT);
const OP_TAIL_NO_COLON = opStep(OP_NO_COLON);
// The restricted single-character set of the old token: technically any \p{Sm}
// should be allowed, but that includes `=`, and `⇒` must stay lexable as the
// Scala 2 `=>` (see fatArrow below).
const OP_SINGLE_UNICODE =
  "\\u00ac\\u00b1\\u00d7\\u00f7\\u2190-\\u2194\\u2200-\\u22ff\\p{So}";
// Multi-character first set of the iother class: opchars not claimed by a
// dedicated precedence level.
const OP_OTHER_FIRST = "#?@\\\\~" + SM_NONASCII + "\\p{So}";

const opRegex = body => new RegExp(body, "u");

// One token per SLS 6.12.3 precedence class, split by the final character:
// `:` makes an operator right-associative, `=` demotes it to assignment
// precedence unless it is `<=`, `>=`, `!=` or starts with `=`. The
// alternatives reproduce the old operator_identifier token exactly, so
// every operator string lexes as precisely one of these tokens.
const OP_TOKEN = {
  assign: token(
    choice(
      opRegex(`[&:+\\-*%\\u005e\\u007c${OP_OTHER_FIRST}]${OP_RUN_SLASH}=`),
      opRegex(`[<>!]${OP_RUN1_SLASH}=`),
      opRegex(`/=`),
      opRegex(`/[${OP_NSS}]${OP_RUN_SLASH}=`),
    ),
  ),
  orLeft: token(
    choice(opRegex("\\u007c"), opRegex(`\\u007c${OP_RUN}${OP_TAIL_LEFT}`)),
  ),
  orRight: token(opRegex(`\\u007c${OP_RUN_SLASH}:`)),
  xorLeft: token(
    choice(opRegex("\\u005e"), opRegex(`\\u005e${OP_RUN}${OP_TAIL_LEFT}`)),
  ),
  xorRight: token(opRegex(`\\u005e${OP_RUN_SLASH}:`)),
  andLeft: token(choice(opRegex("&"), opRegex(`&${OP_RUN}${OP_TAIL_LEFT}`))),
  andRight: token(opRegex(`&${OP_RUN_SLASH}:`)),
  eqLeft: token(
    choice(
      opRegex("!"),
      // The SLS 6.12.4 assignment exceptions (!=, <=, >=) are spelled as
      // literals for visibility; inside token() they lex like the regexes.
      "!=",
      opRegex(`!${OP_RUN}${OP_TAIL_LEFT}`),
      opRegex(`=${OP_RUN}${OP_TAIL_NO_COLON}`),
    ),
  ),
  eqRight: token(opRegex(`[=!]${OP_RUN_SLASH}:`)),
  relLeft: token(
    choice(
      opRegex("[<>]"),
      "<=",
      ">=",
      opRegex(`[<>]${OP_RUN}${OP_TAIL_LEFT}`),
    ),
  ),
  relRight: token(opRegex(`[<>]${OP_RUN_SLASH}:`)),
  colonLeft: token(opRegex(`:${OP_RUN}${OP_TAIL_LEFT}`)),
  colonRight: token(opRegex(`:${OP_RUN_SLASH}:`)),
  addLeft: token(
    choice(opRegex("[+\\-]"), opRegex(`[+\\-]${OP_RUN}${OP_TAIL_LEFT}`)),
  ),
  addRight: token(opRegex(`[+\\-]${OP_RUN_SLASH}:`)),
  mulLeft: token(
    choice(
      opRegex("[/%]"),
      opRegex(`[*%]${OP_RUN}${OP_TAIL_LEFT}`),
      opRegex(`/[${OP_NSS_END_LEFT}]`),
      opRegex(`/[${OP_NSS}]${OP_RUN}${OP_TAIL_LEFT}`),
    ),
  ),
  mulRight: token(
    choice(
      opRegex(`[*%]${OP_RUN_SLASH}:`),
      opRegex("/:"),
      opRegex(`/[${OP_NSS}]${OP_RUN_SLASH}:`),
    ),
  ),
  otherLeft: token(
    choice(
      opRegex(`[#?\\\\~${OP_SINGLE_UNICODE}]`),
      opRegex(`[${OP_OTHER_FIRST}]${OP_RUN}${OP_TAIL_LEFT}`),
    ),
  ),
  otherRight: token(opRegex(`[${OP_OTHER_FIRST}]${OP_RUN_SLASH}:`)),
};

// The left-associative classes are the only ones that can end in `/`, since a
// `:` or `=` ending rules that out. Each is external as well, so the scanner
// can claim that one form, and the token above stays the lexer's fallback.
// The order here is the externals order, which src/scanner.c mirrors.
const OP_LEFT = [
  "or",
  "xor",
  "and",
  "eq",
  "rel",
  "colon",
  "add",
  "mul",
  "other",
];
const opLeft = ($, key) => $[`_op_left_${key}`];

// The pre-split operator identifier as one token, for pure name contexts
// (definitions, imports, types, patterns). A state that also allows an infix
// continuation needs the per-class tokens instead, because one lexer state
// cannot hold both flavors of the same string (see _identifier).
const OP_ID_UNION = token(
  choice(
    opRegex(`[${OP_SINGLE_UNICODE}\\-!#%&*+/\\\\<>?\\u005e\\u007c~]`),
    opRegex(`[${noSlash(OP)}]${OP_STEP}+`),
    opRegex(`/[${OP_NSS}]${OP_RUN}`),
  ),
);

// `⇒` (U+21D2) is the alternate Scala 2 spelling of `=>` (SLS 1.1, dropped in
// Scala 3). It is not lexable as an operator_identifier (the single-opchar
// class excludes it), so the extra token cannot collide with user operators.
const fatArrow = () => choice("=>", alias("⇒", "=>"));

// `=>` or the context-function arrow `?=>`.
const anyArrow = () => choice(fatArrow(), "?=>");

// The arrow of a function type. `->` and `?->` are the pure spellings, and
// either may carry the set of capabilities the function captures. Only types
// take them: `->` is an ordinary infix operator in an expression, where it
// builds a pair.
// Two literals rather than one token with a precedence. A lexical precedence
// outranks match length, so a combined arrow would take the head of `->>`.
const pureArrow = () => choice("->", "?->");

const typeArrow = () => choice(fatArrow(), "?=>", pureArrow());

// XML Name (SLS §10 / XML spec), covering namespaced names like `x:ga`.
const XML_NAME = /[_\p{L}][-.:_\p{L}\p{Nd}]*/;

// Accepts a `:` in both spellings. The scanner lexes a line-final lone colon
// as the external COLON_EOL token (scalac's COLONeol), so every consumer of
// such a colon must handle it.
const colonEol = $ => choice(":", alias($._colon_eol, ":"));

const ascriptionArrowTail = $ =>
  seq(anyArrow(), field("return_type", $._param_type));

// An expression as admitted in statement and RHS positions. do_while lives
// outside $.expression: after `for (...)` a `do` must introduce the for's own
// body, and a do-while fork there doubles the paren-for automaton (~+7MiB).
// The third branch is dead: $._never is never emitted, so it cannot complete.
// It puts the control-tail keywords in every statement-expression follow set
// uniformly, so being inside a try/if/while/do body no longer clones the
// expression automaton per enclosing construct (catch/finally alone were
// measured at 15.5MB of parser.c). It lives here, under visible block nodes,
// because inside $.expression it would break the supertype single-child rule.
const statementExpression = $ => choice($.expression, $.do_while_expression);

// An alphanumeric name as the bare word tokens instead of the identifier
// rule. In positions that never need the soft-keyword, `this`, or `super`
// tokens for another reading, this drops those columns from every state;
// keyword extraction still lexes such words here as names.
const wordName = $ =>
  alias(choice($._alpha_identifier, $._backquoted_id), $.identifier);

// The identifier rule spelled as its alternatives, each aliased back to
// identifier. A single-token alternative carries its alias on the parent
// production, so a plain name shifts straight in with no unit reduction.
const nameChoice = $ =>
  choice(
    alias($._alpha_identifier, $.identifier),
    alias($._backquoted_id, $.identifier),
    alias($._soft_identifier, $.identifier),
    alias("this", $.identifier),
    alias("super", $.identifier),
  );

// The scanner-lexed soft modifiers, each keeping its keyword token's node name.
const erasedMod = $ => alias($._erased_modifier, $.erased_modifier);
const openMod = $ => alias($._open_modifier, $.open_modifier);
const opaqueMod = $ => alias($._opaque_modifier, $.opaque_modifier);
const infixMod = $ => alias($._infix_modifier, $.infix_modifier);
const trackedMod = $ => alias($._tracked_modifier, $.tracked_modifier);
const transparentMod = $ =>
  alias($._transparent_modifier, $.transparent_modifier);
const intoMod = $ => alias($._into_modifier, $.into_modifier);
const inlineMod = $ => alias($._inline_modifier, $.inline_modifier);
const updateMod = $ => alias($._update_modifier, $.update_modifier);
const consumeMod = $ => alias($._consume_modifier, $.consume_modifier);

// The union operator token as a name (see OP_ID_UNION).
const opName = $ => alias($._op_name, $.operator_identifier);

// A name or symbolic name with the same narrow-token property.
const wordOrOpName = $ => choice(wordName($), opName($));

// A name in a position whose lexer states also serve expression operands
// (lambda parameters, bindings, self types). These must share the identifier
// rule's per-class tokens: one lexer state cannot hold both flavors of the
// same string, so swapping a site to $._identifier changes lexing there.
const operandName = $ => choice(nameChoice($), $.operator_identifier);

// A Scala 3 end marker as the last child of the construct it closes. The
// leading semicolon keeps `end` out of every expression follow set, and
// the weight beats the statement reading of the marker line. The tails are
// shared rules so every construct reuses the same post-semicolon states.
const endMarkerTail = ($, marker, weight) =>
  prec.dynamic(
    weight,
    seq($._automatic_semicolon, alias(marker, $.end_marker)),
  );

module.exports = grammar({
  name: "scala",

  extras: $ => [/\s/, $.comment, $.block_comment],

  supertypes: $ => [$.expression, $._definition, $._pattern],

  // Order must mirror enum TokenType in src/scanner.c exactly: the scanner
  // addresses these tokens by index.
  externals: $ => [
    $._automatic_semicolon,
    $._indent,
    $._outdent,
    $._comma_outdent,
    $._simple_string_start,
    $._simple_string_middle,
    $._simple_multiline_string_start,
    $._interpolated_string_middle,
    $._interpolated_multiline_string_middle,
    $._raw_string_start,
    $._raw_string_middle,
    $._raw_string_multiline_middle,
    $._single_line_string_end,
    $._multiline_string_end,
    "else",
    "catch",
    "finally",
    "extends",
    "derives",
    "with",
    // Lexed externally so the leading `/*` token does not occupy a column in
    // nearly every parse-table row. That costs about 0.6MiB of parser.c.
    $.block_comment,
    // Never returned by the scanner. External extras look valid in every
    // state, so the states where `/*` is plain text offer this token as a
    // dead alternative and the scanner skips block comments there.
    $._suppress_block_comment,
    $.error_sentinel,
    $._colon_eol,
    // A symbolic operator in postfix position. Lexed externally so the infix
    // productions never see it and the whole lower-precedence chain reduces
    // first (SLS: PostfixExpr ::= InfixExpr id). The emission conditions
    // live in the operator branch of scan_impl in src/scanner.c, which only
    // offers these for ASCII, non-slash-initial operators. A lone `*` gets
    // its own token so vararg can claim it.
    $._postfix_op,
    $._postfix_star,
    $._floating_point_with_separators,
    // Emitted only where a marker can attach and the line is `end` plus a
    // tag word, so `end` stays an ordinary identifier elsewhere.
    $._end_keyword,
    // Zero-width, emitted right before a control-tail keyword (catch,
    // finally, else, then, do, while) where the grammar allows one. Every
    // construct body then ends with the same single gate column instead of
    // its own keyword follow set, so the expression automaton stops being
    // cloned per enclosing construct (catch/finally alone measured 15.5MB
    // of parser.c).
    $._control_tail_gate,
    // The `<` that opens an XML literal. Only valid where the grammar admits
    // a literal, so the operator `<` is untouched everywhere else.
    $._xml_tag_start,
    // Each of these is a name as well, so a keyword token would cost a
    // soft-identifier alternative in every name position.
    $._erased_modifier,
    $._open_modifier,
    $._opaque_modifier,
    $._infix_modifier,
    $._tracked_modifier,
    $._transparent_modifier,
    $._inline_modifier,
    $._into_modifier,
    $._update_modifier,
    $._consume_modifier,
    // `uses` continues the parent list onto the next line the way `derives`
    // does, so the scanner has to withhold the separator before it.
    "uses",
    // The left-associative operator classes (see OP_LEFT) and the union name
    // token. The scanner only claims an operator ending in `/`, since a regex
    // cannot see whether that `/` opens a comment, and declines every other
    // one so the lexer falls back to the grammar definition.
    ...OP_LEFT.map(key => opLeft($, key)),
    $._op_name,
    // The `>` of `//>`. A regex would have to swallow the `using` after it to
    // see it, and highlighting needs that keyword as a token of its own, so
    // the scanner looks ahead instead and declines the `>` of a plain comment.
    $._using_directive_start,
  ],

  inline: $ => [
    $._definition_pattern,
    $._pattern,
    $._semicolon,
    $._definition,
    $._param_type,
    $._identifier,
    $._postfix_expression_choice,
    $._infix_type_choice,
    $._param_value_type,
    $._simple_type,
    $.literal,
    // Every parenthesized argument list reduces through this rule, so it is
    // one of the most frequent unit reductions in a parse. Inlining it costs
    // about 600 states and buys 5 to 10 percent of parse time.
    $._exprs_in_parens,
    $._argument_list,
    // Small hidden rules that reduce to a token almost immediately. Inlining
    // removes the reduce step and merges states, shrinking parser.c by ~2MB.
    $._asterisk,
    $._super_identifier,
    $._this_identifier,
    $._non_null_literal,
    $._braced_template_body,
    $._indented_template_body,
    // Inlined so the expression and pattern content variants never compete in
    // a reduce, which they cannot be resolved out of.
    $._xml_node,
    $._xml_content,
    $._xml_pattern_content,
    $._structural_type,
    $._refinement,
  ],

  // Doc: https://tree-sitter.github.io/tree-sitter/creating-parsers, search "precedences"
  // These names can be used in the prec functions to define precedence relative only to other names in the array, rather than globally.
  precedences: $ => [
    ["mod", "soft_id"],
    ["new", "structural_type"],
    ["self_type", "lambda"],
    ["annotation", "applied_constructor_type"],
    ["constructor_application", "applied_constructor_type"],
  ],

  conflicts: $ => [
    [$._if_rest],
    [$.repeat_pattern, $._simple_expression],
    [$.tuple_pattern, $._simple_expression],
    [$.tuple_pattern, $._simple_expression, $.binding],
    // 'for'  '('  pattern  '='  _xml_open_tag  '/>'  •  ':'  … — the element
    // and pattern readings of a literal differ only in their content, and the
    // enumerator admits both once the definition pattern is inlined.
    [$.xml_element, $.xml_pattern],
    [$._simple_expression, $._xml_embedded_pattern],
    [$._simple_expression, $._xml_repeat_pattern],
    // _simple_expression  '('  _simple_expression  •  ':'  — reduce toward an
    // ascribed argument or shift the colon of a Scala 2 vararg `x: _*`.
    [$._infix_operand, $.vararg],
    [$.tuple_type, $.parameter_types],
    // Both spell a braced body, so the reduce after `}` is ambiguous.
    [$._structural_body, $.template_body],
    // `->{` opens the captured set of an arrow or the block a plain arrow
    // takes, and a bare name reads as either until the brace closes.
    [$.capture_ref, $._simple_expression],
    // 'extension' • '{' is either an extension definition with a braced body
    // or the soft identifier `extension` (a Scala 2 id) called with a block.
    [$.extension_definition, $._soft_identifier],
    [$.binding, $._simple_expression],
    [$.binding, $._type_identifier],
    // A parenthesized type before `=>` can extend into a function type or stop
    // so the arrow belongs to the enclosing lambda, pattern, or colon argument.
    // The conflicts keep both readings alive so precedence picks per context.
    [$._type, $.parameter_types],
    [$.parameter_types, $._colon_bindings],
    [$.parameter_types, $.bindings],
    [$.while_expression, $._simple_expression],
    [$.if_expression],
    [$.match_expression],
    [$._type_identifier, $.ascription_expression],
    [$._given_constructor, $._type_identifier],
    [$.instance_expression],
    // A semicolon after a body is either a statement separator or the one
    // in front of an end marker. Fork until the tag decides.
    [$.try_expression],
    [$._if_body],
    // Same separator-or-marker fork as above.
    [$.function_definition],
    [$.val_definition],
    [$._dot_match_expression],
    [$.var_definition],
    [$.package_clause],
    [$._object_definition],
    [$.given_definition],
    [$.extension_definition],
    [$.while_expression],
    [$.for_expression],
    [$.enum_definition],
    // In case of: 'extension'  _indent  '{'  'case'  operator_identifier  'if'  operator_identifier  •  '=>'  …
    // we treat `operator_identifier` as `simple_expression`
    [$._simple_expression, $.lambda_expression],
    [$._simple_expression, $._single_lambda_param],
    // '['  identifier  ':'  '{'  identifier. A braced context bound reuses
    // the template machinery, so self type and statement readings coexist.
    [$._single_lambda_param, $.self_type, $._type_identifier],
    [$._single_lambda_param, $._type_identifier],
    // 'class'  _class_constructor  •  _automatic_semicolon  …
    [$._class_definition],
    // 'class'  operator_identifier  •  _automatic_semicolon  …
    [$._class_constructor],
    // 'enum'  _class_constructor  '{'  'case'  operator_identifier  _full_enum_def_repeat1  •  _automatic_semicolon  …
    [$._full_enum_def],
    // _start_val  identifier  ','  identifier  •  ':'  …
    [$.identifiers, $.val_declaration],
    // 'enum'  operator_identifier  _automatic_semicolon  '('  ')'  •  ':'  …
    [$.class_parameters],
    // 'for'  operator_identifier  ':'  _annotated_type  •  ':'  …
    [$._type, $.compound_type],
    // 'for'  'given'  _annotated_type  •  '*'  …
    [$._type, $.infix_type],
    // _simple_expression  ':'  _annotated_type  •  'match'  …
    [$._type, $.match_type],
    // 'given'  '('  '['  _type_parameter  •  ','  …
    [$._variant_type_parameter, $.type_lambda],
    // 'given'  '('  operator_identifier  ':'  _type  •  ','  …
    [$.name_and_type, $.parameter],
    [$._simple_expression, $._type_identifier],
    // 'if'  parenthesized_expression  •  '{'  …
    [$._if_condition_paren, $._simple_expression],
    [$.block, $._braced_template_body1],
    // '{'  identifier  •  ':' starts the braced typed lambda, the block
    // lambda param, and a statement alike.
    [$._simple_expression, $._braced_typed_lambda],
    [$.self_type, $._simple_expression, $._braced_typed_lambda],
    [$._self_type_ascription, $._braced_typed_lambda],
    [$.binding, $._simple_expression, $._type_identifier],
    [$.class_parameter, $._type_identifier],
    // '{'  _single_lambda_param  '=>'  expression  •  '}'  …
    [$._block_statements, $._indentable_expression],
    [$.match_expression, $._simple_expression],
    // _  :  Type  •  '=>'  …
    [$.self_type, $._simple_expression],
    // _simple_expression  ':'  '('  name_and_type  ')'  •  '=>'
    // The parenthesized list is either the lambda parameters of a colon
    // argument or a named tuple type on the ascription reading. What follows
    // `=>` decides, an INDENT or a same-line type.
    [$.named_tuple_type, $._colon_bindings],
    // _simple_expression  ':'  '('  wildcard  •  ','  …
    [$._annotated_type, $.binding],
    // '['  identifier  ':'  '{'  identifier  •  ':'  — a braced context
    // bound reuses the self-type head shapes.
    [$.self_type, $._type_identifier, $._simple_expression],
    // '['  identifier  ':'  '{'  wildcard  •  ':'  …
    [$.self_type, $._annotated_type, $._simple_expression],
    // '['  identifier  ':'  '{'  '('  wildcard  •  ':'  …
    [$.binding, $._annotated_type, $._simple_expression],
    // '['  identifier  ':'  '{'  wildcard  •  '{'  …
    [$._annotated_type, $._simple_expression],
    // '['  identifier  ':'  '{'  wildcard  •  '['  …
    [$.generic_type, $._simple_expression],
  ],

  word: $ => $._alpha_identifier,

  reserved: {
    global: $ => [
      // NOTE: Some keywords are commented out because there are too
      // many places where they are incorrectly parsed as identifiers,
      // and reserving them breaks the parsing completely.
      "abstract",
      "case",
      // 'catch',
      "class",
      "def",
      "do",
      // 'else',
      "enum", // Scala 3
      "export", // Scala 3
      "extends",
      "false",
      "final",
      "finally",
      "for",
      // 'forSome', // Scala 2, not implemented
      "given", // Scala 3
      // 'if',
      "implicit",
      "import",
      "lazy",
      "macro", // Scala 2
      token.immediate("match"),
      "new",
      "null",
      "object",
      "override",
      "package",
      "private",
      "protected",
      "return",
      "sealed",
      "super",
      // 'then', // Scala 3
      "this",
      "throw",
      "trait",
      "true",
      "try",
      "type",
      "val",
      "var",
      "while",
      // 'with',
      "yield",
      ":",
      "=",
      "<-",
      // "=>",
      // "<:",
      "<%", // Scala 2
      // ">:",
      "#",
      "@",
      "=>>",
      "?=>",
    ],
  },

  rules: {
    // TopStats          ::=  TopStat {semi TopStat}
    compilation_unit: $ =>
      seq(
        optional($._shebang),
        optional(trailingSep1($._semicolon, $._top_level_definition)),
      ),

    _top_level_definition: $ => choice($._definition, statementExpression($)),

    _definition: $ =>
      choice(
        $.given_definition,
        $.extension_definition,
        $.class_definition,
        $.import_declaration,
        $.export_declaration,
        $.object_definition,
        $.enum_definition,
        $.trait_definition,
        $.val_definition,
        $.val_declaration,
        $.var_definition,
        $.var_declaration,
        $.type_definition,
        $.function_definition,
        $.function_declaration,
        $.package_clause,
        $.package_object,
      ),

    enum_definition: $ =>
      seq(
        repeat($.annotation),
        optional($.modifiers),
        "enum",
        $._class_constructor,
        field("extend", optional($.extends_clause)),
        field("derive", optional(choice($.derives_clause, $.uses_clause))),
        field("body", $.enum_body),
        optional($._end_marker_named_tail),
      ),

    _enum_block: $ =>
      prec.left(
        seq(
          sep1(
            $._semicolon,
            choice($.enum_case_definitions, $.expression, $._definition),
          ),
          optional($._semicolon),
        ),
      ),

    enum_body: $ =>
      choice(
        prec.left(
          PREC.control,
          seq(
            colonEol($),
            $._indent,
            optional($.self_type),
            $._enum_block,
            $._outdent,
          ),
        ),
        seq("{", optional($.self_type), optional($._enum_block), "}"),
      ),

    enum_case_definitions: $ =>
      seq(
        repeat($.annotation),
        // e.g. `private case External(...) extends ...`
        optional($.modifiers),
        "case",
        choice(commaSep1($.simple_enum_case), $.full_enum_case),
      ),

    simple_enum_case: $ =>
      prec.left(
        seq(
          field("name", $._identifier),
          field("extend", optional($.extends_clause)),
        ),
      ),

    full_enum_case: $ => seq(field("name", $._identifier), $._full_enum_def),

    _full_enum_def: $ =>
      seq(
        field("type_parameters", optional($.type_parameters)),
        field("class_parameters", repeat1($.class_parameters)),
        field("extend", optional($.extends_clause)),
      ),

    package_clause: $ =>
      seq(
        prec.right(
          seq(
            "package",
            field("name", $.package_identifier),
            // This is slightly more permissive than the EBNF in that it allows
            // any kind of declaration inside of the package blocks. As we're
            // more concerned with the structure rather than the validity of
            // the program we'll allow it.
            field("body", optional($.template_body)),
          ),
        ),
        optional($._end_marker_named_tail),
      ),

    package_identifier: $ => prec.right(sep1(".", $._identifier)),

    package_object: $ => seq("package", "object", $._object_definition),

    import_declaration: $ =>
      prec.left(seq("import", sep1(",", $._namespace_expression))),

    export_declaration: $ =>
      prec.left(seq("export", sep1(",", $._namespace_expression))),

    /*
      ImportExpr        ::=  SimpleRef {‘.’ id} ‘.’ ImportSpec
                          |  SimpleRef ‘as’ id
      ImportSpec        ::=  NamedSelector
                          |  WildCardSelector
                          | ‘{’ ImportSelectors) ‘}’
      NamedSelector     ::=  id [‘as’ (id | ‘_’)]
      WildCardSelector  ::=  ‘*’ | ‘given’ [InfixType]
    */
    _namespace_expression: $ =>
      prec.left(
        choice(
          seq(
            field("path", sep1(".", $._namespace_path_segment)),
            optional(
              seq(
                ".",
                choice(
                  // A bound given selector is a WildCardSelector, so it stands
                  // on its own as well as inside braces.
                  $._namespace_given_by_type,
                  $.namespace_wildcard,
                  $.namespace_selectors,
                  // Only allowed in Scala 3
                  // ImportExpr        ::=
                  //    SimpleRef {‘.’ id} ‘.’ ImportSpec |  SimpleRef ‘as’ id
                  $.as_renamed_identifier,
                ),
              ),
            ),
          ),
          $.as_renamed_identifier,
        ),
      ),

    // Scala 3 keywords that are valid identifiers in Scala 2 sources and thus
    // may appear as package names in import/export paths (e.g. `import io.circe.export.Exported`).
    _namespace_path_segment: $ =>
      choice($._identifier, alias(choice("enum", "export"), $.identifier)),

    namespace_wildcard: $ => prec.left(1, choice("*", "_", "given")),

    _namespace_given_by_type: $ => seq("given", $._type),

    namespace_selectors: $ =>
      seq(
        "{",
        trailingCommaSep1(
          choice(
            $._namespace_given_by_type,
            $.namespace_wildcard,
            $._identifier,
            $.arrow_renamed_identifier,
            $.as_renamed_identifier,
          ),
        ),
        "}",
      ),

    // deprecated: Remove when highlight query is updated for Neovim
    _import_selectors: $ => alias($.namespace_selectors, $.import_selectors),

    arrow_renamed_identifier: $ =>
      seq(
        field("name", $._identifier),
        fatArrow(),
        field("alias", choice($._identifier, $.wildcard)),
      ),

    as_renamed_identifier: $ =>
      seq(
        field("name", $._identifier),
        "as",
        field("alias", choice($._identifier, $.wildcard)),
      ),

    object_definition: $ =>
      seq(
        repeat($.annotation),
        optional($.modifiers),
        optional("case"),
        "object",
        $._object_definition,
      ),

    _object_definition: $ =>
      seq(
        prec.left(
          seq(
            field("name", $._identifier),
            field("extend", optional($.extends_clause)),
            field("derive", optional(choice($.derives_clause, $.uses_clause))),
            field("body", optional($._definition_body)),
          ),
        ),
        optional($._end_marker_named_tail),
      ),

    class_definition: $ =>
      seq(
        repeat($.annotation),
        optional($.modifiers),
        optional("case"),
        "class",
        $._class_definition,
      ),

    _class_definition: $ =>
      seq(
        $._class_constructor,
        field("extend", optional($.extends_clause)),
        field("derive", optional(choice($.derives_clause, $.uses_clause))),
        field("body", optional($._definition_body)),
        optional($._end_marker_named_tail),
      ),

    // The weight keeps a next-line braced template attached to its
    // definition instead of becoming a block statement.
    _definition_body: $ =>
      prec.dynamic(
        1,
        seq(optional($._automatic_semicolon), field("body", $.template_body)),
      ),

    /**
     * ClassConstr       ::=  [ClsTypeParamClause] [ConstrMods] ClsParamClauses
     * ConstrMods        ::=  {Annotation} [AccessModifier]
     */
    _class_constructor: $ =>
      seq(
        field("name", $._identifier),
        field("type_parameters", optional($.type_parameters)),
        optional(alias($._constructor_annotation, $.annotation)),
        optional($.access_modifier),
        field(
          "class_parameters",
          repeat(seq(optional($._automatic_semicolon), $.class_parameters)),
        ),
      ),

    trait_definition: $ =>
      prec.left(
        seq(
          repeat($.annotation),
          optional($.modifiers),
          "trait",
          $._class_definition,
        ),
      ),

    // The EBNF makes a distinction between function type parameters and other
    // type parameters as you can't specify variance on function type
    // parameters. This isn't important to the structure of the AST so we don't
    // make that distinction.
    type_parameters: $ =>
      seq("[", trailingCommaSep1($._variant_type_parameter), "]"),

    _variant_type_parameter: $ =>
      seq(
        repeat($.annotation),
        choice(
          $.covariant_type_parameter,
          $.contravariant_type_parameter,
          $._type_parameter, // invariant type parameter
          $.type_lambda,
        ),
      ),

    covariant_type_parameter: $ => seq("+", $._type_parameter),

    contravariant_type_parameter: $ => seq("-", $._type_parameter),

    _type_parameter: $ =>
      seq(
        field("name", choice($.wildcard, $._identifier)),
        optional(alias("^", $.capture_variable)),
        field("type_parameters", optional($.type_parameters)),
        field("bound", optional($.lower_bound)),
        field("bound", optional($.upper_bound)),
        field("bound", optional(repeat($.view_bound))),
        field("bound", optional($._context_bounds)),
      ),

    upper_bound: $ => seq("<:", field("type", choice($._type, $.capture_set))),

    lower_bound: $ => seq(">:", field("type", choice($._type, $.capture_set))),

    view_bound: $ => seq("<%", field("type", $._type)),

    _context_bounds: $ =>
      choice(
        repeat1(seq(":", $.context_bound)),
        seq(":", "{", trailingCommaSep1($.context_bound), "}"),
      ),

    context_bound: $ =>
      seq(
        field("type", $._type),
        optional(seq("as", field("name", $._identifier))),
      ),

    /*
     * TemplateBody      ::=  :<<< [SelfType] TemplateStat {semi TemplateStat} >>>
     */
    template_body: $ =>
      choice($._indented_template_body, $._braced_template_body),

    _indented_template_body: $ =>
      prec.left(
        PREC.control,
        seq(
          colonEol($),
          $._indent,
          choice(
            seq(optional($.self_type), $._block),
            // A self type with an empty body: `trait A:` + `self: B =>`
            // followed by the next definition (or EOF). Mirrors the braced
            // variant in _braced_template_body1.
            $.self_type,
          ),
          // The comma close lets a dedented `,` end a `new:` template used
          // as an argument (`f(adapter = new: ...` + `    ,`) instead of
          // leaving that reading to a losing GLR fork.
          choice($._outdent, $._comma_outdent),
        ),
      ),

    _braced_template_body: $ =>
      prec.left(
        PREC.control,
        seq(
          "{",
          optional(choice($._braced_template_body1, $._braced_template_body2)),
          "}",
        ),
      ),

    _braced_template_body1: $ =>
      choice(
        seq(optional($.self_type), $._block),
        // A self type with an empty body: `trait A { self: B => }`. Without
        // this the only reading of the body is an empty-bodied lambda, which
        // breaks when another statement follows the closing brace.
        $.self_type,
      ),
    _braced_template_body2: $ =>
      seq(
        choice(
          seq($._indent, optional($.self_type)),
          seq(optional($.self_type), $._indent),
        ),
        optional($._block),
        $._outdent,
        // A member indented shallower than its siblings pops the indent
        // region before the closing brace; the remaining members are still
        // part of the same braced body (scalac ignores indentation here).
        optional($._block),
      ),

    /*
     * WithTemplateBody  ::=  <<< [SelfType] TemplateStat {semi TemplateStat} >>>
     */
    with_template_body: $ =>
      choice(
        prec.left(
          PREC.control,
          seq($._indent, optional($.self_type), $._block, $._outdent),
        ),
        seq("{", optional($._block), "}"),
      ),

    _extension_template_body: $ =>
      choice(
        prec.left(PREC.control, seq($._indent, $._block, $._outdent)),
        seq("{", optional($._block), "}"),
      ),

    // One hidden variant per tag set, both shown as (end_marker). Real
    // rules because an alias around a token-only seq shows nothing.
    _end_marker_named: $ =>
      seq(
        $._end_keyword,
        choice(
          "val",
          "given",
          "this",
          // No operator identifiers. They would swallow `end` used as a
          // plain identifier (`${end - start}`).
          alias(choice($._alpha_identifier, $._backquoted_id), "_end_ident"),
        ),
      ),
    _end_marker_named_tail: $ => endMarkerTail($, $._end_marker_named, 1),

    // The extra weight sends `end extension` to the extension itself
    // rather than naming a definition inside it.
    _end_marker_kw_tail: $ => endMarkerTail($, $._end_marker_kw, 2),

    _end_marker_kw: $ =>
      seq(
        $._end_keyword,
        choice("extension", "if", "while", "for", "match", "try", "new"),
      ),

    // Dynamic precedences added here to win over $.call_expression
    self_type: $ =>
      // 2 beats the block lambda reading of `{ this: I => ... }`, which
      // carries dynamic 1.
      prec.dynamic(
        2,
        prec(
          "self_type",
          seq(
            choice(nameChoice($), $.operator_identifier, $.wildcard),
            optional($._self_type_ascription),
            fatArrow(),
          ),
        ),
      ),

    _self_type_ascription: $ => seq(":", $._type),

    annotation: $ =>
      prec.right(
        "annotation",
        seq(
          "@",
          field("name", $._simple_type),
          field("arguments", repeat(prec("annotation", $.arguments))),
        ),
      ),

    // Only allows 0 or 1 argument lists as these annotations
    // usually come from Java, where multiple argument lists are not allowed
    _constructor_annotation: $ =>
      prec(
        "annotation",
        seq(
          "@",
          field("name", $._simple_type),
          optional(
            alias(
              seq(
                // token.immediate here carries an assumption that there are no spaces between
                // an annotation name and its argument list, otherwise this list should be
                // classified as a class constructor list
                token.immediate("("),
                optional($._exprs_in_parens),
                ")",
              ),
              $.arguments,
            ),
          ),
        ),
      ),

    val_definition: $ =>
      seq(
        $._start_val,
        field("pattern", choice($._definition_pattern, $.identifiers)),
        optional(seq(":", field("type", $._type))),
        "=",
        field("value", $._indentable_expression),
        optional($._end_marker_named_tail),
      ),

    val_declaration: $ =>
      seq(
        $._start_val,
        commaSep1(field("name", $._identifier)),
        ":",
        field("type", $._type),
      ),

    _start_val: $ => seq(repeat($.annotation), optional($.modifiers), "val"),

    var_declaration: $ =>
      seq(
        $._start_var,
        commaSep1(field("name", $._identifier)),
        ":",
        field("type", $._type),
      ),

    var_definition: $ =>
      seq(
        $._start_var,
        field("pattern", choice($._definition_pattern, $.identifiers)),
        optional(seq(":", field("type", $._type))),
        "=",
        field("value", $._indentable_expression),
        optional($._end_marker_named_tail),
      ),

    _start_var: $ => seq(repeat($.annotation), optional($.modifiers), "var"),

    type_definition: $ =>
      prec.left(
        seq(
          repeat($.annotation),
          optional($.modifiers),
          optional(opaqueMod($)),
          "type",
          $._type_constructor,
          optional(seq("=", field("type", choice($._type, $.capture_set)))),
        ),
      ),

    // Created for memory-usage optimization during codegen.
    _type_constructor: $ =>
      prec.left(
        seq(
          field("name", $._type_identifier),
          optional(alias("^", $.capture_variable)),
          field("type_parameters", optional($.type_parameters)),
          field("bound", optional($.lower_bound)),
          field("bound", optional($.upper_bound)),
          field("bound", optional($._context_bounds)),
        ),
      ),

    function_definition: $ =>
      seq(
        $._function_declaration,
        choice(
          seq("=", field("body", $._indentable_expression)),
          field("body", $.block),
        ),
        optional($._end_marker_named_tail),
      ),

    function_declaration: $ => $._function_declaration,

    _function_declaration: $ =>
      prec.left(
        seq(
          repeat($.annotation),
          optional($.modifiers),
          "def",
          $._function_constructor,
          optional(seq(":", field("return_type", $._type))),
        ),
      ),

    // Created for memory-usage optimization during codegen.
    _function_constructor: $ =>
      prec.right(
        seq(
          field("name", $._identifier),
          field(
            "parameters",
            repeat(
              seq(
                optional($._automatic_semicolon),
                choice($.parameters, $.type_parameters),
              ),
            ),
          ),
          optional($._automatic_semicolon),
        ),
      ),

    /**
     *   Extension         ::=  'extension' [DefTypeParamClause] {UsingParamClause}
     *                          '(' DefParam ')' {UsingParamClause} ExtMethods
     */
    // The weight beats the reading of `extension (x)` as a call of the
    // soft identifier `extension`.
    extension_definition: $ =>
      prec.dynamic(
        1,
        seq(
          prec.left(
            seq(
              "extension",
              field("type_parameters", optional($.type_parameters)),
              field("parameters", repeat($.parameters)),
              field(
                "body",
                choice(
                  $._extension_template_body,
                  $.function_definition,
                  $.function_declaration,
                ),
              ),
            ),
          ),
          optional($._end_marker_kw_tail),
        ),
      ),

    /**
     * GivenDef          ::=  [GivenSig] (AnnotType ['=' Expr] | StructuralInstance)
     * GivenSig          ::=  [id] [DefTypeParamClause] {UsingParamClause} ':'
     */
    given_definition: $ =>
      seq(
        prec.left(
          seq(
            repeat($.annotation),
            optional($.modifiers),
            "given",
            $._given_tail,
          ),
        ),
        optional($._end_marker_named_tail),
      ),

    // Weighted below the old signature so `given (using a: Int): Int` stays a
    // parameter clause rather than a named tuple joined by an infix `:`.
    // One rule, so the optional signature before it does not duplicate the
    // states the type and body choices need.
    _given_type_and_body: $ =>
      choice(
        field("return_type", $._structural_instance),
        seq(
          // A given type is an infix chain, which is what makes `X is Y` one.
          // The plain reading wins the tie so `(using a: Int):` stays a
          // parameter clause.
          field(
            "return_type",
            choice(
              $._annotated_type,
              prec.dynamic(-1, $.literal_type),
              prec.dynamic(-1, $.infix_type),
            ),
          ),
          // The body needs no `with` in the Scala 3.6 spelling.
          optional($._given_body),
        ),
      ),

    _given_tail: $ =>
      seq(
        optional($._given_constructor),
        repeat($._given_sig),
        $._given_type_and_body,
      ),

    _given_body: $ =>
      choice(
        seq("=", field("body", $._indentable_expression)),
        prec.dynamic(-1, field("body", $.template_body)),
      ),

    _given_sig: $ => seq($._given_conditional, fatArrow()),

    _given_conditional: $ =>
      choice(alias($.parameters, $.given_conditional), $.type_parameters),

    _given_constructor: $ =>
      prec.right(
        seq(
          field("name", optional($._identifier)),
          field("type_parameters", optional($.type_parameters)),
          field(
            "parameters",
            repeat(seq(optional($._automatic_semicolon), $.parameters)),
          ),
          optional($._automatic_semicolon),
          ":",
        ),
      ),

    /**
     * StructuralInstance ::=  ConstrApp {'with' ConstrApp} ['with' WithTemplateBody]
     */
    _structural_instance: $ =>
      prec.left(
        PREC.compound,
        choice(
          seq(
            $._constructor_application,
            choice(colonEol($), "with"),
            field("body", $.with_template_body),
          ),
          // Several constructors and no body. The separator is `with` or a
          // comma. A refinement is a constructor everywhere else, but a brace
          // here is always the body, so the extras leave it out.
          seq(
            $._constructor_application,
            repeat1(
              seq(
                choice("with", ","),
                field("extra", $._constructor_application_extra),
              ),
            ),
          ),
        ),
      ),

    _constructor_application_extra: $ =>
      prec.left(
        "constructor_application",
        choice(
          $._annotated_type,
          $.compound_type,
          seq(
            $._simple_type,
            field(
              "arguments",
              repeat1(prec("constructor_application", $.arguments)),
            ),
          ),
        ),
      ),

    /**
     * ConstrApp         ::=  SimpleType1 {Annotation} {ParArgumentExprs}
     *
     * Note: It would look more elegant if we could make seq(choice(), optional(arguments)),
     * but that doesn't seem to work.
     */
    _constructor_application: $ =>
      prec.left(
        "constructor_application",
        choice(
          $._annotated_type,
          $.compound_type,
          // In theory structural_type should just be added to simple_type,
          // but doing so increases the state of template_body to 4000
          $._structural_type,
          // This adds _simple_type, but not the above intentionally.
          // Constructor arguments attach only to a non-annotated simple type.
          // After an annotation, argument lists belong to the annotation.
          // scalac parses annotation arguments greedily in the same way.
          seq(
            $._simple_type,
            field(
              "arguments",
              repeat1(prec("constructor_application", $.arguments)),
            ),
          ),
        ),
      ),

    _constructor_applications: $ =>
      prec.left(
        choice(
          commaSep1($._constructor_application),
          sep1("with", $._constructor_application),
        ),
      ),

    modifiers: $ =>
      prec.left(
        repeat1(
          prec.left(
            choice(
              "abstract",
              "final",
              "sealed",
              "implicit",
              "lazy",
              "override",
              $.access_modifier,
              inlineMod($),
              erasedMod($),
              infixMod($),
              intoMod($),
              openMod($),
              trackedMod($),
              transparentMod($),
              updateMod($),
              consumeMod($),
            ),
          ),
        ),
      ),

    access_modifier: $ =>
      prec.left(
        seq(choice("private", "protected"), optional($.access_qualifier)),
      ),

    access_qualifier: $ => seq("[", $._identifier, "]"),

    /**
     * InheritClauses    ::=  ['extends' ConstrApps] ['derives' QualId {',' QualId}]
     */
    extends_clause: $ =>
      prec.left(seq("extends", field("type", $._constructor_applications))),

    _derived_names: $ =>
      commaSep1(
        field("type", choice($._type_identifier, $.stable_type_identifier)),
      ),

    derives_clause: $ => prec.left(seq("derives", $._derived_names)),

    uses_clause: $ => prec.left(seq("uses", $._derived_names)),

    class_parameters: $ =>
      prec(
        1,
        seq(
          optional($._automatic_semicolon),
          "(",
          choice(
            seq(
              "using",
              choice(
                trailingCommaSep1($.class_parameter),
                trailingCommaSep1($._param_type),
              ),
            ),
            seq(optional("implicit"), trailingCommaSep($.class_parameter)),
          ),
          ")",
        ),
      ),

    /*
     * DefParamClauses   ::=  {DefParamClause} [[nl] ‘(’ [‘implicit’] DefParams ‘)’]
     * DefParamClause    ::=  [nl] ‘(’ DefParams ‘)’ | UsingParamClause
     * DefParams         ::=  DefParam {‘,’ DefParam}
     */
    _parameters_tail: $ => seq(trailingCommaSep($.parameter), ")"),

    parameters: $ =>
      choice(
        seq("(", optional("implicit"), $._parameters_tail),
        $._using_parameters_clause,
      ),

    /*
     * UsingParamClause  ::=  [nl] ‘(’ ‘using’ (DefParams | FunArgTypes) ‘)’
     * DefParams         ::=  DefParam {‘,’ DefParam}
     * FunArgTypes       ::=  FunArgType { ‘,’ FunArgType }
     */
    _using_parameters_clause: $ =>
      seq(
        "(",
        "using",
        choice(
          trailingCommaSep1($.parameter),
          trailingCommaSep1($._param_type),
        ),
        ")",
      ),

    class_parameter: $ =>
      seq(
        repeat($.annotation),
        optional($.modifiers),
        optional(choice("val", "var")),
        field("name", $._identifier),
        optional(seq(":", field("type", $._param_type))),
        optional(seq("=", field("default_value", $.expression))),
      ),

    /*
     * DefParam          ::=  {Annotation} [‘inline’] Param
     * Param             ::=  id ‘:’ ParamType [‘=’ Expr]
     */
    parameter: $ =>
      prec.left(
        PREC.control,
        seq(
          repeat($.annotation),
          optional(inlineMod($)),
          optional(choice(erasedMod($), consumeMod($))),
          // A given conditional clause takes `tracked val`.
          optional(trackedMod($)),
          optional(choice("val", "var")),
          field("name", $._identifier),
          ":",
          field("type", $._param_type),
          optional(seq("=", field("default_value", $.expression))),
        ),
      ),

    /*
     * NameAndType       ::=  id ':' Type
     */
    name_and_type: $ =>
      prec.left(
        PREC.control,
        // Inside parens scalac turns COLONeol off, but the scanner still emits
        // it, so the typed reading must accept both spellings. Keep in sync
        // with $.binding, whose token path this rule shares.
        seq(
          optional(erasedMod($)),
          field("name", $._identifier),
          colonEol($),
          field("type", $._param_type),
        ),
      ),

    // Any separator may be followed by extra empty statements (`;`), so
    // `}\n;{ ... }` parses like scalac. A semicolon-only block (`{ ;; }`)
    // is kept as its own branch.
    _block: $ =>
      prec.left(
        choice(
          seq(repeat1(";"), optional($._block_statements)),
          $._block_statements,
        ),
      ),

    _semis: $ => seq($._semicolon, repeat(";")),

    _block_statements: $ =>
      prec.left(
        seq(
          sep1($._semis, choice(statementExpression($), $._definition)),
          optional($._semis),
        ),
      ),

    _indentable_expression: $ =>
      prec.right(
        choice($.indented_block, $.indented_cases, statementExpression($)),
      ),

    block: $ =>
      seq(
        "{",
        optional(
          choice(
            $._block,
            alias($._block_lambda_expression, $.lambda_expression),
          ),
        ),
        "}",
      ),

    indented_block: $ =>
      prec.left(
        PREC.control,
        seq(
          $._indent,
          choice(
            $._block,
            // A lambda statement takes the rest of the enclosing block as
            // its body (SLS ResultExpr), exactly as in braces.
            alias($._indented_block_lambda, $.lambda_expression),
          ),
          choice($._outdent, $._comma_outdent),
        ),
      ),

    indented_cases: $ =>
      prec.left(
        seq(
          $._indent,
          repeat1($.case_clause),
          choice($._outdent, $._comma_outdent),
        ),
      ),

    // ---------------------------------------------------------------
    // Types

    _type: $ =>
      choice(
        $.function_type,
        $.compound_type,
        $.capturing_type,
        $.infix_type,
        $.match_type,
        $._annotated_type,
        $.literal_type,
        $._structural_type,
        $.type_lambda,
        $.existential_type,
      ),

    // Scala 2 existential type (SLS §3.2.12): `P[T] forSome { type T }`
    existential_type: $ =>
      prec.left(
        seq(field("type", $._infix_type_choice), "forSome", $._refinement),
      ),

    _annotated_type: $ => prec.right(choice($.annotated_type, $._simple_type)),

    // A literal type carries annotations too (`val x: "abc" @deprecated`), and
    // it is not one of the simple types.
    annotated_type: $ =>
      prec.right(
        seq(choice($._simple_type, $.literal_type), repeat1($.annotation)),
      ),

    _simple_type: $ =>
      choice(
        $.generic_type,
        $.projected_type,
        $.tuple_type,
        $.named_tuple_type,
        $.singleton_type,
        $.stable_type_identifier,
        $._type_identifier,
        $.applied_constructor_type,
        $.wildcard,
      ),

    applied_constructor_type: $ =>
      prec(
        "applied_constructor_type",
        seq(
          choice(
            $.generic_type,
            $.projected_type,
            $.stable_type_identifier,
            $._type_identifier,
          ),
          $.arguments,
        ),
      ),

    compound_type: $ =>
      choice(
        prec.left(
          PREC.compound,
          seq(
            field("base", $._annotated_type),
            repeat1(seq("with", field("extra", $._annotated_type))),
          ),
        ),
        // Dotty's refinedTypeRest recurses on itself, so a type takes any
        // number of refinements. `C { type U = T } { type T = String }`.
        prec.left(
          seq(
            field("base", choice($._annotated_type, $.compound_type)),
            $._refinement,
          ),
        ),
        prec.left(
          -1,
          seq(
            prec.left(
              PREC.compound,
              seq(
                field("base", $._annotated_type),
                repeat1(seq("with", field("extra", $._annotated_type))),
              ),
            ),
            $._refinement,
          ),
        ),
      ),

    // Braced only, as dotty's refinement(indentOK = false) is. The indented
    // spelling belongs to the refinement suffix, which keeps template_body.
    // A type followed by `^`, optionally with the set of capabilities it
    // captures. The braced set outweighs the refinement reading of the same
    // braces.
    capturing_type: $ =>
      choice(
        prec.left(
          2,
          seq(
            field("base", choice($._annotated_type, $.compound_type)),
            "^",
            field("capture_set", $.capture_set),
          ),
        ),
        prec.left(
          1,
          seq(field("base", choice($._annotated_type, $.compound_type)), "^"),
        ),
      ),

    capture_set: $ => seq("{", optional(sep1(",", $.capture_ref)), "}"),

    // A capability, with any run of the suffixes that narrow it: `.only[C]`
    // and `.except[C]` take a class, `.rd` takes none.
    capture_ref: $ =>
      seq(
        choice($._identifier, $.stable_identifier, "cap"),
        repeat(
          seq(
            ".",
            choice(
              seq(
                choice("only", "except"),
                field("type_arguments", $.type_arguments),
              ),
              "rd",
            ),
          ),
        ),
      ),

    _structural_type: $ =>
      prec("structural_type", alias($._structural_body, $.structural_type)),

    // The dynamic weight keeps `new { "ok" }` an instance expression, which
    // shares this body and would otherwise tie with it.
    _structural_body: $ => prec.dynamic(-1, $._braced_template_body),

    _refinement: $ => alias($.template_body, $.refinement),

    // This does not include _simple_type since _annotated_type covers it.
    _infix_type_choice: $ =>
      prec.left(
        choice(
          $.compound_type,
          $.capturing_type,
          $.infix_type,
          $._annotated_type,
          $.literal_type,
        ),
      ),

    infix_type: $ =>
      prec.left(
        seq(
          // SimpleType1 admits a bare Refinement: `A & { type X = Int }`.
          field("left", choice($._infix_type_choice, $._structural_type)),
          field("operator", wordOrOpName($)),
          field("right", choice($._infix_type_choice, $._structural_type)),
        ),
      ),

    tuple_type: $ => seq("(", trailingCommaSep1($._type), ")"),

    named_tuple_type: $ => seq("(", trailingCommaSep1($.name_and_type), ")"),

    singleton_type: $ =>
      prec.left(
        PREC.stable_type_id,
        seq(choice($._identifier, $.stable_identifier), ".", "type"),
      ),

    stable_type_identifier: $ =>
      prec.left(
        PREC.stable_type_id,
        seq(
          choice($._identifier, $.stable_identifier),
          ".",
          $._type_identifier,
        ),
      ),

    stable_identifier: $ =>
      prec.left(
        PREC.stable_id,
        seq(choice($._identifier, $.stable_identifier), ".", $._identifier),
      ),

    generic_type: $ =>
      seq(
        field("type", $._simple_type),
        field("type_arguments", $.type_arguments),
      ),

    projected_type: $ =>
      seq(
        field("type", $._simple_type),
        "#",
        field("selector", $._type_identifier),
      ),

    match_type: $ =>
      prec.left(
        seq(
          $._infix_type_choice,
          "match",
          choice(
            seq($._indent, repeat1($.type_case_clause), $._outdent),
            seq("{", repeat1($.type_case_clause), "}"),
          ),
        ),
      ),

    type_case_clause: $ =>
      prec.left(
        PREC.control,
        // Dotty's typeCaseClause takes one optional `;` after the body, so a
        // separator is allowed after every clause including the last.
        seq(
          "case",
          $._infix_type_choice,
          field("body", $._arrow_then_type),
          optional(";"),
        ),
      ),

    // A plain type wins where both readings are legal (ascription, pattern,
    // colon argument). In a lambda parameter clause a plain type leaves the
    // arrow dangling, so the function type is the only survivor there.
    function_type: $ =>
      prec.dynamic(
        -1,
        prec.left(
          choice(
            seq(
              field("type_parameters", $.type_parameters),
              $._arrow_then_type,
            ),
            seq(
              field("parameter_types", $.parameter_types),
              $._arrow_then_type,
            ),
          ),
        ),
      ),

    _arrow_then_type: $ =>
      prec.right(
        seq(
          typeArrow(),
          optional(field("capture_set", $.capture_set)),
          field("return_type", $._type),
        ),
      ),

    parameter_types: $ =>
      choice(
        $._annotated_type,
        $.capturing_type,
        // Prioritize a parenthesized param list over a single tuple_type.
        prec.dynamic(1, seq("(", trailingCommaSep($._param_type), ")")),
        $.compound_type,
        $.infix_type,
      ),

    _param_type: $ => choice($.lazy_parameter_type, $._param_value_type),

    _param_value_type: $ =>
      choice(field("type", $._type), $.repeated_parameter_type),

    repeated_parameter_type: $ => seq(field("type", $._type), $._asterisk),

    lazy_parameter_type: $ =>
      seq(
        choice(fatArrow(), pureArrow()),
        optional(field("capture_set", $.capture_set)),
        field("type", $._param_value_type),
      ),

    _type_identifier: $ => alias($._identifier, $.type_identifier),

    type_lambda: $ =>
      seq(
        "[",
        trailingCommaSep1($._type_parameter),
        "]",
        "=>>",
        field("return_type", $._type),
      ),

    // ---------------------------------------------------------------
    // Patterns

    _pattern: $ =>
      choice(
        $._definition_pattern,
        $.alternative_pattern,
        $.typed_pattern,
        $.repeat_pattern,
      ),

    // SLS 4.1: the pattern of a val/var definition is a Pattern2, so no
    // top-level alternatives or ascription. `val a: A | B` then reads `|`
    // as a union type instead of an alternative pattern.
    _definition_pattern: $ =>
      choice(
        $._identifier,
        $.stable_identifier,
        $.interpolated_string_expression,
        $.capture_pattern,
        $.tuple_pattern,
        $.named_tuple_pattern,
        $.case_class_pattern,
        $.infix_pattern,
        $.given_pattern,
        $.quote_expression,
        $.literal,
        // The unit value is a pattern too (`case () =>`). Without it the
        // parens read as a tuple pattern and recovery invents its element.
        $.unit,
        $.wildcard,
        $.xml_pattern,
      ),

    // The type arguments are the ones an extractor takes explicitly
    // (`case Foo[Int](x)`), which SLS 8.1.8 admits before the patterns.
    case_class_pattern: $ =>
      seq(
        field("type", choice($._type_identifier, $.stable_type_identifier)),
        optional(field("type_arguments", $.type_arguments)),
        "(",
        choice(
          field("pattern", trailingCommaSep($._pattern)),
          field("pattern", trailingCommaSep($.named_pattern)),
        ),
        ")",
      ),

    // Operands are _definition_pattern: a full _pattern would re-admit the
    // excluded items through the left recursion.
    infix_pattern: $ =>
      prec.left(
        PREC.infix,
        seq(
          field("left", $._definition_pattern),
          field("operator", wordOrOpName($)),
          field("right", $._definition_pattern),
        ),
      ),

    capture_pattern: $ =>
      prec.right(
        PREC.field,
        seq(
          field("name", choice($._identifier, $.wildcard)),
          "@",
          field("pattern", $._pattern),
        ),
      ),

    repeat_pattern: $ =>
      prec.right(seq(field("pattern", $._pattern), $._asterisk)),

    typed_pattern: $ =>
      prec.right(
        -1,
        seq(field("pattern", $._pattern), ":", field("type", $._type)),
      ),

    given_pattern: $ => seq("given", field("type", $._type)),

    // TODO: Flatten this.
    alternative_pattern: $ => prec.left(-2, seq($._pattern, "|", $._pattern)),

    tuple_pattern: $ => seq("(", trailingCommaSep1($._pattern), ")"),

    named_pattern: $ => prec.left(-1, seq($._identifier, "=", $._pattern)),

    named_tuple_pattern: $ => seq("(", trailingCommaSep1($.named_pattern), ")"),

    // ---------------------------------------------------------------
    // Expressions

    expression: $ =>
      choice(
        $.if_expression,
        $.match_expression,
        $.try_expression,
        $.assignment_expression,
        $.lambda_expression,
        $.postfix_expression,
        $.ascription_expression,
        $._infix_operand,
        $.return_expression,
        $.throw_expression,
        $.while_expression,
        $.for_expression,
        $.macro_body,
      ),

    // The operand chain of infix and postfix operators (SLS 6.12 InfixExpr).
    // The single nonterminal keeps the 21 per-class infix productions from
    // multiplying out their operand alternatives in the parse table, so every
    // rule admitting these three forms must go through it (a direct sibling
    // reference would reintroduce a competing unit reduction).
    _infix_operand: $ =>
      choice($.infix_expression, $.prefix_expression, $._simple_expression),

    /**
     *  SimpleExpr        ::=  SimpleRef
     *                      |  Literal
     *                      |  '_'
     *                      |  BlockExpr
     *                      |  ExprSplice
     *                      |  Quoted
     *                      |  quoteId
     *                      |  'new' ConstrApp {'with' ConstrApp} [TemplateBody]
     *                      |  'new' TemplateBody
     *                      |  '(' ExprsInParens ')'
     *                      |  SimpleExpr '.' id
     *                      |  SimpleExpr '.' MatchClause
     *                      |  SimpleExpr TypeArgs
     *                      |  SimpleExpr ArgumentExprs
     *                      |  SimpleExpr ColonArgument
     * TODO: ColonArgument
     */
    _simple_expression: $ =>
      choice(
        nameChoice($),
        $.operator_identifier,
        $.literal,
        $.interpolated_string_expression,
        $.unit,
        $.tuple_expression,
        $.wildcard,
        $.block,
        $.splice_expression,
        $.case_block,
        $.quote_expression,
        $.instance_expression,
        $.parenthesized_expression,
        $.field_expression,
        $.generic_function,
        $.call_expression,
        $.xml_expression,
        $.method_value,
        alias($._dot_match_expression, $.match_expression),
      ),

    /**
     * SimpleExpr ::= SimpleExpr1 '_'  (SLS 6.7 Method Values, `f _`)
     * A simple expression, so it can be an infix operand.
     */
    method_value: $ =>
      prec.left(PREC.call, seq($._simple_expression, $.wildcard)),

    _single_lambda_param: $ =>
      prec.right(seq(optional("implicit"), operandName($))),

    // Keeps a braced `{ x: T => ... }` a lambda instead of a fewer-braces colon
    // argument on `x`.
    lambda_expression: $ =>
      prec.dynamic(
        1,
        prec.right(
          "lambda",
          seq(
            optional(
              seq(field("type_parameters", $.type_parameters), fatArrow()),
            ),
            field(
              "parameters",
              // No unparenthesized typed parameter here. It is only legal
              // inside braces, and `OWrites: c => body` must stay a colon
              // argument.
              choice($.bindings, $.wildcard, $._single_lambda_param),
            ),
            anyArrow(),
            $._indentable_expression,
          ),
        ),
      ),

    /* Special-case lambda expression to handle lambdas in braces (in $.block), e.g.
     * { (...) => val a = 1; val b = 2
     *    3
     * }
     *
     * It exists as a separate rule because grammar generation becomes unacceptably slow
     * if we include $._block right into $.lambda_expression as a viable option for the lambda body.
     */
    _block_lambda_expression: $ =>
      prec.right(
        "lambda",
        choice(
          seq(
            field(
              "parameters",
              choice($.bindings, $.wildcard, $._single_lambda_param),
            ),
            anyArrow(),
            optional(
              choice(
                $._block,
                // Curried form ending in a typed lambda, as in
                // `{ _ => source: Source[ByteString, Any] => body }`.
                alias($._braced_typed_lambda, $.lambda_expression),
              ),
            ),
          ),
          $._braced_typed_lambda,
        ),
      ),

    // The indented twin of _block_lambda_expression, without the typed
    // branches. A typed head there (`x: T =>`) must stay a fewer-braces
    // colon argument on `x`.
    _indented_block_lambda: $ =>
      prec.right(
        "lambda",
        seq(
          field(
            "parameters",
            choice($.bindings, $.wildcard, $._single_lambda_param),
          ),
          anyArrow(),
          optional($._block),
        ),
      ),

    // `{ x: T => body }` is legal only as a block result. The weight beats
    // the reading of `x: T` as a statement taking a colon argument.
    _braced_typed_lambda: $ =>
      prec.dynamic(
        1,
        prec.right(
          "lambda",
          seq(
            field(
              "parameters",
              prec.right(
                seq(optional("implicit"), operandName($), ":", $._type),
              ),
            ),
            anyArrow(),
            $._indentable_expression,
          ),
        ),
      ),

    /*
     *  ::=  [‘inline’] ‘if’ ‘(’ Expr ‘)’ {nl} Expr [[semi] ‘else’ Expr]
     *    |  [‘inline’] ‘if’  Expr ‘then’ Expr [[semi] ‘else’ Expr]
     */
    if_expression: $ => seq(optional(inlineMod($)), $._if_rest),

    _if_rest: $ =>
      seq(
        "if",
        choice(
          // No marker slot here. Paren-if marker heads multiply the block
          // lambda forks over the GLR version limit in brace-heavy files,
          // and real code only marks then-style ifs.
          seq(
            field("condition", $._if_condition_paren),
            // The then-form twin makes the scanner emit a semicolon here.
            optional($._automatic_semicolon),
            $._if_body,
          ),
          seq(
            field("condition", $._if_condition_then),
            $._if_body,
            optional($._end_marker_kw_tail),
          ),
        ),
      ),

    _if_body: $ =>
      seq(
        field("consequence", $._indentable_expression),
        optional(
          // The weight keeps the else attached to this if when a losing
          // statement reading of the else line ties with it.
          prec.dynamic(
            1,
            seq(
              optional(";"),
              $._control_tail_gate,
              "else",
              field("alternative", $._indentable_expression),
            ),
          ),
        ),
      ),

    // NOTE(susliko): the magic dynamic precedence was introduced as a fix to
    // https://github.com/tree-sitter/tree-sitter-scala/issues/263 and
    // https://github.com/tree-sitter/tree-sitter-scala/issues/342
    // Neither do I understand why this works, nor have I found a better solution
    _if_condition_paren: $ => prec.dynamic(4, $.parenthesized_expression),

    _if_condition_then: $ =>
      // A marker slot of a construct inside the condition may emit a
      // semicolon before the `then`. The weight beats the paren form, so
      // `if (a)` + newline + `then x` keeps `then` as the keyword.
      prec.dynamic(
        5,
        seq(
          $._indentable_expression,
          optional($._automatic_semicolon),
          $._control_tail_gate,
          "then",
        ),
      ),

    /*
     *   MatchClause       ::=  'match' <<< CaseClauses >>>
     *
     *   Handles:
     *     InfixExpr MatchClause
     *     ‘inline’ InfixExpr MatchClause
     *     SimpleExpr ‘.’ MatchClause
     */
    match_expression: $ =>
      choice(
        seq(
          optional(inlineMod($)),
          field("value", $.expression),
          "match",
          // A braced case block takes no marker. Real code never writes
          // one, and the marker heads it would add after every `}` of a
          // case block press on the GLR version limit in map-heavy code.
          choice(
            field("body", $.case_block),
            seq(
              field("body", $.indented_cases),
              optional($._end_marker_kw_tail),
            ),
          ),
        ),
        $._dot_match_expression,
      ),

    _dot_match_expression: $ =>
      seq(
        field("value", $._simple_expression),
        ".",
        token.immediate("match"),
        field("body", choice($.case_block, $.indented_cases)),
        optional($._end_marker_kw_tail),
      ),

    // The marker tail sits outside the precedence wrapper. Inside it, the
    // right associativity would force `end f` onto the try instead of the
    // enclosing definition.
    try_expression: $ =>
      seq(
        prec.right(
          PREC.control,
          seq(
            "try",
            field("body", $._indentable_expression),
            optional($.catch_clause),
            optional($.finally_clause),
          ),
        ),
        optional($._end_marker_kw_tail),
      ),

    /*
     *   Catches           ::=  'catch' (Expr | ExprCaseClause)
     */
    catch_clause: $ =>
      prec.right(
        seq(
          $._control_tail_gate,
          "catch",
          choice($._indentable_expression, $._expr_case_clause),
        ),
      ),

    _expr_case_clause: $ =>
      prec.left(
        seq(
          "case",
          $._case_pattern,
          field("body", choice($.expression, $.indented_block)),
        ),
      ),

    finally_clause: $ =>
      prec.right(
        seq($._control_tail_gate, "finally", $._indentable_expression),
      ),

    /*
     * Binding           ::=  (id | ‘_’) [‘:’ Type]
     */
    binding: $ =>
      seq(
        optional(erasedMod($)),
        choice(field("name", operandName($)), $.wildcard),
        // colonEol here mirrors name_and_type, the shared-token twin of this rule.
        optional(seq(colonEol($), field("type", $._param_type))),
      ),

    // Keeps `(f: A => B) => ...` a lambda parameter clause, not a parenthesized
    // expression joined by an infix `=>`.
    bindings: $ => prec.dynamic(2, seq("(", trailingCommaSep($.binding), ")")),

    case_block: $ =>
      choice(prec(-1, seq("{", "}")), seq("{", repeat1($.case_clause), "}")),

    case_clause: $ =>
      prec.left(
        seq("case", $._case_pattern, field("body", optional($._block))),
      ),

    // Dynamic precedence to win over lambda_expression in complex contexts
    _case_pattern: $ =>
      prec.dynamic(
        1,
        seq(field("pattern", $._pattern), optional($.guard), fatArrow()),
      ),

    guard: $ =>
      prec.left(
        PREC.control,
        seq("if", field("condition", $._postfix_expression_choice)),
      ),

    assignment_expression: $ =>
      prec.right(
        PREC.assign,
        seq(
          field("left", choice($.prefix_expression, $._simple_expression)),
          "=",
          field("right", choice(statementExpression($), $.indented_block)),
        ),
      ),

    generic_function: $ =>
      prec(
        PREC.call,
        seq(
          field("function", $.expression),
          field("type_arguments", $.type_arguments),
        ),
      ),

    call_expression: $ =>
      choice(
        prec.left(
          PREC.call,
          seq(
            field("function", $._simple_expression),
            field("arguments", choice($.arguments, $.case_block, $.block)),
          ),
        ),
        prec.right(
          PREC.colon_call,
          seq(
            field("function", $._postfix_expression_choice),
            colonEol($),
            field("arguments", $.colon_argument),
          ),
        ),
      ),

    /**
     *   ColonArgument     ::=  colon [LambdaStart]
     *                          (CaseClauses | Block)
     */
    colon_argument: $ =>
      prec.left(
        PREC.colon_call,
        seq(
          optional(
            field(
              "lambda_start",
              seq(
                choice(
                  alias($._colon_bindings, $.bindings),
                  $._identifier,
                  $.wildcard,
                ),
                // anyArrow so a context-function lambda `x ?=>` also starts a
                // colon argument, not only the plain `=>` form.
                anyArrow(),
              ),
            ),
          ),
          choice($.indented_block, $.indented_cases),
        ),
      ),

    // Parenthesized lambda parameters of a colon argument. Typed parameters go
    // through name_and_type (aliased to binding) so they share a token path
    // with named_tuple_type, avoiding the reduce conflict that killed the lambda.
    // Keeps `foo: () => x` a colon argument with an empty parameter clause, not
    // an ascription of `foo` to `() => x`.
    _colon_bindings: $ =>
      prec.dynamic(
        3,
        seq(
          "(",
          trailingCommaSep(
            choice($.binding, alias($.name_and_type, $.binding)),
          ),
          ")",
        ),
      ),

    field_expression: $ =>
      prec.left(
        PREC.field,
        seq(
          field("value", $._simple_expression),
          ".",
          field("field", $._identifier),
        ),
      ),

    /**
     *   SimpleExpr        ::=  SimpleRef
     *                      |  'new' ConstrApp {'with' ConstrApp} [TemplateBody]
     *                      |  'new' TemplateBody
     */
    instance_expression: $ =>
      choice(
        // 1 beats the colon-argument reading of `new C:` and still loses
        // to the ascription of `new Array: Array`.
        prec.dynamic(
          1,
          seq(
            "new",
            $._constructor_application,
            $.template_body,
            optional($._end_marker_kw_tail),
          ),
        ),
        prec(
          "new",
          seq("new", $.template_body, optional($._end_marker_kw_tail)),
        ),
        prec(
          "new",
          seq(
            "new",
            field("early_defs", $.early_defs),
            "with",
            $._constructor_application,
          ),
        ),
        seq("new", $._constructor_application),
      ),

    // A visible copy of _braced_template_body. It cannot alias that rule
    // because the rule is inlined, which would empty the aliased node.
    early_defs: $ =>
      prec.left(
        PREC.control,
        seq(
          "{",
          optional(choice($._braced_template_body1, $._braced_template_body2)),
          "}",
        ),
      ),

    /**
     * PostfixExpr [Ascription]
     */
    // The arrow tail lives here (not via $.function_type) because after `:`
    // the function-type reading loses statically to self-type/lambda readings.
    // The dynamic penalty keeps `{ x: Int => body }` a lambda, like scalac.
    ascription_expression: $ =>
      prec.left(
        seq(
          choice($._postfix_expression_choice, $.match_expression),
          ":",
          choice(
            seq(
              field("type", $._param_type),
              repeat(prec(1, prec.dynamic(-1, ascriptionArrowTail($)))),
            ),
            // The alias shares the `identifier '=>'` token path with
            // colon_argument's lambda start (shift/shift, not shift/reduce).
            prec.dynamic(
              -1,
              seq(
                field("type", alias($._identifier, $.type_identifier)),
                repeat1(ascriptionArrowTail($)),
              ),
            ),
            // SLS 6.13: `Ascription ::= ':' Annotation {Annotation}`.
            repeat1($.annotation),
          ),
        ),
      ),

    // One production per SLS 6.12.3 precedence class. The operator tokens are
    // partitioned by first character (see OP_TOKEN), so each class carries its
    // own precedence and associativity: alphanumeric operators bind loosest,
    // `=`-ending assignment operators sit below them, and an operator ending
    // in `:` is right-associative. An operator that ends its line continues
    // the expression on the next line (SLS 1.2) with the same tokens: the
    // postfix reading is the external _postfix_op, so after `left op` only the
    // right operand can follow and the newline cannot split the statement.
    // No colon-argument choice belongs here. postfix_expression already
    // covers `a op:` bodies and it shadowed postfix ascriptions.
    infix_expression: $ => {
      const production = (level, assoc, operator) =>
        (assoc === "right" ? prec.right : prec.left)(
          level,
          seq(
            field("left", $._infix_operand),
            field("operator", operator),
            field("right", $._infix_operand),
          ),
        );
      const op = tok => alias(tok, $.operator_identifier);
      // The alphanumeric operator takes the bare word tokens, not the
      // identifier rule: `this`, `super`, and the soft-keyword tokens never
      // continue an infix chain, so their columns drop out of every
      // post-expression state. A soft keyword used as an infix method
      // (`x open y`) still lexes here through keyword extraction, because
      // no soft-keyword token is valid before the statement boundary.
      return choice(
        production(PREC.iname, "left", wordName($)),
        production(PREC.iassign, "left", op(OP_TOKEN.assign)),
        production(PREC.ior, "left", op($._op_left_or)),
        production(PREC.ior, "right", op(OP_TOKEN.orRight)),
        production(PREC.ixor, "left", op($._op_left_xor)),
        production(PREC.ixor, "right", op(OP_TOKEN.xorRight)),
        production(PREC.iand, "left", op($._op_left_and)),
        production(PREC.iand, "right", op(OP_TOKEN.andRight)),
        production(PREC.ieq, "left", op($._op_left_eq)),
        production(PREC.ieq, "right", op(OP_TOKEN.eqRight)),
        production(PREC.irel, "left", op($._op_left_rel)),
        production(PREC.irel, "right", op(OP_TOKEN.relRight)),
        production(PREC.icolon, "left", op($._op_left_colon)),
        production(PREC.icolon, "right", op(OP_TOKEN.colonRight)),
        production(PREC.iadd, "left", op($._op_left_add)),
        production(PREC.iadd, "right", op(OP_TOKEN.addRight)),
        production(
          PREC.imul,
          "left",
          // A lone `*` lexes as the shared _asterisk token, so mulLeft's
          // single-character alternative omits it (see _asterisk).
          choice(alias($._asterisk, $.operator_identifier), op($._op_left_mul)),
        ),
        production(PREC.imul, "right", op(OP_TOKEN.mulRight)),
        production(PREC.iother, "left", op($._op_left_other)),
        production(PREC.iother, "right", op(OP_TOKEN.otherRight)),
      );
    },

    /**
     * PostfixExpr       ::=  InfixExpr [id]
     * A symbolic postfix operator arrives as the external _postfix_op or
     * _postfix_star token, which no infix production accepts, so the whole
     * lower-precedence chain reduces first and the operator attaches to the
     * full InfixExpr. An alphanumeric operator shares the iname level with
     * its infix production; prec.right makes the tie shift, so it reduces as
     * postfix only where the infix reading cannot continue.
     */
    postfix_expression: $ =>
      choice(
        // Bare word tokens for the same reason as the iname production.
        prec.right(PREC.iname, seq($._infix_operand, wordName($))),
        prec.left(
          PREC.postfix,
          seq(
            $._infix_operand,
            alias(
              choice($._postfix_op, $._postfix_star),
              $.operator_identifier,
            ),
          ),
        ),
      ),

    _postfix_expression_choice: $ =>
      prec.left(PREC.postfix, choice($.postfix_expression, $._infix_operand)),

    macro_body: $ => prec.left(PREC.macro, seq("macro", $._infix_operand)),

    /**
     * PrefixExpr        ::=  [PrefixOperator] SimpleExpr
     */
    prefix_expression: $ =>
      prec(PREC.prefix, seq(choice("+", "-", "!", "~"), $._simple_expression)),

    tuple_expression: $ =>
      seq(
        "(",
        $.expression,
        repeat1(seq(",", $.expression)),
        optional(","),
        ")",
      ),

    parenthesized_expression: $ => seq("(", $.expression, ")"),

    // NamedTypeArg ::= id '=' Type. Scala 3 takes named type arguments the
    // way it takes named term arguments.
    type_arguments: $ =>
      seq(
        "[",
        trailingCommaSep1(
          choice($._type, $.capture_set, $.named_type_argument),
        ),
        "]",
      ),

    named_type_argument: $ =>
      seq(field("name", $._identifier), "=", field("type", $._type)),

    arguments: $ =>
      seq(
        "(",
        choice(optional($._argument_list), seq("using", $._exprs_in_parens)),
        ")",
      ),

    // One rule for the plain and vararg-tailed forms. Sharing the repeat means
    // no conflict is needed between continuing the list and starting a vararg,
    // which in turn lets the list inline away its unit reduction.
    _argument_list: $ =>
      seq(sep1(",", choice($.expression, $.vararg)), optional(",")),

    vararg: $ =>
      choice(
        // Scala 3: `args*`. The star in front of a closing delimiter arrives
        // as the external _postfix_star, which the postfix reading also
        // accepts, so the higher level settles the tie in vararg's favor.
        prec(PREC.imul, seq($._simple_expression, $._postfix_star)),
        // Scala 2: `args: _*`. The weight beats the ascription reading of the
        // same text, which the parser also completes.
        prec.dynamic(
          1,
          seq($._simple_expression, ":", token(seq("_", token.immediate("*")))),
        ),
      ),

    // ExprsInParens     ::=  ExprInParens {‘,’ ExprInParens}
    _exprs_in_parens: $ => trailingCommaSep1($.expression),

    // The opening is a single two-character token so that a bare `$` used as
    // an ordinary identifier (e.g. `$(selector)`) still lexes as an
    // identifier. A `$ident` splice already lexes as one identifier anyway.
    splice_expression: $ =>
      prec.left(
        PREC.macro,
        choice(seq("${", $._block, "}"), seq("$[", $._type, "]")),
      ),

    quote_expression: $ =>
      prec.left(
        PREC.macro,
        seq(
          "'",
          choice(
            // `'{}` is a quoted empty block, common in macro code.
            seq("{", optional($._block), "}"),
            // TypeBlock ::= {TypeBlockStat semi} Type. The bindings let a
            // quote name the type variables it matches on.
            seq(
              "[",
              repeat(seq($.type_definition, $._semicolon)),
              $._type,
              "]",
            ),
            $.identifier,
            $.null_literal,
            $.boolean_literal,
          ),
        ),
      ),

    /**
     * id               ::=  plainid
     *                       |  ‘`’ { charNoBackQuoteOrNewline | UnicodeEscape | charEscapeSeq
     */
    identifier: $ =>
      choice(
        $._alpha_identifier,
        $._backquoted_id,
        $._soft_identifier,
        $._this_identifier,
        $._super_identifier,
      ),

    _this_identifier: $ => "this",
    _super_identifier: $ => "super",

    // https://docs.scala-lang.org/scala3/reference/soft-modifier.html
    _soft_identifier: $ => prec("soft_id", "extension"),

    // alphaid          ::=  upper idrest
    //                       |  varid
    // We approximate the above as:
    // /[A-Za-z\$_][A-Z\$_a-z0-9]*(_[\-!#%&*+\/\\:<=>?@\u005e\u007c~]+)?/,
    //
    // The following is more accurate, but the state count goes over the unsigned short int, and should be comparable.
    // /([\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$][\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F0-9]*(_[\-!#%&*+\/\\:<=>?@\u005e\u007c~]+)?|[\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F_][\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F0-9]*(_[\-!#%&*+/\\:<=>?@\u005e\u007c~]+)?|[\-!#%&*+\/\\:<=>?@\u005e\u007c~]+)|[\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F_][\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F0-9]*(_[\-!#%&*+\/\\:<=>?@\u005e\u007c~]+)?/,
    //
    // The `_`-prefixed operator tail follows the same comment rule as an
    // operator, so `setter_=/*c*/` ends at the `=` (see OP_STEP). A bare `_/`
    // is carved out for names like `Int_/`, which the scanner cannot rescue
    // the way it does operators, so `x_//c` still takes the `/` as the name.
    _alpha_identifier: $ =>
      token(
        seq(
          /[\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F\$][\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\$\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F0-9\$_\p{Ll}]*/,
          optional(opRegex(`_(?:${OP_STEP}+|/)`)),
        ),
      ),

    /**
     * Despite what the lexical syntax suggests, the alphaid rule doesn't apply
     * to identifiers that aren't in blocks in interpolated strings (e.g. $foo).
     * A more accurate description is given in
     * https://www.scala-lang.org/files/archive/spec/2.13/01-lexical-syntax.html
     * where it states (regarding dollar sign escapes in interpolated strings) that
     * """
     * The simpler form consists of a ‘$’-sign followed by an identifier starting
     * with a letter and followed only by letters, digits, and underscore characters
     * """
     * where "letters" does not include the $ character.
     *
     * This rule is similar to the _alpha_identifier rule, with the differences
     * being that the $ character is excluded, along with the _(operator_chars)
     * suffix and can be approximated as
     * /[A-Za-z_][A-Z_a-z0-9]/;
     */
    _interpolation_identifier: $ =>
      /[\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F][\p{Lu}\p{Lt}\p{Nl}\p{Lo}\p{Lm}\p{Ll}_\u00AA\u00BB\u02B0-\u02B8\u02C0-\u02C1\u02E0-\u02E4\u037A\u1D78\u1D9B-\u1DBF\u2071\u207F\u2090-\u209C\u2C7C-\u2C7D\uA69C-\uA69D\uA770\uA7F8-\uA7F9\uAB5C-\uAB5F0-9_\p{Ll}]*/,

    _backquoted_id: $ => /`[^\n`]+`/,

    // Name contexts take the union token; sites whose states also allow an
    // infix continuation use operandName instead (see its comment).
    _identifier: $ => choice(nameChoice($), opName($)),

    identifiers: $ => seq(nameChoice($), ",", commaSep1(nameChoice($))),

    wildcard: $ => "_",

    // A single shared `*` token. As a string literal it beats the operator
    // regexes on lexical precedence, so every rule admitting a lone `*` in
    // states where this string token is valid must use this rule (name-only
    // states lex a lone `*` through OP_ID_UNION instead). Before a closing
    // delimiter it arrives as the external _postfix_star (see vararg).
    _asterisk: $ => "*",

    // The union of the per-precedence operator tokens (see OP_TOKEN at the
    // top), so every use site outside infix_expression accepts any class and
    // the partition never changes which strings lex as an operator.
    operator_identifier: $ =>
      choice(
        $._asterisk,
        ...OP_LEFT.map(key => opLeft($, key)),
        ...Object.entries(OP_TOKEN)
          .filter(([key]) => !key.endsWith("Left"))
          .map(([, tok]) => tok),
      ),

    // A rule per left-associative class so it can also be external, and the
    // union name for the same reason (see OP_LEFT).
    ...Object.fromEntries(
      OP_LEFT.map(key => [`_op_left_${key}`, () => OP_TOKEN[`${key}Left`]]),
    ),
    _op_name: $ => OP_ID_UNION,

    // XML literals (SLS §10). The external token $._xml_tag_start decides
    // where markup starts. Comment, CDATA and processing instruction
    // literals need no scanner support, since their tokens are longer than
    // the operator tokens that could match the same text.

    // XmlExpr ::= XmlContent {Element}
    xml_expression: $ => repeat1($._xml_node),

    _xml_node: $ =>
      choice(
        $.xml_element,
        $.xml_comment,
        $.xml_cdata,
        $.xml_processing_instruction,
      ),

    xml_element: $ => xmlElementShape($, $._xml_content),

    // Shared between expression elements and pattern elements so that GLR
    // forks where both are viable shift the same states until the element
    // body disambiguates them.
    _xml_open_tag: $ =>
      seq(
        alias($._xml_tag_start, "<"),
        field("name", alias(token.immediate(XML_NAME), $.xml_name)),
        repeat($.xml_attribute),
      ),

    // The name is immediate, like scalac's xEndTag, which reads it right
    // after the `</`. Trailing space before the `>` stays allowed.
    _xml_end_tag: $ =>
      seq("</", alias(token.immediate(XML_NAME), $.xml_name), ">"),

    xml_attribute: $ =>
      seq(
        field("key", alias(token(XML_NAME), $.xml_name)),
        "=",
        field(
          "value",
          choice(
            alias(token(choice(/"[^<"]*"/, /'[^<']*'/)), $.xml_string),
            $.block,
          ),
        ),
      ),

    _xml_content: $ =>
      choice(
        ...xmlTextAlternatives($),
        $.block,
        $._xml_node,
        // Dead alternative: `/*` in XML content is text, not a comment.
        $._suppress_block_comment,
      ),

    // Outranks the comment and whitespace extras, which would otherwise win
    // the same characters and drop markup text from the tree.
    xml_text: _ => token(prec(PREC.comment + 1, /[^<{}]+/)),

    // The bodies below stop at the first terminator: a run of the closing
    // character can only be consumed when a character that cannot close the
    // literal follows it.
    xml_comment: _ =>
      token(
        seq(
          "<!--",
          repeat(choice(/[^-]/, seq(repeat1("-"), /[^->]/))),
          repeat("-"),
          "-->",
        ),
      ),

    xml_cdata: _ =>
      token(
        seq(
          "<![CDATA[",
          repeat(choice(/[^\]]/, seq(repeat1("]"), /[^\]>]/))),
          repeat("]"),
          "]]>",
        ),
      ),

    xml_processing_instruction: _ =>
      token(
        seq(
          "<?",
          repeat(choice(/[^?]/, seq(repeat1("?"), /[^?>]/))),
          repeat("?"),
          "?>",
        ),
      ),

    // XmlPattern ::= ElemPattern. Embedded `{...}` blocks hold patterns.
    xml_pattern: $ => xmlElementShape($, $._xml_pattern_content),

    _xml_pattern_content: $ =>
      choice(
        ...xmlTextAlternatives($),
        seq("{", commaSep1($._xml_embedded_pattern), "}"),
        $.xml_pattern,
        $.xml_comment,
        $.xml_cdata,
        $.xml_processing_instruction,
        $._suppress_block_comment,
      ),

    // A restricted top-level pattern for XML embeds. Where an element and an
    // element pattern both stay viable, $.block shares the state with this
    // rule. An alternative whose leftmost symbol is the full $._pattern then
    // drags given_pattern and ascriptions into the closure and the conflicts
    // cascade past resolution, so only alternatives with a closed left edge
    // belong here. Nested pattern positions still admit the full $._pattern.
    _xml_embedded_pattern: $ =>
      choice(
        $._identifier,
        $.stable_identifier,
        $.interpolated_string_expression,
        $.capture_pattern,
        $.tuple_pattern,
        $.named_tuple_pattern,
        $.case_class_pattern,
        $.quote_expression,
        $.literal,
        $.wildcard,
        alias($._xml_repeat_pattern, $.repeat_pattern),
        $.xml_pattern,
      ),

    // `_*` / `x*` at the top of an XML embed, without the $._pattern left
    // recursion of $.repeat_pattern.
    _xml_repeat_pattern: $ =>
      seq(field("pattern", choice($.wildcard, $._identifier)), $._asterisk),

    _non_null_literal: $ =>
      choice(
        $.integer_literal,
        $.floating_point_literal,
        alias($._floating_point_with_separators, $.floating_point_literal),
        $.boolean_literal,
        $.character_literal,
        $.string,
      ),

    literal_type: $ => prec.left(PREC.type, $._non_null_literal),

    literal: $ => choice($._non_null_literal, $.null_literal),

    integer_literal: $ =>
      token(
        seq(
          optional(/[-]/),
          choice(
            /[\d](_?\d)*/,
            /0[xX][\da-fA-F](_?[\da-fA-F])*/,
            // Dotty's scanner loops over separators, so `0b0001__0000` is legal
            // and only a trailing `_` is an error.
            /0[bB][01](_*[01])*/,
          ),
          optional(/[lL]/),
        ),
      ),

    floating_point_literal: $ =>
      token(
        seq(
          optional(/[-]/),
          // Digit separators ('_') in the integer part are lexed by the
          // scanner (_floating_point_with_separators); the internal regex
          // cannot, because its DFA drops the `.` transition after an
          // underscore-containing group. The fraction and exponent keep their
          // separators here since the group sits at the end of the pattern.
          choice(
            // digit {digit} ‘.’ digit {digit} [exponentPart] [floatType]
            seq(
              /[\d]+\.[\d](_?\d)*/,
              optional(/[eE][+-]?[\d](_?\d)*/),
              optional(/[dfDF]/),
            ),
            // ‘.’ digit {digit} [exponentPart] [floatType]
            seq(
              /\.[\d](_?\d)*/,
              optional(/[eE][+-]?[\d](_?\d)*/),
              optional(/[dfDF]/),
            ),
            // digit {digit} exponentPart [floatType]
            seq(/[\d]+/, /[eE][+-]?[\d](_?\d)*/, optional(/[dfDF]/)),
            // digit {digit} [exponentPart] floatType
            seq(/[\d]+/, optional(/[eE][+-]?[\d](_?\d)*/), /[dfDF]/),
          ),
        ),
      ),

    boolean_literal: $ => choice("true", "false"),

    character_literal: $ =>
      token(
        seq(
          "'",
          optional(
            choice(
              seq(
                "\\",
                choice(/[^xu]/, /[uU]+[0-9a-fA-F]{4}/, /x[0-9a-fA-F]{2}/),
              ),
              /[^\\'\n]/,
            ),
          ),
          "'",
        ),
      ),

    interpolated_string_expression: $ =>
      choice(
        seq(
          field("interpolator", alias($._raw_string_start, $.identifier)),
          alias($._raw_string, $.interpolated_string),
        ),
        seq(field("interpolator", nameChoice($)), $.interpolated_string),
      ),

    _dollar_escape: $ =>
      alias(token(seq("$", choice("$", '"'))), $.escape_sequence),

    _aliased_interpolation_identifier: $ =>
      alias($._interpolation_identifier, $.identifier),

    interpolation: $ =>
      seq(
        "$",
        choice(
          $._aliased_interpolation_identifier,
          $.block,
          // In pattern position an interpolation may hold a capture pattern,
          // e.g. `case q"${name @ Ident(_)}" =>`. Restricted to
          // capture_pattern to keep the pattern grammar out of expression
          // interpolations.
          prec.dynamic(-1, seq("{", $.capture_pattern, "}")),
        ),
      ),

    interpolated_string: $ =>
      choice(
        seq(
          token.immediate('"'),
          repeat(
            seq(
              $._interpolated_string_middle,
              choice($._dollar_escape, $.interpolation, $.escape_sequence),
            ),
          ),
          $._single_line_string_end,
        ),
        seq(
          token.immediate('"""'),
          repeat(
            seq(
              $._interpolated_multiline_string_middle,
              // Multiline strings ignore escape sequences
              choice($._dollar_escape, $.interpolation),
            ),
          ),
          $._multiline_string_end,
        ),
      ),

    // We need to handle single-line raw strings separately from interpolated strings,
    // because raw strings are not parsed for escape sequences. For example, raw strings
    // are often used for regular expressions, which contain backslashes that would
    // be invalid if parsed as escape sequences. We do not special case multiline
    // raw strings, because multiline strings do not parse escape sequences anyway.
    // Scala handles multiline raw strings identically to other multiline interpolated,
    // so we could parse them as interpolated strings, but I think the code is cleaner
    // if we maintain the distinction.
    _raw_string: $ =>
      choice(
        seq(
          $._simple_string_start,
          seq(
            repeat(
              seq(
                $._raw_string_middle,
                choice($._dollar_escape, $.interpolation),
              ),
            ),
            $._single_line_string_end,
          ),
        ),
        seq(
          $._simple_multiline_string_start,
          repeat(
            seq(
              $._raw_string_multiline_middle,
              choice($._dollar_escape, $.interpolation),
            ),
          ),
          $._multiline_string_end,
        ),
      ),

    escape_sequence: _ =>
      token.immediate(
        seq(
          "\\",
          choice(
            /[tbnrf"'\\]/,
            // The Java spec allows any number of u's and U's at the start of a unicode escape.
            /[uU]+[0-9a-fA-F]{4}/,
            // Octals are not allowed in Scala 3, but are allowed in Scala 2. tree-sitter
            // does not have a mechanism for distinguishing between different versions of a
            // language, so I think it makes sense to allow them. Maybe in the future we
            // should move them to a `deprecated` syntax node?
            /[0-3]?[0-7]{1,2}/,
            // Any other escaped character. scalac accepts these at the lexer
            // level in interpolated strings (their validity depends on the
            // interpolator, e.g. quasiquotes accept a plain `\`).
            /[^\r\n]/,
          ),
        ),
      ),

    string: $ =>
      choice(
        seq(
          $._simple_string_start,
          repeat(seq($._simple_string_middle, $.escape_sequence)),
          $._single_line_string_end,
        ),
        seq(
          $._simple_multiline_string_start,
          /// Multiline strings ignore escape sequences
          $._multiline_string_end,
        ),
      ),

    _semicolon: $ => choice(";", $._automatic_semicolon),

    null_literal: $ => "null",

    unit: $ => prec(PREC.unit, seq("(", ")")),

    return_expression: $ =>
      prec.left(seq("return", optional(statementExpression($)))),

    throw_expression: $ => prec.left(seq("throw", $.expression)),

    /*
     *   Expr1             ::=  'while' '(' Expr ')' {nl} Expr
     *                       |  'while' Expr 'do' Expr
     */
    while_expression: $ =>
      prec(
        PREC.while,
        choice(
          // No marker slot here. It lets a trailing `match` capture the
          // whole loop as its value in `while (c) e match { ... }`.
          prec.right(
            seq(
              "while",
              field("condition", $.parenthesized_expression),
              // The do-form branch makes the scanner emit a semicolon
              // here.
              optional($._automatic_semicolon),
              field("body", $.expression),
            ),
          ),
          seq(
            prec.right(
              seq(
                "while",
                field(
                  "condition",
                  // A marker slot inside the condition may emit a
                  // semicolon before the `do`.
                  seq(
                    $._indentable_expression,
                    optional($._automatic_semicolon),
                    "do",
                  ),
                ),
                field("body", $._indentable_expression),
              ),
            ),
            optional($._end_marker_kw_tail),
          ),
        ),
      ),

    do_while_expression: $ =>
      prec.right(
        seq(
          "do",
          field("body", $.expression),
          "while",
          field("condition", $.parenthesized_expression),
        ),
      ),

    /*
     *  ForExpr           ::=  'for' '(' Enumerators0 ')' {nl} ['do' | 'yield'] Expr
     *                      |  'for' '{' Enumerators0 '}' {nl} ['do' | 'yield'] Expr
     *                      |  'for'     Enumerators0          ('do' | 'yield') Expr
     */
    for_expression: $ =>
      choice(
        // No marker slot on the bracketed heads, for the same reason as the
        // parenthesized while.
        prec.right(
          PREC.control,
          seq(
            "for",
            field(
              "enumerators",
              choice(
                seq("(", $.enumerators, ")"),
                seq("{", $.enumerators, "}"),
              ),
            ),
            choice(
              // The bare body also admits an indented block, since Scala 3
              // allows a multi-statement `for (...)` body with no `do`. It uses
              // plain $.expression so a next-line `do`/`yield` never starts it.
              field("body", choice($.indented_block, $.expression)),
              seq("do", field("body", $._indentable_expression)),
              seq(
                $._control_tail_gate,
                "yield",
                field("body", $._indentable_expression),
              ),
            ),
          ),
        ),
        seq(
          prec.right(
            PREC.control,
            seq(
              "for",
              field("enumerators", $.enumerators),
              choice(
                seq("do", field("body", $._indentable_expression)),
                seq(
                  $._control_tail_gate,
                  "yield",
                  field("body", $._indentable_expression),
                ),
              ),
            ),
          ),
          optional($._end_marker_kw_tail),
        ),
      ),

    enumerators: $ =>
      choice(
        seq(sep1($._semicolon, $.enumerator), optional($._automatic_semicolon)),
        seq(
          $._indent,
          sep1($._semicolon, $.enumerator),
          optional($._automatic_semicolon),
          $._outdent,
        ),
      ),

    /**
     *   Enumerator        ::=  Generator
     *                       |  Guard {Guard}
     *                       |  Pattern1 '=' Expr
     */
    enumerator: $ =>
      choice(
        seq(
          optional("case"),
          $._pattern,
          choice("<-", "←", "="),
          // A generator or `=` binding can bind a braceless indented block, as
          // the guard and the `for` body already do.
          choice($.expression, $.indented_block),
          optional($.guard),
        ),
        repeat1($.guard),
      ),

    // Either a plain `#!...` first line, or the Scala 2 script header form
    // whose shell preamble runs until a closing `!#` line:
    //   #!/bin/sh
    //   exec scala "$0" "$@"
    //   !#
    _shebang: $ =>
      alias(
        token(
          seq(
            "#!",
            /[^\n]*/,
            optional(seq(repeat(seq("\n", /[^\n]*/)), "\n!#")),
          ),
        ),
        $.comment,
      ),

    // The dead $._suppress_block_comment alternative marks the after-`//`
    // state for the external scanner. In `// /* x` the `/*` is comment text.
    comment: $ =>
      seq(
        token("//"),
        choice($.using_directive, $._comment_text, $._suppress_block_comment),
      ),
    _comment_text: $ => token(prec(PREC.comment, /.*/)),

    using_directive: $ =>
      seq(
        alias($._using_directive_start, ">"),
        token("using"),
        $.using_directive_key,
        $.using_directive_value,
      ),
    using_directive_key: $ => token(/[^\s]+/),
    using_directive_value: $ => token(/.*/),
  },
});

function commaSep(rule) {
  return optional(commaSep1(rule));
}

function commaSep1(rule) {
  return sep1(",", rule);
}

function trailingCommaSep(rule) {
  return optional(trailingCommaSep1(rule));
}

function trailingCommaSep1(rule) {
  return trailingSep1(",", rule);
}

function trailingSep1(delimiter, rule) {
  return seq(sep1(delimiter, rule), optional(delimiter));
}

function sep1(delimiter, rule) {
  return seq(rule, repeat(seq(delimiter, rule)));
}

// Plain text and the `{{` / `}}` escaped literal braces, shared between XML
// elements and XML patterns.
function xmlTextAlternatives($) {
  return [$.xml_text, alias("{{", $.xml_text), alias("}}", $.xml_text)];
}

// Element skeleton (self-closing, or open tag + content + end tag), shared
// between xml_element and xml_pattern, which differ only in their content.
function xmlElementShape($, content) {
  return seq(
    $._xml_open_tag,
    choice("/>", seq(">", repeat(content), $._xml_end_tag)),
  );
}
