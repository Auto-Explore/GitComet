/**
 * @file Cue grammar for tree-sitter
 * @author Jean-Philippe Braun <eon@patapon.info>
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @license MIT
 * @see {@link https://cuelang.org|official website}
 * @see {@link https://github.com/cue-lang/cue/blob/master/doc/ref/spec.md|official syntax spec}
 */

// deno-lint-ignore-file ban-ts-comment
/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/**
 * Creates a rule to match one or more of the rules separated by the separator
 * and optionally adds a trailing separator (default is false).
 *
 * @param {Rule} rule
 * @param {string} separator - The separator to use.
 * @param {boolean?} trailing_separator - Whether to allow a trailing separator or not.
 *
 * @return {SeqRule}
 *
 */
function listSeq(rule, separator, trailing_separator = false) {
  return trailing_separator ?
    seq(rule, repeat(seq(separator, rule)), optional(separator)) :
    seq(rule, repeat(seq(separator, rule)));
}

/**
 * Creates a rule to match one or more of the rules separated by a comma
 *
 * @param {Rule} rule
 * @param {boolean?} trailing_separator - Whether to allow a trailing separator or not.
 *
 * @return {SeqRule}
 */
function commaSep1(rule, trailing_separator = false) {
  return listSeq(rule, ',', trailing_separator);
}

/**
 * Creates a rule to optionally match one or more of the rules separated by a comma
 *
 * @param {Rule} rule
 * @param {boolean?} trailing_separator - Whether to allow a trailing separator or not.
 *
 * @return {ChoiceRule}
 *
 */
function commaSep(rule, trailing_separator = false) {
  return optional(commaSep1(rule, trailing_separator));
}

/**
 * Creates a rule to match one or more of the rules optionally separated by a comma
 *
 * @param {Rule} rule
 *
 * @return {SeqRule}
 *
 */
function optionalCommaSep1(rule) {
  return seq(rule, repeat(seq(optional(','), rule)), optional(','));
}

/**
 * Creates a rule to optionally match one or more of the rules optionally separated by a comma
 *
 * @param {Rule} rule
 *
 * @return {ChoiceRule}
 *
 */
function optionalCommaSep(rule) {
  return optional(optionalCommaSep1(rule));
}

const PREC = {
  bitwise_or: 1,
  bitwise_and: 2,
  or: 3,
  and: 4,
  compare: 5,
  add: 6,
  multiply: 7,
  unary: 8,
  call: 9,
};

const primitive_types = [
  'number',
  'float',
  'float32',
  'float64',
  'uint',
  'uint8',
  'uint16',
  'uint32',
  'uint64',
  'uint128',
  'int',
  'int8',
  'int16',
  'int32',
  'int64',
  'int128',
  'string',
  'bytes',
  'bool',
  'rune',
];

module.exports = grammar({
  name: 'cue',

  extras: $ => [
    $.comment,
    /\s/,
  ],

  externals: $ => [
    $._multi_str_content,
    $._multi_bytes_content,
    $._raw_str_content,
    $._raw_bytes_content,
    $._multi_raw_str_content,
    $._multi_raw_bytes_content,
  ],

  inline: $ => [
    $.keyword_identifier,
  ],

  supertypes: $ => [
    $.expression,
    $.primary_expression,
    $.literal,
  ],

  conflicts: $ => [
    [$._embedding, $._label_alias_expr],
    [$._label_name, $.primary_expression],
    [$._label_name, $.string],
  ],

  word: $ => $.identifier,

  rules: {
    source_file: $ => prec(1, seq(
      optionalCommaSep($.attribute),
      optional($.package_clause),
      optionalCommaSep($.import_declaration),
      optionalCommaSep($._declaration),
    )),

    _package_identifier: $ => alias($.identifier, $.package_identifier),

    package_clause: $ => seq(
      'package',
      $._package_identifier,
    ),

    import_declaration: $ => seq(
      'import',
      choice(
        $.import_spec,
        $.import_spec_list,
      ),
    ),

    import_spec: $ => seq(
      optional(field('name', choice(
        '.',
        '_',
        $._package_identifier,
      ))),
      field('path', $.string),
    ),

    import_spec_list: $ => seq(
      '(',
      optionalCommaSep($.import_spec),
      ')',
    ),

    identifier: _ => token(choice(
      seq(
        optional(choice('_#', '#', '_')),
        choice(/\p{L}/, '$', '_'),
        repeat(choice(/\p{L}/, '$', '_', /[0-9]/)),
      ),
      choice('_#', '#', '_'),
    )),
    keyword_identifier: $ => prec(-3, alias(
      choice(
        'let',
        'if',
        'for',
        'else',
        'try',
        ...primitive_types,
      ),
      $.identifier,
    )),

    attribute: $ => seq(
      '@',
      field('name', $.identifier),
      '(',
      optional(choice(',', '-')),
      repeat(
        prec.right(choice(
          commaSep1($._attr_token),
          seq('(', commaSep($._attr_token), ')'),
          seq('[', commaSep($._attr_token), ']'),
          seq('{', commaSep($._attr_token), '}'),
          seq('<', commaSep($._attr_token), '>'),
        )),
      ),
      ')',
    ),
    _attr_token: $ => prec.right(choice(
      $._attr_item,
      seq($._attr_item, '=', choice($._attr_item, $.attr_path)),
      seq('(', $._attr_item, ')', '=', choice($._attr_item, $.attr_path)),
    )),

    _attr_item: $ => choice(
      $.string,
      $.number,
      $.float,
      $.si_unit,
      $.boolean,
      $.null,
      $.top,
      $.bottom,
      $.primitive_type,
      seq('[', ']', alias($._attr_item, $.slice_type)),
      seq('*', alias($._attr_item, $.pointer_type)),
      $.package_path,
      $.identifier,
      prec(-1, /[^0-9_]/),
    ),
    attr_path: _ => /(?:\/[\p{L}\d\.]+)*[\p{L}\d\.]+(?:\/[\p{L}\d\.]+)*(?:\/)?/,
    package_path: $ => seq($.identifier, repeat1(seq('.', $.identifier))),

    builtin_function: _ => choice(
      'len',
      'close',
      'and',
      'or',
      'div',
      'mod',
      'quo',
      'rem',
      'matchN',
      'matchIf',
      'error',
    ),

    _declaration: $ => prec(-1, choice(
      $.attribute,
      $.field,
      $.ellipsis,
      $._embedding,
      $.let_clause,
    )),

    _list_elem: $ => prec.right(choice(
      $.ellipsis,
      seq(
        $._embedding,
        optional(repeat(seq(',', $._embedding))),
        optional(seq(',', $.ellipsis)),
        optional(','),
      ),
    )),

    list_lit: $ => seq('[', repeat($._list_elem), ']'),

    struct_lit: $ => seq(
      '{',
      optionalCommaSep($._declaration),
      '}',
    ),

    ellipsis: $ => prec.left(seq(
      '...',
      optional($.expression),
    )),

    _embedding: $ => choice(
      $.comprehension,
      $._alias_expr,
    ),

    _label_name: $ => choice(
      prec.dynamic(1, $.identifier),
      prec(1, $.keyword_identifier),
      prec(1, alias($.primitive_type, $.identifier)),
      prec(1, alias($.builtin_function, $.identifier)),
      prec.dynamic(1, alias($._simple_string_lit, $.string)),
      prec.dynamic(1, $.selector_expression),
      prec.dynamic(1, alias($.parenthesized_expression, $.dynamic)),
    ),

    _label_alias_expr: $ => alias($._alias_expr, $.optional),

    required: $ => seq($._label_name, '!'),

    optional: $ => seq($._label_name, '?'),

    _label_expr: $ => choice(
      $._label_name,
      $.optional,
      $.required,
      seq('[', $._label_alias_expr, ']'),
    ),

    label: $ => seq(
      optional(seq(
        field('alias', choice($.identifier, $.keyword_identifier)),
        '=',
      )),
      $._label_expr,
    ),

    postfix_alias: $ => seq(
      '~',
      choice(
        field('field', $.identifier),
        seq('(', field('label', $.identifier), ',', field('field', $.identifier), ')'),
      ),
    ),

    field: $ => prec.right(seq(
      repeat1(seq($.label, optional($.postfix_alias), ':')),
      $._value,
      optional($.attribute),
    )),

    _value: $ => alias($._alias_expr, $.value),

    for_clause: $ => seq(
      'for',
      choice($.identifier, '_'),
      optional(seq(',', choice($.identifier, '_'))),
      'in', $.expression,
    ),

    guard_clause: $ => seq('if', field('condition', $.expression)),

    let_clause: $ => seq('let', field('left', $.identifier), '=', field('right', $.expression)),

    _clause: $ => choice($.for_clause, $.guard_clause, $.let_clause, $.try_clause),

    try_clause: $ => seq(
      'try',
      optional(seq(field('left', $.identifier), '=', field('right', $.expression))),
    ),

    comprehension: $ => prec.right(seq(
      choice($.for_clause, $.guard_clause, $.try_clause),
      repeat(seq(optional(','), $._clause)),
      $.struct_lit,
      optional($.else_clause),
    )),

    else_clause: $ => seq('else', $.struct_lit),

    _alias_expr: $ => seq(
      optional(seq(field('alias', $.identifier), '=')),
      $.expression,
    ),

    expression: $ => prec.left(choice(
      $.primary_expression,
      $.unary_expression,
      $.binary_expression,
    )),

    primary_expression: $ => choice(
      $.parenthesized_expression,
      $.selector_expression,
      $.index_expression,
      $.call_expression,
      $.postfix_expression,
      $.identifier,
      $.literal,
    ),

    postfix_expression: $ => prec(PREC.call, seq(
      $.primary_expression,
      '?',
    )),

    binary_expression: $ => {
      const table = [
        ['+', PREC.add],
        ['-', PREC.add],
        ['*', PREC.multiply],
        ['/', PREC.multiply],
        ['|', PREC.bitwise_or],
        ['&', PREC.bitwise_and],
        ['||', PREC.or],
        ['&&', PREC.and],
        [choice('==', '=~', '!~', '!=', '<', '<=', '>', '>='), PREC.compare],
      ];

      // @ts-ignore
      return choice(...table.map(([operator, precedence]) => prec.left(precedence, seq(
        field('left', $.expression),
        // @ts-ignore
        field('operator', operator),
        field('right', $.expression),
      ))));
    },

    unary_expression: $ => {
      const unary_operators = ['+', '-', '!', '*', '!=', '<', '<=', '>', '>=', '=~', '!~'];

      return choice(...unary_operators.map((operator) => prec(PREC.unary, seq(
        field('operator', operator),
        field('argument', $.expression),
      ))));
    },

    parenthesized_expression: $ => seq('(', $.expression, ')'),

    call_expression: $ => prec(PREC.call, seq(
      field('function', choice(
        $.builtin_function,
        $.selector_expression,
        $.identifier,
      )),
      $.arguments,
    )),

    index_expression: $ => seq(
      $.primary_expression,
      token.immediate('['),
      field('index', $.expression),
      ']',
    ),

    selector_expression: $ => seq(
      $.primary_expression,
      '.',
      choice($.identifier, alias($._simple_string_lit, $.string)),
    ),

    arguments: $ => seq(
      '(',
      commaSep(optional($.expression)),
      ')',
    ),

    literal: $ =>
      choice(
        $.struct_lit,
        $.list_lit,
        $.string,
        $.number,
        $.float,
        $.si_unit,
        $.boolean,
        $.null,
        $.top,
        $.bottom,
        $.primitive_type,
      ),

    primitive_type: _ => choice(...primitive_types),

    top: _ => '_',

    bottom: _ => '_|_',

    boolean: _ => choice('true', 'false'),

    null: _ => 'null',

    number: _ => {
      const binary_literal = seq(
        /0[bB]/,
        /[01](_?[01])*/,
      );

      const hex_literal = seq(
        /0[xX]/,
        /[\da-fA-F](_?[\da-fA-F])*/,
      );

      const octal_literal = seq(
        optional(choice('-', '+')),
        /0[oO]/,
        /[0-7](_?[0-7])*/,
      );

      const decimal_digits = /\d(_?\d)*/;
      const signed_integer = seq(optional(choice('-', '+')), decimal_digits);

      const decimal_integer_literal = choice(
        '0',
        seq(optional('0'), /[1-9]/, optional(seq(optional('_'), decimal_digits))),
      );

      const decimal_literal = choice(
        seq(optional(choice('-', '+')), decimal_integer_literal),
        decimal_digits,
        signed_integer,
      );


      return token(choice(
        hex_literal,
        binary_literal,
        decimal_literal,
        octal_literal,
      ));
    },

    float: _ => {
      const hex_digit = /[0-9a-fA-F]/;
      const hex_digits = seq(hex_digit, repeat(seq(optional('_'), hex_digit)));

      const decimal_digits = /\d(_?\d)*/;

      const decimal_exponent = seq(choice('e', 'E'), optional(choice('+', '-')), decimal_digits);

      const decimal_float_literal = choice(
        seq(optional('-'), optional(decimal_digits), '.', optional(decimal_digits), optional(decimal_exponent)),
        seq(decimal_digits, decimal_exponent),
      );

      const hex_exponent = seq(choice('p', 'P'), optional(choice('+', '-')), decimal_digits);
      const hex_mantissa = choice(
        seq(optional('_'), hex_digits, '.', optional(hex_digits)),
        seq(optional('_'), hex_digits),
        seq('.', hex_digits),
      );
      const hex_float_literal = seq('0', choice('x', 'X'), hex_mantissa, hex_exponent);

      return token(choice(decimal_float_literal, hex_float_literal));
    },

    si_unit: $ => seq(
      $.float,
      field('unit',
        token.immediate(seq(
          choice('K', 'M', 'G', 'T', 'P'),
          optional('i'),
        )),
      ),
    ),

    escape_char: _ => token.immediate(seq(
      '\\',
      repeat('#'),
      seq(choice('a', 'b', 'f', 'n', 'r', 't', 'v', '/', '\\', '\'', '"')),
    )),

    escape_byte: _ => token.immediate(seq(
      '\\',
      repeat('#'),
      choice(
        /[0-7]{3}/, // octal byte value
        /x[0-9a-fA-F]{2}/, // hex byte value
      ),
    )),

    _escape_unicode: $ => choice(
      $.escape_char,
      alias(token.immediate(seq(
        '\\',
        repeat('#'),
        choice(
          /u[0-9a-fA-F]{4}/,
          /U[0-9a-fA-F]{8}/,
        ))),
      $.escape_unicode,
      ),
    ),

    string: $ => choice(
      $._simple_string_lit,
      $._simple_bytes_lit,
      $._multiline_string_lit,
      $._multiline_bytes_lit,
      $._simple_raw_string_lit,
      $._simple_raw_bytes_lit,
      $._multiline_raw_string_lit,
      $._multiline_raw_bytes_lit,
    ),

    _simple_string_lit: $ => seq(
      '"',
      repeat(choice(
        token.immediate(prec(1, /[^"\n\\]+/)),
        $.interpolation,
        $._escape_unicode,
      )),
      '"',
    ),

    _simple_bytes_lit: $ => seq(
      '\'',
      repeat(choice(
        token.immediate(prec(1, /[^'\n\\]+/)),
        $.interpolation,
        $.escape_byte,
        $._escape_unicode,
      )),
      '\'',
    ),

    _multiline_string_lit: $ => seq(
      token('"""'),
      repeat(choice(
        $._multi_str_content,
        $.interpolation,
        $._escape_unicode,
      )),
      token('"""'),
    ),

    _multiline_bytes_lit: $ => seq(
      token('\'\'\''),
      repeat(choice(
        $._multi_bytes_content,
        $.interpolation,
        $.escape_byte,
        $._escape_unicode,
      )),
      token('\'\'\''),
    ),

    _simple_raw_string_lit: $ => seq(
      '#"',
      repeat(choice(
        $._raw_str_content,
        $.raw_interpolation,
        $._escape_unicode,
      )),
      '"#',
    ),

    _simple_raw_bytes_lit: $ => seq(
      '#\'',
      repeat(choice(
        $._raw_bytes_content,
        $.raw_interpolation,
        $.escape_byte,
        $._escape_unicode,
      )),
      '\'#',
    ),

    _multiline_raw_string_lit: $ => seq(
      token('#"""'),
      repeat(choice(
        $._multi_raw_str_content,
        $.raw_interpolation,
        $._escape_unicode,
      )),
      token('"""#'),
    ),

    _multiline_raw_bytes_lit: $ => seq(
      token('#\'\'\''),
      repeat(choice(
        $._multi_raw_bytes_content,
        $.raw_interpolation,
        $.escape_byte,
        $._escape_unicode,
      )),
      token('\'\'\'#'),
    ),

    interpolation: $ => seq('\\(', $.expression, ')'),

    raw_interpolation: $ => seq('\\#(', $.expression, ')'),

    comment: _ => token(seq('//', /.*/)),
  },
});

module.exports.PREC = PREC;
