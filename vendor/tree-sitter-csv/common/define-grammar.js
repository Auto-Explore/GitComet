/**
 * @file CSV, PSV, & TSV grammar for tree-sitter
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @license MIT
 */

/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/**
 *
 * @param {string} dialect
 *
 * @param {string} separator
 *
 * @return {GrammarSchema<any>}
 *
 */
module.exports = function defineGrammar(dialect, separator) {
  return grammar({
    name: dialect,

    rules: {
      document: $ => seq(
        repeat(seq($.row, /\r|\r\n|\n/)),
        optional($.row),
      ),

      row: $ => seq($.field, repeat(seq(separator, $.field))),
      field: $ => choice($.text, $.number, $.float, $.boolean),

      number: _ => choice(/\d+/, /0[xX][0-9a-fA-F]+/),
      float: _ => choice(/\d*\.\d+/, /\d+\.\d*/),
      boolean: _ => choice('true', 'false'),
      text: _ => token(choice(
        new RegExp(`[^${separator}\\r\\n]*`),
        seq('"', repeat(choice(/[^"]/, '""')), '"'),
      )),
    },
  });
};
