/**
 * @file Crontab grammar for tree-sitter
 * @license MIT
 *
 * Written for GitComet: no crontab grammar exists on crates.io, and the only one
 * on GitHub carries no licence at all.
 *
 * The shape of the language is that the first five fields are structured and
 * everything after them is an opaque command that runs to the end of the line --
 * `#` included, because crontab has no trailing comments. That asymmetry is the
 * whole grammar: the fields are parsed, the command is one token.
 */

/* eslint-disable no-undef */
module.exports = grammar({
  name: 'crontab',

  extras: () => [/[ \t]/],

  rules: {
    file: ($) => repeat(choice($.comment, $.environment, $.job, /\r?\n/)),

    comment: () => token(seq('#', /.*/)),

    // `MAILTO=""`, `PATH=/usr/bin`. Read by cron itself, not by a shell, so the
    // value is literal to end of line and quotes are not special.
    environment: ($) =>
      seq(field('name', $.variable), '=', optional(field('value', $.value)), /\r?\n/),

    variable: () => /[A-Za-z_][A-Za-z0-9_]*/,
    value: () => /[^\n]*/,

    job: ($) => seq(field('schedule', $.schedule), field('command', $.command), /\r?\n/),

    schedule: ($) =>
      choice($.special, seq($.field, $.field, $.field, $.field, $.field)),

    // `@reboot`, `@daily` and friends replace all five fields.
    special: () =>
      token(
        seq(
          '@',
          choice(
            'reboot',
            'yearly',
            'annually',
            'monthly',
            'weekly',
            'daily',
            'midnight',
            'hourly',
          ),
        ),
      ),

    // One field is a comma-separated list of terms, each optionally stepped.
    field: ($) => seq($._term, repeat(seq(',', $._term))),

    _term: ($) => seq(choice($.range, $.wildcard, $.number, $.name), optional($.step)),

    range: ($) => seq(choice($.number, $.name), '-', choice($.number, $.name)),
    wildcard: () => '*',
    step: ($) => seq('/', $.number),
    number: () => /\d+/,
    // `JAN`..`DEC`, `SUN`..`SAT`. Anything alphabetic in a field position is one
    // of these; cron rejects the rest, and a highlighter need not.
    name: () => /[A-Za-z]{3}/,

    // Everything after the fifth field, `#` included: crontab has no trailing
    // comment, which is the single most common thing people get wrong about it.
    command: () => /[^\n]+/,
  },
});
