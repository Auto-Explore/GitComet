#include "tree_sitter/parser.h"
#include "wctype.h"
#include <ctype.h>

enum { _EOF, MULTI_LINE_COMMENT };

void *tree_sitter_kdl_external_scanner_create() { return NULL; }

void tree_sitter_kdl_external_scanner_destroy(void *payload) {}

unsigned tree_sitter_kdl_external_scanner_serialize(void *payload,
                                                    char *buffer) {
    return 0;
}

void tree_sitter_kdl_external_scanner_deserialize(void       *payload,
                                                  const char *buffer,
                                                  unsigned    length) {}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

bool tree_sitter_kdl_external_scanner_scan(void *payload, TSLexer *lexer,
                                           const bool *valid_symbols) {
    // check for End-of-file
    if (valid_symbols[_EOF] && lexer->lookahead == 0) {
        lexer->result_symbol = _EOF;
        advance(lexer);
        return true;
    }

    // multi-line-comment := '/*' commented-block
    if (lexer->lookahead == '/') {
        advance(lexer);
        if (lexer->lookahead != '*')
            return false;
        advance(lexer);

        bool     after_star = false;
        unsigned nesting_depth = 1;
        // commented-block := '*/' | (multi-line-comment | '*' | '/' | [^*/]+)
        // commented-block
        for (;;) {
            switch (lexer->lookahead) {
                case '\0':
                    return false;
                case '*':
                    advance(lexer);
                    after_star = true;
                    break;
                case '/':
                    if (after_star) {
                        advance(lexer);
                        after_star = false;
                        nesting_depth--;
                        if (nesting_depth == 0) {
                            lexer->result_symbol = MULTI_LINE_COMMENT;
                            return true;
                        }
                    } else {
                        advance(lexer);
                        after_star = false;
                        if (lexer->lookahead == '*') {
                            nesting_depth++;
                            advance(lexer);
                        }
                    }
                    break;
                default:
                    advance(lexer);
                    after_star = false;
                    break;
            }
        }
    }

    return false;
}
