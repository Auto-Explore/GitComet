#include "tree_sitter/parser.h"

enum TokenType
{
    BLOCK_COMMENT_CONTENT
};

void*
tree_sitter_dhall_external_scanner_create()
{
    return NULL;
}

void
tree_sitter_dhall_external_scanner_destroy(void* payload)
{}

unsigned
tree_sitter_dhall_external_scanner_serialize(void* payload, char* buffer)
{
    return 0;
}

void
tree_sitter_dhall_external_scanner_deserialize(void* payload,
    const char* buffer, unsigned length)
{}

bool
tree_sitter_dhall_external_scanner_scan(void* payload, TSLexer* lexer,
    const bool* valid_symbols)
{
    if (valid_symbols[BLOCK_COMMENT_CONTENT]) {
        int depth = 1;

        while (depth > 0) {
            if (lexer->lookahead == '{') {
                lexer->advance(lexer, false);

                if (lexer->lookahead == '-') {
                    lexer->advance(lexer, false);
                    depth += 1;
                }
            } else if (lexer->lookahead == '-') {
                lexer->mark_end(lexer);
                lexer->advance(lexer, false);

                if (lexer->lookahead == '}') {
                    depth -= 1;
                }
            } else if (lexer->lookahead == 0) {
                return false;
            } else {
                lexer->advance(lexer, false);
            }
        }

        lexer->result_symbol = BLOCK_COMMENT_CONTENT;
        return true;
    }

    return false;
}
