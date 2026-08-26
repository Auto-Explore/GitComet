/**
 * @file FSharp signature grammar for tree-sitter
 * @author Nikolaj Sidorenco
 * @license MIT
 * @see {@link https://fsharp.org/specs/language-spec/4.1/FSharpSpec-4.1-latest.pdf f# grammar}
 */

/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

module.exports = grammar(require("../fsharp/grammar"), {
  name: "fsharp_signature",

  conflicts: ($) => [
    [$.long_identifier, $._identifier_or_op],
    [$._type, $._argument_type],
    [$._type, $._curried_return_type],
    [$.union_type_cases],
  ],

  supertypes: ($) => [$._module_signature_elements],

  rules: {
    file: ($, _original) =>
      choice(
        $.named_module,
        repeat1($.namespace),
        repeat($._module_signature_elements),
      ),

    named_module: ($, _original) =>
      seq(
        optional($.attributes),
        "module",
        optional($.access_modifier),
        optional("rec"),
        field("name", $.long_identifier),
        repeat($._module_signature_elements),
      ),

    namespace: ($, _original) =>
      seq(
        "namespace",
        choice(
          "global",
          field("name", seq(optional("rec"), $.long_identifier)),
        ),
        repeat($._module_signature_elements),
      ),

    module_defn: ($, _original) =>
      prec.left(
        seq(
          optional($.attributes),
          "module",
          optional($.access_modifier),
          optional("rec"),
          $.identifier,
          "=",
          scoped(repeat1($._module_signature_elements), $._indent, $._dedent),
        ),
      ),

    _module_signature_elements: ($, _original) =>
      choice(
        $.value_definition,
        $.type_definition,
        $.exception_definition,
        $.import_decl,
        $.module_defn,
        $.module_abbrev,
        $.compiler_directive_decl,
        $.preproc_if,
      ),

    value_definition: ($, _original) =>
      prec.left(
        4,
        seq(
          optional($.attributes),
          "val",
          $.value_declaration_left,
          ":",
          $.curried_spec,
          optional(seq("=", field("body", $._expression_block))),
        ),
      ),

    _expression_block: ($, _original) =>
      choice(seq($._indent, $._expression, $._dedent), $._expression),

    _expression: ($, _original) =>
      choice("null", $.const, $.long_identifier_or_op, $.paren_expression),

    paren_expression: ($, _original) =>
      prec(1, seq("(", choice($.tuple_expression, $._expression), ")")),

    tuple_expression: ($, _original) =>
      prec.left(seq($._expression, ",", $._expression, repeat(seq(",", $._expression)))),
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
