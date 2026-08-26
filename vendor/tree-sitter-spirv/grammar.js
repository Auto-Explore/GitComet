/**
 * @file SPIR-V assembly (spvasm) grammar for tree-sitter
 * @author Tim Besard <tim@juliahub.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// This is a highlighting-grade grammar for the textual SPIR-V assembly syntax
// as consumed by spirv-as and produced by spirv-dis. It is deliberately loose:
// opcodes are recognized by shape (`Op[A-Z]...`) rather than enumerated, so
// new SPIR-V releases don't require grammar changes.
//
// The grammar is line-oriented: an instruction ends at the end of the line,
// as in `spirv-dis` output. Instructions hand-wrapped over multiple lines
// (which `spirv-as` accepts) will not parse; this is an accepted limitation.

export default grammar({
  name: "spirv",

  extras: $ => [
    /[ \t\r]/,
    $.comment,
  ],

  rules: {
    module: $ => seq(
      optional($.instruction),
      repeat(seq(/\n/, optional($.instruction))),
    ),

    instruction: $ => seq(
      optional(seq(field("result", $.id), "=")),
      field("opcode", $.opcode),
      repeat($._operand),
    ),

    // opcodes are recognized positionally and by shape, not enumerated
    opcode: _ => /Op[A-Z][0-9A-Za-z_]*/,

    _operand: $ => choice(
      $.id,
      $.string,
      $.float,
      $.integer,
      $.raw_literal,
      $.mask,
      $.enumerant,
    ),

    // result ids and id references
    id: _ => /%[0-9A-Za-z_]+/,

    string: _ => token(seq(
      '"',
      repeat(choice(/[^"\\]/, /\\./)),
      '"',
    )),

    // hexfloats (0x1.8p+1) as well as decimal floats; spirv-as accepts both
    float: _ => token(choice(
      /-?[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?/,
      /-?0[xX][0-9a-fA-F]+(\.[0-9a-fA-F]*)?[pP][+-]?[0-9]+/,
    )),

    integer: _ => token(choice(
      /-?0[xX][0-9a-fA-F]+/,
      /-?[0-9]+/,
    )),

    // untyped raw words: !<integer>
    raw_literal: _ => token(choice(
      /!0[xX][0-9a-fA-F]+/,
      /![0-9]+/,
    )),

    // bit-mask operands: ReadOnly|Volatile
    mask: $ => seq(
      $.enumerant,
      repeat1(seq(token.immediate("|"), $.enumerant)),
    ),

    // named enumerants (storage classes, decorations, extended-instruction
    // names, ...); enumerated in queries, not in the grammar
    enumerant: _ => /[A-Za-z_][0-9A-Za-z_]*/,

    comment: _ => /;[^\n]*/,
  },
});
