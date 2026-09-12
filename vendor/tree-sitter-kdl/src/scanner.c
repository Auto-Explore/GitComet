#include "tree_sitter/parser.h"

#include <stdlib.h>
#include <string.h>

enum {
    _EOF,
    MULTI_LINE_COMMENT,
    RAW_STRING_START,
    RAW_STRING_CONTENT,
    RAW_STRING_END,
};

typedef struct {
    uint32_t hash_count;
    bool multiline;
    bool allow_newlines;
} Scanner;

void *tree_sitter_kdl_external_scanner_create(void) { return calloc(1, sizeof(Scanner)); }

void tree_sitter_kdl_external_scanner_destroy(void *payload) { free(payload); }

unsigned tree_sitter_kdl_external_scanner_serialize(void *payload, char *buffer) {
    Scanner *scanner = payload;
    buffer[0] = (char)(scanner->multiline | (scanner->allow_newlines << 1));
    memcpy(buffer + 1, &scanner->hash_count, sizeof(scanner->hash_count));
    return 1 + sizeof(scanner->hash_count);
}

void tree_sitter_kdl_external_scanner_deserialize(void *payload, const char *buffer,
                                                  unsigned length) {
    Scanner *scanner = payload;
    *scanner = (Scanner){0};
    if (length != 1 + sizeof(scanner->hash_count)) return;

    scanner->multiline = buffer[0] & 1;
    scanner->allow_newlines = buffer[0] & 2;
    memcpy(&scanner->hash_count, buffer + 1, sizeof(scanner->hash_count));
}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static bool is_newline_start(int32_t c) {
    return c == '\r' || c == '\n' || c == 0x0085 || c == 0x000B || c == 0x000C ||
           c == 0x2028 || c == 0x2029;
}

static void advance_newline(TSLexer *lexer) {
    bool was_carriage_return = lexer->lookahead == '\r';
    advance(lexer);
    if (was_carriage_return && lexer->lookahead == '\n') advance(lexer);
}

static bool consume_hashes(TSLexer *lexer, uint32_t hash_count) {
    for (uint32_t index = 0; index < hash_count; index++) {
        if (lexer->lookahead != '#') return false;
        advance(lexer);
    }
    return true;
}

static bool consume_quotes(TSLexer *lexer, unsigned quotes_needed) {
    for (unsigned index = 0; index < quotes_needed; index++) {
        if (lexer->lookahead != '"') return false;
        advance(lexer);
    }
    return true;
}

static bool scan_raw_start(Scanner *scanner, TSLexer *lexer) {
    bool legacy = lexer->lookahead == 'r';
    if (legacy) advance(lexer);

    uint32_t hash_count = 0;
    while (lexer->lookahead == '#' && hash_count < UINT32_MAX) {
        hash_count++;
        advance(lexer);
    }

    if ((!legacy && hash_count == 0) || lexer->lookahead != '"') return false;
    advance(lexer);
    lexer->mark_end(lexer);

    bool multiline = false;
    if (lexer->lookahead == '"') {
        advance(lexer);
        if (lexer->lookahead == '"') {
            advance(lexer);
            if (!is_newline_start(lexer->lookahead)) return false;
            advance_newline(lexer);
            lexer->mark_end(lexer);
            multiline = true;
        }
    }

    *scanner = (Scanner){
        .hash_count = hash_count,
        .multiline = multiline,
        .allow_newlines = legacy || multiline,
    };
    lexer->result_symbol = RAW_STRING_START;
    return true;
}

static bool scan_raw_body(Scanner *scanner, TSLexer *lexer, const bool *valid_symbols) {
    unsigned quotes_needed = scanner->multiline ? 3 : 1;
    bool has_content = false;
    lexer->mark_end(lexer);

    for (;;) {
        if (lexer->eof(lexer)) return false;
        if (!scanner->allow_newlines && is_newline_start(lexer->lookahead)) return false;

        if (lexer->lookahead == '"') {
            if (consume_quotes(lexer, quotes_needed) &&
                consume_hashes(lexer, scanner->hash_count)) {
                if (has_content) {
                    if (!valid_symbols[RAW_STRING_CONTENT]) return false;
                    lexer->result_symbol = RAW_STRING_CONTENT;
                    return true;
                }
                if (!valid_symbols[RAW_STRING_END]) return false;
                lexer->mark_end(lexer);
                *scanner = (Scanner){0};
                lexer->result_symbol = RAW_STRING_END;
                return true;
            }
        } else {
            advance(lexer);
        }

        has_content = true;
        lexer->mark_end(lexer);
    }
}

// multi-line-comment := '/*' commented-block
static bool scan_multi_line_comment(TSLexer *lexer) {
    advance(lexer);
    if (lexer->lookahead != '*') return false;
    advance(lexer);

    for (unsigned depth = 1; depth > 0;) {
        if (lexer->eof(lexer)) return false;
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == '/' && lexer->lookahead == '*') {
            depth++;
            advance(lexer);
        } else if (c == '*' && lexer->lookahead == '/') {
            depth--;
            advance(lexer);
        }
    }

    lexer->result_symbol = MULTI_LINE_COMMENT;
    return true;
}

bool tree_sitter_kdl_external_scanner_scan(void *payload, TSLexer *lexer,
                                           const bool *valid_symbols) {
    Scanner *scanner = payload;

    if (valid_symbols[RAW_STRING_CONTENT] || valid_symbols[RAW_STRING_END]) {
        return scan_raw_body(scanner, lexer, valid_symbols);
    }

    if (valid_symbols[_EOF] && lexer->eof(lexer)) {
        lexer->result_symbol = _EOF;
        return true;
    }

    if (valid_symbols[RAW_STRING_START] &&
        (lexer->lookahead == 'r' || lexer->lookahead == '#')) {
        return scan_raw_start(scanner, lexer);
    }

    if (valid_symbols[MULTI_LINE_COMMENT] && lexer->lookahead == '/') {
        return scan_multi_line_comment(lexer);
    }

    return false;
}
