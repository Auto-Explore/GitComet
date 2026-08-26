const
  class_ = require('./grammar/class.js'),
  conflicts = require('./grammar/conflicts.js'),
  context = require('./grammar/context.js'),
  data = require('./grammar/data.js'),
  decl = require('./grammar/decl.js'),
  exp = require('./grammar/exp.js'),
  externals = require('./grammar/externals.js'),
  general = require('./grammar/general.js'),
  id = require('./grammar/id.js'),
  inline = require('./grammar/inline.js'),
  lexeme = require('./grammar/lexeme.js'),
  literal = require('./grammar/literal.js'),
  module_ = require('./grammar/module.js'),
  operator = require('./grammar/operator.js'),
  pat = require('./grammar/pat.js'),
  patsyn = require('./grammar/patsyn.js'),
  precedences = require('./grammar/precedences.js'),
  th = require('./grammar/th.js'),
  type = require('./grammar/type.js')

module.exports = grammar({
  name: 'haskell',

  rules: {
    haskell: $ => seq(
      optional($.header),
      optional($._body),
    ),

    ...general,
    ...type,
    ...context,
    ...exp,
    ...pat,
    ...module_,
    ...data,
    ...class_,
    ...decl,
    ...patsyn,
    ...th,
    ...literal,
    ...id,
    ...operator,
    ...lexeme,

  },

  ...externals,
  ...precedences,
  ...inline,
  ...conflicts,

  /**
   * These rules may occur anywhere in the grammar and don't have to be specified in productions.
   */
  extras: $ => [
    /\p{Zs}/,
    /\n/,
    /\r/,
    $.cpp,
    $.comment,
    $.haddock,
    $.pragma,
  ],

  /**
   * Rules with leading underscore are generally omitted from the AST, and can therefore not be used in queries.
   * The rules listed in this attribute are omitted from the AST, but their names can be used in queries in place of
   * their children; as well as in combination with them, using the syntax `expression/variable`.
   * This is most useful for choice rules that represent syntactic categories, like expressions, patterns, and types in
   * Haskell.
   *
   * See the readme for a detailed explanation.
   */
  supertypes: $ => [
    $.expression,
    $.pattern,
    $.type,
    $.quantified_type,
    $.constraint,
    $.constraints,
    $.type_param,
    $.declaration,
    $.decl,
    $.class_decl,
    $.instance_decl,
    $.statement,
    $.qualifier,
    $.guard,
  ],

  /**
   * This rule is used to detect that a reserved keyword is a prefix of an identifier.
   *
   * For example, if the identifier `ifM` occurs in a position where the keyword `if` is valid (so most expressions),
   * the fact that `ifM` matches `variable` prevents tree-sitter from lexing `if` followed by `M` as a `name`.
   */
  word: $ => $.variable,

})
