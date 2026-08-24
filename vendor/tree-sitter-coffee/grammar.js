module.exports = grammar({
  name: 'coffeescript',

  word: $ => $.identifier,

  externals: $ => [
    $._NEWLINE,
    $._INDENT,
    $._DEDENT
  ],

  conflicts: $ => [
    [$.return_statement],
    [$.expression, $.function_call],
    [$.function_definition, $.expression],
    [$.block],
    [$.expression, $.pattern],
    [$.array, $.array_pattern],
    [$.object, $.object_pattern],
    [$.yield_statement],
    [$.unless_statement],
    [$.function_definition, $.expression, $.function_call],
    [$.parameter, $.expression],
    [$.jsx_element],
    [$.super_expression],
    [$.switch_statement],
    [$.method_definition],
    [$.try_statement],
    [$.do_block],
    [$.expression_statement],
    [$.export_statement, $.expression_statement],
    [$.export_statement, $.function_definition],
    [$.method_definition, $.expression],
    [$.binary_expression, $.existential_member_expression, $.existential_function_call],
    [$.method_definition, $.class_property_block],
    [$.method_definition, $.class_property_block, $.expression],
    [$.class_property_block, $.expression],
    [$.class_property_method, $.class_property_assignment],
    [$.class_property_block, $.class_property_method],
    [$.class_property_block, $.class_property_assignment],
    [$.function_definition, $.function_expression],
    [$.parameters, $.function_call],
    [$.function_expression, $.class_property_method],
    [$.parameter, $.expression]
  ],

  extras: $ => [
    /[ \t\f\v\r]/,
    $.comment
  ],

  rules: {
    source_file: $ => seq(
      repeat($._terminator),
      optional(seq(
        $.statement_sequence,
        optional(repeat1($._terminator))
      ))
    ),

    statement_sequence: $ => prec.left(seq(
      $.statement,
      repeat(seq(repeat1($._terminator), $.statement))
    )),

    _terminator: $ => choice($._NEWLINE, ';'),

    statement: $ => choice(
      $.expression_statement,
      $.assignment_statement,
      $.function_definition,
      $.method_definition,
      $.class_definition,
      $.if_statement,
      $.switch_statement,
      $.try_statement,
      $.throw_statement,
      $.import_statement,
      $.export_statement,
      $.module_exports_statement,
      $.for_statement,
      $.while_statement,
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.unless_statement,
      $.until_statement,
      $.loop_statement,
      $.yield_statement,
      $.var_statement,
      $.comment
    ),

    expression_statement: $ => $.expression,

    assignment_statement: $ => seq(
      $.pattern,
      choice('=', '+=', '-=', '*=', '/=', '%=', '||=', '&&=', '?='),
      $.expression
    ),

    function_definition: $ => seq(
      optional('async'),
      optional($.identifier),
      $.parameters,
      choice('->', '=>'),
      $.block
    ),

    function_expression: $ => seq(
      optional('async'),
      optional(choice($.parameters, $.pattern)),
      choice('->', '=>'),
      choice(
        $.block,
        seq(optional(repeat1($._terminator)), $.expression)
      )
    ),

    method_definition: $ => seq(
      $.identifier,
      ':',
      optional('async'),
      optional($.parameters),
      choice('->', '=>'),
      $.block
    ),

    parameters: $ => seq('(', optional(seq($.parameter, repeat(seq(',', $.parameter)))), ')'),

    parameter: $ => seq($.pattern, optional(seq('=', $.expression)), optional('...')),

    class_definition: $ => seq(
      'class',
      $.identifier,
      optional(seq('extends', $.identifier)),
      $.class_body
    ),

    if_statement: $ => prec.left(seq(
      'if',
      $.expression,
      $.block,
      repeat(seq('else if', $.expression, $.block)),
      optional(seq('else', $.block))
    )),

    switch_statement: $ => seq(
      'switch',
      $.expression,
      repeat1($.when_clause),
      optional($.else_clause)
    ),

    when_clause: $ => seq(
      'when',
      $.expression,
      repeat(seq(',', $.expression)),
      $.block
    ),

    else_clause: $ => seq('else', $.block),

    try_statement: $ => seq(
      'try',
      $.block,
      optional($.catch_clause),
      optional($.finally_clause)
    ),

    catch_clause: $ => seq(
      'catch',
      optional(seq('(', $.identifier, ')')),
      $.block
    ),

    finally_clause: $ => seq('finally', $.block),

    break_statement: $ => 'break',

    continue_statement: $ => 'continue',

    for_statement: $ => seq(
      'for',
      optional('own'),
      $.identifier,
      choice('in', 'of'),
      $.expression,
      optional(seq('by', $.expression)),
      $.block
    ),

    while_statement: $ => seq(
      'while',
      $.expression,
      $.block
    ),

    return_statement: $ => seq('return', optional($.expression)),

    throw_statement: $ => seq('throw', $.expression),

    import_statement: $ => choice(
      seq('import', $.identifier, optional(seq(',', $.named_imports)), 'from', $.string),
      seq('import', $.namespace_import, 'from', $.string),
      seq('import', $.named_imports, 'from', $.string),
      seq('import', $.string)
    ),

    namespace_import: $ => seq('*', 'as', $.identifier),

    named_imports: $ => seq(
      '{',
      optional(seq($.import_specifier, repeat(seq(',', $.import_specifier)), optional(','))),
      '}'
    ),

    import_specifier: $ => seq($.identifier, optional(seq('as', $.identifier))),

    export_statement: $ => seq(
      'export',
      choice(
        seq('default', $.expression),
        $.class_definition,
        $.function_definition,
        $.named_exports,
        seq('*', 'from', $.string),
        $.identifier
      )
    ),

    named_exports: $ => seq(
      '{',
      optional(seq($.export_specifier, repeat(seq(',', $.export_specifier)), optional(','))),
      '}'
    ),

    export_specifier: $ => seq($.identifier, optional(seq('as', $.identifier))),

    module_exports_statement: $ => seq(
      'module',
      '.',
      'exports',
      choice('=', '+=', '-=', '*=', '/=', '%=', '||=', '&&=', '?='),
      $.expression
    ),

    unless_statement: $ => seq('unless', $.expression, $.block, optional(seq('else', $.block))),

    until_statement: $ => seq('until', $.expression, $.block),

    loop_statement: $ => seq('loop', $.block),

    yield_statement: $ => seq('yield', optional($.expression)),

    var_statement: $ => seq('var', $.pattern, optional(seq('=', $.expression))),

    block: $ => choice(
      seq(choice(':', 'then'), $.inline_block),
      seq(choice(':', 'then'), $.indented_block),
      $.indented_block
    ),

    inline_block: $ => prec.left(seq(
      $.statement,
      repeat(seq(repeat1($._terminator), $.statement))
    )),

    indented_block: $ => seq(
      $._NEWLINE,
      $._INDENT,
      repeat($._terminator),
      optional(seq(
        $.statement_sequence,
        optional(repeat1($._terminator))
      )),
      $._DEDENT
    ),

    class_body: $ => choice(
      seq(choice(':', 'then'), $.class_inline_body),
      seq(choice(':', 'then'), $.class_indented_body),
      $.class_indented_body
    ),

    class_inline_body: $ => prec.left(seq(
      $.class_member,
      repeat(seq(optional(repeat1($._terminator)), $.class_member))
    )),

    class_indented_body: $ => seq(
      $._NEWLINE,
      $._INDENT,
      repeat($._terminator),
      optional(seq(
        $.class_member,
        repeat(seq(optional(repeat1($._terminator)), $.class_member)),
        optional(repeat1($._terminator))
      )),
      $._DEDENT
    ),

    class_member: $ => choice(
      $.class_property_block,
      $.method_definition,
      $.assignment_statement,
      $.expression_statement,
      $.comment
    ),

    class_property_block: $ => seq(
      $.identifier,
      ':',
      $._NEWLINE,
      $._INDENT,
      repeat($._terminator),
      optional(seq(
        $.class_property_member,
        repeat(seq(optional(repeat1($._terminator)), $.class_property_member)),
        optional(repeat1($._terminator))
      )),
      $._DEDENT
    ),

    class_property_member: $ => choice(
      $.class_property_block,
      $.class_property_method,
      $.class_property_assignment,
      $.comment
    ),

    class_property_method: $ => seq(
      $.identifier,
      ':',
      optional('async'),
      optional($.parameters),
      choice('->', '=>'),
      $.block
    ),

    class_property_assignment: $ => seq(
      $.identifier,
      ':',
      $.expression
    ),

    expression: $ => choice(
      $.identifier,
      $.number,
      $.string,
      $.boolean,
      $.null_literal,
      $.undefined_literal,
      $.array,
      $.object,
      $.range,
      $.comprehension,
      $.instance_variable,
      $.regex,
      $.jsx_element,
      $.embedded_html,
      $.embedded_js,
      $.new_expression,
      $.delete_expression,
      $.typeof_expression,
      $.await_expression,
      $.super_expression,
      $.do_block,
      $.function_expression,
      $.function_call,
      $.member_expression,
      $.existential_member_expression,
      $.existential_function_call,
      $.existential_check,
      $.postfix_conditional,
      $.binary_expression,
      $.unary_expression,
      $.parenthesized_expression
    ),

    function_call: $ => prec.left(13, seq(
      $.expression,
      '(',
      optional(seq($.expression, repeat(seq(',', $.expression)))),
      ')'
    )),

    member_expression: $ => prec.left(11, seq(
      $.expression,
      '.',
      $.identifier
    )),

    binary_expression: $ => choice(
      prec.left(1, seq($.expression, '+', $.expression)),
      prec.left(1, seq($.expression, '-', $.expression)),
      prec.left(2, seq($.expression, '*', $.expression)),
      prec.left(2, seq($.expression, '/', $.expression)),
      prec.left(3, seq($.expression, '<<', $.expression)),
      prec.left(3, seq($.expression, '>>', $.expression)),
      prec.left(3, seq($.expression, '>>>', $.expression)),
      prec.left(4, seq($.expression, '&', $.expression)),
      prec.left(5, seq($.expression, '^', $.expression)),
      prec.left(6, seq($.expression, '|', $.expression)),
      prec.left(7, seq($.expression, '==', $.expression)),
      prec.left(7, seq($.expression, '!=', $.expression)),
      prec.left(7, seq($.expression, 'is', $.expression)),
      prec.left(7, seq($.expression, 'isnt', $.expression)),
      prec.left(7, seq($.expression, '<', $.expression)),
      prec.left(7, seq($.expression, '>', $.expression)),
      prec.left(7, seq($.expression, '<=', $.expression)),
      prec.left(7, seq($.expression, '>=', $.expression)),
      prec.left(7, seq($.expression, 'instanceof', $.expression)),
      prec.left(8, seq($.expression, '&&', $.expression)),
      prec.left(8, seq($.expression, 'and', $.expression)),
      prec.left(9, seq($.expression, '||', $.expression)),
      prec.left(9, seq($.expression, 'or', $.expression)),
      prec.left(10, seq($.expression, '::', $.expression))
    ),

    unary_expression: $ => choice(
      seq('!', $.expression),
      seq('-', $.expression),
      seq('not', $.expression),
      seq('~', $.expression)
    ),

    existential_check: $ => prec.right(13, seq($.expression, '?')),

    existential_member_expression: $ => prec.left(12, seq(
      $.expression,
      '?.',
      $.identifier
    )),

    existential_function_call: $ => seq(
      $.expression,
      '?.',
      '(',
      optional(seq($.expression, repeat(seq(',', $.expression)))),
      ')'
    ),

    postfix_conditional: $ => prec.right(13, seq(
      $.expression,
      choice('if', 'unless', 'while', 'until'),
      $.expression
    )),

    parenthesized_expression: $ => seq('(', $.expression, ')'),

    instance_variable: $ => seq('@', $.identifier),

    regex: $ => choice(

      $.single_line_regex,

      $.multi_line_regex

    ),

    single_line_regex: $ => seq('/', token(/([^\/]|\\.)*/), '/', token(/[a-z]*/)),

    multi_line_regex: $ => seq('///', token(/([^\/]|\\.)*/), '///', token(/[a-z]*/)),

    new_expression: $ => seq('new', $.expression),

    delete_expression: $ => seq('delete', $.expression),

    typeof_expression: $ => seq('typeof', $.expression),

    await_expression: $ => seq('await', $.expression),

    super_expression: $ => seq('super', optional(seq('(', optional(seq($.expression, repeat(seq(',', $.expression)))), ')'))),

    do_block: $ => seq(
      'do',
      optional($.parameters),
      choice('->', '=>'),
      $.block
    ),

    jsx_element: $ => seq('<', $.identifier, repeat($.jsx_attribute), optional('/>'), optional(seq('>', repeat($.jsx_child), '</', $.identifier, '>'))),

    jsx_attribute: $ => seq($.identifier, optional(seq('=', choice($.string, seq('{', $.expression, '}'))))),

    jsx_child: $ => choice($.jsx_element, seq('{', $.expression, '}'), /[^<>]+/),

    embedded_html: $ => seq(
      '```',
      optional(token.immediate('html')),
      /\r?\n/,
      repeat(choice(
        token.immediate(/[^\r\n`]+/),
        token.immediate(/`[^`]/),
        token.immediate(/``[^`]/),
        token.immediate(/\r?\n/)
      )),
      '```'
    ),

    embedded_js: $ => seq('`', /([^`]|\\.)*/, '`'),

    pattern: $ => choice(
      $.identifier,
      $.instance_variable,
      $.array_pattern,
      $.object_pattern
    ),

    array_pattern: $ => seq(
      '[',
      optional(seq(
        $.pattern,
        repeat(seq(',', $.pattern)),
        optional(seq(',', $.splat_pattern))
      )),
      ']'
    ),

    splat_pattern: $ => seq('...', $.identifier),

    object_pattern: $ => seq('{', optional(seq($.identifier, repeat(seq(',', $.identifier)))), '}'),

    array: $ => seq('[', optional(seq($.expression, repeat(seq(',', $.expression)))), ']'),

    range: $ => choice(
      seq('[', $.expression, '..', $.expression, ']'),
      seq('[', $.expression, '...', $.expression, ']')
    ),

    comprehension: $ => prec.right(seq(
      '(',
      $.expression,
      'for',
      $.pattern,
      choice('in', 'of'),
      $.expression,
      optional(seq('when', $.expression)),
      ')'
    )),

    object: $ => seq('{', optional(seq($.pair, repeat(seq(',', $.pair)))), '}'),

    pair: $ => seq($.identifier, ':', $.expression),

    identifier: $ => /[a-zA-Z_$][a-zA-Z0-9_$]*/,

    decimal_number: $ => token(choice(
      seq(
        /\d+\.\d+/,
        optional(seq(
          /[eE]/,
          optional(choice('+', '-')),
          /\d+/
        ))
      ),
      seq(
        /\d+/,
        optional(seq(
          /[eE]/,
          optional(choice('+', '-')),
          /\d+/
        ))
      )
    )),

    hex_number: $ => /0[xX][0-9a-fA-F]+/,

    binary_number: $ => /0[bB][01]+/,

    octal_number: $ => /0[oO][0-7]+/,

    number: $ => choice(
      $.decimal_number,
      $.hex_number,
      $.binary_number,
      $.octal_number
    ),

    string: $ => choice(
      $.double_quoted_string,
      $.single_quoted_string,
      $.heredoc_string
    ),

    boolean: $ => choice('true', 'false', 'yes', 'on', 'no', 'off'),

    null_literal: $ => 'null',

    undefined_literal: $ => 'undefined',

    double_quoted_string: $ => seq(
      '"',
      repeat(choice(
        $.double_quoted_content,
        alias(prec(1, token.immediate('#')), $.double_quoted_content),
        $.escape_sequence,
        $.interpolation
      )),
      '"'
    ),

    single_quoted_string: $ => seq("'", repeat(choice($.single_quoted_content, $.escape_sequence)), "'"),

    heredoc_string: $ => choice(

      seq(
        '"""',
        repeat(choice(
          $.double_quoted_content,
          alias(prec(1, token.immediate('#')), $.double_quoted_content),
          $.escape_sequence,
          $.interpolation,
          token.immediate(/\r?\n/)
        )),
        '"""'
      ),

      seq("'''", repeat(choice($.single_quoted_content, $.escape_sequence, token.immediate(/\r?\n/))), "'''")

    ),

    double_quoted_content: $ => token.immediate(/[^"\\#\n\r]+/),

    single_quoted_content: $ => token.immediate(/[^'\\\n\r]+/),

    escape_sequence: $ => /\\./,

    interpolation: $ => seq(prec(2, token.immediate('#{')), $.expression, '}'),

    comment: $ => choice(
      token(prec(-1, seq('#', /[^\n]*/))),
      token(seq('###', /[\s\S]*?/, '###'))
    )
  }
});
