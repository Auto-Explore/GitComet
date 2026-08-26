/**
 * @file KDL grammar for tree-sitter
 * @author Amaan Qureshi <amaanq12@gmail.com>
 * @license MIT
 * @see {@link https://kdl.dev|official website}
 * @see {@link https://github.com/kdl-org/kdl/blob/main/SPEC.md|official syntax spec}
 */

// deno-lint-ignore-file no-control-regex
/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const ANNOTATION_BUILTINS = [
  'i8',
  'i16',
  'i32',
  'i64',
  'u8',
  'u16',
  'u32',
  'u64',
  'isize',
  'usize',
  'f32',
  'f64',
  'decimal64',
  'decimal128',
  'date-time',
  'time',
  'date',
  'duration',
  'decimal',
  'currency',
  'country-2',
  'country-3',
  'country-subdivision',
  'email',
  'idn-email',
  'hostname',
  'idn-hostname',
  'ipv4',
  'ipv6',
  'url',
  'url-reference',
  'irl',
  'iri-reference',
  'url-template',
  'uuid',
  'regex',
  'base64',
];

module.exports = grammar({
  name: 'kdl',

  conflicts: $ => [
    [$.document],
    [$._node_space],
    [$.node_children],
  ],

  externals: $ => [$._eof, $.multi_line_comment],

  extras: $ => [$.multi_line_comment],

  word: $ => $._normal_bare_identifier,

  rules: {
    // nodes := linespace* (node nodes?)? linespace*
    document: $ =>
      seq(
        repeat($._linespace),
        optional(seq(
          $.node,
          repeat(seq(
            repeat($._linespace),
            $.node,
          )),
        )),
        repeat($._linespace),
      ),

    // node := ('/-' node-space*)? type? identifier (node-space+ node-prop-or-arg)* (node-space* node-children ws*)? node-space* node-terminator
    node: $ => prec(1,
      seq(
        alias(optional(seq('/-', repeat($._node_space))), $.node_comment),
        optional($.type),
        $.identifier,
        repeat(seq(repeat1($._node_space), $.node_field)),
        optional(seq(repeat($._node_space), field('children', $.node_children), repeat($._ws))),
        repeat($._node_space),
        $._node_terminator,
      ),
    ),

    // node-prop-or-arg (field) := ('/-' node-space*)? (prop | value)
    // _node_prop_or_arg: $ =>
    //   seq(
    //     alias(optional(seq('/-', repeat($._node_space))), $.node_prop_or_arg_slash_dash),
    //     field('node_prop_or_arg', choice($.prop, $.value)),
    //   ),
    node_field: $ => choice($._node_field_comment, $._node_field),
    _node_field_comment: $ => alias(seq('/-', repeat($._node_space), $._node_field), $.node_field_comment),
    _node_field: $ => choice($.prop, $.value),
    // node-children := ('/-' node-space*)? '{' nodes '}'
    node_children: $ =>
      seq(
        optional(seq(alias('/-', $.node_children_comment), repeat($._node_space))),
        '{',
        seq(
          repeat($._linespace),
          optional(seq($.node, repeat(seq(repeat($._linespace), $.node)))),
          repeat($._linespace),
        ),
        '}',
      ),
    // node-space := ws* escline ws* | ws+
    _node_space: $ =>
      choice(
        seq(repeat($._ws), $._escline, repeat($._ws)),
        repeat1($._ws),
      ),
    // node-terminator := single-line-comment | newline | ';' | eof
    _node_terminator: $ =>
      choice($.single_line_comment, $._newline, ';', $._eof),

    // identifier := string | bare-identifier
    identifier: $ => choice($.string, $._bare_identifier),
    // bare-identifier := ((identifier-char - digit - sign) identifier-char* | sign ((identifier-char - digit) identifier-char*)?) - keyword
    _bare_identifier: $ =>
      choice(
        $._normal_bare_identifier,
        seq($._sign, optional(seq($.__identifier_char_no_digit, repeat($._identifier_char)))),
      ),

    // _normal_bare_identifier: $ => $.__identifier_char_no_digit_sign,
    _normal_bare_identifier: _ => token(
      seq(
        /[\u4E00-\u9FFF\p{L}\p{M}\p{N}\p{Emoji}_~!@#\$%\^&\*.:'\|\?&&[^\s\d\/(){}<>;\[\]=,"]]/,
        /[\u4E00-\u9FFF\p{L}\p{M}\p{N}\p{Emoji}\-_~!@#\$%\^&\*.:'\|\?+&&[^\s\/(){}<>;\[\]=,"]]*/,
      ),
    ),
    // identifier-char := unicode - linespace - [\/(){}<>;[]=,"]
    _identifier_char: _ => token(
      /[\u4E00-\u9FFF\p{L}\p{M}\p{N}\-_~!@#\$%\^&\*.:'\|\?+&&[^\s\/(){}<>;\[\]=,"]]/,
    ),

    // can't start with a digit
    __identifier_char_no_digit: _ => token(
      /[\u4E00-\u9FFF\p{L}\p{M}\p{N}\-_~!@#\$%\^&\*.:'\|\?+&&[^\s\d\/(){}<>;\[\]=,"]]/,
    ),

    // can't start with a digit or sign
    __identifier_char_no_digit_sign: _ => token(
      /[\u4E00-\u9FFF\p{L}\p{M}\p{N}\-_~!@#\$%\^&\*.:'\|\?&&[^\s\d\+\-\/(){}<>;\[\]=,"]]/,
    ),

    // keyword := boolean | 'null'
    keyword: $ => choice($.boolean, 'null'),
    // type annotations
    annotation_type: _ => choice(...ANNOTATION_BUILTINS),
    // prop := identifier '=' value
    prop: $ => seq($.identifier, '=', $.value),
    // value := type? (string | number | keyword)
    value: $ => seq(optional($.type), choice($.string, $.number, $.keyword)),
    // type := '(' identifier ')'
    type: $ => seq('(', choice($.identifier, $.annotation_type), ')'),

    // String
    // string := raw-string | escaped-string
    string: $ => choice($._raw_string, $._escaped_string),
    // escaped-string := '"' character* '"'
    _escaped_string: $ => seq('"', alias(repeat(choice($.escape, /[^"]/)), $.string_fragment), '"'),
    // character := '\' escape | [^\"]
    _character: $ => choice($.escape, /[^"]/),
    // escape := ["\\/bfnrt] | 'u{' hex-digit{1, 6} '}'
    escape: _ =>
      token.immediate(/\\\\|\\"|\\\/|\\b|\\f|\\n|\\r|\\t|\\u\{[0-9a-fA-F]{1,6}\}/),
    // hex-digit := [0-9a-fA-F]
    _hex_digit: _ => /[0-9a-fA-F]/,

    // // raw-string := 'r' raw-string-hash
    // raw_string: $ => seq('r', $._raw_string_hash),
    // // raw-string-hash := '#' raw-string-hash '#' | raw-string-quotes
    // _raw_string_hash: $ => choice(seq('#', $._raw_string_hash, '#'), $._raw_string_quotes),
    // // raw-string-quotes := '"' .* '"'
    // _raw_string_quotes: _ => seq('"', /.*/, '"'),
    _raw_string: _ =>
      seq(
        choice(
          // raw-string-hash := '#' raw-string-hash '#' | raw-string-quotes
          // yes this isn't perfect but it works, ideally this should be handled in an external scanner
          seq(token.immediate(seq('r', repeat1('#'))), /[^#]*/, repeat1('#')),
          // raw-string-quotes := '"' . '"'
          seq(token.immediate(seq('r', '"')), /[^"]*/, '"'),
        ),
      ),

    // number := decimal | hex | octal | binary
    number: $ => choice($._decimal, $._hex, $._octal, $._binary),

    // decimal := sign? integer ('.' integer)? exponent?
    _decimal: $ =>
      seq(
        optional($._sign),
        $._integer,
        optional(seq('.', alias($._integer, $.decimal))),
        optional(alias($._exponent, $.exponent)),
      ),

    // exponent := ('e' | 'E') sign? integer
    _exponent: $ => seq(choice('e', 'E'), optional($._sign), $._integer),
    // integer := digit (digit | '_')*
    _integer: $ => seq($._digit, repeat(choice($._digit, '_'))),
    // digit := [0-9]
    _digit: _ => /[0-9]/,
    // sign := '+' | '-'
    _sign: _ => choice('+', '-'),

    // hex := sign? '0x' hex-digit (hex-digit | '_')*
    _hex: $ => seq(optional($._sign), '0x', $._hex_digit, repeat(choice($._hex_digit, '_'))),
    // octal := sign? '0o' [0-7] [0-7_]*
    _octal: $ => seq(optional($._sign), '0o', /[0-7]/, repeat(choice(/[0-7]/, '_'))),
    // binary := sign? '0b' ('0' | '1') ('0' | '1' | '_')*
    _binary: $ => seq(optional($._sign), '0b', choice('0', '1'), repeat(choice('0', '1', '_'))),

    // boolean := 'true' | 'false'
    boolean: _ => choice('true', 'false'),

    // escline := '\\' ws* (single-line-comment | newline)
    _escline: $ => seq('\\', repeat($._ws), choice($.single_line_comment, $._newline)),

    // linespace := newline | ws | single-line-comment
    _linespace: $ => choice($._newline, $._ws, $.single_line_comment),

    // newline := See Table (All line-break white_space)
    // Newline
    // The following characters should be treated as new lines:
    //
    // ╭──────────────────────────────────────────────────────────╮
    // │  Acronym  Name                           Code Pt         │
    // │  CR       Carriage Return                U+000D          │
    // │  LF       Line Feed                      U+000A          │
    // │  CRLF     Carriage Return and Line Feed  U+000D + U+000A │
    // │  NEL      Next Line                      U+0085          │
    // │  FF       Form Feed                      U+000C          │
    // │  LS       Line Separator                 U+2028          │
    // │  PS       Paragraph Separator            U+2029          │
    // ╰──────────────────────────────────────────────────────────╯
    // Note that for the purpose of new lines, CRLF is considered a single newline.
    _newline: _ => choice(/\r'/, /\n/, /\r\n/, /\u0085/, /\u000C/, /\u2028/, /\u2029/),

    // ws := bom | unicode-space | multi-line-comment
    _ws: $ => choice($._bom, $._unicode_space, $.multi_line_comment),

    // bom := '\u{FEFF}'
    _bom: _ => /\u{FEFF}/,

    // unicode-space := See Table (All White_Space unicode characters which are not `newline`)
    // Whitespace
    // The following characters should be treated as non-Newline white space:
    //
    // ╭────────────────────────────────────╮
    // │  Name                      Code Pt │
    // │  Character Tabulation      U+0009  │
    // │  Space                     U+0020  │
    // │  No-Break Space            U+00A0  │
    // │  Ogham Space Mark          U+1680  │
    // │  En Quad                   U+2000  │
    // │  Em Quad                   U+2001  │
    // │  En Space                  U+2002  │
    // │  Em Space                  U+2003  │
    // │  Three-Per-Em Space        U+2004  │
    // │  Four-Per-Em Space         U+2005  │
    // │  Six-Per-Em Space          U+2006  │
    // │  Figure Space              U+2007  │
    // │  Punctuation Space         U+2008  │
    // │  Thin Space                U+2009  │
    // │  Hair Space                U+200A  │
    // │  Narrow No-Break Space     U+202F  │
    // │  Medium Mathematical Space U+205F  │
    // │  Ideographic Space         U+3000  │
    // ╰────────────────────────────────────╯
    _unicode_space: _ =>
      /[\u0009\u0020\u00A0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200A\u202F\u205F\u3000]/,

    // single-line-comment := '//' ^newline+ (newline | eof)
    single_line_comment: $ =>
      seq(
        '//',
        repeat(/[^\r\n\u0085\u000C\u2028\u2029]/),
        choice($._newline, $._eof),
      ),
  },
});
