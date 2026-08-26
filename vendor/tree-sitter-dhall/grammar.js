/**
 * @file Dhall grammar for the tree-sitter parsing library
 * @author jae beller <foss@jae.zone>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const bit = /[01]/;
const digit = /[0-9]/;
const hexdigit = /[0-9A-Fa-f]/;
const binary_natural = seq("0b", repeat1(bit));
const decimal_natural = choice("0", seq(/[1-9]/, repeat(digit)));
const hexadecimal_natural = seq("0x", repeat1(hexdigit));
const double_exponent = seq(/[Ee][+-]?/, repeat1(digit));
const full_date = /[0-9]{4}-[0-9]{2}-[0-9]{2}/;
const time_offset = /[+-][0-9]{2}:[0-9]{2}/;
const partial_time = seq(
  /[0-9]{2}:[0-9]{2}:[0-9]{2}/,
  optional(seq(".", repeat1(digit))),
  optional(choice(/[Zz]/, time_offset)),
);

/**
 * @type {(
 *   precedence: number,
 *   $: GrammarSymbols<"_operator_expression">,
 *   symbol: string | ChoiceRule
 * ) => Rule}
 */
const operator = (precedence, $, symbol) =>
  prec.left(
    precedence,
    seq(
      $._operator_expression,
      alias(symbol, $.infix_operator),
      $._operator_expression,
    ),
  );

const http_raw = () => {
  const octet = /[0-9]{1,3}/;
  const ipv4 = seq(octet, ".", octet, ".", octet, ".", octet);

  const h16 = /[0-9A-Fa-f]{1,4}/;
  const ls32 = choice(seq(h16, ":", h16), ipv4);
  const ipv6 = choice(
    ...(function* () {
      /** @type {(n: number, x: Rule) => Array<Rule>} */
      const repeatN = (n, x) => Array(n).fill(x);

      yield seq(...repeatN(6, seq(h16, ":")), ls32);
      yield seq("::", ...repeatN(5, seq(h16, ":")), ls32);
      for (let i = 0; i < 5; i++) {
        yield seq(
          optional(
            i === 0 ? h16 : seq(h16, ...repeatN(i, optional(seq(":", h16)))),
          ),
          "::",
          ...repeatN(4 - i, seq(h16, ":")),
          ls32,
        );
      }
      yield seq(
        optional(seq(h16, ...repeatN(5, optional(seq(":", h16))))),
        "::",
        h16,
      );
      yield seq(
        optional(seq(h16, ...repeatN(6, optional(seq(":", h16))))),
        "::",
      );
    })(),
  );

  const alphanum = /[0-9A-Za-z]/;
  const unreserved = choice(alphanum, "-", ".", "_", "~");
  const pct_encoded = seq("%", hexdigit, hexdigit);
  const sub_delims = /[\!\$\&\'\*\+;=]/;

  const ipvfuture = seq(
    "v",
    repeat1(digit),
    ".",
    repeat1(choice(unreserved, sub_delims, ":")),
  );

  const domainlabel = seq(
    repeat1(alphanum),
    repeat(seq(repeat1("-"), repeat1(alphanum))),
  );
  const domain = seq(domainlabel, repeat(seq(".", domainlabel)), optional("."));

  const pchar = choice(unreserved, pct_encoded, sub_delims, ":", "@");

  const http_scheme = seq(choice("http", "https"), "://");
  const http_authority = seq(
    optional(
      seq(repeat(choice(unreserved, pct_encoded, sub_delims, ":")), "@"),
    ),
    choice(seq("[", choice(ipv6, ipvfuture), "]"), ipv4, domain),
    optional(seq(":", repeat(digit))),
  );
  const http_path_segment = repeat(pchar);
  const http_query = repeat(choice(pchar, "/", "?"));

  return seq(
    http_scheme,
    http_authority,
    repeat(seq("/", http_path_segment)),
    optional(seq("?", http_query)),
  );
};

module.exports = grammar({
  name: "dhall",

  externals: ($) => [$.block_comment_content, $.block_comment_end],

  extras: ($) => [$.line_comment, $.block_comment, /\s/],

  inline: ($) => [
    $.empty_list_literal,
    $._double_quote_chunk,
    $._single_quote_chunk,
  ],

  word: ($) => $.label,

  rules: {
    expression: ($) =>
      choice(
        $.lambda_expression,
        $.if_then_else_expression,
        $.let_expression,
        $.forall_expression,
        $.function_type,
        $.with_expression,
        $.empty_list_literal,
        $.assert_expression,
        seq($._operator_expression, optional($._type)),
      ),
    label: (_) => /`[ -_a-~]*`|[A-Z_a-z][0-9\-/A-Z_a-z]*/,
    _type: ($) => seq(alias(":", $.type_operator), field("type", $.expression)),

    _label_or_some: ($) => choice($.label, alias("Some", $.label)),

    keyword: (_) =>
      choice(
        "if",
        "then",
        "else",
        "let",
        "in",
        "as",
        "using",
        "merge",
        "missing",
        "Infinity",
        "NaN",
        "Some",
        "toMap",
        "assert",
        "forall",
        "\u2200",
        "with",
        "showConstructor",
      ),
    builtin: (_) =>
      choice(
        "Natural/fold",
        "Natural/build",
        "Natural/isZero",
        "Natural/even",
        "Natural/odd",
        "Natural/toInteger",
        "Natural/show",
        "Integer/toDouble",
        "Integer/show",
        "Integer/negate",
        "Integer/clamp",
        "Natural/subtract",
        "Double/show",
        "List/build",
        "List/fold",
        "List/length",
        "List/head",
        "List/last",
        "List/indexed",
        "List/reverse",
        "Text/show",
        "Text/replace",
        "Date/show",
        "Time/show",
        "TimeZone/show",
        "Bool",
        "Optional",
        "None",
        "Natural",
        "Integer",
        "Double",
        "Text",
        "Bytes",
        "Date",
        "Time",
        "TimeZone",
        "List",
        "Type",
        "Kind",
        "Sort",
      ),

    lambda_expression: ($) => seq($.lambda_operator, $._function_expression),
    forall_expression: ($) => seq($.forall_operator, $._function_expression),
    _function_expression: ($) =>
      seq(
        "(",
        field("label", $.label),
        $._type,
        ")",
        $.arrow_operator,
        field("expression", $.expression),
      ),
    arrow_operator: (_) => choice("\u2192", "->"),
    lambda_operator: (_) => choice("\u03BB", "\\"),
    forall_operator: (_) => choice("\u2200", "forall"),

    if_then_else_expression: ($) =>
      seq(
        "if",
        field("predicate", $.expression),
        "then",
        field("expression", $.expression),
        "else",
        field("alternate", $.expression),
      ),

    let_expression: ($) => seq(repeat1($.let_binding), "in", $.expression),
    let_binding: ($) =>
      seq(
        "let",
        field("label", $.label),
        optional($._type),
        alias("=", $.assign_operator),
        $.expression,
      ),

    function_type: ($) =>
      seq(
        field("left", $._operator_expression),
        $.arrow_operator,
        field("right", $.expression),
      ),

    with_expression: ($) =>
      seq(
        choice($.with_expression, $._import_expression),
        "with",
        seq(
          $._with_component,
          alias(
            repeat(seq(alias(".", $.selector_dot), $._with_component)),
            $.selector,
          ),
          alias("=", $.assign_operator),
          $._operator_expression,
        ),
      ),
    _with_component: ($) =>
      choice($._label_or_some, alias("?", $.question_mark)),

    empty_list_literal: ($) =>
      seq(
        alias($._empty_list_literal_primitive, $.primitive_expression),
        $._type,
      ),
    _empty_list_literal_primitive: ($) =>
      alias($._empty_list_literal_text, $.list_literal),
    _empty_list_literal_text: (_) => seq("[", optional(","), "]"),

    assert_expression: ($) =>
      seq("assert", alias(":", $.assert_operator), $.expression),

    _operator_expression: ($) =>
      choice(
        $.equivalence_expression,
        $.import_alt_expression,
        $.or_expression,
        $.plus_expression,
        $.text_append_expression,
        $.list_append_expression,
        $.and_expression,
        $.combine_expression,
        $.prefer_expression,
        $.combine_types_expression,
        $.times_expression,
        $.equal_expression,
        $.not_equal_expression,
        $.application_expression,
        $._import_expression,
      ),
    equivalence_expression: ($) => operator(0, $, choice("\u2261", "===")),
    import_alt_expression: ($) => operator(1, $, "?"),
    or_expression: ($) => operator(2, $, "||"),
    plus_expression: ($) => operator(3, $, "+"),
    text_append_expression: ($) => operator(4, $, "++"),
    list_append_expression: ($) => operator(5, $, "#"),
    and_expression: ($) => operator(6, $, "&&"),
    combine_expression: ($) => operator(7, $, choice("\u2227", "/\\")),
    prefer_expression: ($) => operator(8, $, choice("\u2AFD", "//")),
    combine_types_expression: ($) => operator(9, $, choice("\u2A53", "//\\\\")),
    times_expression: ($) => operator(10, $, "*"),
    equal_expression: ($) => operator(11, $, "=="),
    not_equal_expression: ($) => operator(12, $, "!="),

    application_expression: ($) =>
      seq(
        choice($.builtin_function, $._import_expression),
        $._import_expression,
        repeat($._import_expression),
      ),
    builtin_function: (_) =>
      choice("merge", "Some", "toMap", "showConstructor"),

    _import_expression: ($) =>
      choice($.completion, $.primitive_expression, $.import),
    completion: ($) =>
      seq(
        field("type", $.primitive_expression),
        alias("::", $.completion_operator),
        $.primitive_expression,
      ),

    primitive_expression: ($) =>
      seq(
        choice(
          $.temporal_literal,
          $.numeric_literal,
          $.text_literal,
          $.bytes_literal,
          $.record_literal,
          $.record_type,
          $.union_type,
          $.list_literal,
          $.boolean_literal,
          $.identifier,
          seq("(", $.expression, ")"),
        ),
        optional(field("selector", $.selector)),
      ),
    selector: ($) =>
      repeat1(
        seq(
          alias(".", $.selector_dot),
          choice($.label, $.label_set, $.type_selector),
        ),
      ),
    label_set: ($) =>
      seq(
        "{",
        optional(","),
        optional(seq($._label_or_some, repeat(seq(",", $._label_or_some)))),
        optional(","),
        "}",
      ),
    type_selector: ($) => seq("(", $.expression, ")"),

    import: ($) =>
      prec.right(
        seq(
          $._import_type,
          optional($.import_hash),
          optional(
            seq(
              "as",
              choice(
                alias("Text", $.import_as_text),
                alias("Location", $.import_as_location),
                alias("Bytes", $.import_as_bytes),
              ),
            ),
          ),
        ),
      ),
    _import_type: ($) =>
      choice(
        alias("missing", $.missing_import),
        $.local_import,
        $.http_import,
        $.env_import,
      ),
    import_hash: (_) => /sha256:[0-9A-Fa-f]{64}/,

    local_import: (_) =>
      token(
        seq(
          optional(choice(".", "..", "~")),
          repeat1(
            choice(
              /\/[^\x00- \"\#\(\),\/<>\?\[\\\]\{\}\x7f]+/,
              /\/"[^\x00-\x1f\\\/]+"/,
            ),
          ),
        ),
      ),

    http_import: ($) =>
      seq($.http_raw, optional(seq("using", $._import_expression))),
    http_raw: (_) => token(http_raw()),

    env_import: ($) => seq(/[eE][nN][vV]:/, $.env_variable),
    env_variable: ($) => choice($._env_bash, $._env_posix),
    _env_bash: (_) => /[A-Z_a-z][0-9A-Z_a-z]*/,
    _env_posix: (_) => /"(:?\\[\"\\abfnrtv]|[ \!\#-<>-\[\]-~])+"/,

    line_comment: ($) => seq($.line_comment_prefix, $.line_comment_content),
    line_comment_prefix: (_) => "--",
    line_comment_content: (_) => /.*/,

    block_comment: ($) =>
      choice(
        seq(
          $.block_comment_open,
          $.block_comment_content,
          $.block_comment_close,
        ),
      ),
    block_comment_open: (_) => "{-",
    block_comment_close: (_) => "-}",

    temporal_literal: (_) =>
      token(
        choice(
          seq(full_date, /[Tt]/, partial_time),
          partial_time,
          full_date,
          time_offset,
        ),
      ),

    numeric_literal: ($) =>
      choice($.double_literal, $.natural_literal, $.integer_literal),
    double_literal: (_) =>
      choice(
        "-Infinity",
        "Infinity",
        "NaN",
        token(
          seq(
            optional(choice("+", "-")),
            repeat1(digit),
            choice(
              seq(".", repeat1(digit), optional(double_exponent)),
              double_exponent,
            ),
          ),
        ),
      ),
    natural_literal: (_) =>
      token(choice(binary_natural, decimal_natural, hexadecimal_natural)),
    integer_literal: (_) =>
      token(
        seq(
          choice("+", "-"),
          choice(binary_natural, decimal_natural, hexadecimal_natural),
        ),
      ),

    text_literal: ($) => choice($.double_quote_literal, $.single_quote_literal),
    double_quote_literal: ($) => seq('"', repeat($._double_quote_chunk), '"'),
    _double_quote_chunk: ($) =>
      choice(
        prec(1, $.interpolation),
        $.double_quote_escaped,
        token.immediate(prec(1, /[^\x00-\x1f\"\\\$]+/)),
        token.immediate("$"),
      ),
    double_quote_escaped: (_) =>
      token(
        choice(
          /\\["\$\\/bfnrt]/,
          /\\u[0-9A-Fa-f]{4}/,
          /\\u\{0*[0-9A-Fa-f]{1,6}\}/,
        ),
      ),
    single_quote_literal: ($) =>
      seq("''", choice("\n", "\r\n"), repeat($._single_quote_chunk), "''"),
    _single_quote_chunk: ($) =>
      choice(
        prec(1, $.interpolation),
        $.single_quote_escaped,
        token.immediate(prec(1, /[^\'\$]+/)),
        token.immediate("'"),
        token.immediate("$"),
      ),
    single_quote_escaped: (_) => token(choice("'''", "''${")),
    interpolation: ($) => seq("${", $.expression, "}"),

    // The grammar requires hexdigits be in pairs, but I dislike how this causes
    // flashing between bytes and error as you type. I've loosened it to accept
    // any number of bytes.
    bytes_literal: (_) => seq('0x"', repeat(hexdigit), '"'),

    record_literal: ($) =>
      seq(
        "{",
        optional(","),
        choice(
          alias("=", $.empty_record_literal),
          seq($.record_literal_entry, repeat(seq(",", $.record_literal_entry))),
        ),
        optional(","),
        "}",
      ),
    record_literal_entry: ($) =>
      seq(
        $._label_or_some,
        optional(
          seq(
            alias(
              repeat(seq(alias(".", $.selector_dot), $._label_or_some)),
              $.selector,
            ),
            alias("=", $.assign_operator),
            $.expression,
          ),
        ),
      ),

    record_type: ($) =>
      seq(
        "{",
        optional(","),
        optional(
          seq(
            $.record_type_entry,
            repeat(seq(",", $.record_type_entry)),
            optional(","),
          ),
        ),
        "}",
      ),
    record_type_entry: ($) => seq($._label_or_some, $._type),

    union_type: ($) =>
      seq(
        "<",
        optional("|"),
        optional(
          seq(
            $.union_type_entry,
            repeat(seq("|", $.union_type_entry)),
            optional("|"),
          ),
        ),
        ">",
      ),
    union_type_entry: ($) => seq($._label_or_some, optional($._type)),

    list_literal: ($) =>
      seq(
        "[",
        optional(","),
        $.expression,
        repeat(seq(",", $.expression)),
        optional(","),
        "]",
      ),

    boolean_literal: (_) => choice("True", "False"),

    identifier: ($) =>
      choice(seq($.label, optional($.de_bruijn_index)), $.builtin),
    de_bruijn_index: ($) => seq("@", $.natural_literal),
  },
});
