#include <tree_sitter/parser.h>

enum TokenType {
  MULTI_STR_CONTENT,
  MULTI_BYTES_CONTENT,
  RAW_STR_CONTENT,
  RAW_BYTES_CONTENT,
  MULTI_RAW_STR_CONTENT,
  MULTI_RAW_BYTES_CONTENT,
};

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static bool scan_multiline(TSLexer *lexer, int c) {
  bool has_content = false;
  if (c == '"') {
    lexer->result_symbol = MULTI_STR_CONTENT;
  } else if (c == '\'') {
    lexer->result_symbol = MULTI_BYTES_CONTENT;
  }

  while (true) {
    switch (lexer->lookahead) {
    case '\'':
    case '"':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == c) {
        advance(lexer);
        if (lexer->lookahead == c) {
          if (has_content) {
            return true;
          } else {
            return false;
          }
        }
      }
      break;
    case '\\':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '(') {
        if (has_content) {
          return true;
        } else {
          return false;
        }
      } else {
        // FIXME: Accept anything after '\'
        advance(lexer);
      }
      has_content = true;
      break;
    case '\0':
      if (lexer->eof(lexer)) {
        return false;
      }
      advance(lexer);
      has_content = true;
      break;
    default:
      advance(lexer);
      has_content = true;
      break;
    }
  }
}

static bool scan_raw_multiline(TSLexer *lexer, int c) {
  bool has_content = false;
  if (c == '"') {
    lexer->result_symbol = MULTI_RAW_STR_CONTENT;
  } else if (c == '\'') {
    lexer->result_symbol = MULTI_RAW_BYTES_CONTENT;
  }

  while (true) {
    switch (lexer->lookahead) {
    case '\'':
    case '"':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == c) {
        advance(lexer);
        if (lexer->lookahead == c) {
          advance(lexer);
          if (lexer->lookahead == '#') {
            if (has_content) {
              return true;
            } else {
              return false;
            }
          }
        }
      }
      break;
    case '\\':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '#') {
        advance(lexer);
        if (lexer->lookahead == '(') {
          if (has_content) {
            return true;
          } else {
            return false;
          }
        }
      }
      has_content = true;
      break;
    case '\0':
      if (lexer->eof(lexer)) {
        return false;
      }
      advance(lexer);
      has_content = true;
      break;
    default:
      advance(lexer);
      has_content = true;
      break;
    }
  }
}

static bool scan_raw(TSLexer *lexer, int c) {
  bool has_content = false;
  if (c == '"') {
    lexer->result_symbol = RAW_STR_CONTENT;
  } else if (c == '\'') {
    lexer->result_symbol = RAW_BYTES_CONTENT;
  }

  while (true) {
    switch (lexer->lookahead) {
    case '\'':
    case '"':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '#') {
        if (has_content) {
          return true;
        } else {
          return false;
        }
      }
      break;
    case '\\':
      lexer->mark_end(lexer);
      advance(lexer);
      if (lexer->lookahead == '#') {
        advance(lexer);
        if (lexer->lookahead == '(') {
          if (has_content) {
            return true;
          } else {
            return false;
          }
        }
      } else {
        advance(lexer);
      }
      has_content = true;
      break;
    case '\0':
      if (lexer->eof(lexer)) {
        return false;
      }
      advance(lexer);
      has_content = true;
      break;
    default:
      advance(lexer);
      has_content = true;
      break;
    }
  }
}

static bool scan(TSLexer *lexer, const bool *valid_symbols) {
  if (valid_symbols[MULTI_STR_CONTENT]) {
    return scan_multiline(lexer, '"');
  } else if (valid_symbols[MULTI_BYTES_CONTENT]) {
    return scan_multiline(lexer, '\'');
  } else if (valid_symbols[MULTI_RAW_STR_CONTENT]) {
    return scan_raw_multiline(lexer, '"');
  } else if (valid_symbols[MULTI_RAW_BYTES_CONTENT]) {
    return scan_raw_multiline(lexer, '\'');
  } else if (valid_symbols[RAW_STR_CONTENT]) {
    return scan_raw(lexer, '"');
  } else if (valid_symbols[RAW_BYTES_CONTENT]) {
    return scan_raw(lexer, '\'');
  }

  return false;
}

#ifdef __cplusplus
extern "C" {
#endif

void *tree_sitter_cue_external_scanner_create() { return NULL; }

bool tree_sitter_cue_external_scanner_scan(void *payload, TSLexer *lexer,
                                           const bool *valid_symbols) {
  return scan(lexer, valid_symbols);
}

unsigned tree_sitter_cue_external_scanner_serialize(void *payload,
                                                    char *buffer) {
  return 0;
}

void tree_sitter_cue_external_scanner_deserialize(void *payload,
                                                  const char *buffer,
                                                  unsigned length) {}

void tree_sitter_cue_external_scanner_destroy(void *payload) {}

#ifdef __cplusplus
}
#endif
