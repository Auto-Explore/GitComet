#include "tree_sitter/parser.h"

#include <wctype.h>

enum TokenType {
    DESCENDANT_OP,
    PSEUDO_CLASS_SELECTOR_COLON,
    // GitComet: a custom property's value, taken whole.
    CUSTOM_PROPERTY_VALUE,
    ERROR_RECOVERY,
};

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

void *tree_sitter_css_external_scanner_create() { return NULL; }

void tree_sitter_css_external_scanner_destroy(void *payload) {}

unsigned tree_sitter_css_external_scanner_serialize(void *payload, char *buffer) { return 0; }

void tree_sitter_css_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {}

// GitComet: scan a custom property's value as one opaque token.
//
// A CSS custom property takes any balanced token sequence, so its value cannot be
// parsed as ordinary CSS. This consumes to the `;` that ends the declaration --
// tracking `{}`, `()` and `[]` nesting and skipping strings, so a `;` inside any
// of those does not end it.
//
// It returns *false* unless the value actually contains a brace. Declining is what
// keeps this safe: `--brand: #3366ff` and `--empty: ;` never take this path, so
// every value ordinary CSS can already express keeps exactly the tree it had.
// Returning false costs nothing -- tree-sitter rewinds the lexer to where the
// token started. `mark_end` is called only on the path that returns true; calling
// it and then returning false leaves the lexer marked past input the parser still
// needs, and the token is silently never produced.
static bool scan_custom_property_value(TSLexer *lexer) {
    unsigned depth = 0;
    bool saw_brace = false;
    bool saw_content = false;

    while (lexer->lookahead) {
        int32_t c = lexer->lookahead;

        if (depth == 0 && (c == ';' || c == '}')) {
            break;
        }

        if (c == '"' || c == '\'') {
            int32_t quote = c;
            advance(lexer);
            while (lexer->lookahead && lexer->lookahead != quote) {
                if (lexer->lookahead == '\\') {
                    advance(lexer);
                    if (!lexer->lookahead) {
                        break;
                    }
                }
                advance(lexer);
            }
            if (lexer->lookahead == quote) {
                advance(lexer);
            }
            saw_content = true;
            continue;
        }

        if (c == '{' || c == '(' || c == '[') {
            if (c == '{') {
                saw_brace = true;
            }
            depth++;
        } else if (c == '}' || c == ')' || c == ']') {
            depth--;
        }

        if (!iswspace(c)) {
            saw_content = true;
        }
        advance(lexer);
    }

    if (!saw_brace || !saw_content) {
        return false;
    }

    lexer->mark_end(lexer);
    lexer->result_symbol = CUSTOM_PROPERTY_VALUE;
    return true;
}

bool tree_sitter_css_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    if (valid_symbols[ERROR_RECOVERY]) {
        return false;
    }

    // GitComet: only where the grammar says a custom property's value can start.
    if (valid_symbols[CUSTOM_PROPERTY_VALUE]) {
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        return scan_custom_property_value(lexer);
    }

    if (iswspace(lexer->lookahead) && valid_symbols[DESCENDANT_OP]) {
        lexer->result_symbol = DESCENDANT_OP;

        skip(lexer);
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        lexer->mark_end(lexer);

        if (lexer->lookahead == '#' || lexer->lookahead == '.' || lexer->lookahead == '[' || lexer->lookahead == '-' ||
            lexer->lookahead == '*' || iswalnum(lexer->lookahead)) {
            return true;
        }

        if (lexer->lookahead == ':') {
            advance(lexer);
            if (iswspace(lexer->lookahead)) {
                return false;
            }
            for (;;) {
                if (lexer->lookahead == ';' || lexer->lookahead == '}' || lexer->eof(lexer)) {
                    return false;
                }
                if (lexer->lookahead == '{') {
                    return true;
                }
                advance(lexer);
            }
        }
    }

    if (valid_symbols[PSEUDO_CLASS_SELECTOR_COLON]) {
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        if (lexer->lookahead == ':') {
            advance(lexer);
            if (lexer->lookahead == ':') {
                return false;
            }
            lexer->mark_end(lexer);
            lexer->result_symbol = PSEUDO_CLASS_SELECTOR_COLON;

            // We need a `{` to be a pseudo class selector, a `;` indicates a property.
            // This does not apply if we're in a comment, however.
            bool in_comment = false;
            while (lexer->lookahead != ';' && lexer->lookahead != '}' && !lexer->eof(lexer)) {
                advance(lexer);
                if (lexer->lookahead == '{' && !in_comment) {
                    return true;
                }
                if (lexer->lookahead == '/' && !in_comment) {
                    advance(lexer);
                    if (lexer->lookahead == '*') {
                        in_comment = true;
                    }
                } else if (lexer->lookahead == '*' && in_comment) {
                    advance(lexer);
                    if (lexer->lookahead == '/') {
                        in_comment = false;
                    }
                }
            }

            // If we're at eof, and we happened to *not* find an opening brace to indicate we have a pseudo class
            // selector, we should *still* return one at EOF. This will improve error recovery, and the malformed code
            // can be parsed as an erroneous pseudo-class selector, rather than an erroneous property.
            return lexer->eof(lexer);
        }
    }

    return false;
}
