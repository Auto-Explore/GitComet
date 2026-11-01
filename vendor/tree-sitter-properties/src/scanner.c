#include "tree_sitter/alloc.h"
#include "tree_sitter/parser.h"

#ifdef _MSC_VER
#pragma warning(disable : 4100)
#elif defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic ignored "-Wunused-parameter"
#endif

enum TokenType { FAKE_EOL };

// GitComet: the flag lives in the payload, not upstream's file-scope `static`
// shared by every parser; a concurrent parse could flip it and loop this one at EOF.

bool tree_sitter_properties_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  bool *reached_eof = payload;
  lexer->result_symbol = FAKE_EOL;
  return *reached_eof = !*reached_eof && valid_symbols[FAKE_EOL] && lexer->eof(lexer);
}

unsigned tree_sitter_properties_external_scanner_serialize(void *payload, char *buffer) {
  return *(bool *)payload;
}

void tree_sitter_properties_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  *(bool *)payload = length;
}

void *tree_sitter_properties_external_scanner_create() { return ts_calloc(1, sizeof(bool)); }

void tree_sitter_properties_external_scanner_destroy(void *payload) { ts_free(payload); }
