/**
 * @file Razor grammar for tree-sitter
 * @author Tristan Knight <admin@snappeh.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

import CSHARP from '../tree-sitter-c-sharp/grammar.js';

/**
 * Zero or more comma-separated items.
 * @param {RuleOrLiteral} rule
 */
function commaSep(rule) {
  return optional(commaSep1(rule));
}

/**
 * One or more comma-separated items.
 * @param {RuleOrLiteral} rule
 */
function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)));
}

export default grammar(CSHARP, {
  name: 'razor',

  externals: ($, original) =>
    original.concat([$._implicit_dot, $._implicit_paren, $._implicit_bracket, $._implicit_question, $._script_text, $._style_text, $._comment_text, $._line_text, $._line_end, $._void_tag_name, $._script_tag_name, $._style_tag_name, $._razor_if_start, $._razor_for_start, $._razor_foreach_start, $._razor_while_start, $._razor_do_start, $._razor_switch_start, $._razor_try_start, $._razor_using_start, $._razor_lock_start, $._razor_code_start, $.html_text, $._razor_else_continuation, $._razor_catch_continuation, $._razor_finally_continuation, $._razor_while_continuation]),

  extras: ($) => [$.razor_comment, $.comment, /[\s\uFEFF]+/],

  conflicts: ($, o) => [
    [$.extension_body],
    [
      $.preproc_if,
      $.preproc_if_in_top_level,
      $.preproc_if_in_expression,
      $.preproc_if_in_attribute_list,
    ],
    [$.preproc_if, $.preproc_if_in_top_level],
    [$.preproc_if, $.preproc_if_in_top_level, $.preproc_if_in_expression],
    [$.preproc_if, $.preproc_if_in_top_level, $.preproc_if_in_attribute_list],
    [
      $.preproc_else,
      $.preproc_else_in_top_level,
      $.preproc_else_in_expression,
      $.preproc_else_in_attribute_list,
    ],
    [$.declaration, $.preproc_if_in_top_level],
    [$.type_declaration, $.declaration],
    [$.method_declaration, $._local_function_declaration],
    [$.declaration, $.preproc_else_in_top_level],
    [
      $.preproc_elif,
      $.preproc_elif_in_top_level,
      $.preproc_elif_in_expression,
      $.preproc_elif_in_attribute_list,
    ],

    [$.destructor_declaration, $._simple_name],

    [$.field_declaration, $.local_declaration_statement],
    ...o,
  ],

  rules: {
    compilation_unit: ($) => repeat(choice(seq($._directive, $._line_end), $._node, $.html_text)),

    _directive: ($) => choice(
      $.shebang_directive,
      $.razor_page_directive, $.razor_using_directive, $.razor_model_directive,
      $.razor_rendermode_directive, $.razor_inject_directive, $.razor_implements_directive,
      $.razor_layout_directive, $.razor_inherits_directive, $.razor_attribute_directive,
      $.razor_typeparam_directive, $.razor_namespace_directive, $.razor_preservewhitespace_directive,
      $.razor_addtaghelper_directive, $.razor_removetaghelper_directive, $.razor_taghelperprefix_directive,
    ),

    _csharp_nodes: ($) =>
      choice(
        $.statement,
        $._node,

        $.preproc_region,
        $.preproc_endregion,
        $.preproc_line,
        $.preproc_pragma,
        $.preproc_nullable,
        $.preproc_error,
        $.preproc_warning,
        $.preproc_define,
        $.preproc_undef,
      ),

    block: ($) => seq('{', repeat($._csharp_nodes), '}'),

    _node: ($) =>
      prec.right(
        choice(
          $.razor_comment,
          $.razor_escape,
          $.razor_if,
          $.razor_switch,
          $.razor_for,
          $.razor_foreach,
          $.razor_while,
          $.razor_do_while,
          $.razor_try,
          $.explicit_line_transition,
          $.razor_implicit_expression,
          $.razor_explicit_expression,
          $.razor_section,
          $.razor_helper,
          $.razor_block,
          $.razor_compound_using,
          $.razor_lock,
          $.element,
          $.html_comment,
          $.html_doctype,
        ),
      ),

    _razor_marker: (_) => token('@'),

    razor_escape: ($) => seq(alias('@', 'at_at_escape'), $.escaped_at),
    escaped_at: (_) => token.immediate('@'),

    razor_page_directive: ($) =>
      seq(alias(seq($._razor_marker, 'page'), 'at_page'), optional($.string_literal)),
    razor_using_directive: ($) =>
      seq(
        alias(seq($._razor_marker, 'using'), 'at_using'),
        choice(
          seq(optional('unsafe'), field('name', $.identifier), '=', $.type),
          seq(optional('static'), optional('unsafe'), $._name),
        ),
      ),
    razor_model_directive: ($) =>
      seq(alias(seq($._razor_marker, 'model'), 'at_model'), field('name', $._name)),
    razor_preservewhitespace_directive: ($) =>
      seq(
        alias(seq($._razor_marker, 'preservewhitespace'), 'at_preservewhitespace'),
        $.boolean_literal,
      ),
    razor_attribute_directive: ($) =>
      seq(alias(seq($._razor_marker, 'attribute'), 'at_attribute'), $.attribute_list),
    razor_implements_directive: ($) =>
      seq(alias(seq($._razor_marker, 'implements'), 'at_implements'), field('name', $._name)),
    razor_layout_directive: ($) =>
      seq(alias(seq($._razor_marker, 'layout'), 'at_layout'), field('name', $._name)),
    razor_inherits_directive: ($) =>
      seq(alias(seq($._razor_marker, 'inherits'), 'at_inherits'), field('name', $._name)),
    razor_typeparam_directive: ($) =>
      seq(alias(seq($._razor_marker, 'typeparam'), 'at_typeparam'), field('name', $.identifier), repeat($.type_parameter_constraints_clause)),
    razor_inject_directive: ($) =>
      seq(alias(seq($._razor_marker, 'inject'), 'at_inject'), $.variable_declaration),
    razor_namespace_directive: ($) =>
      seq(alias(seq($._razor_marker, 'namespace'), 'at_namespace'), $._name),
    razor_rendermode_directive: ($) =>
      seq(alias(seq($._razor_marker, 'rendermode'), 'at_rendermode'), $.razor_rendermode),
    razor_rendermode: ($) => choice($.expression, $.razor_explicit_expression),

    _taghelper_target: ($) =>
      seq(choice($._name, alias('*', $.taghelper_wildcard)), ',', field('assembly', $._name)),
    razor_addtaghelper_directive: ($) =>
      seq(
        alias(seq($._razor_marker, 'addTagHelper'), 'at_addtaghelper'),
        choice($.string_literal, $._taghelper_target),
      ),
    razor_removetaghelper_directive: ($) =>
      seq(
        alias(seq($._razor_marker, 'removeTagHelper'), 'at_removetaghelper'),
        choice($.string_literal, $._taghelper_target),
      ),
    razor_taghelperprefix_directive: ($) =>
      seq(
        alias(seq($._razor_marker, 'tagHelperPrefix'), 'at_taghelperprefix'),
        choice($.string_literal, token(/[A-Za-z_][A-Za-z0-9_:.-]*/)),
      ),

    razor_block: ($) =>
      prec.left(
        seq(
          alias(choice('@', $._razor_code_start), 'at_block'),
          '{',
          repeat(choice($.declaration, $._csharp_nodes)),
          '}',
        ),
      ),

    razor_explicit_expression: ($) =>
      prec.right(
        seq(
          alias($._razor_marker, 'at_explicit'),
          prec.right(
            choice(
              $.parenthesized_expression,
              alias($._parenthesized_lvalue_expression, $.parenthesized_expression),
            ),
          ),
        ),
      ),

    razor_implicit_expression: ($) => seq(alias($._razor_marker, 'at_implicit'), $._implicit_expression),
    _implicit_expression: ($) => choice(
      $._implicit_chain, $.boolean_literal, $.null_literal,
      alias($._implicit_await, $.await_expression),
    ),
    _implicit_await: ($) => seq('await', $._implicit_chain),
    _implicit_chain: ($) => choice(
      alias($._implicit_identifier_start, $.identifier), 'this',
      $.typeof_expression,
      alias($._implicit_member, $.member_access_expression),
      alias($._implicit_invocation, $.invocation_expression),
      alias($._implicit_index, $.element_access_expression),
      alias($._implicit_conditional, $.conditional_access_expression),
    ),
    _implicit_identifier_start: (_) => token(/(\p{XID_Start}|_|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8})(\p{XID_Continue}|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8})*/),
    _implicit_identifier: (_) => token.immediate(/(\p{XID_Start}|_|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8})(\p{XID_Continue}|\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8})*/),
    _implicit_member: ($) => seq(field('expression', $._implicit_chain), alias($._implicit_dot, '.'), field('name', alias($._implicit_identifier, $.identifier))),
    _implicit_argument_list: ($) => seq(alias($._implicit_paren, '('), commaSep($.argument), ')'),
    _implicit_invocation: ($) => seq(field('function', $._implicit_chain), field('arguments', alias($._implicit_argument_list, $.argument_list))),
    _implicit_subscript: ($) => seq(alias($._implicit_bracket, '['), commaSep1($.argument), ']'),
    _implicit_index: ($) => seq(field('expression', $._implicit_chain), field('subscript', alias($._implicit_subscript, $.bracketed_argument_list))),
    _implicit_binding: ($) => seq(alias($._implicit_dot, '.'), field('name', alias($._implicit_identifier, $.identifier))),
    _implicit_conditional: ($) => seq(field('condition', $._implicit_chain), alias($._implicit_question, '?'), choice(alias($._implicit_binding, $.member_binding_expression), alias($._implicit_subscript, $.element_binding_expression))),

    razor_helper: ($) => seq(alias(seq('@', 'helper'), 'at_helper'), field('name', $.identifier), $.parameter_list, $.block),
    razor_template: ($) => seq(alias('@', 'at_template'), $.element),
    non_lvalue_expression: ($, original) => choice(original, $.razor_template),

    razor_lock: ($) =>
      seq(
        alias($._razor_lock_start, 'at_lock'),
        '(',
        $.expression,
        ')',
        '{',
        optional($._blended_content),
        '}',
      ),

    razor_compound_using: ($) =>
      seq(
        alias($._razor_using_start, 'at_using'),
        '(',
        choice(alias($.using_variable_declaration, $.variable_declaration), $.expression),
        ')',
        '{',
        optional($._blended_content),
        '}',
      ),

    razor_if: ($) =>
      seq(
        alias($._razor_if_start, 'at_if'),
        $.razor_condition,
        seq('{', optional($._blended_content), '}'),
        repeat(choice($.razor_else_if, $.razor_else)),
      ),

    razor_try: ($) =>
      prec.right(
        seq(
          alias($._razor_try_start, 'at_try'),
          '{',
          optional($._blended_content),
          '}',
          repeat(choice($.razor_catch, $.razor_finally)),
        ),
      ),

    razor_catch: ($) =>
      seq(
        alias($._razor_catch_continuation, 'catch'),
        repeat(choice($.catch_declaration, $.catch_filter_clause)),
        '{',
        optional($._blended_content),
        '}',
      ),

    razor_finally: ($) => seq(alias($._razor_finally_continuation, 'finally'), '{', optional($._blended_content), '}'),

    razor_else_if: ($) =>
      seq(alias($._razor_else_continuation, 'else'), 'if', $.razor_condition, '{', optional($._blended_content), '}'),

    razor_else: ($) => seq(alias($._razor_else_continuation, 'else'), '{', optional($._blended_content), '}'),

    razor_switch: ($) =>
      seq(
        alias($._razor_switch_start, 'at_switch'),
        $.razor_condition,
        '{',
        repeat(choice($.razor_switch_case, $.razor_switch_default)),
        '}',
      ),

    razor_condition: ($) => prec(10, seq('(', $.expression, ')')),

    razor_switch_case: ($) =>
      prec.left(seq('case', $.razor_case_condition, ':', optional($._blended_content))),

    razor_switch_default: ($) =>
      prec.right(seq('default', ':', optional($._blended_content))),

    razor_case_condition: ($) => choice($.expression, seq($.pattern, optional($.when_clause))),

    _razor_for_initializer: ($) =>
      seq(
        alias($._razor_for_start, 'at_for'),
        '(',
        field('initializer', optional(choice($.variable_declaration, $.expression))),
        ';',
        field('condition', optional($.expression)),
        ';',
        field('update', optional($.expression)),
        ')',
      ),

    razor_for: ($) => seq($._razor_for_initializer, '{', field('body', optional($._blended_content)), '}'),

    _blended_content: ($) =>
      repeat1(prec(10, choice($._node, $.explicit_line_transition, $.statement, $.comment))),

    _razor_foreach_initializer: ($) =>
      seq(
        alias($._razor_foreach_start, 'at_foreach'),
        '(',
        choice(
          seq(field('type', $.type), field('left', choice($.identifier, $.tuple_pattern))),
          field('left', $.expression),
        ),
        'in',
        field('right', $.expression),
        ')',
      ),

    razor_foreach: ($) =>
      seq($._razor_foreach_initializer, '{', field('body', optional($._blended_content)), '}'),

    razor_while: ($) =>
      seq(
        alias($._razor_while_start, 'at_while'),
        $.razor_condition,
        '{',
        optional($._blended_content),
        '}',
      ),

    _razor_while_condition: ($) => seq(alias($._razor_while_continuation, 'while'), $.razor_condition),

    razor_do_while: ($) =>
      seq(
        alias($._razor_do_start, 'at_do'),
        '{',
        optional($._blended_content),
        '}',
        $._razor_while_condition,
        ';',
      ),

    razor_section: ($) =>
      seq(
        alias(seq($._razor_marker, 'section'), 'at_section'),
        $.identifier,
        '{',
        optional($._blended_content),
        '}',
      ),

    explicit_line_transition: ($) => seq(alias('@:', 'at_colon_transition'), optional($.razor_line_markup), $._line_end),
    razor_line_markup: ($) => repeat1(choice($._line_text, $._markup_transition)),
    _markup_transition: ($) => choice($.razor_explicit_expression, $.razor_implicit_expression, $.razor_escape, $.razor_comment),

    razor_comment: (_) => token(seq('@*', repeat(choice(/[^*]/, /\*+[^@*]/)), optional(/\*+/), optional('@'))),
    razor_attribute_name: ($) => seq('@', choice(
      token(prec(10, /bind(-[A-Za-z_][A-Za-z0-9_]*)?/)),
      'attributes', 'key', 'ref', 'rendermode', token(prec(10, /on[a-z]+/)),
    ), optional($.razor_attribute_modifier)),
    razor_attribute_modifier: (_) => choice(':culture', ':preventDefault', ':stopPropagation', ':after', ':get', ':set'),
    razor_literal_attribute_name: (_) => token(prec(10, /@(formname|bind(-[A-Za-z_][A-Za-z0-9_]*)?:(event|format))/)),
    razor_attribute_literal: ($) => choice(
      seq('"', repeat(choice($._markup_transition, alias(/[^"@]+/, $.razor_attribute_literal_text))), '"'),
      seq("'", repeat(choice($._markup_transition, alias(/[^'@]+/, $.razor_attribute_literal_text))), "'"),
    ),
    razor_attribute_value: ($) => choice(
      seq('"', choice($.expression, $.razor_explicit_expression, $.razor_implicit_expression), '"'),
      seq("'", choice($.expression, $.razor_explicit_expression, $.razor_implicit_expression), "'"),
    ),
    razor_html_attribute: ($) => choice(
      seq($.razor_attribute_name, optional(seq('=', $.razor_attribute_value))),
      seq($.razor_literal_attribute_name, '=', $.razor_attribute_literal),
    ),

    html_comment: ($) => seq('<!--', repeat(choice($._comment_text, $._markup_transition)), '-->'),
    html_doctype: (_) => token(seq('<!', /[Dd][Oo][Cc][Tt][Yy][Pp][Ee]/, /[^>]*/, '>')),
    razor_tag_opt_out: (_) => token.immediate('!'),
    _tag_name: (_) => /[a-zA-Z_][a-zA-Z0-9_.:-]*/,
    _end_tag: ($) => seq('</', optional($.razor_tag_opt_out), alias($._tag_name, "razor_tag_name"), '>'),
    _html_attribute_name: (_) => /[a-zA-Z0-9_:.\-]+/,
    _html_attribute_value: ($) => choice(
      seq('"', repeat(choice($._markup_transition, $._attr_text_dq)), '"'),
      seq("'", repeat(choice($._markup_transition, $._attr_text_sq)), "'"),
      alias($._attr_text_unquoted, $.attribute_value_unquoted),
      $.razor_explicit_expression, $.razor_implicit_expression,
    ),
    _attr_text_dq: (_) => token(prec(1, /([^"@]|[\p{L}\p{N}]@[\p{L}\p{N}])+/)),
    _attr_text_sq: (_) => token(prec(1, /([^'@]|[\p{L}\p{N}]@[\p{L}\p{N}])+/)),
    _attr_text_unquoted: (_) => /[^\s"'=<>`@]+/,
    _html_attribute: ($) => seq($._html_attribute_name, optional(seq('=', $._html_attribute_value))),
    _attributes: ($) => repeat1(choice($._html_attribute, $.razor_html_attribute, $.razor_explicit_expression, $.razor_implicit_expression)),
    element: ($) => choice(
      seq('<', optional($.razor_tag_opt_out), $._void_tag_name, optional($._attributes), choice('>', '/>')),
      seq('<', optional($.razor_tag_opt_out), alias($._tag_name, "razor_tag_name"), optional($._attributes), choice('/>', seq('>', repeat(choice($._node, $.html_text)), $._end_tag))),
      seq('<', optional($.razor_tag_opt_out), $._script_tag_name, optional($._attributes), '>', optional(alias($._script_markup, $.razor_raw_markup)), '</', /[sS][cC][rR][iI][pP][tT]/, '>'),
      seq('<', optional($.razor_tag_opt_out), $._style_tag_name, optional($._attributes), '>', optional(alias($._style_markup, $.razor_raw_markup)), '</', /[sS][tT][yY][lL][eE]/, '>'),
    ),
    _script_markup: ($) => repeat1(choice($._script_text, $._markup_transition)),
    _style_markup: ($) => repeat1(choice($._style_text, $._markup_transition)),
  },
});
