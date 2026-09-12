/**
 * @file Parser for Jinja2 and daughter languages: nunjucks, twig
 * @author Benny Powers <web@bennypowers.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  CONDITIONAL: 1,
  OR: 2,
  AND: 3,
  IS: 4,
  MEMBERSHIP: 5,
  COMPARISON: 6,
  CONCAT: 7,
  ADDITIVE: 8,
  MULTIPLICATIVE: 9,
  UNARY: 10,
  POWER: 11,
  FILTER: 12,
  MEMBER: 13,
};

export default grammar({
  name: "jinja",

  externals: $ => [
    $.raw_text,
    $.comment_content,
    // GitComet: a leading `---` YAML block, so it can be injected as YAML.
    $.front_matter,
  ],

  extras: $ => [/\s/],

  word: $ => $.identifier,

  conflicts: $ => [
    [$._argument, $.tuple],
    [$._argument, $.parenthesized_expression],
  ],

  rules: {
    source_file: $ => seq(optional($.front_matter), repeat($._node)),

    _node: $ => choice(
      $.statement,
      $.output,
      $.comment,
      $.text,
    ),

    text: $ => /([^{]|\{[^{%#])+/,

    // ── Output {{ }} ──────────────────────────────────────────────

    output: $ => seq(
      $._output_start,
      $._expression,
      $._output_end,
    ),

    _output_start: _ => choice('{{', '{{-', '{{~'),
    _output_end: _ => choice('}}', '-}}', '~}}'),

    // ── Comment {# #} ─────────────────────────────────────────────

    comment: $ => seq(
      $._comment_start,
      optional($.comment_content),
      $._comment_end,
    ),

    _comment_start: _ => choice('{#', '{#-', '{#~'),
    _comment_end: _ => choice('#}', '-#}', '~#}'),

    // ── Statements {% %} ──────────────────────────────────────────

    statement: $ => choice(
      $.if_statement,
      $.for_statement,
      $.block_statement,
      $.macro_statement,
      $.call_statement,
      $.filter_statement,
      $.raw_statement,
      $.with_statement,
      $.autoescape_statement,
      $.set_statement,
      $.extends_statement,
      $.include_statement,
      $.import_statement,
      $.from_statement,
      $.do_statement,
      $.generic_tag,
      $._unpaired_tag,
    ),

    generic_tag: $ => seq(
      $._statement_start,
      field('name', $.identifier),
      repeat($._expression),
      $._statement_end,
    ),

    _unpaired_tag: $ => prec(-10, seq(
      $._statement_start,
      choice(
        'endif', 'endfor', 'endblock', 'endmacro', 'endcall',
        'endfilter', 'endwith', 'endautoescape', 'endset',
      ),
      optional($.identifier),
      $._statement_end,
    )),

    _statement_start: _ => choice('{%', '{%-', '{%~'),
    _statement_end: _ => choice('%}', '-%}', '~%}'),

    // ── Block statements ──────────────────────────────────────────

    if_statement: $ => seq(
      $._statement_start, 'if', field('condition', $._expression), $._statement_end,
      repeat(choice($._node, $.elseif_clause, $.else_clause)),
      $._statement_start, 'endif', $._statement_end,
    ),

    elseif_clause: $ => seq(
      $._statement_start, choice('elif', 'elseif'), field('condition', $._expression), $._statement_end,
    ),

    else_clause: $ => seq(
      $._statement_start, 'else', $._statement_end,
    ),

    for_statement: $ => seq(
      $._statement_start, 'for',
      field('variable', $._identifier_list),
      'in',
      field('iterable', $._expression),
      optional($.for_condition),
      optional('recursive'),
      $._statement_end,
      repeat(choice($._node, $.else_clause)),
      $._statement_start, 'endfor', $._statement_end,
    ),

    for_condition: $ => seq('if', $._expression),

    block_statement: $ => seq(
      $._statement_start, 'block', field('name', $.identifier), optional('scoped'), $._statement_end,
      repeat($._node),
      $._statement_start, 'endblock', optional(field('end_name', $.identifier)), $._statement_end,
    ),

    macro_statement: $ => seq(
      $._statement_start, 'macro', field('name', $.identifier), $.argument_list, $._statement_end,
      repeat($._node),
      $._statement_start, 'endmacro', $._statement_end,
    ),

    call_statement: $ => seq(
      $._statement_start, 'call',
      optional($.argument_list),
      $._expression,
      $._statement_end,
      repeat($._node),
      $._statement_start, 'endcall', $._statement_end,
    ),

    filter_statement: $ => seq(
      $._statement_start, 'filter',
      field('name', $.identifier),
      optional($.argument_list),
      $._statement_end,
      repeat($._node),
      $._statement_start, 'endfilter', $._statement_end,
    ),

    raw_statement: $ => seq(
      $._statement_start, choice('raw', 'verbatim'), $._statement_end,
      optional($.raw_text),
      $._statement_start, choice('endraw', 'endverbatim'), $._statement_end,
    ),

    with_statement: $ => seq(
      $._statement_start, 'with',
      optional($.assignment_list),
      $._statement_end,
      repeat($._node),
      $._statement_start, 'endwith', $._statement_end,
    ),

    autoescape_statement: $ => seq(
      $._statement_start, 'autoescape', $._expression, $._statement_end,
      repeat($._node),
      $._statement_start, 'endautoescape', $._statement_end,
    ),

    // ── Inline statements ─────────────────────────────────────────

    set_statement: $ => choice(
      seq(
        $._statement_start, 'set',
        field('variable', $._identifier_list),
        '=',
        field('value', $._expression),
        $._statement_end,
      ),
      seq(
        $._statement_start, 'set',
        field('variable', $.identifier),
        $._statement_end,
        repeat($._node),
        $._statement_start, 'endset', $._statement_end,
      ),
    ),

    extends_statement: $ => seq(
      $._statement_start, 'extends', $._expression, $._statement_end,
    ),

    include_statement: $ => seq(
      $._statement_start, 'include', $._expression,
      optional(choice(
        seq('with', 'context'),
        seq('without', 'context'),
      )),
      optional(seq('ignore', 'missing')),
      $._statement_end,
    ),

    import_statement: $ => seq(
      $._statement_start, 'import', $._expression,
      'as', field('alias', $.identifier),
      optional(choice(
        seq('with', 'context'),
        seq('without', 'context'),
      )),
      $._statement_end,
    ),

    from_statement: $ => seq(
      $._statement_start, 'from', $._expression,
      'import', $.import_list,
      optional(choice(
        seq('with', 'context'),
        seq('without', 'context'),
      )),
      $._statement_end,
    ),

    do_statement: $ => seq(
      $._statement_start, 'do', $._expression, $._statement_end,
    ),


    // ── Supporting rules ──────────────────────────────────────────

    _identifier_list: $ => choice(
      $.identifier,
      $.identifier_list,
    ),

    identifier_list: $ => seq(
      $.identifier,
      repeat1(seq(',', $.identifier)),
    ),

    import_list: $ => seq(
      $.import_name,
      repeat(seq(',', $.import_name)),
    ),

    import_name: $ => seq(
      $.identifier,
      optional(seq('as', field('alias', $.identifier))),
    ),

    assignment_list: $ => seq(
      $.assignment,
      repeat(seq(',', $.assignment)),
    ),

    assignment: $ => seq(
      $.identifier, '=', $._expression,
    ),

    // ── Expressions ───────────────────────────────────────────────

    _expression: $ => choice(
      $.conditional_expression,
      $.binary_expression,
      $.unary_expression,
      $.filter_expression,
      $.test_expression,
      $.member_expression,
      $.subscript_expression,
      $.call_expression,
      $.identifier,
      $.string,
      $.integer,
      $.float,
      $.boolean,
      $.none,
      $.list,
      $.tuple,
      $.dict,
      $.parenthesized_expression,
    ),

    conditional_expression: $ => choice(
      // Jinja2/Nunjucks: value if condition else alternative
      prec.right(PREC.CONDITIONAL, seq(
        field('value', $._expression),
        'if',
        field('condition', $._expression),
        'else',
        field('alternative', $._expression),
      )),
      // Twig: condition ? consequence : alternative
      prec.right(PREC.CONDITIONAL, seq(
        field('condition', $._expression),
        '?',
        field('consequence', $._expression),
        ':',
        field('alternative', $._expression),
      )),
    ),

    binary_expression: $ => choice(
      // Logical
      prec.left(PREC.OR, seq(field('left', $._expression), 'or', field('right', $._expression))),
      prec.left(PREC.AND, seq(field('left', $._expression), 'and', field('right', $._expression))),

      // Membership
      prec.left(PREC.MEMBERSHIP, seq(field('left', $._expression), 'in', field('right', $._expression))),
      prec.left(PREC.MEMBERSHIP, seq(field('left', $._expression), 'not', 'in', field('right', $._expression))),

      // Comparison
      prec.left(PREC.COMPARISON, seq(
        field('left', $._expression),
        field('operator', choice('==', '!=', '<', '>', '<=', '>=', '===', '!==')),
        field('right', $._expression),
      )),

      // Concatenation
      prec.left(PREC.CONCAT, seq(field('left', $._expression), '~', field('right', $._expression))),

      // Arithmetic
      prec.left(PREC.ADDITIVE, seq(
        field('left', $._expression),
        field('operator', choice('+', '-')),
        field('right', $._expression),
      )),
      prec.left(PREC.MULTIPLICATIVE, seq(
        field('left', $._expression),
        field('operator', choice('*', '/', '//', '%')),
        field('right', $._expression),
      )),
      prec.right(PREC.POWER, seq(field('left', $._expression), '**', field('right', $._expression))),

      // Twig null coalesce
      prec.left(PREC.OR, seq(field('left', $._expression), '??', field('right', $._expression))),

      // Twig bitwise
      prec.left(PREC.IS, seq(
        field('left', $._expression),
        field('operator', choice('b-and', 'b-or', 'b-xor')),
        field('right', $._expression),
      )),
    ),

    unary_expression: $ => prec.right(PREC.UNARY, seq(
      field('operator', choice('not', '+', '-')),
      field('operand', $._expression),
    )),

    filter_expression: $ => prec.left(PREC.FILTER, seq(
      $._expression,
      '|',
      field('name', $.identifier),
      optional($.argument_list),
    )),

    test_expression: $ => prec.left(PREC.IS, seq(
      $._expression,
      'is',
      optional('not'),
      field('name', $.identifier),
      optional($.argument_list),
    )),

    member_expression: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expression),
      choice('.', '?.'),
      field('property', $.identifier),
    )),

    subscript_expression: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expression),
      '[',
      field('index', $._expression),
      ']',
    )),

    call_expression: $ => prec.left(PREC.MEMBER, seq(
      field('function', $._expression),
      $.argument_list,
    )),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    argument_list: $ => seq(
      '(',
      optional(seq(
        $._argument,
        repeat(seq(',', $._argument)),
        optional(','),
      )),
      ')',
    ),

    _argument: $ => choice(
      $._expression,
      $.keyword_argument,
      $.spread_argument,
    ),

    keyword_argument: $ => seq(
      field('key', $.identifier),
      choice('=', ':'),
      field('value', $._expression),
    ),

    spread_argument: $ => seq('**', $._expression),

    // ── Literals ──────────────────────────────────────────────────

    identifier: _ => /[a-zA-Z_][a-zA-Z0-9_]*/,

    string: _ => token(choice(
      seq('"', repeat(choice(/[^"\\]/, /\\./)), '"'),
      seq("'", repeat(choice(/[^'\\]/, /\\./)), "'"),
    )),

    integer: _ => token(choice(
      /0[xX][0-9a-fA-F]+/,
      /0[oO][0-7]+/,
      /0[bB][01]+/,
      /[0-9]+/,
    )),

    float: _ => token(choice(
      /[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?/,
      /[0-9]+[eE][+-]?[0-9]+/,
      /\.[0-9]+([eE][+-]?[0-9]+)?/,
    )),

    boolean: _ => choice('true', 'false', 'True', 'False'),

    none: _ => choice('none', 'null', 'None'),

    list: $ => seq(
      '[',
      optional(seq(
        $._expression,
        repeat(seq(',', $._expression)),
        optional(','),
      )),
      ']',
    ),

    tuple: $ => seq(
      '(',
      $._expression,
      ',',
      optional(seq(
        $._expression,
        repeat(seq(',', $._expression)),
        optional(','),
      )),
      ')',
    ),

    dict: $ => seq(
      '{',
      optional(seq(
        $.pair,
        repeat(seq(',', $.pair)),
        optional(','),
      )),
      '}',
    ),

    pair: $ => seq(
      field('key', $._expression),
      choice(':', '='),
      field('value', $._expression),
    ),
  },
});
