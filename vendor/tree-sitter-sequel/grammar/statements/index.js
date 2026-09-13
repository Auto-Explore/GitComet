import { optional_parenthesis, wrapped_in_parenthesis } from "../helpers.js";

import create_rules from "./create.js";
import alter_rules from "./alter.js";
import drop_rules from "./drop.js";
import rename_rules from "./rename.js";
import optimize_rules from "./optimize.js";
import merge_rules from "./merge.js";
import comment_rules from "./comment.js";
import delete_rules from "./delete.js";
import insert_rules from "./insert.js";
import update_rules from "./update.js";
import truncate_rules from "./truncate.js";
import copy_rules from "./copy.js";
import select_rules from "./select.js";
import set_rules from "./set.js";
import refresh_rules from "./refresh.js";

export default {

  block: $ => seq(
    $.keyword_begin,
    optional(';'),
    repeat(
      seq(
        $.statement,
        ';'
      ),
    ),
    $.keyword_end,
  ),

  statement: $ => seq(
    optional(seq(
      $.keyword_explain,
      optional($.keyword_analyze),
      optional($.keyword_verbose),
    )),
    choice(
      $._ddl_statement,
      $._dml_write,
      optional_parenthesis($._dml_read),
      $.while_statement,
    ),
  ),

  // GitComet: the `BEGIN ... END` branch is removed, as are the two T-SQL body
  // alternatives in create-function.js and create-procedure.js.
  //
  // All three are `repeat($.statement)` with no separator between statements,
  // which is ambiguous enough to cost 11,800 parse states -- upstream's
  // STATE_COUNT goes 17,377 -> 29,311 and `.rodata` 1.90 -> 3.87 MB, for syntax
  // that only works when the body has no semicolons anyway. Without them the
  // grammar is 18,810 states / 2.11 MB and every other fix since 0.3.11 is
  // kept: backtick identifiers, HAVING without GROUP BY, policies, procedures,
  // RLIKE/REGEXP and MATERIALIZED VIEW. A simple `WHILE cond stmt` still
  // parses; a T-SQL body falls back to the behaviour of the 0.3.11 release.
  while_statement: $ => prec.left(seq(
    $.keyword_while,
    optional_parenthesis($._expression),
    seq(
      $.statement,
      optional(';'),
    ),
  )),

  var_declarations: $ => seq($.keyword_declare, repeat1($.var_declaration)),
  var_declaration: $ => seq(
    $.identifier,
    $._type,
    optional(
      seq(
        choice($.keyword_default, '='),
        $.literal,
      ),
    ),
    optional(','),
  ),

  _ddl_statement: $ => choice(
    $._create_statement,
    $._alter_statement,
    $._drop_statement,
    $._rename_statement,
    $._optimize_statement,
    $._merge_statement,
    $._refresh_statement,
    $.comment_statement,
    $.set_statement,
    $.reset_statement,
  ),

  ...create_rules,
  ...alter_rules,
  ...drop_rules,
  ...rename_rules,
  ...optimize_rules,
  ...merge_rules,
  ...refresh_rules,
  ...comment_rules,

  _dml_write: $ => seq(
    seq(
      optional(
        $._cte,
      ),
      choice(
        $._delete_statement,
        $._insert_statement,
        $._update_statement,
        $._truncate_statement,
        $._copy_statement,
      ),
    ),
  ),

  ...delete_rules,
  ...insert_rules,
  ...update_rules,
  ...truncate_rules,
  ...copy_rules,

  _dml_read: $ => seq(
    optional(optional_parenthesis($._cte)),
    optional_parenthesis(
      choice(
        $._select_statement,
        $.set_operation,
        $._show_statement,
        $._unload_statement,
      ),
    ),
  ),

  ...select_rules,
  ...set_rules,

  _show_statement: $ => seq(
    $.keyword_show,
    choice(
      $._show_create,
      $.keyword_all, // Postgres
      $._show_tables // trino/presto
    ),
  ),

  _show_create: $ => seq(
    $.keyword_create,
    choice(
      // Trino/Presto/MySQL
      $.keyword_schema,
      $.keyword_table,
      seq(optional($.keyword_materialized), $.keyword_view),
      // MySQL
      $.keyword_user,
      $.keyword_trigger,
      $.keyword_procedure,
      $.keyword_function
    ),
    $.object_reference
  ),

  _show_tables: $ => seq(
    $.keyword_tables,
    optional(seq($.keyword_from, $._qualified_field)),
    optional(seq($.keyword_like, $._expression))
  ),

  // athena
  _unload_statement: $ => seq(
    $.keyword_unload,
    wrapped_in_parenthesis($._select_statement),
    $.keyword_to,
    $._single_quote_string,
    $.storage_parameters,
  ),

};
