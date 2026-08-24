module.exports = grammar({
    name: 'ebnf',

    extras: $ => [
        / |\n|\t|\r/,
        $.comment,
    ],

    rules: {
        syntax: $ => repeat1($.syntax_rule),

        terminal: $ => /'[^']*'|"[^"]*"/,
        identifier: $ => /[a-zA-Z][a-zA-Z0-9_]*/,
        integer: $ => /[0-9]+/,
        comment: $ => /\(\*[^*]*\*+(?:[^)*][^*]*\*+)*\)/,
        special_sequence: $ => /\?[^?]*\?/,

        syntax_rule: $ => seq(field('name', $.identifier), '=', field('definition', optional($._expression)), ';'),

        _expression: $ =>
            choice(
                $._atom,
                $.binary_expression,
                $.group,
            ),
        _atom: $ =>
            choice(
                $.identifier,
                $.terminal,
                $.special_sequence,
            ),
        binary_expression: $ =>
            choice(
                ...[
                    [$._expression, '|', $._expression],
                    [$._expression, ',', $._expression],
                    [$._expression, '-', optional($._expression)],
                    [$.integer, '*', $._expression],
                ].map(([left, op, right], index) =>
                    prec.left(
                        index + 1,
                        seq(
                            field('left', left),
                            field('operator', op),
                            field('right', right),
                        ),
                    )
                ),
            ),
        group: $ =>
            choice(
                seq('[', optional($._expression), ']'),
                seq('{', optional($._expression), '}'),
                seq('(', optional($._expression), ')'),
            ),
    },
})
