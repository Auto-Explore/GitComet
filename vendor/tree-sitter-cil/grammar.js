/**
 * @file CIL (MSIL) grammar for tree-sitter
 * @license MIT
 *
 * Written for GitComet: nothing anywhere provides a tree-sitter grammar for
 * .NET's intermediate language.
 *
 * Deliberately a lexer rather than a parser. CIL's real grammar is large -- every
 * directive takes its own argument shapes, and a method reference carries a full
 * signature -- and none of that structure changes what a highlighter draws. What
 * does change it is telling the six kinds of token apart, which is all this does:
 * a flat stream of directives, labels, opcodes, literals and punctuation. The one
 * thing it must get right is that `::` binds tighter than `:`, or every
 * `System.Object::.ctor` would read as a label.
 */

/* eslint-disable no-undef */
module.exports = grammar({
  name: 'cil',

  extras: ($) => [/\s/, $.line_comment, $.block_comment],

  conflicts: ($) => [[$.label, $._item]],

  rules: {
    source_file: ($) => repeat($._item),

    _item: ($) =>
      choice(
        $.directive,
        $.label,
        $.quoted_identifier,
        $.string,
        $.number,
        $.identifier,
        $.operator,
        '(',
        ')',
        '{',
        '}',
        '[',
        ']',
        '<',
        '>',
        ',',
        '::',
        ':',
        '=',
      ),

    // `.assembly`, `.class`, `.method`, `.maxstack`, `.entrypoint`. In CIL these
    // outnumber the instructions.
    directive: () => token(seq('.', /[A-Za-z_][A-Za-z0-9_]*/)),

    // `IL_0000:`. The `::` token below out-lexes the `:` here, which is what
    // stops `System.Object::.ctor` being read as a label named `System.Object`.
    //
    // `prec.dynamic` rather than a plain `prec`: an identifier and a label start
    // identically and the choice is only settled by whether a `:` follows, which
    // is a GLR decision, not a lexing one.
    label: ($) => prec.dynamic(1, seq(field('name', $.identifier), ':')),

    // Opcodes carry dots -- `ldc.i4.0`, `br.s`, `callvirt` -- and so do type
    // names, so one rule serves both and the query tells them apart by shape.
    identifier: () => /[A-Za-z_@$][A-Za-z0-9_@$.`]*/,

    // `'a field with spaces'`, `'class'`. Any characters at all, so a reserved
    // word can be a member name.
    quoted_identifier: () => token(seq("'", /[^'\n]*/, "'")),

    string: () => token(seq('"', repeat(choice(/[^"\\\n]/, /\\./)), '"')),

    number: () => token(/-?(0[xX][0-9a-fA-F]+|\d+\.\d+([eE][-+]?\d+)?|\d+)/),

    operator: () => choice('!', '&', '*', '+', '-', '/', '?'),

    line_comment: () => token(seq('//', /[^\n]*/)),
    block_comment: () => token(seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/')),
  },
});
