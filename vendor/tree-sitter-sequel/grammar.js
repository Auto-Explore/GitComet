import keyword_rules from "./grammar/keywords.js";
import type_rules from "./grammar/types.js";
import column_list_rules from "./grammar/column-lists.js";
import expression_rules from "./grammar/expressions.js";
import transaction_rules from "./grammar/transactions.js";
import statement_rules from "./grammar/statements/index.js";

export default grammar({
  name: 'sql',

  extras: $ => [
    /\s\n/,
    /\s/,
    $.comment,
    $.marginalia,
  ],


  externals: $ => [
    $._dollar_quoted_string_start_tag,
    $._dollar_quoted_string_end_tag,
    $._dollar_quoted_string,
  ],

  conflicts: $ => [
    [$.object_reference, $._qualified_field],
    [$.field, $._qualified_field],
    [$._column, $._qualified_field],
    [$.object_reference],
    [$.between_expression, $.binary_expression],
    [$.time],
    [$.timestamp],
  ],

  precedences: $ => [
    [
      'binary_is',
      'unary_not',
      'binary_exp',
      'binary_times',
      'binary_plus',
      'unary_other',
      'binary_other',
      'binary_in',
      'binary_compare',
      'binary_relation',
      'pattern_matching',
      'between',
      'clause_connective',
      'clause_disjunctive',
    ],
  ],

  word: $ => $._identifier,

  rules: {
    program: $ => seq(
      // any number of transactions, statements, or blocks with a terminating ;
      repeat(
        seq(
          choice(
            $.transaction,
            $.statement,
            $.block,
          ),
          ';',
        ),
      ),
      // optionally, a single statement without a terminating ;
      optional(
        $.statement,
      ),
    ),

    comment: _ => /--.*/,
    // https://stackoverflow.com/questions/13014947/regex-to-match-a-c-style-multiline-comment
    marginalia: _ => /\/\*[^*]*\*+(?:[^/*][^*]*\*+)*\//,

    ...keyword_rules,
    ...type_rules,
    ...column_list_rules,
    ...expression_rules,
    ...transaction_rules,
    ...statement_rules,

  }

});
