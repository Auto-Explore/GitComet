#include "tree_sitter/alloc.h"
#include "tree_sitter/parser.h"

enum TokenType {
  RAW_TEXT,
  COMMENT_CONTENT,
  FRONT_MATTER,
};

// GitComet addition. Front matter is only legal at byte 0, but error recovery
// marks every external token valid, so the first scan of a parse is remembered
// and later `---` lines can never be promoted.
typedef struct {
  bool past_start;
} Scanner;

void *tree_sitter_jinja_external_scanner_create(void) {
  return ts_calloc(1, sizeof(Scanner));
}

void tree_sitter_jinja_external_scanner_destroy(void *payload) {
  ts_free(payload);
}

unsigned tree_sitter_jinja_external_scanner_serialize(void *payload,
                                                      char *buffer) {
  Scanner *scanner = (Scanner *)payload;
  buffer[0] = (char)scanner->past_start;
  return 1;
}

void tree_sitter_jinja_external_scanner_deserialize(void *payload,
                                                    const char *buffer,
                                                    unsigned length) {
  Scanner *scanner = (Scanner *)payload;
  scanner->past_start = length > 0 && buffer[0] != 0;
}

static bool is_whitespace(int32_t c) {
  return c == ' ' || c == '\t' || c == '\n' || c == '\r';
}

static bool check_endraw(TSLexer *lexer) {
  if (lexer->lookahead != 'e') return false;
  lexer->advance(lexer, false);
  if (lexer->lookahead != 'n') return false;
  lexer->advance(lexer, false);
  if (lexer->lookahead != 'd') return false;
  lexer->advance(lexer, false);

  if (lexer->lookahead == 'r') {
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'a') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'w') return false;
    lexer->advance(lexer, false);
  } else if (lexer->lookahead == 'v') {
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'e') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'r') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'b') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'a') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 't') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'i') return false;
    lexer->advance(lexer, false);
    if (lexer->lookahead != 'm') return false;
    lexer->advance(lexer, false);
  } else {
    return false;
  }

  return is_whitespace(lexer->lookahead) ||
         lexer->lookahead == '%' ||
         lexer->lookahead == '-' ||
         lexer->lookahead == '~';
}

static bool scan_raw_text(TSLexer *lexer) {
  bool has_content = false;

  while (lexer->lookahead != 0) {
    if (lexer->lookahead == '{') {
      lexer->mark_end(lexer);
      lexer->advance(lexer, false);

      if (lexer->lookahead == '%') {
        lexer->advance(lexer, false);

        if (lexer->lookahead == '-' || lexer->lookahead == '~') {
          lexer->advance(lexer, false);
        }

        while (is_whitespace(lexer->lookahead)) {
          lexer->advance(lexer, false);
        }

        if (check_endraw(lexer)) {
          lexer->result_symbol = RAW_TEXT;
          return has_content;
        }
      }

      lexer->mark_end(lexer);
      has_content = true;
    } else {
      lexer->advance(lexer, false);
      lexer->mark_end(lexer);
      has_content = true;
    }
  }

  return false;
}

static bool scan_comment_content(TSLexer *lexer) {
  bool has_content = false;

  while (lexer->lookahead != 0) {
    if (lexer->lookahead == '-' || lexer->lookahead == '~') {
      lexer->mark_end(lexer);
      lexer->advance(lexer, false);

      if (lexer->lookahead == '#') {
        lexer->advance(lexer, false);
        if (lexer->lookahead == '}') {
          lexer->result_symbol = COMMENT_CONTENT;
          return has_content;
        }
        lexer->mark_end(lexer);
        has_content = true;
      } else {
        lexer->mark_end(lexer);
        has_content = true;
      }
    } else if (lexer->lookahead == '#') {
      lexer->mark_end(lexer);
      lexer->advance(lexer, false);

      if (lexer->lookahead == '}') {
        lexer->result_symbol = COMMENT_CONTENT;
        return has_content;
      }

      lexer->mark_end(lexer);
      has_content = true;
    } else {
      lexer->advance(lexer, false);
      lexer->mark_end(lexer);
      has_content = true;
    }
  }

  return false;
}

// `---` newline, any lines, then a line that is exactly `---` (newline or EOF).
// Without a closer the block stays ordinary `text`, as before.
static bool scan_front_matter(TSLexer *lexer) {
  if (lexer->get_column(lexer) != 0) return false;
  for (int i = 0; i < 3; i++) {
    if (lexer->lookahead != '-') return false;
    lexer->advance(lexer, false);
  }
  if (lexer->lookahead == '\r') lexer->advance(lexer, false);
  if (lexer->lookahead != '\n') return false;
  lexer->advance(lexer, false);

  while (!lexer->eof(lexer)) {
    int dashes = 0;
    while (lexer->lookahead == '-' && dashes < 3) {
      lexer->advance(lexer, false);
      dashes++;
    }
    if (dashes == 3) {
      if (lexer->lookahead == '\r') lexer->advance(lexer, false);
      if (lexer->lookahead == '\n') lexer->advance(lexer, false);
      if (lexer->get_column(lexer) == 0 || lexer->eof(lexer)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = FRONT_MATTER;
        return true;
      }
    }
    while (!lexer->eof(lexer) && lexer->lookahead != '\n') {
      lexer->advance(lexer, false);
    }
    if (lexer->lookahead == '\n') lexer->advance(lexer, false);
  }
  return false;
}

bool tree_sitter_jinja_external_scanner_scan(void *payload, TSLexer *lexer,
                                             const bool *valid_symbols) {
  Scanner *scanner = (Scanner *)payload;
  if (valid_symbols[FRONT_MATTER] && !scanner->past_start) {
    scanner->past_start = true;
    if (scan_front_matter(lexer)) return true;
  }

  if (valid_symbols[RAW_TEXT]) {
    return scan_raw_text(lexer);
  }

  if (valid_symbols[COMMENT_CONTENT]) {
    return scan_comment_content(lexer);
  }

  return false;
}
