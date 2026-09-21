/**
 * @file KDL grammar for tree-sitter
 * @author Amaan Qureshi <contact@amaanq.com>
 * @license MIT
 * @see {@link https://kdl.dev|official website}
 * @see {@link https://github.com/kdl-org/kdl/blob/1.0.0/SPEC.md|KDL 1 syntax spec}
 * @see {@link https://github.com/kdl-org/kdl/blob/2.0.0/SPEC.md|KDL 2 syntax spec}
 */

// deno-lint-ignore-file no-control-regex
/* eslint-disable arrow-parens */
/* eslint-disable camelcase */
/* eslint-disable-next-line spaced-comment */
/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const BOOLEAN_KEYWORDS = ['true', 'false', '#true', '#false'];
const NULL_KEYWORDS = ['null', '#null'];
const KEYWORD_NUMBERS = ['#inf', '#-inf', '#nan'];
const UNICODE_SPACES = '\\u0009\\u0020\\u00A0\\u1680\\u2000-\\u200A\\u202F\\u205F\\u3000';
const NEWLINES = '\\r\\n\\u0085\\u000B\\u000C\\u2028\\u2029';
const DISALLOWED_LITERAL =
  '\\u0000-\\u0008\\u000E-\\u001F\\u007F\\u200E\\u200F\\u202A-\\u202E\\u2066-\\u2069\\uFEFF';
const IDENTIFIER_EXCLUSIONS = `${UNICODE_SPACES}${NEWLINES}${DISALLOWED_LITERAL}\\\\\\/(){};\\[\\]"=`;
const IDENTIFIER_CHAR = `[^${IDENTIFIER_EXCLUSIONS}]`;
const IDENTIFIER_START = `[^${IDENTIFIER_EXCLUSIONS}0-9+\\-]`;
const IDENTIFIER_START_AFTER_SIGN = `[^${IDENTIFIER_EXCLUSIONS}0-9]`;
const STRING_CHAR = `[^"\\\\${DISALLOWED_LITERAL}]`;
const SIGN = '[+-]';
// integer := digit (digit | '_')*
const DIGITS = '[0-9][0-9_]*';
// hex-unicode := hex-digit{1, 6} - surrogate - above-max-scalar
const HEX_CHARS = '0-9a-fA-F';
const HEX = `[${HEX_CHARS}]`;
const NON_SURROGATE_4 = `(?:[0-9a-cA-Ce-fE-F]${HEX}{3}|[dD][0-7]${HEX}{2})`;
const SCALAR_5 = `(?:[1-9a-fA-F]${HEX}{4}|0${NON_SURROGATE_4})`;
const SCALAR_6 = `(?:0${SCALAR_5}|10${HEX}{4})`;
const UNICODE_SCALAR_ESCAPE = new RegExp(
  `u\\{(?:${HEX}{1,3}|${NON_SURROGATE_4}|${SCALAR_5}|${SCALAR_6})\\}`,
);

/**
 * nodes := line-space* (node line-space*)*
 *
 * @param {GrammarSymbols<string>} $
 * @returns {SeqRule}
 */
function linespacedNodes($) {
  return seq(
    repeat($._linespace),
    repeat(seq($.node, repeat($._linespace))),
  );
}

/**
 * (node-space* slashdash node-children)* (node-space* node-children)?
 * (node-space* slashdash node-children)*
 *
 * Written with the trailing slashdashes nested inside the optional block, since
 * the flat spelling cannot say which repeat owns them when the block is absent.
 *
 * @param {GrammarSymbols<string>} $
 * @returns {SeqRule}
 */
function childBlocks($) {
  const commented = seq(repeat($._node_space), field('children', $.node_children_comment));
  const block = seq(repeat($._node_space), field('children', $.node_children));
  return seq(repeat(commented), optional(seq(block, repeat(commented))));
}

export default grammar({
  name: 'kdl',

  conflicts: $ => [
    [$.version, $._ws],
    [$.identifier, $.value],
  ],

  externals: $ => [
    $._eof,
    $.multi_line_comment,
    $._raw_string_start,
    $._raw_string_content,
    $._raw_string_end,
  ],

  extras: $ => [$.multi_line_comment],

  word: $ => $._normal_bare_identifier,

  rules: {
    // nodes := linespace* (node nodes?)? linespace*
    document: $ => seq(optional($._bom), optional(field('version', $.version)), linespacedNodes($)),

    version: $ => prec(2, seq(
      '/-',
      repeat($._unicode_space),
      'kdl-version',
      repeat1($._unicode_space),
      field('version', choice('1', '2')),
      repeat($._unicode_space),
      $._newline,
    )),

    // node := ('/-' node-space*)? type? identifier (node-space+ node-prop-or-arg)* (node-space* node-children ws*)? node-space* node-terminator
    node: $ => seq($._base_node, $._node_terminator),
    _final_node: $ => seq(alias($._base_node, $.node), '}'),
    _base_node: $ =>
      seq(
        optional($.node_comment),
        optional(seq(field('type', $.type), repeat($._node_space))),
        field('name', $.identifier),
        repeat($.node_field),
        childBlocks($),
        repeat($._node_space),
      ),

    // node-prop-or-arg (field) := ('/-' node-space*)? (prop | value)
    node_field: $ => choice(
      seq(repeat1($._node_space), $._node_field),
      seq(repeat($._node_space), $.node_field_comment),
    ),
    node_comment: $ => seq('/-', repeat($._linespace)),
    node_field_comment: $ => seq('/-', repeat($._linespace), $._node_field),
    _node_field: $ => choice(field('property', $.prop), field('argument', $.value)),
    // node-children := ('/-' node-space*)? '{' nodes '}'
    node_children_comment: $ => seq('/-', repeat($._linespace), $.node_children),
    node_children: $ => seq(
      '{',
      repeat(choice($._linespace, $.node)),
      choice('}', $._final_node),
    ),
    // node-space := ws* escline ws* | ws+
    _node_space: $ => choice($._ws, $._escline),
    // node-terminator := single-line-comment | newline | ';' | eof
    _node_terminator: $ =>
      choice($.single_line_comment, $._newline, ';', $._eof),

    // identifier := string | bare-identifier
    identifier: $ => choice($.string, $._bare_identifier),

    // bare-identifier := ((identifier-char - digit - sign) identifier-char* | sign ((identifier-char - digit) identifier-char*)?) - keyword
    _bare_identifier: $ => choice($._normal_bare_identifier, $._signed_bare_identifier),

    _normal_bare_identifier: _ =>
      token(new RegExp(`${IDENTIFIER_START}${IDENTIFIER_CHAR}*`)),

    _signed_bare_identifier: _ =>
      token(new RegExp(`${SIGN}(?:${IDENTIFIER_START_AFTER_SIGN}${IDENTIFIER_CHAR}*)?`)),

    keyword: $ => choice($.boolean, ...NULL_KEYWORDS.map(value => token(prec(2, value)))),

    prop: $ => seq(
      field('key', $.identifier),
      repeat($._node_space),
      '=',
      repeat($._node_space),
      field('value', $.value),
    ),

    // value := type? (string | number | keyword)
    value: $ => seq(
      optional(seq(field('type', $.type), repeat($._node_space))),
      field('value', choice($.string, alias($._bare_identifier, $.string), $.number, $.keyword)),
    ),

    // type := '(' identifier ')'
    type: $ => seq(
      '(',
      repeat($._node_space),
      field('name', $.identifier),
      repeat($._node_space),
      ')',
    ),

    // String
    string: $ => choice($._raw_string, $.multi_line_string, $._escaped_string),
    // raw-string := '#' raw-string-quotes '#' | '#' raw-string '#'
    _raw_string: $ => seq(
      $._raw_string_start,
      optional(alias($._raw_string_content, $.string_fragment)),
      $._raw_string_end,
    ),
    // quoted-string := '"""' newline (multi-line-string-body newline)?
    //                  (unicode-space | ws-escape)* '"""'
    multi_line_string: $ => seq(
      $._multiline_open,
      repeat(choice(
        alias($._multiline_fragment, $.string_fragment),
        $.escape,
        $.escaped_whitespace,
      )),
      '"""',
    ),
    _multiline_open: _ => token(seq('"""', choice('\r\n', new RegExp(`[${NEWLINES}]`)))),
    // multi-line-string-body := ('"' ^'"' | '""' ^'"' | string-character)*?
    _multiline_fragment: _ =>
      token.immediate(new RegExp(`(?:${STRING_CHAR}|"${STRING_CHAR}|""${STRING_CHAR})+`)),
    // escaped-string := '"' character* '"'
    _escaped_string: $ => seq(
      '"',
      repeat(choice(
        alias($._string_fragment, $.string_fragment),
        $.escape,
        $.escaped_whitespace,
      )),
      '"',
    ),
    // character := '\' escape | [^\"]
    _string_fragment: _ => token.immediate(new RegExp(`${STRING_CHAR}+`)),
    // escape := ["\\/bfnrts] | 'u{' hex-digit{1, 6} '}'
    escape: _ => token.immediate(seq(
      '\\',
      choice(
        '\\',
        '"',
        '/',
        'b',
        'f',
        'n',
        'r',
        't',
        's',
        UNICODE_SCALAR_ESCAPE,
      ),
    )),
    escaped_whitespace: _ => token.immediate(
      /\\(?:\r\n|[\u0009\u0020\u00A0\u1680\u2000-\u200A\u202F\u205F\u3000\r\n\u0085\u000B\u000C\u2028\u2029])+/,
    ),
    number: $ => choice($.keyword_number, $._decimal, $._hex, $._octal, $._binary),

    // decimal := sign? integer ('.' integer)? exponent?
    _decimal: $ =>
      seq(
        $._integer,
        optional(seq('.', alias($._fraction, $.decimal))),
        optional(alias($._exponent, $.exponent)),
      ),

    // integer := digit (digit | '_')*
    _integer: _ => token(new RegExp(`${SIGN}?${DIGITS}`)),
    _fraction: _ => token.immediate(new RegExp(DIGITS)),
    // exponent := ('e' | 'E') sign? integer
    _exponent: _ => token.immediate(new RegExp(`[eE]${SIGN}?${DIGITS}`)),
    // hex := sign? '0x' hex-digit (hex-digit | '_')*
    _hex: _ => token(new RegExp(`${SIGN}?0x${HEX}[${HEX_CHARS}_]*`)),
    // octal := sign? '0o' [0-7] [0-7_]*
    _octal: _ => token(new RegExp(`${SIGN}?0o[0-7][0-7_]*`)),
    // binary := sign? '0b' ('0' | '1') ('0' | '1' | '_')*
    _binary: _ => token(new RegExp(`${SIGN}?0b[01][01_]*`)),

    keyword_number: _ => choice(...KEYWORD_NUMBERS.map(value => token(prec(2, value)))),

    boolean: _ => choice(...BOOLEAN_KEYWORDS.map(value => token(prec(2, value)))),

    // escline := '\\' ws* (single-line-comment | newline)
    _escline: $ => seq('\\', repeat($._ws), choice($.single_line_comment, $._newline, $._eof)),

    // linespace := newline | ws | single-line-comment
    _linespace: $ => choice($._ws, $._escline, $._newline, $.single_line_comment),

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
    _newline: _ => choice(/\r\n/, /\r/, /\n/, /\u0085/, /\u000B/, /\u000C/, /\u2028/, /\u2029/),

    // ws := bom | unicode-space | multi-line-comment
    _ws: $ => choice($._unicode_space, $.multi_line_comment),

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
        repeat(/[^\r\n\u0085\u000B\u000C\u2028\u2029]/),
        choice($._newline, $._eof),
      ),
  },
});
