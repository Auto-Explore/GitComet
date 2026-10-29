#include "tree_sitter/alloc.h"
#include "tree_sitter/array.h"
#include "tree_sitter/parser.h"

#include <string.h>
#include "identifier_ranges.h"

enum TokenType {
    OPT_SEMI,
    INTERPOLATION_REGULAR_START,
    INTERPOLATION_VERBATIM_START,
    INTERPOLATION_RAW_START,
    INTERPOLATION_START_QUOTE,
    INTERPOLATION_END_QUOTE,
    INTERPOLATION_OPEN_BRACE,
    INTERPOLATION_CLOSE_BRACE,
    INTERPOLATION_STRING_CONTENT,
    RAW_STRING_START,
    RAW_STRING_END,
    RAW_STRING_CONTENT,
    LAMBDA_PAREN_OPEN,
    IMPLICIT_DOT, IMPLICIT_PAREN, IMPLICIT_BRACKET, IMPLICIT_QUESTION,
    SCRIPT_TEXT, STYLE_TEXT, COMMENT_TEXT, LINE_TEXT, LINE_END,
    VOID_TAG_NAME, SCRIPT_TAG_NAME, STYLE_TAG_NAME,
    RAZOR_IF_START, RAZOR_FOR_START, RAZOR_FOREACH_START, RAZOR_WHILE_START, RAZOR_DO_START, RAZOR_SWITCH_START, RAZOR_TRY_START, RAZOR_USING_START, RAZOR_LOCK_START, RAZOR_CODE_START, HTML_TEXT,
    RAZOR_ELSE_CONTINUATION, RAZOR_CATCH_CONTINUATION, RAZOR_FINALLY_CONTINUATION, RAZOR_WHILE_CONTINUATION,
};

typedef enum {
    REGULAR = 1 << 0,
    VERBATIM = 1 << 1,
    RAW = 1 << 2,
} StringType;

typedef struct {
    uint8_t dollar_count;
    uint8_t open_brace_count;
    uint8_t quote_count;
    StringType string_type;
} Interpolation;

static inline bool is_regular(Interpolation *interpolation) { return interpolation->string_type & REGULAR; }

static inline bool is_verbatim(Interpolation *interpolation) { return interpolation->string_type & VERBATIM; }

static inline bool is_raw(Interpolation *interpolation) { return interpolation->string_type & RAW; }

typedef struct {
    uint8_t quote_count;
    Array(Interpolation) interpolation_stack;
} Scanner;

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

// ---- Helpers for LAMBDA_PAREN_OPEN scanning ---------------------------

static inline bool is_id_start(int32_t c) {
    return set_contains(razor_identifier_start_ranges, sizeof(razor_identifier_start_ranges) / sizeof(*razor_identifier_start_ranges), c);
}

static inline bool is_id_continue(int32_t c) {
    return set_contains(razor_identifier_continue_ranges, sizeof(razor_identifier_continue_ranges) / sizeof(*razor_identifier_continue_ranges), c);
}

static bool razor_space(int32_t c) {
    return (c >= 9 && c <= 13) || c == 32 || c == 0x85 || c == 0xa0 || c == 0x1680 ||
           (c >= 0x2000 && c <= 0x200a) || c == 0x2028 || c == 0x2029 || c == 0x202f ||
           c == 0x205f || c == 0x3000 || c == 0xfeff;
}

// Skip whitespace, line comments (//...), and block comments (/* ... */).
// Does NOT skip preprocessor directives — simple-lambda parameter lists
// cannot legally contain them.
static void skip_ws_and_comments(TSLexer *lexer) {
    for (;;) {
        int32_t c = lexer->lookahead;
        if (razor_space(c)) {
            advance(lexer);
        } else if (c == '/') {
            advance(lexer);
            if (lexer->lookahead == '/') {
                while (lexer->lookahead != 0 && lexer->lookahead != '\n') {
                    advance(lexer);
                }
            } else if (lexer->lookahead == '*') {
                advance(lexer);
                int32_t prev = 0;
                while (lexer->lookahead != 0 &&
                       !(prev == '*' && lexer->lookahead == '/')) {
                    prev = lexer->lookahead;
                    advance(lexer);
                }
                if (lexer->lookahead == '/') advance(lexer);
            } else {
                // a stray '/' isn't valid in a param list; bail out by
                // making the caller see something unexpected
                return;
            }
        } else {
            return;
        }
    }
}

// Consume an identifier into a fixed-size buffer. Returns the actual
// length consumed (may exceed `max`, in which case `buf` is only filled
// up to `max` bytes — useful for "too long to be a keyword" detection).
static size_t consume_identifier_into(TSLexer *lexer, char *buf, size_t max, int32_t *last) {
    size_t n = 0;
    while (is_id_continue(lexer->lookahead)) {
        if (n < max) {
            buf[n] = lexer->lookahead <= 127 ? (char)lexer->lookahead : 127;
        }
        n++;
        if (last) *last = lexer->lookahead;
        advance(lexer);
    }
    return n;
}

// True iff the first `n` bytes of `buf` equal the C string `kw`.
static bool buf_equals(const char *buf, size_t n, const char *kw) {
    size_t klen = strlen(kw);
    return n == klen && memcmp(buf, kw, klen) == 0;
}

void *tree_sitter_razor_external_scanner_create() {
    Scanner *scanner = ts_calloc(1, sizeof(Scanner));
    array_init(&scanner->interpolation_stack);
    return scanner;
}

void tree_sitter_razor_external_scanner_destroy(void *payload) {
    Scanner *scanner = (Scanner *)payload;
    array_delete(&scanner->interpolation_stack);
    ts_free(scanner);
}

unsigned tree_sitter_razor_external_scanner_serialize(void *payload, char *buffer) {
    Scanner *scanner = (Scanner *)payload;

    if (scanner->interpolation_stack.size * 4 + 2 > TREE_SITTER_SERIALIZATION_BUFFER_SIZE) {
        return 0;
    }

    unsigned size = 0;

    buffer[size++] = (char)scanner->quote_count;
    buffer[size++] = (char)scanner->interpolation_stack.size;

    for (unsigned i = 0; i < scanner->interpolation_stack.size; i++) {
        Interpolation interpolation = scanner->interpolation_stack.contents[i];
        buffer[size++] = (char)interpolation.dollar_count;
        buffer[size++] = (char)interpolation.open_brace_count;
        buffer[size++] = (char)interpolation.quote_count;
        buffer[size++] = (char)interpolation.string_type;
    }

    return size;
}

void tree_sitter_razor_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
    Scanner *scanner = (Scanner *)payload;

    scanner->quote_count = 0;
    array_clear(&scanner->interpolation_stack);
    unsigned size = 0;

    if (length > 0) {
        scanner->quote_count = (unsigned char)buffer[size++];
        scanner->interpolation_stack.size = (unsigned char)buffer[size++];
        array_reserve(&scanner->interpolation_stack, scanner->interpolation_stack.size);

        for (unsigned i = 0; i < scanner->interpolation_stack.size; i++) {
            Interpolation interpolation = {0};
            interpolation.dollar_count = buffer[size++];
            interpolation.open_brace_count = buffer[size++];
            interpolation.quote_count = buffer[size++];
            interpolation.string_type = (unsigned char)buffer[size++];
            scanner->interpolation_stack.contents[i] = interpolation;
        }
    }

    assert(size == length);
}

// Outcome of `scan_lambda_paren_open` — distinguishes "didn't start"
// (lexer untouched, wrapper can fall through to other token handlers)
// from "started and failed" (lexer cursor advanced past characters
// the wrapper must not let other handlers consume — they would emit
// tokens with wrong spans, e.g. an `INTERPOLATION_REGULAR_START`
// span covering `(false, $` in `return (false, $"...");`).
typedef enum {
    LAMBDA_SCAN_NO_PAREN,         // not a '(', no advance() done
    LAMBDA_SCAN_FAILED_AFTER_PAREN, // '(' was consumed, dirty state
    LAMBDA_SCAN_SUCCESS,
} LambdaScanResult;

// Try to recognize a C# 14 simple-lambda parameter list at the current
// position. On SUCCESS, `lexer->result_symbol` is LAMBDA_PAREN_OPEN
// and `mark_end` is set on the opening '('. On NO_PAREN, the lexer is
// untouched (only `skip(lexer)` over leading whitespace, which doesn't
// participate in token boundaries). On FAILED_AFTER_PAREN, the lexer
// has advanced past at least the opening '(' and the caller must
// surface this as a false return from the external_scanner_scan
// function so tree-sitter rewinds.
//
// Grammar of the pattern (after the opening '('):
//
//   element (',' element)* ')' '=>'
//
//   element  := modifier+ identifier
//             | identifier
//
//   modifier ∈ { scoped, ref, out, in, readonly }
//
// At least one element must carry a modifier; otherwise we'd over-fire
// on plain `(x, y) => ...` which is already handled by the existing
// `_lambda_parameters` choice.
//
// Implementation note: identifier-tokens are consumed *whole* into a
// small buffer before being classified as modifier-or-name. A
// character-by-character probe against each candidate keyword would be
// unworkable because `ref` is a prefix of `readonly` (they share `re`)
// and TSLexer has no rewind primitive: any probe order leaves one of
// the two keywords unreachable. Buffering the whole identifier
// sidesteps the prefix conflict entirely.
static LambdaScanResult scan_lambda_paren_open(TSLexer *lexer) {
    // External scanners run before whitespace `extras` are skipped, so
    // we need to skip leading whitespace/comments ourselves before
    // checking for the opening '('. `skip(lexer)` consumes the char as
    // an extra (not part of any token), so even on NO_PAREN return the
    // wrapper's fall-through into other handlers is safe — those
    // handlers do their own whitespace skipping.
    while (razor_space(lexer->lookahead)) {
        skip(lexer);
    }
    if (lexer->lookahead != '(') return LAMBDA_SCAN_NO_PAREN;
    advance(lexer);
    lexer->mark_end(lexer);

    // From this point on, the lexer cursor has moved past '('. Any
    // return must be FAILED_AFTER_PAREN unless we reach SUCCESS.
    #define BAIL return LAMBDA_SCAN_FAILED_AFTER_PAREN

    // We require at least one "hard" modifier (ref/out/in/readonly) across
    // the whole list before committing. `scoped` alone is not a valid C#
    // parameter modifier — it must always combine with a hard modifier —
    // and `scoped` is also a legal type name, so accepting `(scoped x) =>`
    // as a modifier+name pair would collide with the existing
    // `parameter_list` path on input that semantically means
    // type+identifier.
    bool saw_hard_modifier = false;
    bool expecting_element = true;

    for (;;) {
        skip_ws_and_comments(lexer);
        int32_t c = lexer->lookahead;

        if (c == 0) BAIL;   // EOF mid-list

        if (c == ')') {
            advance(lexer);
            skip_ws_and_comments(lexer);
            if (!saw_hard_modifier) BAIL;
            if (lexer->lookahead != '=') BAIL;
            advance(lexer);
            if (lexer->lookahead != '>') BAIL;
            lexer->result_symbol = LAMBDA_PAREN_OPEN;
            return LAMBDA_SCAN_SUCCESS;
        }

        if (!expecting_element) {
            if (c != ',') BAIL;
            advance(lexer);
            expecting_element = true;
            continue;
        }

        // Consume identifier-tokens in this element. Each one is either a
        // parameter modifier (`scoped`/`ref`/`out`/`in`/`readonly`) — in which
        // case we continue looking for more — or the parameter name, which
        // ends the element. Consuming the whole token before classifying
        // avoids any need to rewind the lexer on a prefix-conflict (e.g. ref
        // vs readonly).
        bool consumed_name = false;
        while (!consumed_name) {
            skip_ws_and_comments(lexer);
            if (!is_id_start(lexer->lookahead)) BAIL;

            char buf[9];  // "readonly" is 8 chars
            size_t n = consume_identifier_into(lexer, buf, sizeof(buf), NULL);

            bool is_hard_modifier = (n <= 8) && (
                buf_equals(buf, n, "ref") ||
                buf_equals(buf, n, "out") ||
                buf_equals(buf, n, "in") ||
                buf_equals(buf, n, "readonly")
            );
            bool is_soft_modifier = (n == 6) && buf_equals(buf, n, "scoped");

            if (is_hard_modifier) {
                saw_hard_modifier = true;
            } else if (is_soft_modifier) {
                // `scoped` is a modifier only when followed by a hard
                // modifier; keep scanning. If the element turns out to
                // end with `scoped <identifier>` and no hard modifier
                // appeared, the final `saw_hard_modifier` check fails
                // and we fall back to the regular parameter_list path.
            } else {
                consumed_name = true;
            }
        }
        expecting_element = false;
    }
}
#undef BAIL

static bool razor_identifier_start(int32_t c) {
    return is_id_start(c);
}

typedef enum { CONTINUATION_NONE, CONTINUATION_MATCH, CONTINUATION_PREFIX } ContinuationResult;

static ContinuationResult scan_implicit_continuation(TSLexer *lexer, const bool *valid_symbols) {
    const int32_t c = lexer->lookahead;
    if (c == '(' && valid_symbols[IMPLICIT_PAREN]) {
        advance(lexer);
        lexer->result_symbol = IMPLICIT_PAREN;
        return CONTINUATION_MATCH;
    }
    if (c == '[' && valid_symbols[IMPLICIT_BRACKET]) {
        advance(lexer);
        lexer->result_symbol = IMPLICIT_BRACKET;
        return CONTINUATION_MATCH;
    }
    if (c == '.' && valid_symbols[IMPLICIT_DOT]) {
        advance(lexer);
        lexer->mark_end(lexer);
        if (!razor_identifier_start(lexer->lookahead)) return CONTINUATION_PREFIX;
        lexer->result_symbol = IMPLICIT_DOT;
        return CONTINUATION_MATCH;
    }
    if (c == '?' && valid_symbols[IMPLICIT_QUESTION]) {
        advance(lexer);
        lexer->mark_end(lexer);
        if (lexer->lookahead != '.' && lexer->lookahead != '[') return CONTINUATION_PREFIX;
        lexer->result_symbol = IMPLICIT_QUESTION;
        return CONTINUATION_MATCH;
    }
    return CONTINUATION_NONE;
}

static bool scan_markup_text(TSLexer *lexer, enum TokenType type, bool consumed, int32_t previous) {
    lexer->mark_end(lexer);
    while (!lexer->eof(lexer)) {
        int32_t c = lexer->lookahead;
        if (type == LINE_TEXT && (c == '\r' || c == '\n')) break;
        if (type == HTML_TEXT && c == '<') break;
        if (c == '@') {
            advance(lexer);
            if (!is_id_continue(previous) || !is_id_continue(lexer->lookahead)) break;
        } else if (type != LINE_TEXT && type != HTML_TEXT && c == (type == COMMENT_TEXT ? '-' : '<')) {
            const char *ending = type == SCRIPT_TEXT ? "</script" : type == STYLE_TEXT ? "</style" : "-->";
            bool matched = true;
            for (const char *p = ending; *p; p++) {
                int32_t actual = lexer->lookahead;
                if (actual >= 'A' && actual <= 'Z') actual += 'a' - 'A';
                if (actual != *p) { matched = false; break; }
                previous = lexer->lookahead;
                advance(lexer);
            }
            if (matched && (type == COMMENT_TEXT || lexer->lookahead == '>' || razor_space(lexer->lookahead))) break;
            lexer->mark_end(lexer);
            consumed = true;
            if (lexer->eof(lexer)) { lexer->mark_end(lexer); consumed = true; break; }
            continue;
        } else {
            advance(lexer);
        }
        previous = c;
        consumed = true;
        lexer->mark_end(lexer);
    }
    lexer->result_symbol = type;
    return consumed;
}

static bool scan_tag_name(TSLexer *lexer, const bool *valid_symbols) {
    char name[8];
    unsigned size = 0;
    bool qualified = false;
    while ((lexer->lookahead >= 'a' && lexer->lookahead <= 'z') ||
           (lexer->lookahead >= 'A' && lexer->lookahead <= 'Z') ||
           (lexer->lookahead >= '0' && lexer->lookahead <= '9') ||
           lexer->lookahead == '_' || lexer->lookahead == '-' || lexer->lookahead == '.' || lexer->lookahead == ':') {
        int32_t c = lexer->lookahead;
        advance(lexer);
        if (c == ':') { size = 0; qualified = true; continue; }
        if (c >= 'A' && c <= 'Z') c += 'a' - 'A';
        if (size < sizeof(name) - 1) name[size] = (char)c;
        size++;
    }
    if (size >= sizeof(name)) return false;
    name[size] = 0;
    if (!qualified && valid_symbols[SCRIPT_TAG_NAME] && strcmp(name, "script") == 0) {
        lexer->result_symbol = SCRIPT_TAG_NAME;
        return true;
    }
    if (!qualified && valid_symbols[STYLE_TAG_NAME] && strcmp(name, "style") == 0) {
        lexer->result_symbol = STYLE_TAG_NAME;
        return true;
    }
    if (valid_symbols[VOID_TAG_NAME]) {
        const char *names[] = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"};
        for (unsigned i = 0; i < sizeof(names) / sizeof(*names); i++) {
            if (strcmp(name, names[i]) == 0) {
                lexer->result_symbol = VOID_TAG_NAME;
                return true;
            }
        }
    }
    return false;
}

static bool scan_control_start(Scanner *scanner, TSLexer *lexer, const bool *valid_symbols) {
    if (lexer->lookahead != '@') return false;
    advance(lexer);
    if (lexer->lookahead == '$' && valid_symbols[INTERPOLATION_VERBATIM_START]) {
        advance(lexer);
        if (lexer->lookahead != '"') return false;
        lexer->mark_end(lexer);
        Interpolation interpolation = { .dollar_count = 1, .string_type = VERBATIM | REGULAR };
        array_push(&scanner->interpolation_stack, interpolation);
        lexer->result_symbol = INTERPOLATION_VERBATIM_START;
        return true;
    }
    char word[10];
    unsigned size = 0;
    while (is_id_continue(lexer->lookahead)) {
        if (size < sizeof(word) - 1) word[size] = lexer->lookahead <= 127 ? (char)lexer->lookahead : 127;
        size++;
        advance(lexer);
    }
    if (size >= sizeof(word)) return false;
    word[size] = 0;
    if (strcmp(word, "await") == 0) {
        if (!razor_space(lexer->lookahead)) return false;
        while (razor_space(lexer->lookahead)) advance(lexer);
        size = 0;
        while (is_id_continue(lexer->lookahead)) {
            if (size < sizeof(word) - 1) word[size] = lexer->lookahead <= 127 ? (char)lexer->lookahead : 127;
            size++;
            advance(lexer);
        }
        if (size >= sizeof(word)) return false;
        word[size] = 0;
        if (strcmp(word, "foreach") != 0 && strcmp(word, "using") != 0) return false;
    }
    lexer->mark_end(lexer);
    skip_ws_and_comments(lexer);
    const char *names[] = {"if", "for", "foreach", "while", "do", "switch", "try", "using", "lock", "code"};
    for (unsigned i = 0; i < sizeof(names) / sizeof(*names); i++) {
        enum TokenType type = RAZOR_IF_START + i;
        bool block = type == RAZOR_DO_START || type == RAZOR_TRY_START || type == RAZOR_CODE_START;
        if (valid_symbols[type] && (strcmp(word, names[i]) == 0 || (type == RAZOR_CODE_START && strcmp(word, "functions") == 0)) && lexer->lookahead == (block ? '{' : '(')) {
            lexer->result_symbol = type;
            return true;
        }
    }
    return false;
}

static bool scan_control_continuation(TSLexer *lexer, const bool *valid_symbols) {
    char word[8];
    int32_t previous = 0;
    const size_t size = consume_identifier_into(lexer, word, sizeof(word), &previous);
    const char *names[] = {"else", "catch", "finally", "while"};
    for (unsigned i = 0; i < sizeof(names) / sizeof(*names); i++) {
        enum TokenType type = RAZOR_ELSE_CONTINUATION + i;
        if (valid_symbols[type] && buf_equals(word, size, names[i])) {
            lexer->result_symbol = type;
            return true;
        }
    }
    if (valid_symbols[HTML_TEXT]) return scan_markup_text(lexer, HTML_TEXT, size > 0, previous);
    return false;
}

bool tree_sitter_razor_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    Scanner *scanner = (Scanner *)payload;

    uint8_t brace_advanced = 0;
    uint8_t quote_count = 0;
    bool did_advance = false;

    if (!(valid_symbols[OPT_SEMI] && valid_symbols[INTERPOLATION_REGULAR_START])) {
        if (valid_symbols[LINE_END] && (lexer->eof(lexer) || lexer->lookahead == '\n' || lexer->lookahead == '\r')) {
            if (lexer->lookahead == '\r') advance(lexer);
            if (lexer->lookahead == '\n') advance(lexer);
            lexer->result_symbol = LINE_END;
            return true;
        }
        ContinuationResult continuation = scan_implicit_continuation(lexer, valid_symbols);
        if (continuation == CONTINUATION_MATCH) return true;
        for (enum TokenType type = SCRIPT_TEXT; type <= LINE_TEXT; type++) {
            if (valid_symbols[type]) return scan_markup_text(lexer, type, continuation == CONTINUATION_PREFIX, 0);
        }
        if (continuation == CONTINUATION_PREFIX) {
            if (valid_symbols[HTML_TEXT]) return scan_markup_text(lexer, HTML_TEXT, true, 0);
            return false;
        }
        if (valid_symbols[VOID_TAG_NAME] || valid_symbols[SCRIPT_TAG_NAME] || valid_symbols[STYLE_TAG_NAME]) {
            return scan_tag_name(lexer, valid_symbols);
        }
        for (enum TokenType type = RAZOR_IF_START; type <= RAZOR_CODE_START; type++) {
            if (valid_symbols[type]) {
                while (razor_space(lexer->lookahead)) skip(lexer);
                if (lexer->lookahead == '@') return scan_control_start(scanner, lexer, valid_symbols);
                break;
            }
        }
        for (enum TokenType type = RAZOR_ELSE_CONTINUATION; type <= RAZOR_WHILE_CONTINUATION; type++) {
            if (valid_symbols[type]) {
                while (razor_space(lexer->lookahead)) skip(lexer);
                return scan_control_continuation(lexer, valid_symbols);
            }
        }
        if (valid_symbols[HTML_TEXT]) return scan_markup_text(lexer, HTML_TEXT, false, 0);
    }

    // Lambda-paren scanning advances forward speculatively past the
    // opening `(`. If it consumes input and then bails, the lexer
    // cursor is mispositioned and must not be reused by other token
    // handlers below — they would emit tokens with spans starting at
    // the original scan position but ending at the dirty cursor (e.g.
    // an INTERPOLATION_REGULAR_START swallowing `(false, $` in
    // `return (false, $"...");`). When the scanner consumed `(` but
    // didn't confirm a lambda, return false here so tree-sitter
    // rewinds and lets the built-in `(` tokenizer match.
    if (valid_symbols[LAMBDA_PAREN_OPEN]) {
        switch (scan_lambda_paren_open(lexer)) {
            case LAMBDA_SCAN_SUCCESS:
                return true;
            case LAMBDA_SCAN_FAILED_AFTER_PAREN:
                return false;
            case LAMBDA_SCAN_NO_PAREN:
                break;  // lexer untouched; fall through
        }
    }

    // error recovery, gives better trees this way
    if (valid_symbols[OPT_SEMI] && valid_symbols[INTERPOLATION_REGULAR_START]) {
        return false;
    }

    if (valid_symbols[OPT_SEMI]) {
        lexer->result_symbol = OPT_SEMI;
        if (lexer->lookahead == ';') {
            advance(lexer);
        }
        return true;
    }

    if (valid_symbols[RAW_STRING_START]) {
        while (razor_space(lexer->lookahead)) {
            skip(lexer);
        }

        if (lexer->lookahead == '"') {
            while (lexer->lookahead == '"') {
                advance(lexer);
                quote_count++;
            }

            if (quote_count >= 3) {
                lexer->result_symbol = RAW_STRING_START;
                scanner->quote_count = quote_count;
                return true;
            }
        }
    }

    if (valid_symbols[RAW_STRING_END] && lexer->lookahead == '"') {
        while (lexer->lookahead == '"') {
            advance(lexer);
            quote_count++;
        }

        if (quote_count == scanner->quote_count) {
            lexer->result_symbol = RAW_STRING_END;
            scanner->quote_count = 0;
            return true;
        }

        did_advance = quote_count > 0;
    }

    if (valid_symbols[RAW_STRING_CONTENT]) {
        while (lexer->lookahead) {
            if (lexer->lookahead == '"') {
                lexer->mark_end(lexer);
                quote_count = 0;

                while (lexer->lookahead == '"') {
                    advance(lexer);
                    quote_count++;
                }

                if (quote_count == scanner->quote_count) {
                    lexer->result_symbol = RAW_STRING_CONTENT;
                    return true;
                }
            }
            advance(lexer);
            did_advance = true;
        }
        lexer->mark_end(lexer);
        lexer->result_symbol = RAW_STRING_CONTENT;
        return true;
    }

    if (valid_symbols[INTERPOLATION_REGULAR_START] || valid_symbols[INTERPOLATION_VERBATIM_START] ||
        valid_symbols[INTERPOLATION_RAW_START]) {
        while (razor_space(lexer->lookahead)) {
            skip(lexer);
        }

        uint8_t dollar_advanced = 0;

        bool is_verbatim = false;

        if (lexer->lookahead == '@') {
            is_verbatim = true;
            advance(lexer);
        }

        while (lexer->lookahead == '$' && quote_count == 0) {
            advance(lexer);
            dollar_advanced++;
        }

        if (dollar_advanced > 0 && (lexer->lookahead == '"' || lexer->lookahead == '@')) {
            lexer->result_symbol = INTERPOLATION_REGULAR_START;
            Interpolation interpolation = {
                .dollar_count = dollar_advanced,
                .open_brace_count = 0,
                .quote_count = 0,
                .string_type = 0,
            };

            if (is_verbatim || lexer->lookahead == '@') {
                if (lexer->lookahead == '@') {
                    advance(lexer);
                    is_verbatim = true;
                }
                lexer->result_symbol = INTERPOLATION_VERBATIM_START;
                interpolation.string_type = VERBATIM;
            }

            lexer->mark_end(lexer);
            advance(lexer);

            if (lexer->lookahead == '"' && !is_verbatim) {
                advance(lexer);
                if (lexer->lookahead == '"') {
                    lexer->result_symbol = INTERPOLATION_RAW_START;
                    interpolation.string_type |= RAW;
                    array_push(&scanner->interpolation_stack, interpolation);
                }
                // If we find 1 or 3 quotes, we push an interpolation.
                // If there's only two quotes, that's just an empty string
            } else {
                interpolation.string_type |= REGULAR;
                array_push(&scanner->interpolation_stack, interpolation);
            }

            return true;
        }
    }

    if (valid_symbols[INTERPOLATION_START_QUOTE] && scanner->interpolation_stack.size > 0) {
        Interpolation *current_interpolation = array_back(&scanner->interpolation_stack);

        if (is_verbatim(current_interpolation) || is_regular(current_interpolation)) {
            if (lexer->lookahead == '"') {
                advance(lexer);
                current_interpolation->quote_count++;
            }
        } else {
            while (lexer->lookahead == '"') {
                advance(lexer);
                current_interpolation->quote_count++;
            }
        }

        lexer->result_symbol = INTERPOLATION_START_QUOTE;
        return current_interpolation->quote_count > 0;
    }

    if (valid_symbols[INTERPOLATION_END_QUOTE] && scanner->interpolation_stack.size > 0) {
        Interpolation *current_interpolation = array_back(&scanner->interpolation_stack);

        while (lexer->lookahead == '"') {
            advance(lexer);
            quote_count++;
        }

        if (quote_count == current_interpolation->quote_count) {
            lexer->result_symbol = INTERPOLATION_END_QUOTE;
            array_pop(&scanner->interpolation_stack);
            return true;
        }

        did_advance = quote_count > 0;
    }

    if (valid_symbols[INTERPOLATION_OPEN_BRACE] && scanner->interpolation_stack.size > 0) {
        Interpolation *current_interpolation = array_back(&scanner->interpolation_stack);

        while (lexer->lookahead == '{' && brace_advanced < current_interpolation->dollar_count) {
            advance(lexer);
            brace_advanced++;
        }

        if (brace_advanced > 0 && brace_advanced == current_interpolation->dollar_count &&
            (brace_advanced == 0 || lexer->lookahead != '{')) {
            current_interpolation->open_brace_count = brace_advanced;
            lexer->result_symbol = INTERPOLATION_OPEN_BRACE;
            return true;
        }
    }

    if (valid_symbols[INTERPOLATION_CLOSE_BRACE] && scanner->interpolation_stack.size > 0) {
        uint8_t brace_advanced = 0;
        Interpolation *current_interpolation = array_back(&scanner->interpolation_stack);

        while (razor_space(lexer->lookahead)) {
            advance(lexer);
        }

        while (lexer->lookahead == '}') {
            advance(lexer);
            brace_advanced++;

            if (brace_advanced == current_interpolation->open_brace_count) {
                current_interpolation->open_brace_count = 0;
                lexer->result_symbol = INTERPOLATION_CLOSE_BRACE;
                return true;
            }
        }

        return false;
    }

    if (valid_symbols[INTERPOLATION_STRING_CONTENT] && scanner->interpolation_stack.size > 0) {
        lexer->result_symbol = INTERPOLATION_STRING_CONTENT;
        Interpolation *current_interpolation = array_back(&scanner->interpolation_stack);

        while (lexer->lookahead) {
            // top-down approach, first see if it's raw
            if (is_raw(current_interpolation)) {
                if (lexer->lookahead == '"') {
                    lexer->mark_end(lexer);
                    advance(lexer);
                    if (lexer->lookahead == '"') {
                        advance(lexer);
                        uint8_t quote_advanced = 2;
                        while (lexer->lookahead == '"') {
                            quote_advanced++;
                            advance(lexer);
                        }
                        if (quote_advanced == current_interpolation->quote_count) {
                            return did_advance;
                        }
                    }
                }

                if (lexer->lookahead == '{') {
                    lexer->mark_end(lexer);

                    while (lexer->lookahead == '{' && brace_advanced < current_interpolation->open_brace_count) {
                        advance(lexer);
                        brace_advanced++;
                    }

                    if (brace_advanced == current_interpolation->open_brace_count &&
                        (brace_advanced == 0 || lexer->lookahead != '{')) {
                        return did_advance;
                    }
                }
            }

            // then verbatim, since it could be verbatim + raw, but run the raw branch first
            else if (is_verbatim(current_interpolation)) {
                if (lexer->lookahead == '"') {
                    lexer->mark_end(lexer);
                    advance(lexer);
                    if (lexer->lookahead == '"') {
                        advance(lexer);
                        continue;
                    }
                    return did_advance;
                }

                if (lexer->lookahead == '{') {
                    lexer->mark_end(lexer);

                    while (lexer->lookahead == '{' && brace_advanced < current_interpolation->open_brace_count) {
                        advance(lexer);
                        brace_advanced++;
                    }

                    if (brace_advanced == current_interpolation->open_brace_count &&
                        (brace_advanced == 0 || lexer->lookahead != '{')) {
                        return did_advance;
                    }
                }
            }

            // finally regular
            else if (is_regular(current_interpolation)) {
                if (lexer->lookahead == '\\' || lexer->lookahead == '\n' || lexer->lookahead == '"') {
                    lexer->mark_end(lexer);
                    return did_advance;
                }

                if (lexer->lookahead == '{') {
                    lexer->mark_end(lexer);

                    while (lexer->lookahead == '{' && brace_advanced < current_interpolation->open_brace_count) {
                        advance(lexer);
                        brace_advanced++;
                    }

                    if (brace_advanced == current_interpolation->open_brace_count &&
                        (brace_advanced == 0 || lexer->lookahead != '{')) { // if we're in a brace we're not allowed to
                                                                            // collect more than the open_brace_count
                        return did_advance;
                    }
                }
            }

            if (lexer->lookahead != '{') {
                brace_advanced = 0;
            }
            advance(lexer);
            did_advance = true;
        }

        lexer->mark_end(lexer);
        return did_advance;
    }

    return false;
}
