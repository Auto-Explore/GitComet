/**
 * @file Caddyfile grammar for tree-sitter
 * @author Matthew Penner <me@matthewp.io>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const IPV4_REGEX = /((25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)/;

const IPV6_REGEX =
  // eslint-disable-next-line max-len
  /(([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))/;

const URL_SCHEME_REGEX = /[a-z]+:\/\//;

const PORT_REGEX = /[0-9]{1,5}/;

// Hostname regex, allows:
// - Domains (example.com)
// - Subdomains (sub.example.com)
// - Punycode (example.xn--ses554g)
//
// This regex does not match wildcards (*.com), or bare domains (localhost).
//
// The exclusion of bare domains from this regex is intentional to avoid
// matching arguments as hostnames, as there is no way to determine if
// something is a bare hostname or an argument without having knowledge
// of every single directive and the argument types it supports.
//
// If you want something like `localhost` or `my-hostname` to match, add a
// scheme so it can get matched as a network address.
const HOSTNAME_REGEX =
  // TODO: doesn't end with `-` validation?
  /([a-z][a-z0-9\-]+)(\.[a-z][a-z0-9\-]+)*(\.[a-z]{2,}|xn--[a-z0-9]+)/;

const BARE_HOSTNAME_REGEX =
  // TODO: doesn't end with `-` validation?
  /([a-z][a-z0-9\-]+)(\.[a-z][a-z0-9\-]+)*(\.[a-z]{2,}|xn--[a-z0-9]+)?/;

const subdirectiveFields = $ => [
  repeat(choice($.url, $.network_address, $.unix_socket, $.placeholder, $._string_literal, $.int_literal, $.argument)),
  choice($.block, token.immediate(/\r?\n/)),
];

const directiveFields = $ => [optional($.matcher), ...subdirectiveFields($)];

module.exports = grammar({
  name: 'caddyfile',

  extras: $ => [$.comment, /\s/],

  rules: {
    source_file: $ =>
      seq(
        // Allow a single "global options" block at the beginning of the file.
        optional($.global_options),

        // Allow multiple snippet definitions and/or named routes.
        repeat(choice($.snippet_definition, $.named_route)),

        // Allow site definitions, either a single site or multiple site blocks.
        optional($.sites),
      ),

    // Global options is a special block that only allows the use of directives.
    // Defining snippets or named matchers is not allowed in it's scope.
    //
    // https://caddyserver.com/docs/caddyfile/concepts#snippets
    global_options: $ => seq('{', token.immediate(/\r?\n/), repeat($.directive), '}'),

    // Snippets are re-usable parts of a caddyfile, the content of a snippet can be anything
    // that is allowed inside of a site block.
    //
    // Snippets must be defined outside of a site block and after global options (if present).
    snippet_name: _ => token(seq('(', /[a-zA-Z0-9\-_]+/, ')')),
    snippet_definition: $ => seq(field('name', $.snippet_name), $.block),

    // Experimental; named routes use a syntax similar to snippets.
    //
    // Named routes are a special block, defined outside of site blocks.
    //
    // https://caddyserver.com/docs/caddyfile/concepts#named-routes
    named_route_identifier: _ => token(seq('&(', /[a-zA-Z0-9\-_]+/, ')')),
    named_route: $ => seq(field('identifier', $.named_route_identifier), $.block),

    // Comment is available at the start (or during) a line that contains a # with preceding whitespace
    comment: _ => token(seq('#', /(#+(.|\r?\n)|[^#\n])*/)),

    // Argument is pretty much anything that isn't a matcher
    argument: _ => /[a-zA-Z\-_+.\\\/*]([a-zA-Z\-_+.\\\/*0-9]*)/,

    // Website URLs or a Unix Socket path
    url: _ =>
      // Needs prec of 1 or higher to avoid having a comment take precedence if
      // the URL contains a fragment (`#`).
      // TODO: figure out why URLs with a `#` result in slow parsing.
      token(
        prec(
          1,
          seq(
            'http',
            optional('s'),
            '://',
            choice(IPV4_REGEX, seq('[', IPV6_REGEX, ']'), BARE_HOSTNAME_REGEX),
            optional(PORT_REGEX),
            // Require at least a trailing slash to be considered a URL.
            //
            // This portion of the URL token is what separates a URL from a
            // network address.
            repeat1(seq('/', /([A-Za-z0-9\-_.~!&'\(\)*+,;=:#]|%[0-9a-fA-F]{2})*/)),
          ),
        ),
      ),

    // Placeholder is used for environment variables or runtime value substitution
    placeholder: $ => $._placeholder,
    _placeholder: $ => choice($.__placeholder, $._environment_variable),
    __placeholder: $ =>
      seq(
        '{',
        // TODO: this is probably not the best way to write this.
        //
        // You cannot nest placeholders in each other, but you can nest
        // environment variables. Environment variables are replaced separately
        // of placeholders when the Caddyfile is parsed which is how this works.
        //
        // The issue with this grammar is he environment variable can only be at
        // the end of the placeholder, but the Caddyfile parser allows there to
        // be any number of environment variables inside of a placeholder.
        token.immediate(/[a-zA-Z0-9][a-zA-Z0-9_.\[\]]*/),
        optional($._environment_variable),
        '}',
      ),
    // TODO: allow for `:<DEFAULT>`
    _environment_variable: _ => token(seq('{$', token.immediate(/[a-zA-Z0-9][a-zA-Z0-9_.\[\]]*/), '}')),

    // Directives
    directive_name: _ => /[a-zA-Z_\-+]+/,
    directive: $ => seq(field('name', $.directive_name), ...directiveFields($)),

    // https://caddyserver.com/docs/caddyfile/matchers#named-matchers
    matcher_name: _ => token(seq('@', /[a-zA-Z0-9\-_]+/)),
    named_matcher: $ => seq(field('name', $.matcher_name), ...subdirectiveFields($)),
    matcher: $ =>
      choice(
        // Allow a lone `*`
        '*',
        // Path matching
        token(seq(choice('/', '\\'), /([a-zA-Z0-9\-_%\\\/.]+)*(\*){0,1}/)),
        // Named matcher
        $.matcher_name,
      ),

    //
    // Sites
    //

    sites: $ => choice($.single_site, repeat1($.site_definition)),

    single_site: $ => seq(field('name', commaSep1($.site_address)), field('definitions', repeat($._definition))),

    site_definition: $ => seq(field('name', commaSep1($.site_address)), $.block),

    _definition: $ => choice($.directive, $.named_matcher),

    // Block is a site block that is allowed to define directives and named matchers.
    block: $ => seq('{', token.immediate(/\r?\n/), repeat($._definition), '}'),

    //
    // Literals
    //

    _string_literal: $ => choice($.raw_string_literal, $.interpreted_string_literal),

    // TODO: this will allow new-lines which we don't want to allow.
    raw_string_literal: _ => token(seq('`', repeat(/[^`]/), '`')),

    interpreted_string_literal: $ => seq('"', repeat(choice($._interpreted_string_literal_basic_content, $.escape_sequence)), token.immediate('"')),
    _interpreted_string_literal_basic_content: _ => token.immediate(prec(1, /[^"\n\\]+/)),

    escape_sequence: _ => token.immediate(seq('\\', choice(/[^xuU]/, /\d{2,3}/, /x[0-9a-fA-F]{2,}/, /u[0-9a-fA-F]{4}/, /U[0-9a-fA-F]{8}/))),

    int_literal: _ => token(choice('0', seq(/[1-9]/, repeat(/[0-9]/)))),

    //
    // Addresses
    //

    site_address: $ =>
      choice(
        // Bare protocols
        'http://',
        'https://',

        // Bare port
        token(seq(':', PORT_REGEX)),

        // Environment variable.
        //
        // According to the Caddy docs, placeholders cannot be used in addresses,
        // but you can use environment variables. `{$ENV_VAR}`, not `{env.ENV_VAR}`
        $._environment_variable,

        token(
          seq(
            optional(URL_SCHEME_REGEX),
            choice(
              IPV4_REGEX,
              seq('[', IPV6_REGEX, ']'),
              // Hostname regex, allows:
              // - Bare domains (localhost, my-system, etc),
              // - Domains (example.com)
              // - Subdomains (sub.example.com)
              // - Punycode (example.xn--ses554g)
              //
              // Also allows for a wildcard (*) as the furthest left subdomain.
              /(\*|[a-z][a-z0-9\-]+)(\.[a-z][a-z0-9\-]+)*(\.[a-z]{2,}|xn--[a-z0-9]+)?/,
            ),
            optional(seq(':', PORT_REGEX)),
          ),
        ),
      ),

    // TODO: do we want to merge this into network_address?
    unix_socket: _ => token(prec(2, seq(/[a-z][a-z+]*/, '//', /[a-zA-Z0-9_\-./*]+/))),

    // TODO: do we really want to use token and prec on each one of these
    // cases?
    network_address: _ =>
      choice(
        token(prec(1, seq(URL_SCHEME_REGEX, choice(IPV4_REGEX, seq('[', IPV6_REGEX, ']'), BARE_HOSTNAME_REGEX), optional(seq(':', PORT_REGEX))))),
        token(prec(2, seq(optional(URL_SCHEME_REGEX), choice(IPV4_REGEX, seq('[', IPV6_REGEX, ']'), HOSTNAME_REGEX), optional(seq(':', PORT_REGEX))))),
        token(prec(3, seq(BARE_HOSTNAME_REGEX, ':', PORT_REGEX))),
      ),
  },
});

/**
 * Creates a rule to optionally match one or more of the rules separated by a comma
 *
 * @param {Rule} rule
 *
 * @returns {ChoiceRule}
 */
// eslint-disable-next-line no-unused-vars
function commaSep(rule) {
  return optional(commaSep1(rule));
}

/**
 * Creates a rule to match one or more of the rules separated by a comma
 *
 * @param {Rule} rule
 *
 * @returns {SeqRule}
 */
function commaSep1(rule) {
  return seq(rule, repeat(seq(token.immediate(/, /), rule)));
}
