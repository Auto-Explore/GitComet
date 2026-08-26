module.exports = grammar({
    name: 'asm',
    extras: $ => [
        / |\t|\r/,
        $.line_comment,
        $.block_comment,
    ],
    conflicts: $ => [
        [
            $._expr,
            $._tc_expr,
        ],
    ],

    rules: {
        program: $ => sep(repeat1('\n'), $._item),
        _item: $ =>
            choice(
                $.meta,
                $.label,
                $.const,
                $.instruction,
            ),

        // GitComet: a directive takes a comma-separated list of mixed operands.
        //
        // Upstream allowed a single `ident`, or a list all of one type -- ints, or
        // floats, or strings -- so a directive mixing kinds did not parse. The
        // case that matters is the one gcc and clang emit after *every* function:
        //
        //     .size   main, .-main
        //
        // `main` is an ident and `.-main` is an expression, so the list was
        // rejected and everything after the comma became an ERROR. `_tc_expr`
        // already covers ident / int / string / infix, which is what makes
        // `.-main` parse as `.` minus `main`; `float` is kept alongside it because
        // it is not one of `_tc_expr`'s alternatives.
        meta: $ =>
            seq(
                field('kind', $.meta_ident),
                optional(sep(',', choice($._tc_expr, $.float))),
            ),
        label: $ =>
            choice(
                seq(
                    // GitComet: `mnemonic` is in this choice because it now
                    // out-lexes `word` and `_ident` everywhere (see below), so
                    // without it `main:` lexed as an instruction and left the
                    // `:` as an ERROR. The `:` is what still tells the two
                    // apart -- only a label has one.
                    choice(
                        $.meta_ident,
                        alias($.word, $.ident),
                        alias($._ident, $.ident),
                        alias($.mnemonic, $.ident),
                    ),
                    ':',
                    optional(choice(seq('(', $.ident, ')'), $.meta)),
                ),
                seq(
                    'label',
                    field('name', $.word),
                ),
            ),
        const: $ => seq('const', field('name', $.word), field('value', $._tc_expr)),
        // GitComet: `kind` is `$.mnemonic`, not `$.word`.
        //
        // Upstream's `word` is /[a-zA-Z0-9_]+/, which cannot span the `.` in an
        // arm64 condition suffix. `b.eq` therefore lexed as `_ident` (the only
        // token that admits a dot, and the one a label starts with), found no
        // `:` after it, and became an ERROR node -- so a whole conditional
        // branch was uncoloured and unclickable.
        instruction: $ =>
            seq(field('kind', $.mnemonic), choice(sep(',', $._expr), repeat($._tc_expr))),
        _expr: $ => choice($.ptr, $.ident, $.int, $.string, $.float, $.list),

        // ARMv7
        list: $ =>
            seq(
                '{',
                optional(seq($.reg, repeat(seq(choice(',', '-'), $.reg)), optional(','))),
                '}'
            ),

        ptr: $ =>
            choice(
                seq(
                    optional(seq(choice('byte', 'word', 'dword', 'qword'), 'ptr')),
                    '[',
                    $.reg,
                    optional(seq(choice('+', '-'), choice($.int, $.ident))),
                    ']',
                ),
                seq(
                    optional($.int),
                    '(',
                    $.reg,
                    ')',
                ),
                seq(
                    '*',
                    'rel',
                    '[',
                    $.int,
                    ']',
                ),
                // Aarch64
                seq(
                    '[',
                    $.reg,
                    optional(seq(',', $.int)),
                    ']',
                    optional('!'),
                ),
            ),
        // Turing Complete
        _tc_expr: $ =>
            choice(
                $.ident,
                $.int,
                $.string,
                $.tc_infix,
            ),
        tc_infix: $ =>
            choice(
                ...[
                    ['+', 0],
                    ['-', 0],
                    ['*', 1],
                    ['/', 1],
                    ['%', 1],
                    ['|', 2],
                    ['^', 3],
                    ['&', 4],
                ].map(([op, p]) =>
                    prec.left(
                        p,
                        seq(field('lhs', $._tc_expr), field('op', op), field('rhs', $._tc_expr)),
                    )
                ),
            ),

        int: $ => {
            const _int = /-?([0-9][0-9_]*|(0x|\$)[0-9A-Fa-f][0-9A-Fa-f_]*|0b[01][01_]*)/
            return choice(
                // GitComet: the ARM immediate is ONE token, not `#` followed by
                // one.
                //
                // Upstream wrote `seq('#', token.immediate(_int))`, which makes
                // the lexer emit a bare `#` and only then look for digits. In any
                // state where an operand is still possible -- after a mnemonic
                // with no operands, or with one that could be followed by more --
                // that bare `#` out-competes `line_comment`, so a GAS or MIPS
                // comment was swallowed and its prose parsed as instructions:
                //
                //     nop            # an explicit nop, because the slot must
                //     mflo    $t2    # and must be moved out explicitly
                //
                // both lost their mnemonic and their comment. As one token the
                // lexer has to see the digits before it commits, so `# an ...`
                // falls through to `line_comment` while `#8` still lexes as an
                // immediate. `prec` makes that explicit; match length alone would
                // decide the same way.
                //
                // The case this cannot fix is a comment whose text begins with a
                // number -- `#42 is a note` still lexes as the immediate 42 --
                // because telling the two apart needs lookahead past the digits,
                // which a token regex has no way to express.
                token(prec(1, seq('#', _int))),
                _int,
            )
        },
        float: $ => /-?[0-9][0-9_]*\.([0-9][0-9_]*)?/,
        string: $ =>
            choice(
                /"[^"]*"/,
                /'[^']*'/
	    ),

        word: $ => /[a-zA-Z0-9_]+/,
        // GitComet: a mnemonic and its dot-separated suffixes as one token.
        //
        // Covers arm64 (`b.eq`, `csel.w`, `fcmp.s`), ARM's `it` blocks, and the
        // plain undotted mnemonics every other target uses. Higher lexical
        // precedence than `_ident` so a line that could begin either a label or
        // an instruction resolves to the instruction; a real label still wins,
        // because `label` is the only rule with a `:` after it.
        mnemonic: $ => token(prec(1, /[a-zA-Z_][a-zA-Z0-9_]*(\.[a-zA-Z0-9_]+)*/)),
        _reg: $ => /%?[a-z0-9]+/,
        // GitComet: the Z80 shadow registers.
        //
        // `AF'`, `BC'`, `DE'` and `HL'` end in an apostrophe, and upstream has no
        // token that admits one -- so the `'` opened a single-quoted string
        // instead. In fixtures/syntax_test/assembly/z80/registers.asm the `EX AF,
        // AF'` on line 45 swallowed the next fifty lines, up to the `'A'` on line
        // 93, and rendered all of them as one string. The corpus file says so in
        // its own prose: "the quote is also an apostrophe".
        //
        // Four fixed names rather than a general "identifier may end in `'`",
        // because that is all Z80 has and a looser rule would start eating the
        // opening quote of real character literals in every other dialect. Longest
        // match already picks this over `word` for `AF`; `prec` says so out loud.
        shadow_reg: $ => token(prec(1, /([aA][fF]|[bB][cC]|[dD][eE]|[hH][lL])'/)),
        address: $ => /[=\$][a-zA-Z0-9_]+/, // GAS x86 address
        reg: $ => choice($._reg, $.word, $.address, $.shadow_reg),
        // GitComet: uppercase and digits admitted.
        //
        // Upstream's /\.[a-z_]+/ misses `.LBB0_1`, `.Lfunc_end0` and `.p2align`
        // -- the labels and directives clang emits for every function it
        // compiles, which is most of what a real `.s` file contains.
        meta_ident: $ => /\.[a-zA-Z_][a-zA-Z0-9_]*/,
        _ident: $ => /[a-zA-Z_0-9.]+/,
        ident: $ => choice($._ident, $.meta_ident, $.reg),

        line_comment: $ =>
            choice(
                seq('#', token.immediate(/.*/)),
                /(\/\/|;).*/,
            ),
        block_comment: $ =>
            token(seq(
                '/*',
                /[^*]*\*+([^/*][^*]*\*+)*/,
                '/',
            )),
    },
})

function sep(separator, rule) {
    return optional(seq(rule, repeat(seq(separator, rule)), optional(separator)))
}
