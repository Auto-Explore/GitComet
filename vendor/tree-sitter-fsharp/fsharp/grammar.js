/**
 * @file FSharp grammar for tree-sitter
 * @author Nikolaj Sidorenco
 * @license MIT
 * @see {@link https://fsharp.org/specs/language-spec/4.1/FSharpSpec-4.1-latest.pdf f# grammar}
 */

/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  SEQ_EXPR: 1,
  APP_EXPR: 16,
  THEN_EXPR: 2,
  RARROW: 3,
  INFIX_OP: 4,
  LET_EXPR: 60,
  LET_DECL: 7,
  DO_EXPR: 8,
  FUN_EXPR: 8,
  MATCH_EXPR: 8,
  MATCH_DECL: 9,
  DO_DECL: 10,
  ELSE_EXPR: 11,
  INTERFACE: 12,
  COMMA: 13,
  INFIX_OR: 13,
  INFIX_AND: 14,
  PREFIX_EXPR: 15,
  SPECIAL_INFIX: 16,
  LARROW: 16,
  TUPLE_EXPR: 16,
  CE_EXPR: 15,
  SPECIAL_PREFIX: 17,
  // DO_EXPR: 17,
  IF_EXPR: 18,
  DOT: 19,
  INDEX_EXPR: 20,
  PAREN_APP: 21,
  TYPED_EXPR: 22,
  PAREN_EXPR: 21,
  DOTDOT: 22,
  DOTDOT_SLICE: 23,
  NEW_OBJ: 24,
};

module.exports = grammar({
  name: "fsharp",

  extras: ($) => [
    /[ \s\f\uFEFF\u2060\u200B]/,
    $.block_comment,
    $.line_comment,
    $.xml_doc,
    $.preproc_line,
    $.compiler_directive_decl,
    $.preproc_inactive,
    ";",
  ],

  // The external scanner (scanner.c) allows us to inject "dummy" tokens into the grammar.
  // These tokens are used to track the indentation-based scoping used in F#
  externals: ($) => [
    $._newline, // we distinguish new scoped based on newlines.
    $._indent, // starts a new indentation-based scope.
    $._dedent, // signals that the current indentation scope has ended.
    "then",
    "else",
    "elif",
    "#if",
    "#else",
    "#endif",
    "class",
    "begin",
    $._struct_begin,
    $._interface_begin,
    "end",
    "and",
    "with",
    $._triple_quoted_content,
    $._format_triple_quoted_content,
    $.block_comment_content,
    $._inside_string_marker,
    $._newline_not_aligned,
    $._tuple_marker,
    $._quoted_close,
    $._untyped_quoted_close,
    $._multi_dollar_triple_quote_start,
    $._multi_dollar_triple_quoted_content,
    $._multi_dollar_interp_start,
    $._multi_dollar_interp_end,
    $._multi_dollar_triple_quote_end,
    $._tyapp_open, // type application opening '<' (Section 15.3 lookahead)
    $._paren_indent, // like _indent but pushes 0 onto indent stack for paren contexts
    $._type_app_indent, // like _paren_indent but only fires after a newline; closed by '>' (used for multi-line generic type args)
    $._type_decl_newline, // lookahead token: fires at newline/EOF when the next non-blank line is not more indented, used to match bare type declarations
    $._in, // external 'in' keyword token for let...in expressions; only produced when valid, so 'in' as identifier in query/CE contexts is unaffected
    $._do_keyword, // external 'do' terminating a while/for header; distinct from do_expression's 'do' so `while a && b do` reduces the condition instead of shifting 'do' as an application argument
    $._try_indent, // like _indent, but opens a try-body scope that can be force-closed when its 'with'/'finally' sits at the same column as the body
    $.preproc_inactive, // extra: an inactive `#else`..`#endif` region (or dangling `#endif`) of a directive whose `#if` line was skipped as trivia because the grammar has no preproc rule at that position
    $._elem_separator, // fires at a column-0 line while only the base indent level is open: separates top-level module elements so an application expression cannot absorb the next element
    $._brace_indent, // opens a '{...}' record/CE field block: closed by '}', under-indented lines still emit NEWLINE so items keep separating

    $._error_sentinel, // unused token to detect parser errors in external parser.
  ],

  conflicts: ($) => [
    [$.long_identifier, $._identifier_or_op],
    [$.simple_type, $.type_argument],
    // `new ^T ...` is ambiguous between an SRTP type arg (^T or ^U ...) and a
    // plain generic type var; only reachable via generic_new_expression,
    // resolved dynamically.
    [$._srtp_type_argument, $._static_type_identifier],
    [$._module_expression, $._expression],
    [$.declaration_expression, $._comp_or_range_expression],
    [$.rules],
    // Singleton: union_type_cases conflicts with itself (shift the optional
    // _newline before the next '|' case vs. reduce), like [$.rules] above.
    [$.union_type_cases],
    [$.prefixed_expression, $._low_prec_app, $.infix_expression],
    [$._type, $._argument_type],
    [$._type, $._curried_return_type],
    // A leading _elem_separator (fired after a file-opening extra such as a
    // doc comment) could belong to the plain module-elements repeat or open
    // named_module/namespace; both must stay alive until the header keyword
    // and the presence/absence of '=' decide.
    [$.file, $.named_module],
    // Singleton: a separator after a namespace body could continue that
    // namespace's element repeat or lead the next namespace in the file.
    [$.namespace],
  ],

  word: ($) => $.identifier,

  inline: ($) => [
    $._expression_or_range,
    $._object_expression_inner,
    $._record_type_defn_inner,
    $._union_type_defn_inner,
    $._then_expression,
  ],

  supertypes: ($) => [
    $._module_elem,
    $._pattern,
    $._expression,
    $._type,
    $._type_defn_body,
    $._static_parameter,
  ],

  rules: {
    //
    // Top-level rules (BEGIN)
    //
    // Top-level elements are separated by an optional _elem_separator: the
    // scanner emits one at a column-0 line while only the base indent level
    // is open, which stops an application expression from absorbing the
    // next element (e.g. `f 1` followed by `let g x = ...` parsing as a
    // let-in argument of `f`). A dedicated token — reusing _newline here
    // would also fire at before-element newline positions inside preproc
    // branches and derail those parses. Both arms of the choice must stay
    // bare: wrapping either in a seq (seq(elem, optional(sep)) or
    // seq(sep, elem) were both tried) changes static conflict resolution
    // and kills the GLR fork that continues a column-0 union case after a
    // trailing line comment. Because the separator therefore sits in a
    // plain repeat — where the parser would shift a zero-width token
    // forever, allocating without bound — the scanner-side token consumes
    // the newline it fires on, so each emission makes progress and cannot
    // repeat at the same position.
    file: ($) =>
      choice(
        repeat1($.namespace),
        prec(-1, repeat(choice($._module_elem, $._elem_separator))),
        prec(1, $.named_module),
      ),

    // The optional leading _elem_separator matters when the file opens with
    // an extra (e.g. a doc comment) before the header keyword: the scanner
    // fires the separator after the extra's newline, and if only the plain
    // module-elements arm of `file` could shift it, doing so would commit
    // the parse away from namespace/named_module before the keyword is even
    // seen (`module M` then mis-parses as a module_defn missing its `=`).
    namespace: ($) =>
      seq(
        optional($._elem_separator),
        "namespace",
        choice(
          "global",
          field("name", seq(optional("rec"), $.long_identifier)),
        ),
        repeat(choice($._module_elem, $._elem_separator)),
      ),

    named_module: ($) =>
      seq(
        optional($._elem_separator),
        optional($.attributes),
        "module",
        optional($.access_modifier),
        optional("rec"),
        field("name", $.long_identifier),
        repeat(choice($._module_elem, $._elem_separator)),
      ),

    _preproc_toplevel_module: ($) =>
      seq(
        optional($.attributes),
        "module",
        optional($.access_modifier),
        optional("rec"),
        field("name", $.long_identifier),
        repeat($._module_elem),
      ),

    _module_body_elem: ($) =>
      choice(
        alias($.value_declaration, $.declaration_expression),
        $.module_defn,
        $.module_abbrev,
        $.import_decl,
        $.fsi_directive_decl,
        $.type_definition,
        $.exception_definition,
        $.extern_binding,
        alias($.preproc_if_in_module_body, $.preproc_if),
        $._module_expression,
        // $.exception_defn
      ),

    _module_expression: ($) =>
      choice(
        "null",
        $.const,
        $.paren_expression,
        $.begin_end_expression,
        $.long_identifier_or_op,
        $.typed_expression,
        $.infix_expression,
        $.index_expression,
        $.mutate_expression,
        $.list_expression,
        $.array_expression,
        $.ce_expression,
        $.prefixed_expression,
        $.brace_expression,
        $.anon_record_expression,
        $.typecast_expression,
        $.do_expression,
        $.fun_expression,
        $.function_expression,
        $.if_expression,
        $.while_expression,
        $.for_expression,
        $.match_expression,
        $.try_expression,
        $.literal_expression,
        $.tuple_expression,
        $.application_expression,
        $.dot_expression,
        $.srtp_call_expression,
        // (static-typars : (member-sig) expr)
      ),

    _module_elem: ($) =>
      choice(
        alias($.value_declaration, $.declaration_expression),
        $.module_defn,
        $.module_abbrev,
        $.import_decl,
        $.fsi_directive_decl,
        $.type_definition,
        $.exception_definition,
        $.extern_binding,
        $._expression,
        $.preproc_if,
        alias($._attribute_expression, $.declaration_expression),
        // $.exception_defn
      ),

    _attribute_expression: ($) =>
      prec(PREC.DO_DECL - 1, seq($.attributes, $._expression)),

    module_abbrev: ($) =>
      seq(
        optional($.attributes),
        "module",
        $.identifier,
        "=",
        scoped($.long_identifier, $._indent, $._dedent),
      ),

    module_defn: ($) =>
      prec.left(
        seq(
          optional($.attributes),
          "module",
          optional($.access_modifier),
          optional("rec"),
          $.identifier,
          "=",
          choice(
            scoped($._module_body, $._indent, $._dedent),
            seq(
              "begin",
              scoped(optional($._module_body), $._indent, $._dedent),
              "end",
            ),
          ),
        ),
      ),

    _module_body: ($) =>
      seq(
        $._module_body_elem,
        repeat(
          prec(
            // Make sure to parse a module node before a sequential expression
            // NOTE: This removes all sequential expressions from module bodies
            PREC.SEQ_EXPR + 1,
            seq(alias($._newline, ";"), $._module_body_elem),
          ),
        ),
      ),

    import_decl: ($) => seq("open", optional("type"), $.long_identifier),

    //
    // Attributes (BEGIN)
    //
    attributes: ($) => prec.left(repeat1($._attribute_set)),
    _attribute_set: ($) =>
      seq(
        "[<",
        $.attribute,
        prec(PREC.SEQ_EXPR + 1, repeat(seq($._newline, $.attribute))),
        // A trailing separator is allowed: `[<Fact; >]`.
        optional($._newline),
        ">]",
      ),
    attribute: ($) =>
      seq(
        optional(seq(field("target", $.identifier), ":")),
        $._object_construction,
      ),

    _object_construction: ($) =>
      prec.left(PREC.SEQ_EXPR + 1, seq($._type, optional($._expression))),

    //
    // Attributes (END)
    //

    value_declaration: ($) =>
      seq(
        optional($.attributes),
        choice(
          prec(PREC.LET_DECL, $.function_or_value_defn),
          prec(PREC.DO_DECL, $.do),
        ),
      ),

    do: ($) => prec(PREC.DO_EXPR + 1, seq("do", $._expression_block)),

    _function_or_value_defns: ($) =>
      prec.right(
        seq(
          $._function_or_value_defn_body,
          repeat(seq("and", $._function_or_value_defn_body)),
        ),
      ),

    function_or_value_defn: ($) =>
      seq(
        choice("let", "let!"),
        choice(
          $._function_or_value_defn_body,
          seq("rec", $._function_or_value_defns),
        ),
      ),

    _function_or_value_defn_body: ($) =>
      seq(
        choice($.function_declaration_left, $.value_declaration_left),
        // The return type may carry attributes: `let f(x) : [<A>] int = ...`.
        optional(seq(":", optional($.attributes), $._type)),
        optional($.type_argument_constraints),
        "=",
        field("body", $._expression_block_for_let),
      ),

    function_declaration_left: ($) =>
      prec.left(
        3,
        seq(
          optional("inline"),
          optional($.access_modifier),
          prec(100, $._identifier_or_op),
          optional($.type_arguments),
          $.argument_patterns,
        ),
      ),

    value_declaration_left: ($) =>
      prec.left(
        2,
        seq(
          optional("mutable"),
          optional($.access_modifier),
          $._pattern,
          optional($.type_arguments),
        ),
      ),

    access_modifier: (_) =>
      prec(100, token(prec(1000, choice("private", "internal", "public")))),
    //
    // Top-level rules (END)
    //

    class_as_reference: ($) => seq("as", $.identifier),

    primary_constr_args: ($) =>
      seq(
        optional($.attributes),
        optional($.access_modifier),
        "(",
        optional($._pattern),
        ")",
        optional($.class_as_reference),
      ),

    //
    // Pattern rules (BEGIN)

    // Pattern precedence (used by repeat_pattern, typed_pattern, optional_pattern):
    //   optional_pattern (3) > typed_pattern (2) > comma-element (1).
    // This ensures `?x: T` parses as `(typed (optional x) T)` and `(a: T, b: T)`
    // parses as a repeat of two typed_patterns rather than a typed_pattern over
    // the whole repeat.
    repeat_pattern: ($) =>
      prec.right(seq($._pattern, repeat1(prec(1, seq(",", $._pattern))))),

    _pattern: ($) =>
      choice(
        "null",
        alias("_", $.wildcard_pattern),
        $.typed_const_pattern,
        $.const,
        $.as_pattern,
        $.disjunct_pattern,
        $.conjunct_pattern,
        $.cons_pattern,
        $.repeat_pattern,
        $.paren_pattern,
        $.list_pattern,
        $.array_pattern,
        $.record_pattern,
        $.typed_pattern,
        $.attribute_pattern,
        $.type_check_pattern,
        $.optional_pattern,
        $.identifier_pattern,
        $.named_field_pattern,
      ),

    optional_pattern: ($) => prec.left(3, seq("?", $._pattern)),

    type_check_pattern: ($) =>
      prec.right(seq(":?", $.atomic_type, optional(seq("as", $.identifier)))),

    attribute_pattern: ($) => prec.left(seq($.attributes, $._pattern)),

    paren_pattern: ($) => prec(1, seq("(", $._pattern, ")")),

    as_pattern: ($) => prec.left(0, seq($._pattern, "as", $.identifier)),
    cons_pattern: ($) => prec.left(0, seq($._pattern, "::", $._pattern)),
    disjunct_pattern: ($) => prec.left(0, seq($._pattern, "|", $._pattern)),
    conjunct_pattern: ($) => prec.left(0, seq($._pattern, "&", $._pattern)),
    typed_pattern: ($) =>
      prec.left(
        2,
        seq(
          $._pattern,
          ":",
          $._type,
          field("constraints", optional($.type_argument_constraints)),
        ),
      ),

    typed_const_pattern: ($) =>
      prec(
        PREC.PAREN_EXPR,
        seq(
          $.const,
          $._tyapp_open,
          optional(choice($.types, $.measure)),
          prec(PREC.PAREN_EXPR, ">"),
        ),
      ),

    argument_patterns: ($) =>
      // argument patterns are generally no different from normal patterns.
      // however, any time an argument pattern is a valid node, (i.e. inside a beginning fun decl)
      // it is always the correct node to construct.
      prec.left(1000, repeat1($._atomic_pattern)),

    field_pattern: ($) =>
      prec(
        1,
        seq($.long_identifier, "=", $._pattern),
      ),

    _atomic_pattern: ($) =>
      choice(
        "null",
        "_",
        $.typed_const_pattern,
        $.const,
        $.long_identifier,
        $.list_pattern,
        $.record_pattern,
        $.array_pattern,
        seq("(", $._pattern, ")"),
        $.type_check_pattern,
      ),

    _list_pattern_content: ($) =>
      scoped(
        seq(
          optional($._newline),
          $._pattern,
          repeat(seq($._newline, $._pattern)),
        ),
        $._indent,
        $._dedent,
      ),

    list_pattern: ($) => seq("[", optional($._list_pattern_content), "]"),
    array_pattern: ($) => seq("[|", optional($._list_pattern_content), "|]"),
    // The whole field list sits in one indent scope opened right after '{'
    // (like field_initializers in brace_expression). Opening the scope before
    // the first field keeps _indent from becoming valid mid-field, where a
    // zero-width INDENT would cut off field values like `A.B` or `Some y`.
    record_pattern: ($) =>
      seq(
        "{",
        $._indent,
        $.field_pattern,
        repeat(seq($._newline, $.field_pattern)),
        optional($._newline),
        $._dedent,
        "}",
      ),

    named_field: ($) => seq(optional(seq($.identifier, "=")), $._pattern),

    named_field_pattern: ($) =>
      prec.left(
        seq("(", $.named_field, repeat(seq($._newline, $.named_field)), ")"),
      ),

    identifier_pattern: ($) =>
      prec.left(
        1,
        seq(
          $.long_identifier_or_op,
          optional($._pattern),
          optional($._pattern),
        ),
      ),

    //
    // Pattern rules (END)
    //

    //
    // Expressions (BEGIN)
    //

    _expression_block: ($) => seq($._indent, $._expression, $._dedent),

    // Like _expression_block, but also allows the block to be terminated by
    // the 'in' keyword instead of a dedent.  Used for let/use bindings so that
    // single-line  let x = 1 in x  works (where no newline → no DEDENT).
    _expression_block_for_let: ($) =>
      seq($._indent, $._expression, choice($._dedent, $._in)),

    _paren_expression_block: ($) => seq($._paren_indent, $._expression, $._dedent),

    _mutable_expression: ($) => choice($.long_identifier_or_op, $.index_expression, $.dot_expression),

    _expression: ($) =>
      choice(
        "null",
        $.const,
        $.paren_expression,
        $.begin_end_expression,
        $.long_identifier_or_op,
        $.typed_expression,
        $.infix_expression,
        $.index_expression,
        $.mutate_expression,
        $.list_expression,
        $.array_expression,
        $.ce_expression,
        $.prefixed_expression,
        $.generic_new_expression,
        $.brace_expression,
        $.anon_record_expression,
        $.typecast_expression,
        $.declaration_expression,
        $.do_expression,
        $.fun_expression,
        $.function_expression,
        $.sequential_expression,
        $.if_expression,
        $.while_expression,
        $.for_expression,
        // Short-form comprehension `for x in xs -> e` so nested comprehensions
        // like `[ for l in 0..3 do for h in 0..3 -> (l, h) ]` parse — the
        // do-form body is a plain expression block.
        $.short_comp_expression,
        $.match_expression,
        $.try_expression,
        $.literal_expression,
        $.tuple_expression,
        $.application_expression,
        $.dot_expression,
        alias($.preproc_if_in_expression, $.preproc_if),
        $.srtp_call_expression,
      ),

    literal_expression: ($) =>
      prec(
        PREC.PAREN_EXPR,
        choice(
          seq("<@", $._expression, $._quoted_close),
          seq("<@@", $._expression, $._untyped_quoted_close),
        ),
      ),

    srtp_call_expression: ($) =>
      prec.right(
        PREC.PAREN_EXPR,
        seq(
          // A support constraint over several typars is written parenthesized:
          // `((^a or ^b) : (static member op_Implicit: ^a -> ^b) x)`. The
          // single-typar form `(^a : ...)` takes no inner parens. Either way
          // the outer parens belong to the surrounding paren_expression; the
          // scanner keeps the group's '(' from opening one of its own (see
          // is_srtp_typar_group_ahead).
          choice(
            alias($._srtp_type_argument, $.type_argument),
            seq("(", alias($._srtp_type_argument, $.type_argument), ")"),
          ),
          ":",
          "(",
          $.trait_member_constraint,
          ")",
          $._expression,
        ),
      ),

    // Like type_argument but restricted to ^-prefixed identifiers (not '-prefixed)
    // to avoid ambiguity with char literals in expression context.
    _srtp_type_argument: ($) =>
      prec(
        10,
        seq(
          "^",
          $.identifier,
          repeat(seq("or", "^", $.identifier)),
        ),
      ),

    long_identifier_or_op: ($) =>
      prec.right(
        choice(
          $.long_identifier,
          seq($.long_identifier, ".", $._identifier_or_op),
          $._identifier_or_op,
        ),
      ),

    tuple_expression: ($) =>
      prec.right(
        PREC.TUPLE_EXPR,
        seq($._expression, ",", optional($._tuple_marker), $._expression),
      ),

    brace_expression: ($) =>
      prec(
        PREC.CE_EXPR + 1,
        seq(
          "{",
          // Optional so an additional constructor can hand back an empty
          // object: `new (x) = {}` / `{ }`, and so the inherit-only form
          // `new () = { inherit Base() }` needs no field initializers.
          optional(
            scoped(
              choice(
                $.field_initializers,
                $.object_expression,
                $.with_field_expression,
                seq(
                  $.class_inherits_decl,
                  optional(seq(optional($._newline), $.field_initializers)),
                ),
              ),
              $._brace_indent,
              $._dedent,
            ),
          ),
          "}",
        ),
      ),

    anon_record_expression: ($) =>
      prec(
        PREC.PAREN_EXPR,
        seq(
          "{|",
          optional(scoped(
            choice($.field_initializers, $.with_field_expression),
            $._indent,
            $._dedent,
          )),
          "|}",
        ),
      ),

    _object_expression_inner: ($) =>
      seq($._object_members, repeat($.interface_implementation)),

    object_expression: ($) =>
      prec(
        PREC.NEW_OBJ + 1,
        seq(
          "new",
          $._expression,
          optional(seq("as", $.identifier)),
          $._object_expression_inner,
        ),
      ),

    with_field_expression: ($) =>
      seq(
        $._expression,
        "with",
        // Brace-kind scope: the fields live inside a literal `{ ... }`, so an
        // under-indented continuation field must separate (NEWLINE), not close
        // the scope — `{ q with A = ...;` with the next field left of the
        // first one is valid F# (the closing `}` bounds the scope).
        scoped($.field_initializers, $._brace_indent, $._dedent),
      ),

    prefixed_expression: ($) =>
      seq(
        choice(
          "return",
          "return!",
          "yield",
          "yield!",
          "lazy",
          "assert",
          "upcast",
          "downcast",
          "new",
          "fixed",
          $.prefix_op,
        ),
        prec.right(PREC.PREFIX_EXPR, $._expression),
      ),

    // `new 'T()` / `new 'T(args)`: constructing a value of a generic type
    // parameter. A type variable ('T / ^T) is not a general expression atom
    // (that would clash with char literals), so this is gated on `new`.
    generic_new_expression: ($) =>
      prec.right(
        PREC.NEW_OBJ,
        seq(
          "new",
          $.type_argument,
          choice(
            $.unit,
            seq(token.immediate(prec(10000, "(")), $._paren_expression_block, ")"),
          ),
        ),
      ),

    typecast_expression: ($) =>
      prec.right(
        PREC.SPECIAL_INFIX,
        seq($._expression, choice(":", ":>", ":?", ":?>"), $._type),
      ),

    for_expression: ($) =>
      prec(
        PREC.DO_EXPR + 1,
        seq(
          "for",
          choice(
            seq($._pattern, "in", $._expression_or_range),
            seq(
              $.identifier,
              "=",
              $._expression,
              choice("to", "downto"),
              $._expression,
            ),
          ),
          alias($._do_keyword, "do"),
          $._expression_block,
          optional("done"),
        ),
      ),

    while_expression: ($) =>
      prec(
        PREC.DO_EXPR + 1,
        seq(
          choice("while", "while!"),
          $._expression,
          alias($._do_keyword, "do"),
          $._expression_block,
          optional("done"),
        ),
      ),

    _else_expression: ($) => seq("else", field("else", $._expression_block)),

    _then_expression: ($) => seq("then", field("then", $._expression_block)),

    elif_expression: ($) =>
      seq("elif", field("guard", $._expression_block), $._then_expression),

    _if_branch: ($) => seq("if", field("guard", $._expression_block)),

    if_expression: ($) =>
      seq(
        $._if_branch,
        $._then_expression,
        repeat($.elif_expression),
        optional($._else_expression),
      ),

    fun_expression: ($) =>
      prec.right(
        PREC.FUN_EXPR,
        seq("fun", $.argument_patterns, arrow(), $._expression_block),
      ),

    try_expression: ($) =>
      prec(
        PREC.MATCH_EXPR,
        seq(
          "try",
          seq($._try_indent, $._expression, $._dedent),
          optional($._newline),
          choice(seq("with", $.rules), seq("finally", $._expression_block)),
        ),
      ),

    match_expression: ($) =>
      seq(
        choice("match", "match!"),
        $._expression,
        optional($._newline),
        "with",
        choice(
          seq($._newline, $.rules),
          scoped($.rules, $._indent, $._dedent),
          $.rules,
        ),
      ),

    function_expression: ($) =>
      prec(
        PREC.MATCH_EXPR,
        seq(
          "function",
          // Same alternatives as match_expression: rules may sit in an indent
          // scope, follow a newline, or share the line (e.g. `|> function | A -> ...`,
          // where the mid-line '| ' suppresses the zero-width INDENT).
          choice(
            seq($._newline, $.rules),
            scoped($.rules, $._indent, $._dedent),
            $.rules,
          ),
        ),
      ),

    mutate_expression: ($) =>
      prec.right(
        PREC.LARROW,
        seq(
          field("assignee", $._mutable_expression),
          "<-",
          field("value", choice(prec(1, $._expression_block), $._expression)),
        ),
      ),

    index_expression: ($) =>
      prec(
        PREC.INDEX_EXPR,
        seq(
          $._expression,
          ".[",
          choice(field("index", $._expression), $.slice_ranges),
          "]",
        ),
      ),

    typed_expression: ($) =>
      prec(
        PREC.PAREN_EXPR,
        seq(
          $._expression,
          $._tyapp_open,
          optional(choice($.types, $.measure)),
          prec(PREC.PAREN_EXPR, ">"),
        ),
      ),

    declaration_expression: ($) =>
      seq(
        choice(
          seq(choice("use", "use!"), optional("mutable"), choice($.identifier, $.paren_pattern), optional(seq(":", $._type)), "=", $._expression_block_for_let),
          seq(
            $.function_or_value_defn,
            repeat($.and_bang),
          ),
        ),
        field("in", $._expression),
      ),

    and_bang: ($) =>
      seq("and!", $._pattern, "=", $._expression_block_for_let),

    do_expression: ($) =>
      prec(PREC.DO_EXPR, seq(choice("do", "do!"), $._expression_block)),

    _list_elements: ($) =>
      prec.right(
        PREC.COMMA + 100,
        seq(
          optional($._newline),
          $._expression,
          repeat(
            prec.right(
              PREC.COMMA + 100,
              seq(alias($._newline, ";"), $._expression),
            ),
          ),
          optional($._newline),
        ),
      ),

    // _paren_indent (not _indent): the scope is delimited by the closing
    // bracket, so under-indented continuation lines (e.g. `["a";"b";`
    // followed by a less-indented line) must not DEDENT the list shut.
    _list_element: ($) =>
      seq(
        $._paren_indent,
        choice(
          $._list_elements,
          seq(optional($._newline), $._comp_or_range_expression),
          seq(optional($._newline), $.slice_ranges),
        ),
        $._dedent,
      ),

    list_expression: ($) => seq("[", optional($._list_element), "]"),

    array_expression: ($) => seq("[|", optional($._list_element), "|]"),

    range_expression: ($) =>
      prec(
        PREC.DOTDOT,
        seq(
          $._expression,
          "..",
          $._expression,
          optional(seq("..", $._expression)),
        ),
      ),

    _expression_or_range: ($) => choice($._expression, $.range_expression),

    rule: ($) =>
      prec.right(
        seq(
          field("pattern", $._pattern),
          optional(seq("when", field("guard", $._expression))),
          arrow(),
          field("block", $._expression_block),
        ),
      ),

    rules: ($) =>
      seq(
        optional("|"),
        $.rule,
        repeat(seq(optional($._newline), "|", $.rule)),
      ),

    begin_end_expression: ($) =>
      prec(
        PREC.PAREN_EXPR,
        seq("begin", scoped($._expression, $._indent, $._dedent), "end"),
      ),

    paren_expression: ($) =>
      prec(PREC.PAREN_EXPR, seq("(", $._paren_expression_block, ")")),

    _high_prec_app: ($) =>
      prec.left(
        PREC.DOT + 1,
        seq(
          $._expression,
          choice(
            $.unit,
            seq(token.immediate(prec(10000, "(")), $._paren_expression_block, ")"),
          ),
        ),
      ),

    _low_prec_app: ($) =>
      prec.left(PREC.APP_EXPR, seq($._expression, $._expression)),

    application_expression: ($) => choice($._high_prec_app, $._low_prec_app),

    dot_expression: ($) =>
      prec.right(
        PREC.DOT,
        seq(
          field("base", $._expression),
          ".",
          field("field", $.long_identifier_or_op),
        ),
      ),

    infix_expression: ($) =>
      choice(
        prec.left(
          PREC.INFIX_OR,
          seq($._expression, alias(choice("||", "or"), $.infix_op), $._expression),
        ),
        prec.left(
          PREC.INFIX_AND,
          seq($._expression, alias("&&", $.infix_op), $._expression),
        ),
        prec.left(
          PREC.SPECIAL_INFIX,
          seq($._expression, $.infix_op, $._expression),
        ),
      ),

    ce_expression: ($) =>
      prec.left(
        PREC.CE_EXPR,
        seq(
          prec(-1, $._expression),
          "{",
          scoped($._comp_expression_block, $._brace_indent, $._dedent),
          "}",
        ),
      ),

    _comp_expression_block: ($) =>
      seq(
        $._comp_or_range_expression,
        repeat(seq(alias($._newline, ";"), $._comp_or_range_expression)),
      ),

    sequential_expression: ($) =>
      prec.right(
        PREC.SEQ_EXPR,
        seq(
          $._expression,
          repeat1(
            prec.right(
              PREC.SEQ_EXPR,
              seq(alias($._newline, ";"), $._expression),
            ),
          ),
        ),
      ),

    //
    // Expressions (END)
    //

    //
    // Computation expression (BEGIN)
    //

    _comp_or_range_expression: ($) =>
      choice(
        alias($.comp_declaration_expression, $.declaration_expression),
        // short_comp_expression is reachable via _expression now.
        $.range_expression,
        $._expression,
      ),

    comp_declaration_expression: ($) =>
      seq(
        choice(
          seq(choice("use", "use!"), choice($.identifier, $.paren_pattern), optional(seq(":", $._type)), "=", $._expression_block_for_let),
          seq(
            $.function_or_value_defn,
            repeat($.and_bang),
          ),
        ),
        field("in", $._comp_or_range_expression),
      ),

    // _comp_expressions: $ =>
    //   choice(
    //     $.let_ce_expressions,
    //     $.do_ce_expressions,
    //     $.use_ce_expressions,
    //     $.yield_ce_expressions,
    //     $.return_ce_expressions,
    //     $.if_ce_expressions,
    //     $.match_ce_expressions,
    //     $.try_ce_expressions,
    //     $.while_expressions,
    //     $.for_ce_expressions,
    //     $.sequential_ce_expressions,
    //     $._expression,
    //   ),

    // for_ce_expressions: $ =>
    //   prec.left(
    //   seq(
    //     "for",
    //     choice(
    //         seq($._pattern, "in", $._expression_or_range),
    //         seq($.identifier, "=", $._expression, "to", $._expression),
    //     ),
    //     "do",
    //       $._virtual_open_section,
    //       $._comp_expressions,
    //       $._virtual_end_section,
    //     optional("done"),
    //   )),
    //
    // try_ce_expressions: $ =>
    //   prec(PREC.MATCH_EXPR,
    //   seq(
    //     "try",
    //     $._virtual_open_section,
    //     $._comp_expressions,
    //     $._virtual_end_section,
    //     choice(
    //       seq("with", $.comp_rules),
    //       seq("finally", $.comp_rules)
    //     ),
    //   )),
    //
    // match_ce_expressions: $ =>
    //   prec(PREC.MATCH_EXPR,
    //   seq(
    //     "match",
    //     $._expression,
    //     "with",
    //     $.comp_rules,
    //   )),
    //
    // sequential_ce_expressions: $ =>
    //   prec.left(PREC.SEQ_EXPR,
    //   seq(
    //     $._comp_expressions,
    //     repeat1(prec.right(PREC.SEQ_EXPR, seq(choice(";", $._newline), $._comp_expressions))),
    //   )),
    //
    // _else_ce_expressions: $ =>
    //   prec(PREC.ELSE_EXPR,
    //   seq(
    //     "else",
    //     $._virtual_open_section,
    //     field("else_branch", $._comp_expressions),
    //     $._virtual_end_section,
    //   )),
    //
    // elif_ce_expressions: $ =>
    //   prec(PREC.ELSE_EXPR,
    //   seq(
    //     "elif",
    //     $._virtual_open_section,
    //     field("guard", $._expression),
    //     $._virtual_end_section,
    //     "then",
    //     $._virtual_open_section,
    //     field("then", $._comp_expressions),
    //     $._virtual_end_section,
    //   )),
    //
    // if_ce_expressions: $ =>
    //   prec.left(PREC.IF_EXPR,
    //   seq(
    //     "if",
    //     $._virtual_open_section,
    //     field("guard", $._expression),
    //     $._virtual_end_section,
    //     "then",
    //     field("then", $._comp_expressions),
    //     repeat($.elif_ce_expressions),
    //     optional($._else_ce_expressions),
    //   )),
    //
    // return_ce_expressions: $ =>
    //   prec.left(PREC.PREFIX_EXPR,
    //   seq(
    //     choice("return!", "return"),
    //     $._expression,
    //   )),
    //
    // yield_ce_expressions: $ =>
    //   prec.left(PREC.PREFIX_EXPR,
    //   seq(
    //     choice("yield!", "yield"),
    //     $._expression,
    //   )),
    //
    // do_ce_expressions: $ =>
    //   seq(
    //     choice("do!", "do"),
    //     $._expression,
    //     $._comp_expressions,
    //   ),
    //
    // use_ce_expressions: $ =>
    //   seq(
    //     choice("use!", "use"),
    //     $._pattern,
    //     "=",
    //     $._virtual_open_section,
    //     $._expression,
    //     $._virtual_end_section,
    //     $._comp_expressions,
    //   ),
    //
    // let_ce_expressions: $ =>
    //   seq(
    //     choice("let!", "let"),
    //     $._pattern,
    //     "=",
    //     $._virtual_open_section,
    //     $._expression,
    //     $._virtual_end_section,
    //     $._comp_expressions,
    //   ),

    short_comp_expression: ($) =>
      seq("for", $._pattern, "in", $._expression_or_range, arrow(), $._expression),

    // comp_rule: $ =>
    //   seq(
    //     $._pattern,
    //     "->",
    //     $._comp_expressions,
    //   ),
    //
    // comp_rules: $ =>
    //   prec.left(2,
    //   seq(
    //     optional("|"),
    //     $.comp_rule,
    //     repeat(seq("|", $.comp_rule)),
    //   )),

    slice_ranges: ($) => seq($.slice_range, repeat(seq(",", $.slice_range))),

    _slice_range_special: ($) =>
      prec.left(
        PREC.DOTDOT_SLICE,
        choice(
          seq(field("from", $._expression), token(prec(PREC.DOTDOT, ".."))),
          seq(
            token(prec(PREC.DOTDOT + 100000, "..")),
            field("to", $._expression),
          ),
          seq(
            field("from", $._expression),
            token(prec(PREC.DOTDOT, "..")),
            field("to", $._expression),
          ),
          seq(
            field("from", $._expression),
            token(prec(PREC.DOTDOT, "..")),
            field("step", $._expression),
            token(prec(PREC.DOTDOT, "..")),
            field("to", $._expression),
          ),
        ),
      ),

    slice_range: ($) => choice($._slice_range_special, $._expression, "*"),

    //
    // Computation expression (END)
    //

    //
    // Type rules (BEGIN)
    //
    _type: ($) =>
      prec(
        4,
        choice(
          $.simple_type,
          $.generic_type,
          $.paren_type,
          $.function_type,
          $.compound_type,
          $.postfix_type,
          $.list_type,
          $.byref_type,
          $.static_type,
          $.type_argument,
          $.constrained_type,
          $.flexible_type,
          $.anon_record_type,
          $.struct_type,
        ),
      ),

    // Like _type but excludes compound_type and function_type, used in member
    // signature argument positions. Per F# spec, T * T before -> is always two
    // separate positional arguments (not a tuple-typed arg); tuple/function-typed
    // arguments must be parenthesized: (T * T) or (T -> T).
    _argument_type: ($) =>
      prec(
        4,
        choice(
          $.simple_type,
          $.generic_type,
          $.paren_type,
          $.postfix_type,
          $.list_type,
          $.static_type,
          $.type_argument,
          $.constrained_type,
          $.flexible_type,
          $.anon_record_type,
          $.struct_type,
        ),
      ),

    // Like _type but excludes function_type, used as the return type in
    // curried_spec so that -> is always consumed by the arguments_spec repeat
    // rather than being parsed as part of a function_type. Function return types
    // must be parenthesized: (T -> T).
    _curried_return_type: ($) =>
      prec(
        4,
        choice(
          $.simple_type,
          $.generic_type,
          $.paren_type,
          $.compound_type,
          $.postfix_type,
          $.list_type,
          $.static_type,
          $.type_argument,
          $.constrained_type,
          $.flexible_type,
          $.anon_record_type,
          $.struct_type,
        ),
      ),

    measure_atom: ($) =>
      choice(
        $.simple_type,
        $.type_argument,
        seq("(", $.measure, ")"),
        "_",
        "1",
      ),

    measure_power: ($) =>
      prec.right(
        7,
        seq(
          $.measure_atom,
          choice(
            seq("^", $._measure_exponent),
            // `s^-1`: the lexer takes `^-` as a single symbolic-operator
            // token, so a negative exponent cannot be spelled `^` `-` int.
            seq("^-", $.int),
          ),
        ),
      ),

    // The exponent may also be a parenthesized rational (`kg^(1/2)`,
    // `s^(-1/2)`) — F# spec 9.4 "Measure expressions".
    _measure_exponent: ($) =>
      choice($.int, seq("(", optional("-"), $.int, "/", $.int, ")")),

    _measure_operand: ($) =>
      choice(
        $.measure_power,
        $.measure_atom,
        $.compound_type,
      ),

    // Product by juxtaposition: `kg m`, `m s^-1`, `'u 'v`. F# also spells the
    // product `kg * m`, which arrives here as a compound_type operand. Binds
    // tighter than '/' so `kg m / s^2` is `(kg m) / s^2`.
    measure_product: ($) =>
      prec.left(6, seq($._measure_operand, repeat1($._measure_operand))),

    // Left-associative, so `m / s / s` is `(m / s) / s` rather than an error.
    // A chain of divisions is one node with the operands in order, matching how
    // FSC parses it: `m / s / s` is a flat `SynType.Tuple [Type; Slash; Type;
    // Slash; Type]`, folded left only later during checking. A single '/' gives
    // the same two children as the earlier binary rule did.
    measure_quotient: ($) =>
      prec.left(
        5,
        seq(
          choice(
            $._measure_operand,
            $.measure_product,
            // `kg m` is indistinguishable from a postfix type until a
            // measure-only token appears, and the postfix reading wins the
            // (statically resolved) reduce/reduce — so accept it here, which
            // is what lets `kg m / s^2` parse. Only on the left of '/': as a
            // general operand it would also make a plain `type T = A.B`
            // abbreviation ambiguous with a measure.
            $.postfix_type,
          ),
          repeat1(seq("/", choice($._measure_operand, $.measure_product))),
        ),
      ),

    measure: ($) =>
      choice(
        $.measure_quotient,
        $.measure_power,
        $.measure_product,
        seq("(", $.measure, ")"),
        // The dimensionless measure `1`, e.g. `float<1>`. Reuses the same "1"
        // literal as measure_atom (no new token) so quotient/power measures like
        // `1/s` are unaffected; only a bare `1` reduces to this standalone form.
        alias("1", $.measure_atom),
      ),

    simple_type: ($) => choice($.long_identifier, $._static_type_identifier),
    generic_type: ($) =>
      prec.right(
        5,
        seq($.long_identifier, "<", optional($.type_attributes), ">"),
      ),
    paren_type: ($) => seq("(", $._type, ")"),
    function_type: ($) => prec.right(seq($._type, arrow(), $._type)),
    compound_type: ($) =>
      prec.right(seq($._type, repeat1(prec.right(seq("*", $._type))))),
    struct_type: ($) => seq("struct", $.paren_type),
    postfix_type: ($) => prec.left(4, seq($._type, $.long_identifier)),
    // '[]', '[,]', '[,,]', ... — multidimensional array suffixes share the
    // token so 'float[,]' lexes as one postfix rather than index syntax.
    // Interior whitespace is permitted ('int[ ]', 'float[ , ]').
    list_type: ($) => seq($._type, alias(token(/\[[ \t]*(,[ \t]*)*\]/), "[]")),
    // Byref type: 'int&', 'float32&'. The '&' must immediately follow the type
    // (token.immediate) so it is not confused with the '&' infix/pattern op.
    byref_type: ($) => prec.left(4, seq($._type, token.immediate("&"))),
    static_type: ($) => prec(10, seq($._type, $.type_arguments)),
    constrained_type: ($) => prec.right(seq($.type_argument, ":>", $._type)),
    flexible_type: ($) => prec.right(seq("#", $._type)),
    anon_record_type: ($) =>
      seq(optional("struct"), "{|", scoped($.record_fields, $._indent, $._dedent), "|}"),
    types: ($) =>
      seq($._type, repeat(prec.left(PREC.COMMA - 1, seq(",", $._type)))),

    _static_type_identifier: ($) =>
      prec(10, seq(choice("^", token(prec(100, "'"))), $.identifier)),

    _static_parameter: ($) =>
      // $.named_static_parameter,
      choice($.static_parameter_value, $.named_static_parameter),

    named_static_parameter: ($) =>
      prec(3, seq($.identifier, "=", choice($.static_parameter_value, $.long_identifier))),

    type_attribute: ($) =>
      choice(
        $._type,
        $._static_parameter,
        $.measure,
      ),

    type_attributes: ($) =>
      seq(
        $.type_attribute,
        repeat(prec.right(PREC.COMMA, seq(",", $.type_attribute))),
      ),


    _multiline_generic_type: ($) =>
      prec.right(
        5,
        seq($.long_identifier, "<", $._indent, optional($.type_attributes), ">", $._dedent),
      ),

    _multiline_generic_type_head: ($) =>
      prec.right(5, seq($.long_identifier, "<", $._indent, optional($.type_attributes), ">")),

    _multiline_generic_function_type: ($) =>
      prec.right(
        6,
        seq(
          alias($._multiline_generic_type_head, $.generic_type),
          arrow(),
          $._type,
          $._dedent,
        ),
      ),

    // A tuple/compound type abbreviation whose first operand is a generic type,
    // e.g. `type T = Foo<'a> * Bar`. The generic head opens an indent scope at
    // '<' (see _multiline_generic_type_head); the matching _dedent is deferred
    // until after the whole compound so the single-line form composes correctly.
    _multiline_generic_compound_type: ($) =>
      prec.right(
        6,
        seq(
          alias($._multiline_generic_type_head, $.generic_type),
          repeat1(prec.right(seq("*", $._type))),
          $._dedent,
        ),
      ),

    atomic_type: ($) =>
      prec.right(
        choice(
          seq("#", $._type),
          $.type_argument,
          seq("(", $._type, ")"),
          $.long_identifier,
          seq($.long_identifier, "<", $.type_attributes, ">"),
        ),
      ),

    constraint: ($) =>
      prec(
        1000000,
        choice(
          seq($.type_argument, ":>", $._type),
          seq($.type_argument, ":", "null"),
          seq(
            // `when (^t or ^u or ^v) : (static member M : string)` — a
            // support constraint over several typars parenthesizes them.
            choice($.type_argument, seq("(", $.type_argument, ")")),
            ":",
            "(",
            choice(
              $.trait_member_constraint,
              seq("new", ":", "unit", arrow(), $._type),
            ),
            ")",
          ),
          seq($.type_argument, ":", "struct"),
          seq($.type_argument, ":", "not", "struct"),
          seq($.type_argument, ":", "enum", "<", $._type, ">"),
          seq($.type_argument, ":", "unmanaged"),
          seq($.type_argument, ":", "equality"),
          seq($.type_argument, ":", "comparison"),
          seq(
            $.type_argument,
            ":",
            "delegate",
            "<",
            $._type,
            ",",
            $._type,
            ">",
          ),
          seq("default", $.type_argument, ":", $._type),
          // Self-constrained typar: `when IStaticProperty<'T>` / `when
          // AverageOps<'T>` names an interface (or a constraint abbreviation)
          // the typar must satisfy. Only the generic form is accepted, so the
          // leading token can never be confused with the typar-first
          // alternatives above.
          $.generic_type,
        ),
      ),

    type_argument_constraints: ($) =>
      seq("when", $.constraint, repeat(seq("and", $.constraint))),

    type_argument: ($) =>
      prec(
        10,
        choice(
          "_",
          seq(
            $._static_type_identifier,
            repeat(seq("or", $._static_type_identifier)),
          ),
        ),
      ),

    type_argument_defn: ($) => seq(optional($.attributes), $.type_argument),

    type_arguments: ($) =>
      seq(
        "<",
        $.type_argument_defn,
        repeat(prec.left(PREC.COMMA, seq(",", $.type_argument_defn))),
        optional($.type_argument_constraints),
        ">",
      ),

    trait_member_constraint: ($) =>
      seq(optional("static"), "member", $._identifier_or_op, ":", $._type),

    member_signature: ($) =>
      prec.left(
        seq(
          $.identifier,
          optional($.type_arguments),
          ":",
          $.curried_spec,
          optional(
            choice(
              seq("with", "get"),
              seq("with", "set"),
              seq("with", "get", ",", "set"),
              seq("with", "set", ",", "get"),
            ),
          ),
        ),
      ),

    curried_spec: ($) => seq(repeat(seq($.arguments_spec, arrow())), $._curried_return_type),

    argument_spec: ($) =>
      prec.left(
        seq(optional($.attributes), optional($.argument_name_spec), $._argument_type),
      ),

    arguments_spec: ($) =>
      seq($.argument_spec, repeat(seq("*", $.argument_spec))),

    argument_name_spec: ($) =>
      seq(optional("?"), field("name", $.identifier), ":"),

    interface_spec: ($) => seq("interface", $._type),

    static_parameter: ($) =>
      choice(
        $.static_parameter_value,
        seq("id", "=", $.static_parameter_value),
      ),

    static_parameter_value: ($) => choice($.const, seq($.const, $._expression)),

    exception_definition: ($) =>
      seq(
        optional($.attributes),
        "exception",
        optional($.access_modifier),
        field("exception_name", $.long_identifier),
        optional(
          seq(
            "of",
            $.union_type_fields,
          )
        ),
        optional(alias($._type_extension_with, $.type_extension_elements)),
      ),

    type_definition: ($) =>
      prec.left(
        seq(
          optional($.attributes),
          "type",
          // Attributes may also sit between `type` and the name:
          // `type [<Struct>] R = ...`
          optional($.attributes),
          $._type_defn_body,
          repeat(
            seq(
              "and",
              choice(
                seq(optional($.attributes), $._type_defn_body),
                seq($._indent, optional($.attributes), $._type_defn_body, $._dedent),
              ),
            ),
          ),
        ),
      ),

    _type_defn_body: ($) =>
      choice(
        $.delegate_type_defn,
        $.record_type_defn,
        $.union_type_defn,
        $.interface_type_defn,
        $.anon_type_defn,
        $.enum_type_defn,
        $.type_abbrev_defn,
        $.type_extension,
        $.type_declaration,
      ),

    // Bare type declaration with no body, used for e.g. [<Measure>] type kg
    // _type_decl_newline fires only at end-of-line, making this unambiguous with anon_type_defn
    type_declaration: ($) => seq($.type_name, $._type_decl_newline),

    type_name: ($) =>
      prec(
        2,
        seq(
          optional($.access_modifier),
          choice(
            seq(
              field("type_name", $.long_identifier),
              optional($.type_arguments),
            ),
            seq(optional($.type_argument), field("type_name", $.identifier)), // Covers `type 'a option = Option<'a>`
          ),
          // Trailing constraint clause, e.g. `type 'a C when 'a : comparison = ...`
          optional($.type_argument_constraints),
        ),
      ),

    type_extension: ($) =>
      seq(
        $.type_name,
        alias($._type_extension_with, $.type_extension_elements),
      ),

    _type_extension_with: ($) =>
      seq("with", scoped($._type_extension_inner, $._indent, $._dedent)),

    delegate_type_defn: ($) =>
      seq($.type_name, "=", scoped($.delegate_signature, $._indent, $._dedent)),

    delegate_signature: ($) => seq("delegate", "of", $._type),

    type_abbrev_defn: ($) =>
      seq(
        $.type_name,
        "=",
        field(
          "block",
          seq(
            $._indent,
            choice(
              alias($._multiline_generic_function_type, $.function_type),
              alias($._multiline_generic_compound_type, $.compound_type),
              $._type,
              $.measure,
              alias($._multiline_generic_type, $.generic_type),
            ),
            $._dedent,
          ),
        ),
      ),

    _class_type_body_inner: ($) =>
      choice($.class_inherits_decl, $.type_extension_elements),

    _class_type_body: ($) =>
      seq(
        $._class_type_body_inner,
        repeat(seq($._newline, $._class_type_body_inner)),
      ),

    _record_type_defn_inner: ($) =>
      seq(
        optional($.access_modifier),
        "{",
        scoped($.record_fields, $._indent, $._dedent),
        "}",
        optional($.type_extension_elements),
      ),

    record_type_defn: ($) =>
      prec.left(
        seq(
          $.type_name,
          "=",
          scoped($._record_type_defn_inner, $._indent, $._dedent),
        ),
      ),

    record_fields: ($) =>
      seq(
        $.record_field,
        repeat(seq($._newline, $.record_field)),
        optional($._newline),
      ),

    record_field: ($) =>
      seq(
        optional($.attributes),
        optional("mutable"),
        optional($.access_modifier),
        $.identifier,
        ":",
        $._type,
      ),

    enum_type_defn: ($) =>
      seq(
        $.type_name,
        "=",
        choice(
          scoped($.enum_type_cases, $._indent, $._dedent),
          $.enum_type_cases,
        ),
      ),

    enum_type_cases: ($) =>
      seq(optional("|"), $.enum_type_case, repeat(seq("|", $.enum_type_case))),

    enum_type_case: ($) => seq($.identifier, "=", $.const),

    _union_type_defn_inner: ($) =>
      seq(
        optional($.access_modifier),
        $.union_type_cases,
        optional(seq(optional($._newline), $.type_extension_elements)),
      ),

    union_type_defn: ($) =>
      prec.left(
        seq(
          $.type_name,
          "=",
          choice(
            scoped($._union_type_defn_inner, $._indent, $._dedent),
            $._union_type_defn_inner,
            // Cases aligned with the enclosing scope (same column as 'type'),
            // mirroring match_expression's `seq(_newline, rules)` branch:
            //   type T =
            //   | A
            seq($._newline, $._union_type_defn_inner),
          ),
        ),
      ),

    union_type_cases: ($) =>
      seq(
        optional("|"),
        $.union_type_case,
        repeat(seq(optional($._newline), "|", $.union_type_case)),
      ),

    union_type_case: ($) =>
      prec(
        8,
        seq(
          optional($.attributes),
          $.identifier,
          optional(choice(seq("of", $.union_type_fields), seq(":", $._type))),
        ),
      ),

    union_type_fields: ($) =>
      seq($.union_type_field, repeat(seq("*", $.union_type_field))),

    union_type_field: ($) =>
      prec.left(choice($._type, seq($.identifier, ":", $._type))),

    interface_type_defn: ($) =>
      prec.left(
        1,
        seq(
          $.type_name,
          "=",
          seq(
            alias($._interface_begin, "interface"),
            // An interface body may open with base interfaces:
            //   type I1 = interface inherit I0 end
            scoped(
              repeat(choice($._type_defn_elements, $.class_inherits_decl)),
              $._indent,
              $._dedent,
            ),
            "end",
          ),
        ),
      ),

    anon_type_defn: ($) =>
      prec.left(
        seq(
          $.type_name,
          optional($.primary_constr_args),
          "=",
          choice(
            alias($.inline_line_comment, $.line_comment),
            scoped($._class_type_body, $._indent, $._dedent),
            seq(
              choice("begin", "class"),
              scoped(optional(seq(optional($._newline), optional($._class_type_body))), $._indent, $._dedent),
              "end",
            ),
            seq(
              alias($._struct_begin, "struct"),
              scoped(repeat($._type_defn_elements), $._indent, $._dedent),
              "end",
            ),
          ),
        ),
      ),

    _class_function_or_value_defn: ($) =>
      seq(
        optional($.attributes),
        optional("static"),
        choice($.function_or_value_defn, seq("do", $._expression_block)),
      ),

    _type_extension_inner: ($) =>
      repeat1(choice($._class_function_or_value_defn, $._type_defn_elements)),

    type_extension_elements: ($) =>
      prec.left(
        choice(
          seq("with", scoped($._type_extension_inner, $._indent, $._dedent)),
          $._type_extension_inner,
        ),
      ),

    _type_defn_elements: ($) =>
      choice(
        $.member_defn,
        $.interface_implementation,
        alias($.preproc_if_in_class_definition, $.preproc_if),
        // $._interface_signature
      ),

    interface_implementation: ($) =>
      prec.left(seq("interface", $._type, optional($._object_members))),

    _member_defns: ($) =>
      prec.left(
        seq($.member_defn, repeat(seq(optional($._newline), $.member_defn))),
      ),

    _object_members: ($) =>
      seq("with", scoped($._member_defns, $._indent, $._dedent)),

    member_defn: ($) =>
      prec(
        PREC.APP_EXPR + 100000,
        seq(
          optional($.attributes),
          choice(
            seq(
              optional("static"),
              "member",
              optional("inline"),
              optional($.access_modifier),
              $.method_or_prop_defn,
            ),
            seq(
              // `static abstract` declares an interface member that
              // implementing types must provide statically (F# 7 IWSAMs).
              // The accessibility modifier is accepted on either side of
              // `abstract` (FSC parses both and rejects it later, FS0561),
              // and `inline` likewise parses here (rejected as FS3151).
              optional($.access_modifier),
              optional("static"),
              "abstract",
              optional("member"),
              optional("inline"),
              optional($.access_modifier),
              $.member_signature,
            ),
            seq("member", "val", optional($.access_modifier), $.property_or_ident, $._val_property_defn),
            seq("override", optional($.access_modifier), $.method_or_prop_defn),
            seq("default", optional($.access_modifier), $.method_or_prop_defn),
            seq(
              optional("static"),
              "val",
              optional("mutable"),
              optional($.access_modifier),
              $.identifier,
              ":",
              $._type,
            ),
            seq("static", $.value_declaration),
            $.additional_constr_defn,
          ),
        ),
      ),

    property_or_ident: ($) =>
      choice(
        seq(
          field("instance", $.identifier),
          ".",
          field("method", $.identifier),
        ),
        $._identifier_or_op,
      ),

    _method_defn: ($) =>
      choice(
        seq(
          optional($.type_arguments),
          field("args", repeat1($._pattern)),
          "=",
          $._expression_block,
        ),
      ),

    _property_accessor_body: ($) =>
      seq(
        $.argument_patterns,
        optional(seq(":", $._type)),
        "=",
        $._expression_block,
      ),

    // Each accessor may carry its own attributes and `inline`:
    // `with [<A>] get () = v and [<B>] inline set x = ...`.
    property_accessor: ($) =>
      seq(
        optional($.attributes),
        optional("inline"),
        choice("get", "set"),
        $._property_accessor_body,
      ),

    _property_defn: ($) =>
      prec.left(
        PREC.APP_EXPR + 100001,
        seq(
          optional(seq(":", $._type)),
          choice(
            seq("=", $._expression_block),
            seq(
              "with",
              scoped(
                seq(
                  $.property_accessor,
                  repeat(seq("and", $.property_accessor)),
                ),
                $._indent,
                $._dedent,
              ),
            ),
          ),
        ),
      ),

    _val_property_defn: ($) =>
      prec.left(
        PREC.APP_EXPR + 100000,
        seq(
          optional(seq(":", $._type)),
          "=",
          $._expression,
          optional(
            seq(
              "with",
              choice(
                "get",
                "set",
                seq("get", ",", "set"),
                seq("set", ",", "get"),
              ),
            ),
          ),
        ),
      ),

    method_or_prop_defn: ($) =>
      prec(
        3,
        seq(
          field("name", $.property_or_ident),
          choice(
            $._method_defn,
            $._property_defn,
            seq(
              "with",
              scoped($._function_or_value_defns, $._indent, $._dedent),
            ),
          ),
        ),
      ),

    additional_constr_defn: ($) =>
      seq(
        optional($.access_modifier),
        "new",
        $._pattern,
        "=",
        $._expression_block,
        optional(seq("then", $._expression_block)),
      ),

    // additional_constr_expr: $ =>
    //   prec.left(
    //     choice(
    //       // seq($.additional_constr_expr, ';', $.additional_constr_expr),
    //       // seq($.additional_constr_expr, 'then', $._expression),
    //       // seq('if', $._expression, 'then', $.additional_constr_expr, 'else', $.additional_constr_expr),
    //       // seq('let', $._function_or_value_defn_body, 'in', $.additional_constr_expr), // TODO: "in" is optional?
    //       $.additional_constr_init_expr,
    //     )),
    //
    // additional_constr_init_expr: $ =>
    //   choice(
    //     // seq('{', $.class_inherits_decl, $.field_initializers, '}'),
    //     $._expression,
    //   ),

    extern_binding: ($) =>
      seq(
        optional($.attributes),
        "extern",
        field("return_type", $._type),
        field("name", $.identifier),
        "(",
        field(
          "parameters",
          optional(
            seq($.extern_param, repeat(seq(",", $.extern_param))),
          ),
        ),
        ")",
      ),

    extern_param: ($) =>
      seq(
        optional($.attributes),
        field("type", $._type),
        field("name", $.identifier),
      ),

    class_inherits_decl: ($) =>
      prec.left(
        seq(
          "inherit",
          scoped(
            seq(
              choice(
                $._type,
                alias($._inline_multiline_generic_type, $.generic_type),
              ),
              optional($._expression),
            ),
            $._indent,
            $._dedent,
          ),
        ),
      ),

    // The DEDENT precedes '>': the scanner emits it on seeing '>' (as a
    // bracket-end-style close of the TYPE_APP_INDENT), so '>' on the same
    // line as the last arg closes the scope without needing a NEWLINE first.
    _inline_multiline_generic_type: ($) =>
      prec.right(
        5,
        seq(
          $.long_identifier,
          "<",
          $._type_app_indent,
          $.type_attributes,
          $._dedent,
          ">",
        ),
      ),

    field_initializer: ($) =>
      prec(
        PREC.SPECIAL_INFIX + 1,
        seq(
          field("field", $.long_identifier),
          token(prec(10000000, "=")),
          field("value", $._expression),
        ),
      ),

    field_initializers: ($) =>
      prec(
        10000000,
        seq(
          $.field_initializer,
          repeat(seq($._newline, $.field_initializer)),
          optional($._newline),
        ),
      ),

    //
    // Type rules (END)
    //

    //
    // Constants (BEGIN)
    //
    _escape_char: (_) => token.immediate(prec(100, /\\["\'ntbrafv]/)),
    _non_escape_char: (_) => token.immediate(prec(100, /\\[^"\'ntbrafv]/)),
    // using \u0008 to model \b
    _simple_char_char: (_) => token.immediate(/[^\n\t\r\u0008\a\f\v'\\]/),
    _unicodegraph_short: (_) => /\\u[0-9a-fA-F]{4}/,
    // The long form is spelled with a capital U (`\UXXXXXXXX`); with a
    // lowercase `u` the short form above already claims the first four
    // digits, so the long alternative was unreachable.
    _unicodegraph_long: (_) => /\\U[0-9a-fA-F]{8}/,
    _trigraph: (_) => /\\[0-9]{3}/,
    _hexgraph_short: (_) => /\\x[0-9a-fA-F]{2}/,

    _char_char: ($) =>
      choice(
        $._simple_char_char,
        $._escape_char,
        $._trigraph,
        $._unicodegraph_short,
        $._hexgraph_short,
      ),

    // note: \n, \r and \t are all allowed in strings.
    // \r must be permitted so multi-line string literals survive CRLF
    // endings (the \r of a \r\n break sits inside the string
    // body); \t so a literal tab in a string is not a terminator.
    _simple_string_char: ($) =>
      choice(
        $._inside_string_marker,
        token.immediate(prec(1, /[^\u0008\a\f\v\\"]/)),
      ),

    _string_char: ($) =>
      choice(
        $._simple_string_char,
        $._escape_char,
        $._trigraph,
        $._unicodegraph_short,
        $._hexgraph_short,
        $._non_escape_char,
        $._unicodegraph_long,
      ),

    char: (_) =>
      prec(
        -1,
        /'([^\n\t\r\u0008\a\f\v\\]|\\["\'ntbrafv]|\\[0-9]{3}|\\u[0-9a-fA-F]{4}|\\U[0-9a-fA-F]{8}|\\x[0-9a-fA-F]{2}|(\\\\))?'B?/,
      ),

    format_string_eval: ($) =>
      choice(
        seq(token.immediate(prec(1000, "{")), $._expression, "}"),
        seq($._multi_dollar_interp_start, $._expression, $._multi_dollar_interp_end),
      ),

    format_string: ($) =>
      choice(
        seq(
          token(prec(100, '$"')),
          repeat(
            choice(
              $.format_string_eval,
              // '{{' / '}}' are literal-brace escapes; must outrank the
              // interpolation-hole '{' (prec 1000).
              token.immediate(prec(1001, /\{\{|\}\}/)),
              $._string_char,
            ),
          ),
          '"',
        ),
        // Verbatim interpolated: $@"..." / @$"..." — backslashes are literal,
        // '""' is an escaped quote, {expr} holes still apply.
        seq(
          token(prec(100, choice('$@"', '@$"'))),
          repeat(
            choice(
              $.format_string_eval,
              token.immediate(prec(1001, /\{\{|\}\}/)),
              $._verbatim_string_char,
            ),
          ),
          token.immediate('"'),
        ),
      ),

    _string_literal: ($) => seq('"', repeat($._string_char), '"'),

    string: ($) => choice($._string_literal, $.format_string),

    _verbatim_string_char: ($) =>
      choice($._simple_string_char, $._non_escape_char, "\\", /\"\"/),
    verbatim_string: ($) =>
      seq('@"', repeat($._verbatim_string_char), token.immediate('"')),
    bytearray: ($) => seq('"', repeat($._string_char), token.immediate('"B')),
    verbatim_bytearray: ($) =>
      seq('@"', repeat($._verbatim_string_char), token.immediate('"B')),

    format_triple_quoted_string: ($) =>
      choice(
        seq(
          token(prec(100, '$"""')),
          optional($._format_triple_quoted_content),
          repeat(seq($.format_string_eval, optional($._format_triple_quoted_content))),
          '"""',
        ),
        seq(
          $._multi_dollar_triple_quote_start,
          optional($._multi_dollar_triple_quoted_content),
          repeat(seq($.format_string_eval, optional($._multi_dollar_triple_quoted_content))),
          $._multi_dollar_triple_quote_end,
        ),
      ),

    triple_quoted_string: ($) =>
      choice(
        seq('"""', $._triple_quoted_content, '"""'),
        $.format_triple_quoted_string,
      ),

    bool: (_) => token(choice("true", "false")),

    unit: (_) => token(prec(100000, /\([ \t]*\)/)),

    const: ($) =>
      choice(
        $.sbyte,
        $.int16,
        $.int32,
        $.int64,
        $.byte,
        $.uint16,
        $.uint32,
        $.int,
        $.xint,
        $.nativeint,
        $.unativeint,
        $.decimal,
        $.float,
        $.uint64,
        $.ieee32,
        $.ieee64,
        $.bignum,
        $.char,
        $.string,
        $.verbatim_string,
        $.triple_quoted_string,
        $.bytearray,
        $.verbatim_bytearray,
        $.bool,
        $.unit,
      ),

    // Identifiers:
    identifier: (_) =>
      token(
        choice(
          /[_\p{XID_Start}][_'\p{XID_Continue}]*/,
          // Double-backtick identifier: any run of non-newline chars delimited
          // by ``. Single backticks may appear inside (e.g. ``a `b` c``); the
          // delimiter is the next `` pair, so a backtick is only consumed when
          // followed by a non-backtick.
          /``([^`\n\r\t]|`[^`\n\r\t])+``/,
        ),
      ),

    long_identifier: ($) =>
      prec.right(seq($.identifier, repeat(seq(".", $.identifier)))),

    active_pattern: ($) =>
      prec(
        1000,
        seq(
          "(|",
          alias($.identifier, $.active_pattern_op_name),
          repeat(seq("|", alias($.identifier, $.active_pattern_op_name))),
          optional(seq("|", alias("_", $.wildcard_active_pattern_op))),
          "|)",
        ),
      ),

    op_identifier: (_) =>
      token(
        prec(
          // Above the 10000 of _high_prec_app's immediate '(' so a section
          // like f(+) lexes as an operator reference, not application-paren.
          100001,
          seq(
            "(",
            /\s*/,
            // '-' sits last so it is a literal, not a `+`-to-`.` range that
            // would admit ',' as an operator character.
            choice("?", /[!%&*+./<=>@^|~$?-][!%&*+./<=>@^|~?-]*/, ".. .."),
            /\s*/,
            ")",
          ),
        ),
      ),

    _identifier_or_op: ($) =>
      choice($.identifier, $.op_identifier, $.active_pattern),

    _infix_or_prefix_op: (_) => choice("+", "-", "+.", "-.", "%", "&"),

    prefix_op: ($) =>
      prec.left(
        choice($._infix_or_prefix_op, "&&", "%%", repeat1("~"), /[!?][!%&*+./<=>@^|~?-]*/),
      ),

    infix_op: ($) =>
      prec(
        PREC.INFIX_OP,
        choice(
          $._infix_or_prefix_op,
          token.immediate(prec(1, /[+-]/)),
          /[-+=<>|&^*'%@?][!%&*+./<=>@^|~?-]*/,
          /\/[!%&*+.<=>@^|~?-]*/,
          // Dotted custom operators such as .*. .-. ./. — F# allows '.' to begin
          // an operator. A lone '.' is member access and '..' is a range, so the
          // char right after the first '.' must be a non-dot operator char.
          /\.[!%&*+/<=>@^|~?-][!%&*+./<=>@^|~?-]*/,
          "=",
          "!=",
          ":=",
          "::",
          "$",
          "?",
          "?<-",
          "?->",
        ),
      ),

    // Numbers
    int: (_) => token(/[+-]?([0-9]_?)+/),
    xint: (_) =>
      token(
        choice(/0[xX]([0-9a-fA-F]_?)+/, /0[oO]([0-7]_?)+/, /0[bB]([0-1]_?)+/),
      ),

    sbyte: ($) => seq(choice($.int, $.xint), token.immediate("y")),
    byte: ($) => seq(choice($.int, $.xint), token.immediate("uy")),
    int16: ($) => seq(choice($.int, $.xint), token.immediate("s")),
    uint16: ($) => seq(choice($.int, $.xint), token.immediate("us")),
    int32: ($) => seq(choice($.int, $.xint), token.immediate("l")),
    uint32: ($) =>
      seq(choice($.int, $.xint), token.immediate(choice("ul", "u"))),
    nativeint: ($) => seq(choice($.int, $.xint), token.immediate("n")),
    unativeint: ($) => seq(choice($.int, $.xint), token.immediate("un")),
    int64: ($) => seq(choice($.int, $.xint), token.immediate("L")),
    uint64: ($) =>
      seq(choice($.int, $.xint), token.immediate(choice("UL", "uL"))),

    ieee32: ($) =>
      choice(
        seq($.float, token.immediate(/[fF]/)),
        seq($.xint, token.immediate("lf")),
      ),
    ieee64: ($) => seq($.xint, token.immediate("LF")),

    bignum: ($) => seq($.int, token.immediate(/[QRZING]/)),
    decimal: ($) => seq(choice($.float, $.int), token.immediate(/[Mm]/)),

    float: ($) =>
      prec.right(
        alias(
          choice(
            seq(
              $.int,
              token.immediate("."),
              optional(token.immediate(/([0-9]_?)+/)),
            ),
            seq(
              $.int,
              optional(
                seq(
                  token.immediate("."),
                  token.immediate(/([0-9]_?)+/),
                ),
              ),
              token.immediate(/[eE][+-]?/),
              token.immediate(/([0-9]_?)+/),
            ),
          ),
          "float",
        ),
      ),

    //
    // Constants (END)
    //
    //
    block_comment: ($) =>
      seq("(*", $.block_comment_content, token.immediate("*)")),
    inline_line_comment: (_) =>
      token.immediate(
        choice(
          /[ \t]*\/\/([^/\n\r][^\n\r]*)?/,
          /[ \t]*\/{4,}[^\n\r]*/,
        ),
      ),
    line_comment: (_) =>
      token(
        choice(
          /\/\/([^/\n\r][^\n\r]*)?/,
          /\/{4,}[^\n\r]*/,
          // Shebang line for .fsx scripts (e.g. #!/usr/bin/env dotnet fsi).
          // F# has no #-comment syntax; fsi simply skips this line, so we lex
          // it as trivia. Widening this token costs ~0 table size (unlike a
          // compiler_directive_decl production).
          /#![^\n\r]*/,
        ),
      ),
    xml_doc: (_) => token(/\/\/\/([^/\n\r][^\n\r]*)?/),

    // preprocessors
    compiler_directive_decl: ($) =>
      prec(
        100000,
        choice(
          seq(
            "#nowarn",
            repeat1(choice(alias($._string_literal, $.string), $.int)),
            $._newline_not_aligned,
          ),
          seq("#warnon", $.int, $._newline_not_aligned),
          // `#light` also takes an explicit argument: `#light "off"` selects
          // the legacy verbose syntax, `#light "on"` the default. The string
          // is optional so bare `#light` keeps working.
          seq(
            "#light",
            optional(alias($._string_literal, $.string)),
            $._newline_not_aligned,
          ),
        ),
      ),

    fsi_directive_decl: ($) =>
      seq(
        choice("#r", "#load", "#time", "#I", "#help", "#quit"),
        optional(choice(alias($._string_literal, $.string), $.verbatim_string)),
        // _newline_not_aligned instead of a literal /\n/ so the directive can
        // close right before '#endif' (the scanner eats the raw newline while
        // peeking at '#') and at EOF. Also gives extras a definite ending.
        $._newline_not_aligned,
      ),

    preproc_line: ($) =>
      seq(
        alias(/#(line)?/, "#line"),
        $.int,
        optional(choice(alias($._string_literal, $.string), $.verbatim_string)),
        $._newline_not_aligned,
      ),

    _preproc_expression: ($) =>
      choice(
        $.identifier,
        $.bool,
        $.preproc_if_not_expression,
        $.preproc_if_and_expression,
        $.preproc_if_or_expression,
        seq("(", $._preproc_expression, ")"),
      ),

    preproc_if_not_expression: ($) => seq("!", $._preproc_expression),

    preproc_if_and_expression: ($) =>
      prec.left(2, seq($._preproc_expression, "&&", $._preproc_expression)),

    preproc_if_or_expression: ($) =>
      prec.left(1, seq($._preproc_expression, "||", $._preproc_expression)),

    ...preprocIf(
      "",
      // Optional: branches may be empty (e.g. `#if INTERACTIVE\n#else ...`),
      // and an fsi directive consumed as an extra must not leave the branch
      // unparseable.
      ($) =>
        optional(
          choice(
            // Optional trailing attributes: a branch may end with attributes
            // that decorate the declaration following #endif, e.g.
            //   #if !NETSTANDARD
            //   [<HubName("CompanyHub")>]
            //   #endif
            //   type CompanyHub() = ...
            seq(
              repeat1(seq($._module_elem, optional($._newline))),
              optional($.attributes),
            ),
            $.attributes,
            alias($._preproc_toplevel_module, $.named_module),
          ),
        ),
    ),
    ...preprocIf(
      "_in_expression",
      // Besides expressions, allow body-less let bindings: a branch often
      // ends with `let x = ...` whose body is the code following #endif
      // (the classic `#if INTERACTIVE` pattern).
      // prec(-3) on the items: at module-body positions the same content
      // also parses as _module_elem; prefer that statically instead of
      // declaring a GLR conflict — the split otherwise stays alive for the
      // whole branch and multiplies version pressure inside module-spanning
      // directives.
      ($) =>
        repeat(
          prec(
            -3,
            seq(
              optional($._newline),
              choice(
                $._expression,
                alias($.value_declaration, $.declaration_expression),
              ),
            ),
          ),
        ),
      -2,
    ),
    ...preprocIf(
      "_in_module_body",
      ($) => repeat(seq(optional($._newline), $._module_body_elem)),
      // -1 (above _in_expression's -2): where both variants apply — inside a
      // `module X =` body — the module-body variant subsumes expressions, so
      // prefer it statically rather than keeping a GLR split alive.
      -1,
    ),
    ...preprocIf(
      "_in_class_definition",
      ($) => repeat(seq(optional($._newline), $._class_type_body_inner)),
      -2,
    ),
    ...preprocIf("_in_member_definition", ($) => repeat($.member_defn), -2),
  },
});

/**
 *
 * @param {Rule} rule
 *
 * @param {Rule} indent
 *
 * @param {Rule} dedent
 *
 * @return {Rule}
 */
function scoped(rule, indent, dedent) {
  return field("block", seq(indent, rule, dedent));
}

/**
 * The '->' token with lexical precedence 2, so that `expr->` (no space, e.g.
 * a when-guard ending `isPfOwned->`) beats infix_op's token.immediate
 * (prec 1, /[+-]/), which would otherwise lex the '-' alone and error on '>'.
 * All occurrences must share this one definition: identical token specs merge
 * into a single lexer token, while diverging precedences would split it.
 *
 * @return {RuleOrLiteral}
 */
function arrow() {
  return token(prec(2, "->"));
}

/**
 *
 * @param {string} suffix
 *
 * @param {RuleBuilder<string>} content
 *
 * @param {number} precedence
 *
 * @return {RuleBuilders<string, string>}
 */
function preprocIf(suffix, content, precedence = 0) {
  /**
   *
   * @param {GrammarSymbols<string>} $
   *
   * @return {Rule}
   *
   */
  function alternativeBlock($) {
    return suffix
      ? alias($["preproc_else" + suffix], $.preproc_else)
      : $.preproc_else;
  }

  return {
    ["preproc_if" + suffix]: ($) =>
      prec(
        precedence,
        seq(
          // Top-level only: attributes may precede the #if when the directive
          // wraps the attributed declaration, e.g. `[<Literal>]` / `#if X` /
          // `let c = ...`. Other variants get ambiguous with attribute rules.
          ...(suffix === "" ? [optional($.attributes)] : []),
          "#if",
          field("condition", $._preproc_expression),
          $._newline_not_aligned,
          content($),
          field("alternative", optional(alternativeBlock($))),
          "#endif",
        ),
      ),

    ["preproc_else" + suffix]: ($) =>
      prec(precedence, seq("#else", content($))),
  };
}
