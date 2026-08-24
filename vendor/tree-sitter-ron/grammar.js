/**
 * @file RON grammar for tree-sitter
 * @author Marius Cobzarenco <marius@reinfer.io>
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @license MIT
 * @see {@link https://github.com/ron-rs/ron/blob/master/docs/grammar.md|specification}
 */

/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

module.exports = grammar({
  name: 'ron',

  extras: $ => [
    /\s/,
    $.line_comment,
    $.block_comment,
  ],

  externals: $ => [
    $._string_content,
    $.raw_string,
    $.float,
    $.block_comment,
  ],

  rules: {
    source_file: $ => seq(
      $._value,
    ),

    _value: $ => choice(
      $.array,
      $.map,
      $.struct,
      $.tuple,

      $._literal,

      $.enum_variant,
    ),

    enum_variant: $ => $.identifier,

    array: $ => seq(
      '[',
      sepBy(',', $._value),
      optional(','),
      ']',
    ),

    map: $ => seq(
      '{',
      sepBy(',', $.map_entry),
      optional(','),
      '}',
    ),

    struct: $ => choice(
      $.unit_struct,
      $._tuple_struct,
      $._named_struct,
    ),

    unit_struct: _ => choice(
      '()',
    ),

    struct_name: $ => $.identifier,

    _tuple_struct: $ => seq(
      $.struct_name,
      $.tuple,
    ),

    _named_struct: $ => seq(
      optional($.struct_name),
      field('body', $._struct_body),
    ),

    _struct_body: $ => seq(
      '(',
      sepBy(',', $.struct_entry),
      optional(','),
      ')',
    ),

    tuple: $ => seq(
      '(',
      sepBy1(',', $._value),
      optional(','),
      ')',
    ),

    map_entry: $ => seq(
      $._value,
      ':',
      $._value,
    ),

    struct_entry: $ => seq(
      $.identifier,
      ':',
      $._value,
    ),

    _literal: $ => choice(
      $.string,
      $.char,
      $.boolean,
      $.integer,
      $.float,
      $.negative,
    ),

    integer: _ => token(seq(
      choice(
        /[0-9][0-9_]*/,
        /0x[0-9a-fA-F_]+/,
        /0b[01_]+/,
        /0o[0-7_]+/,
      ),
    )),

    negative: $ => seq('-', choice($.integer, $.float)),

    string: $ => seq(
      alias(/b?"/, '"'),
      repeat(choice(
        $._escape_sequence,
        $._string_content,
      )),
      token.immediate('"'),
    ),

    char: $ => seq(
      optional('b'),
      '\'',
      optional(choice(
        $._escape_sequence,
        /[^\\']/,
      )),
      '\'',
    ),

    _escape_sequence: $ =>
      choice(
        prec(2, token.immediate(seq('\\', /[^abfnrtvxu'\"\\\?]/))),
        prec(1, $.escape_sequence),
      ),
    escape_sequence: _ => token.immediate(
      seq('\\',
        choice(
          /[^xu]/,
          /[0-7]{1,3}/,
          /u[0-9a-fA-F]{4}/,
          // GitComet: braces escaped. Upstream wrote /u{[0-9a-fA-F]+}/, which the
          // modern tree-sitter CLI reads as a repetition quantifier and rejects,
          // so the grammar as published could not be regenerated at all.
          /u\{[0-9a-fA-F]+\}/,
          /x[0-9a-fA-F]{2}/,
        ),
      )),

    boolean: _ => choice('true', 'false'),

    identifier: _ => /(r#)?[_\p{XID_Start}][_\p{XID_Continue}]*/,

    comment: $ => choice(
      $.line_comment,
      $.block_comment,
    ),

    line_comment: _ => token(seq(
      '//', /.*/,
    )),
  },
});

/**
 * Creates a rule to match one or more of the rules separated by the separator.
 *
 * @param {string|RegExp} sep - The separator to use.
 * @param {Rule|RegExp} rule
 *
 * @return {SeqRule}
 *
 */
function sepBy1(sep, rule) {
  return seq(rule, repeat(seq(sep, rule)));
}

/**
 * Creates a rule to optionally match one or more of the rules separated by the separator.
 *
 * @param {string|RegExp} sep - The separator to use.
 * @param {Rule|RegExp} rule
 *
 * @return {ChoiceRule}
 *
 */
function sepBy(sep, rule) {
  return optional(sepBy1(sep, rule));
}
