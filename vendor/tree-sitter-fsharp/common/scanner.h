#ifndef TREE_SITTER_FSHARP_SCANNER_H_
#define TREE_SITTER_FSHARP_SCANNER_H_

#include "tree_sitter/alloc.h"
#include "tree_sitter/array.h"
#include "tree_sitter/parser.h"

enum TokenType {
  NEWLINE,
  INDENT,
  DEDENT,
  THEN,
  ELSE,
  ELIF,
  PREPROC_IF,
  PREPROC_ELSE,
  PREPROC_END,
  CLASS,
  BEGIN,
  STRUCT,
  INTERFACE,
  END,
  AND,
  WITH,
  TRIPLE_QUOTE_CONTENT,
  FORMAT_TRIPLE_QUOTE_CONTENT,
  BLOCK_COMMENT_CONTENT,
  INSIDE_STRING,
  NEWLINE_NO_ALIGNED,
  TUPLE_MARKER,
  QUOTED_CLOSE,
  UNTYPED_QUOTED_CLOSE,
  MULTI_DOLLAR_TRIPLE_QUOTE_START,
  MULTI_DOLLAR_TRIPLE_QUOTED_CONTENT,
  MULTI_DOLLAR_INTERP_START,
  MULTI_DOLLAR_INTERP_END,
  MULTI_DOLLAR_TRIPLE_QUOTE_END,
  TYAPP_OPEN,
  PAREN_INDENT,
  TYPE_APP_INDENT,
  TYPE_DECL_NEWLINE,
  IN,
  DO_KEYWORD,
  TRY_INDENT,
  PREPROC_INACTIVE,
  ELEM_SEP,
  BRACE_INDENT,
  ERROR_SENTINEL
};

typedef enum {
  INDENT_NORMAL = 0,
  INDENT_PAREN = 1,
  INDENT_TYPE_APP = 2,
  // Body of a `try` expression. Behaves like a normal indent for all dedent
  // logic, but can be force-closed when its terminating `with`/`finally` sits
  // at the same column as the body (where an ordinary dedent would not fire).
  INDENT_TRY = 3,
  // Body of a `{...}` record / computation-expression block. Closed by '}'
  // (never DEDENTs on under-indentation, like a paren indent), but an
  // under-indented line still emits a NEWLINE so record/CE items keep
  // separating even when a continuation item sits left of the first one.
  INDENT_BRACE = 4,
} IndentKind;

// OR-ed into the indent_kinds byte when the scope's INDENT fired mid-line
// (no newline crossed before the anchor token), so the anchor column is a
// mid-line position like the `Some` in `let a = Some <|`. A continuation
// line that sits strictly BETWEEN such an anchor and the enclosing level is
// part of the expression (F# offside is measured from the construct start,
// not the anchor), so DEDENT must not fire into it.
#define INDENT_KIND_MIDLINE_FLAG 0x80
// OR-ed into the indent_kinds byte when a mid-line scope was opened on a
// STRANDED line — one that itself hangs between two open levels and was
// introduced by the stranded-dedent NEWLINE (e.g. `let c = ()` at col 8
// under a col-4 module body). A following line at that column is a sibling
// declaration, not a continuation, so the midline-anchor dedent guard must
// not apply.
#define INDENT_KIND_STRANDED_LINE_FLAG 0x40
#define INDENT_KIND_FLAGS_MASK 0xC0

// How an open `#if` directive entered the parse. STRUCTURED directives were
// handed to the grammar (preproc_if rules) and both branches parse as syntax.
// STRAY directives appeared at a position the grammar has no preproc rule for
// and their `#if` line was consumed as trivia; only the active (first) branch
// is parsed — a later `#else` swallows everything to the matching `#endif`.
typedef enum {
  PREPROC_STRUCTURED = 0,
  PREPROC_STRAY = 1,
} PreprocKind;

typedef struct {
  Array(uint16_t) indents;
  Array(uint8_t) indent_kinds;
  Array(uint16_t) preprocessor_indents;
  Array(uint8_t) preproc_kinds;
  // Set when the current line was introduced by a stranded-dedent NEWLINE
  // (the line hangs between two open levels); cleared when a scan's
  // whitespace walk crosses onto the next line. Durable because the
  // stranded NEWLINE is an emitted token, so the flag is captured in the
  // serialized state that later same-line scans restore.
  uint8_t line_stranded;
  uint8_t multi_dollar_count;
  // Set when a DEDENT pops a level but the current line still sits *above* the
  // new enclosing level (a "stranded"/partial dedent). Consumed on the very
  // next scan to emit the NEWLINE the enclosing block owes as an item
  // separator. See the emit site in the found_end_of_line handling.
  uint8_t stranded_dedent;
} Scanner;

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static inline bool is_word_char(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
         (c >= '0' && c <= '9') || c == '_' || c == '\'';
}

static inline bool keyword_equals(const char *word, const char *keyword) {
  size_t i = 0;
  while (word[i] != '\0' && word[i] == keyword[i]) {
    i++;
  }
  return word[i] == '\0' && keyword[i] == '\0';
}

static inline void push_indent(Scanner *scanner, uint16_t indent_length,
                               IndentKind kind) {
  array_push(&scanner->indents, indent_length);
  array_push(&scanner->indent_kinds, (uint8_t)kind);
}

static inline void pop_indent(Scanner *scanner) {
  if (scanner->indents.size > 0) {
    array_pop(&scanner->indents);
  }
  if (scanner->indent_kinds.size > 0) {
    array_pop(&scanner->indent_kinds);
  }
}

static inline uint16_t peek_indent_length(Scanner *scanner) {
  return *array_back(&scanner->indents);
}

static inline void push_preproc_kind(Scanner *scanner, PreprocKind kind) {
  array_push(&scanner->preproc_kinds, (uint8_t)kind);
}

static inline void pop_preproc_kind(Scanner *scanner) {
  if (scanner->preproc_kinds.size > 0) {
    array_pop(&scanner->preproc_kinds);
  }
}

static inline bool top_preproc_is_stray(Scanner *scanner) {
  return scanner->preproc_kinds.size > 0 &&
         *array_back(&scanner->preproc_kinds) == (uint8_t)PREPROC_STRAY;
}

static inline bool top_preproc_is_structured(Scanner *scanner) {
  return scanner->preproc_kinds.size > 0 &&
         *array_back(&scanner->preproc_kinds) == (uint8_t)PREPROC_STRUCTURED;
}

// Consume everything from the current position (just past a stray `#else`)
// through the end of the matching `#endif` line, tracking nested directives
// textually (proper nesting is guaranteed by the F# lexer, and the inactive
// branch is never parsed, so line-start matching is sufficient). Stops before
// the trailing newline so normal newline/indent processing resumes after the
// region. At EOF (unterminated directive) the rest of the file is consumed.
static inline void swallow_inactive_region(TSLexer *lexer) {
  int depth = 1;
  for (;;) {
    while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
      advance(lexer);
    }
    if (lexer->eof(lexer)) {
      return;
    }
    advance(lexer); // consume the newline
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t' ||
           lexer->lookahead == '\r') {
      advance(lexer);
    }
    if (lexer->lookahead != '#') {
      continue;
    }
    advance(lexer);
    if (lexer->lookahead == 'i') {
      advance(lexer);
      if (lexer->lookahead == 'f') {
        advance(lexer);
        if (!is_word_char(lexer->lookahead)) {
          depth++;
        }
      }
    } else if (lexer->lookahead == 'e') {
      advance(lexer);
      if (lexer->lookahead == 'n') {
        advance(lexer);
        if (lexer->lookahead == 'd') {
          advance(lexer);
          if (lexer->lookahead == 'i') {
            advance(lexer);
            if (lexer->lookahead == 'f') {
              advance(lexer);
              if (!is_word_char(lexer->lookahead)) {
                depth--;
                if (depth == 0) {
                  while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
                    advance(lexer);
                  }
                  return;
                }
              }
            }
          }
        }
      }
    }
  }
}

static inline IndentKind peek_indent_kind(Scanner *scanner) {
  if (scanner->indent_kinds.size == 0) return INDENT_NORMAL;
  return (IndentKind)(*array_back(&scanner->indent_kinds) &
                      ~INDENT_KIND_FLAGS_MASK);
}

static inline bool top_indent_is_midline_anchor(Scanner *scanner) {
  return scanner->indent_kinds.size > 0 &&
         (*array_back(&scanner->indent_kinds) & INDENT_KIND_MIDLINE_FLAG) != 0;
}

static inline bool top_indent_is_stranded_line(Scanner *scanner) {
  return scanner->indent_kinds.size > 0 &&
         (*array_back(&scanner->indent_kinds) &
          INDENT_KIND_STRANDED_LINE_FLAG) != 0;
}

static inline bool peek_is_paren_indent(Scanner *scanner) {
  IndentKind kind = peek_indent_kind(scanner);
  return kind == INDENT_PAREN || kind == INDENT_TYPE_APP;
}

static inline bool peek_is_brace_indent(Scanner *scanner) {
  return peek_indent_kind(scanner) == INDENT_BRACE;
}

static inline bool peek_is_type_app_indent(Scanner *scanner) {
  return peek_indent_kind(scanner) == INDENT_TYPE_APP;
}

static inline bool peek_is_try_indent(Scanner *scanner) {
  return peek_indent_kind(scanner) == INDENT_TRY;
}

// Peek forward from after a '<' to its matching '>' and check that the content
// is type-arg-shaped per F# spec Section 15.3 (identifiers, whitespace, and
// ',', '*', '->', '(', ')', '[', ']', '<', '>', '^', '#', ':', '{|', '|}').
// If `out_saw_newline` is non-NULL, also reports whether a newline appeared
// before the close — used to distinguish multi-line type apps where the first
// arg sits on the same line as '<' (which `found_end_of_line` alone misses).
static inline bool is_type_application_open_ex(TSLexer *lexer,
                                               bool *out_saw_newline) {
  int angle_depth = 1;
  int paren_depth = 0;
  bool saw_newline = false;
  // Whether the character just consumed was a '^'. Only the exact `^-<int>`
  // spelling of a negative measure exponent may relax the '-' rule below, so
  // this is per-character state, not "a caret appeared somewhere".
  bool prev_was_caret = false;

  while (!lexer->eof(lexer) && angle_depth > 0) {
    int32_t c = lexer->lookahead;
    bool after_caret = prev_was_caret;
    prev_was_caret = false;

    if (c == '\n' || c == '\r') {
      saw_newline = true;
      advance(lexer);
      continue;
    }

    if (is_word_char(c) || c == ' ' || c == '\t' || c == ',' || c == '*' ||
        c == '.' || c == ':' || c == '#' || c == '^' || c == '/' || c == '|' ||
        c == '{' || c == '}' || c == '[' || c == ']' ||
        // Backtick-quoted measure/type names may contain '%' and '`',
        // e.g. 0.95m<``Risk %``>.
        c == '`' || c == '%') {
      prev_was_caret = (c == '^');
      advance(lexer);
      continue;
    }

    if (c == '(') { paren_depth++; advance(lexer); continue; }
    if (c == ')') {
      // Unbalanced ')' rules out type args (e.g. `(l<r)`).
      if (paren_depth <= 0) return false;
      paren_depth--; advance(lexer); continue;
    }
    if (c == '<') { angle_depth++; advance(lexer); continue; }
    if (c == '>') {
      angle_depth--;
      if (angle_depth == 0) {
        // A '>' inside unbalanced parens (e.g. the ':>' in
        // `box<|(inst :> IFoo).M`) cannot close a type application.
        if (paren_depth > 0) return false;
        if (out_saw_newline) *out_saw_newline = saw_newline;
        return true;
      }
      advance(lexer);
      continue;
    }
    if (c == '-') {
      // '->' is valid in function types, bare '-' is not.
      advance(lexer);
      if (lexer->lookahead == '>') { advance(lexer); continue; }
      // A negative measure exponent: `float<m s^-1>` directly after the '^',
      // or the numerator of a rational one, `23<kg^(-12345/123)>`, where the
      // '-' sits inside the parens. Both spellings are narrow, so ordinary
      // comparison chains (`a<b-1>c`, `a<b^2 - 1>c`) remain non-type-apps.
      if ((after_caret || paren_depth > 0) && lexer->lookahead >= '0' &&
          lexer->lookahead <= '9') {
        continue;
      }
      return false;
    }
    // Anything else (`&`, `!`, `=`, `+`, `;`, `@`, `$`, `%`, `?`, …) rules out type args.
    return false;
  }

  return false;
}

static inline bool is_type_application_open(TSLexer *lexer) {
  return is_type_application_open_ex(lexer, NULL);
}

// Peek the body of a parenthesised multi-typar group that opens an SRTP trait
// call: `^a or ^b) : (` / `'T1 or 'T2) : (`, as in
// `((^a or ^b) : (static member op_Implicit: ^a -> ^b) x)`. Called with the
// lookahead on the first typar — the group's '(' has already been passed,
// either by the parser or by one of scan()'s own probes. At least two typars
// joined by `or`, then the group's ')' and the following `: (`, are required,
// so the body of a parenthesised expression can never match.
static inline bool is_srtp_typar_group_ahead(TSLexer *lexer) {
  unsigned typars = 0;
  for (;;) {
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
      advance(lexer);
    }
    if (lexer->lookahead != '^' && lexer->lookahead != '\'') {
      return false;
    }
    advance(lexer);
    if (!is_word_char(lexer->lookahead)) {
      return false;
    }
    while (is_word_char(lexer->lookahead)) {
      advance(lexer);
    }
    typars++;
    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
      advance(lexer);
    }
    if (lexer->lookahead != 'o') {
      break;
    }
    advance(lexer);
    if (lexer->lookahead != 'r') {
      return false;
    }
    advance(lexer);
    if (is_word_char(lexer->lookahead)) {
      return false;  // an identifier starting with "or", not the keyword
    }
  }
  if (typars < 2 || lexer->lookahead != ')') {
    return false;
  }
  advance(lexer);
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    advance(lexer);
  }
  if (lexer->lookahead != ':') {
    return false;
  }
  advance(lexer);
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    advance(lexer);
  }
  return lexer->lookahead == '(';
}

static inline bool is_multiline_type_app_ahead(TSLexer *lexer) {
  bool saw_newline = false;
  return is_type_application_open_ex(lexer, &saw_newline) && saw_newline;
}

static inline bool scan_n_chars(TSLexer *lexer, char ch, uint8_t count) {
  lexer->mark_end(lexer);
  for (uint8_t i = 0; i < count; i++) {
    if (lexer->lookahead != ch) {
      return false;
    }
    advance(lexer);
  }
  lexer->mark_end(lexer);
  return true;
}

static inline bool scan_block_comment(TSLexer *lexer) {
  lexer->mark_end(lexer);
  if (lexer->lookahead != '(')
    return false;

  advance(lexer);
  if (lexer->lookahead != '*')
    return false;

  advance(lexer);

  while (true) {
    switch (lexer->lookahead) {
    case '(':
      scan_block_comment(lexer);
      break;
    case '*':
      advance(lexer);
      if (lexer->lookahead == ')') {
        advance(lexer);
        return true;
      }
      break;
    case '\0':
      return true;
    default:
      advance(lexer);
    }
  }
}

static inline bool is_infix_op_start(TSLexer *lexer) {
  switch (lexer->lookahead) {
  case '+':
  case '-':
    skip(lexer);
    return !(lexer->lookahead >= '0' && lexer->lookahead <= '9');
  case '*':
  case '%':
  case '&':
  case '=':
  case '?':
  case '<':
  case '>':
  case '^':
  // A line-leading ',' continues the previous line (leading-comma style in
  // multiline tuples / named arguments), exactly like an infix operator.
  case ',':
    return true;
  case '/':
    skip(lexer);
    return lexer->lookahead != '/';
  case '.':
    skip(lexer);
    return lexer->lookahead != '.';
  case '!':
    skip(lexer);
    return lexer->lookahead == '=';
  case ':':
    skip(lexer);
    return lexer->lookahead == '=' || lexer->lookahead == ':' ||
           lexer->lookahead == '?' || lexer->lookahead == ' ' ||
           lexer->lookahead == '>';
  case 'o':
    skip(lexer);
    if (lexer->lookahead != 'r') {
      return false;
    }
    skip(lexer);
    // Only the standalone `or` operator counts; identifiers that merely start
    // with "or" (e.g. `orderId`) must not be treated as an infix op, otherwise
    // the newline before them is suppressed and they get glued to the previous
    // token (e.g. a record field type).
    return !is_word_char(lexer->lookahead);
  case '@':
  case '$':
    skip(lexer);
    return lexer->lookahead != '"';
  default:
    return false;
  }
}

static inline bool is_bracket_end(TSLexer *lexer) {
  switch (lexer->lookahead) {
  case ')':
  case ']':
  case '}':
    return true;
  default:
    return false;
  }
  }

  // Match remaining characters of a keyword after the first character.
  // Returns true if the rest of the keyword matches and is followed by a non-word char.
  static inline bool match_keyword_rest(TSLexer *lexer, const char *rest) {
    for (; *rest; rest++) {
      if (lexer->lookahead != *rest) return false;
      advance(lexer);
    }
    return !is_word_char(lexer->lookahead);
  }

  // Table-driven block opener matching for class/begin/struct/interface.
  typedef struct {
    char first_char;
    const char *rest;
    enum TokenType token;
  } BlockOpener;

  static const BlockOpener block_openers[] = {
    {'c', "lass",      CLASS},
    {'b', "egin",      BEGIN},
    {'s', "truct",     STRUCT},
    {'i', "nterface",  INTERFACE},
  };

  // Check if a preprocessor directive requires emitting DEDENT before itself.
  // Used by both #endif and #else handlers to pop indent when the preprocessor
  // indent is less than the current block indent.
  static inline bool try_dedent_for_preproc(Scanner *scanner, TSLexer *lexer) {
    if (scanner->indents.size > 0 &&
        scanner->preprocessor_indents.size > 0) {
      uint16_t current_indent_length = peek_indent_length(scanner);
      uint16_t current_preproc_length =
          *array_back(&scanner->preprocessor_indents);
      if (current_preproc_length < current_indent_length) {
        pop_indent(scanner);
        lexer->result_symbol = DEDENT;
        return true;
      }
    }
    return false;
  }

static bool scan(Scanner *scanner, TSLexer *lexer, const bool *valid_symbols) {
  // A stranded-dedent flag lives for exactly one scan: capture it and clear
  // the persistent copy up front so it is consumed by the immediately
  // following scan (the NEWLINE emit below) and never leaks further.
  bool prev_stranded_dedent = scanner->stranded_dedent;
  scanner->stranded_dedent = false;

  if (valid_symbols[ERROR_SENTINEL]) {
    // During error recovery, all valid_symbols are true. Tree-sitter's error
    // recovery mechanism cannot emit external scanner tokens, so we must still
    // produce tokens like DEDENT and PREPROC_END when we can identify them.
    // This enables partial parse tree recovery -- e.g., "match x with" needs
    // DEDENT to be recognized as a partially correct match-statement for
    // syntax highlighting purposes.

    // At EOF, emit DEDENT to drain the indent stack. This is critical for
    // closing partial parse trees at end of input.
    if (lexer->eof(lexer) && scanner->indents.size > 1) {
      pop_indent(scanner);
      lexer->result_symbol = DEDENT;
      return true;
    }

    // For non-EOF cases, fall through to normal scanning logic below.
    // The normal path handles whitespace consumption and emits DEDENT/NEWLINE
    // based on actual indentation levels. Features that should not run during
    // error recovery (multi-dollar strings, quotation closers, etc.) are
    // already guarded by !valid_symbols[ERROR_SENTINEL] checks.
  }

  if (valid_symbols[INSIDE_STRING] && !valid_symbols[ERROR_SENTINEL]) {
    return false;
  }

  // `preproc_inactive` is an extra, so it is valid in nearly every state —
  // including states that had no valid external tokens before it existed and
  // therefore never invoked this scanner. The full scan logic below assumes
  // some structural token is wanted (several paths emit DEDENT/NEWLINE
  // unconditionally), so in those states handle only the stray-directive
  // fallback and otherwise stay out of the internal lexer's way.
  if (!valid_symbols[ERROR_SENTINEL]) {
    bool any_structural_valid = false;
    for (int i = 0; i < PREPROC_INACTIVE; i++) {
      if (valid_symbols[i]) {
        any_structural_valid = true;
        break;
      }
    }
    // BRACE_INDENT sits after the PREPROC_INACTIVE/ELEM_SEP "extra" tokens in
    // the enum but is a structural indent token like INDENT, so it must count
    // here — otherwise a state where only BRACE_INDENT is valid (right after a
    // record/CE '{') bails out and the token is never emitted.
    if (valid_symbols[BRACE_INDENT]) {
      any_structural_valid = true;
    }
    if (!any_structural_valid) {
      while (lexer->lookahead == ' ' || lexer->lookahead == '\t' ||
             lexer->lookahead == '\n' || lexer->lookahead == '\r' ||
             lexer->lookahead == '\f') {
        skip(lexer);
      }
      if (lexer->lookahead != '#') {
        return false;
      }
      advance(lexer);
      if (lexer->lookahead == 'i') { // #if — grammar cannot place it here
        advance(lexer);
        if (lexer->lookahead != 'f') return false;
        advance(lexer);
        if (is_word_char(lexer->lookahead)) return false;
        push_preproc_kind(scanner, PREPROC_STRAY);
        while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
          advance(lexer);
        }
        lexer->mark_end(lexer);
        lexer->result_symbol = PREPROC_INACTIVE;
        return true;
      }
      if (lexer->lookahead != 'e') {
        return false;
      }
      advance(lexer);
      if (lexer->lookahead == 'l') { // #else of a stray directive
        advance(lexer);
        if (lexer->lookahead != 's') return false;
        advance(lexer);
        if (lexer->lookahead != 'e') return false;
        advance(lexer);
        if (is_word_char(lexer->lookahead) || !top_preproc_is_stray(scanner)) {
          return false;
        }
        pop_preproc_kind(scanner);
        swallow_inactive_region(lexer);
        lexer->mark_end(lexer);
        lexer->result_symbol = PREPROC_INACTIVE;
        return true;
      }
      if (lexer->lookahead == 'n') { // #endif of a stray directive
        advance(lexer);
        if (lexer->lookahead != 'd') return false;
        advance(lexer);
        if (lexer->lookahead != 'i') return false;
        advance(lexer);
        if (lexer->lookahead != 'f') return false;
        advance(lexer);
        if (is_word_char(lexer->lookahead) || !top_preproc_is_stray(scanner)) {
          return false;
        }
        pop_preproc_kind(scanner);
        lexer->mark_end(lexer);
        lexer->result_symbol = PREPROC_INACTIVE;
        return true;
      }
      return false;
    }
  }

  // Type application '<' disambiguation (F# spec Section 15.3).
  // When the grammar expects TYAPP_OPEN (i.e., a '<' immediately after an expression),
  // peek ahead to determine if the content between '<' and '>' looks like type arguments.
  // If not (e.g., it's a comparison operator like l<r), return false so the grammar
  // falls through to infix_op.
  if (valid_symbols[TYAPP_OPEN] && !valid_symbols[ERROR_SENTINEL] &&
      lexer->lookahead == '<') {
    lexer->mark_end(lexer);
    advance(lexer);
    // Mark end right after '<' - this is what we want the token to contain
    lexer->mark_end(lexer);
    // Now peek ahead (advancing further) to check if content looks like type args.
    // Even though we advance past the type content, mark_end is already set to
    // just after '<', so the emitted token will be exactly '<'.
    if (is_type_application_open(lexer)) {
      lexer->result_symbol = TYAPP_OPEN;
      return true;
    }
    // Not a type application - don't consume the '<', let grammar handle it as infix_op.
    // But we already advanced past '<' and potentially more. That's OK because
    // we return false and tree-sitter will reset the lexer position.
    return false;
  }

  if (!valid_symbols[ERROR_SENTINEL] && scanner->multi_dollar_count > 1) {
    if (valid_symbols[MULTI_DOLLAR_INTERP_START] && lexer->lookahead == '{') {
      if (scan_n_chars(lexer, '{', scanner->multi_dollar_count)) {
        lexer->result_symbol = MULTI_DOLLAR_INTERP_START;
        return true;
      }
    }

    if (valid_symbols[MULTI_DOLLAR_INTERP_END] && lexer->lookahead == '}') {
      if (scan_n_chars(lexer, '}', scanner->multi_dollar_count)) {
        lexer->result_symbol = MULTI_DOLLAR_INTERP_END;
        return true;
      }
    }

    if (valid_symbols[MULTI_DOLLAR_TRIPLE_QUOTE_END] && lexer->lookahead == '"') {
      if (scan_n_chars(lexer, '"', 3)) {
        scanner->multi_dollar_count = 0;
        lexer->result_symbol = MULTI_DOLLAR_TRIPLE_QUOTE_END;
        return true;
      }
    }
  }

  if (!valid_symbols[ERROR_SENTINEL] &&
      (valid_symbols[TRIPLE_QUOTE_CONTENT] || valid_symbols[FORMAT_TRIPLE_QUOTE_CONTENT] ||
       valid_symbols[MULTI_DOLLAR_TRIPLE_QUOTED_CONTENT])) {
    bool is_format = valid_symbols[FORMAT_TRIPLE_QUOTE_CONTENT];
    bool is_multi_dollar = valid_symbols[MULTI_DOLLAR_TRIPLE_QUOTED_CONTENT];
    bool has_content = false;
    lexer->mark_end(lexer);
    while (true) {
      if (lexer->lookahead == '\0') {
        break;
      }
      if ((is_format || is_multi_dollar) && lexer->lookahead == '{') {
        // In format triple-quoted strings, stop at '{' to allow interpolation.
        // Multi-dollar interpolated strings require N braces, where N is the
        // number of leading '$' characters.
        uint8_t brace_count = is_multi_dollar ? scanner->multi_dollar_count : 1;
        lexer->mark_end(lexer);

        if (!is_multi_dollar) {
          advance(lexer);
          if (lexer->lookahead == '{') {
            advance(lexer);
            lexer->mark_end(lexer);
            has_content = true;
            continue;
          }
          if (!has_content) {
            return false;
          }
          break;
        }

        bool matches_interp_start = true;
        for (uint8_t i = 0; i < brace_count; i++) {
          if (lexer->lookahead != '{') {
            matches_interp_start = false;
            break;
          }
          advance(lexer);
        }
        if (matches_interp_start) {
          if (!has_content) {
            return false;
          }
          break;
        }
        has_content = true;
        lexer->mark_end(lexer);
        continue;
      }
      if (lexer->lookahead != '"') {
        advance(lexer);
        has_content = true;
      } else {
        if (is_multi_dollar) {
          advance(lexer);
          if (lexer->lookahead == '"') {
            advance(lexer);
            if (lexer->lookahead == '"') {
              break;
            }
          }
          has_content = true;
          lexer->mark_end(lexer);
        } else {
          lexer->mark_end(lexer);
          skip(lexer);
          if (lexer->lookahead == '"') {
            skip(lexer);
            if (lexer->lookahead == '"') {
              skip(lexer);
              break;
            }
          }
        }
        has_content = true;
        lexer->mark_end(lexer);
      }
    }
    if (is_multi_dollar) {
      lexer->result_symbol = MULTI_DOLLAR_TRIPLE_QUOTED_CONTENT;
    } else {
      lexer->result_symbol = is_format ? FORMAT_TRIPLE_QUOTE_CONTENT : TRIPLE_QUOTE_CONTENT;
    }
    return true;
  }

  if (valid_symbols[TYPE_DECL_NEWLINE] && !valid_symbols[ERROR_SENTINEL]) {
    // Only fire at EOF or newline; if the current character is something else
    // (e.g. '=' during GLR exploration), fall through to general scanning —
    // the lexer position is unchanged so this is safe.
    if (lexer->eof(lexer)) {
      lexer->result_symbol = TYPE_DECL_NEWLINE;
      return true;
    }
    if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
      // Emit a zero-width token: fix the token end at the newline so the newline
      // itself is NOT consumed. This matters inside module bodies, where the
      // newline is the separator between elements; if TYPE_DECL_NEWLINE ate it,
      // the module would close after a single bare type declaration.
      lexer->mark_end(lexer);
      // Peek ahead: skip newlines/whitespace to find indentation of next content.
      // If next content is NOT more indented than current scope, this is a bare
      // type declaration (e.g. [<Measure>] type Dollars).
      // If next content IS more indented, the type has a body (e.g. type CsvFile
      //   private (...) = ...) and TYPE_DECL_NEWLINE should not fire.
      uint32_t next_indent = 0;
      for (;;) {
        if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
          next_indent = 0;
          skip(lexer);
        } else if (lexer->lookahead == ' ') {
          next_indent++;
          skip(lexer);
        } else if (lexer->lookahead == '\t') {
          next_indent += 8;
          skip(lexer);
        } else {
          break;
        }
      }
      if (lexer->eof(lexer)) {
        lexer->result_symbol = TYPE_DECL_NEWLINE;
        return true;
      }
      uint32_t scope_indent = (scanner->indents.size > 0)
        ? (uint32_t)*array_back(&scanner->indents)
        : 0;
      if (next_indent <= scope_indent) {
        // Next line is at same or lower indentation — bare type declaration
        lexer->result_symbol = TYPE_DECL_NEWLINE;
        return true;
      }
      // else: next line is more indented — type has a body, don't fire
      return false;
    }
  }

  // Block-comment content must be handled before the whitespace/offside walk
  // below. It is only ever valid immediately after a '(*' opener, so nothing
  // else needs scanning here; running the ws-walk instead lets a '#' that is
  // the first non-space char of the comment (e.g. a Markdown '(* # Heading')
  // be mis-read as a preprocessor directive, breaking the comment.
  if (valid_symbols[BLOCK_COMMENT_CONTENT] && !valid_symbols[ERROR_SENTINEL]) {
    // Scan position is directly after a shifted '(*'. If the very next char
    // is ')', the source text was `(*)` — F# defines that as the
    // multiplication operator reference, never a comment (matching FSC's
    // lexer). Decline so the GLR version that lexed '(*' as a comment opener
    // dies immediately instead of swallowing an arbitrary span hunting for
    // '*)' (`Constant(Checked.(*) l r, t)` arms repeated in one match were
    // compounding such zombie versions past the GLR version cap, killing the
    // correct parse — ExpressionOptimizer.fs whole-file wrap).
    if (lexer->lookahead == ')') {
      return false;
    }
    lexer->mark_end(lexer);
    while (true) {
      if (lexer->lookahead == '\0') {
        break;
      }
      if (lexer->lookahead != '(' && lexer->lookahead != '*') {
        advance(lexer);
      } else if (lexer->lookahead == '*') {
        lexer->mark_end(lexer);
        advance(lexer);
        if (lexer->lookahead == ')') {
          break;
        }
      } else if (scan_block_comment(lexer)) {
        lexer->mark_end(lexer);
        advance(lexer);
        if (lexer->lookahead == '*') {
          break;
        }
      }
    }
    lexer->result_symbol = BLOCK_COMMENT_CONTENT;
    return true;
  }

  lexer->mark_end(lexer);

  bool found_end_of_line = false;
  bool found_end_of_line_semi_colon = false;
  bool found_start_of_infix_op = false;
  bool found_same_line_pipe_infix = false;
  bool found_bracket_end = false;
  bool found_preprocessor_end = false;
  bool found_preproc_if = false;
  bool found_preproc_else = false;
  bool found_comment_start = false;
  bool advanced_in_ws_walk = false;
  bool skipped_open_paren = false;
  uint32_t indent_length = lexer->get_column(lexer);

  for (;;) {
    if (lexer->lookahead == '\n') {
      found_end_of_line = true;
      indent_length = 0;
      skip(lexer);
    } else if (lexer->lookahead == ' ') {
      indent_length++;
      skip(lexer);
    } else if (lexer->lookahead == '\r' || lexer->lookahead == '\f') {
      indent_length = 0;
      skip(lexer);
    } else if (lexer->lookahead == '\t') {
      indent_length += 8;
      skip(lexer);
    } else if (lexer->eof(lexer)) {
      found_end_of_line = true;
      indent_length = 0;
      break;
    } else if (lexer->lookahead == '/') {
      skip(lexer);
      if (!valid_symbols[INSIDE_STRING] && lexer->lookahead == '/') {
        // Once the loop has scanned past a directive (`#endif`/`#else` that
        // couldn't be emitted yet, or a `#if` line), declining on a trailing
        // line comment would leave the directive — not the comment — as the
        // next text for the internal lexer, which cannot lex it (ERROR).
        // Skip the comment like whitespace instead, exactly as if the line
        // were blank, so the pending NEWLINE/DEDENT decision proceeds.
        if (!found_preproc_if && !found_preprocessor_end &&
            !found_preproc_else) {
          return false;
        }
        while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
          skip(lexer);
        }
      } else {
        return false;
      }
    } else if (lexer->lookahead == '#') {
      advanced_in_ws_walk = true;
      advance(lexer);
      if (lexer->lookahead == 'e') {
        advance(lexer);
        if (lexer->lookahead == 'n') {
          advance(lexer);
          if (lexer->lookahead == 'd') {
            advance(lexer);
            if (lexer->lookahead == 'i') {
              advance(lexer);
              if (lexer->lookahead == 'f') {
                advance(lexer);
                // The innermost open directive was consumed as trivia (no
                // grammar rule at its position), so this `#endif` closes it
                // textually: consume it as an inactive-trivia extra instead
                // of handing it to the grammar.
                if (top_preproc_is_stray(scanner) &&
                    !valid_symbols[ERROR_SENTINEL] &&
                    !is_word_char(lexer->lookahead)) {
                  pop_preproc_kind(scanner);
                  lexer->mark_end(lexer);
                  lexer->result_symbol = PREPROC_INACTIVE;
                  return true;
                }
                found_preprocessor_end = true;
                if (try_dedent_for_preproc(scanner, lexer)) {
                  return true;
                }
                if (valid_symbols[PREPROC_END]) {
                  if (scanner->preprocessor_indents.size > 0) {
                    array_pop(&scanner->preprocessor_indents);
                  }
                  pop_preproc_kind(scanner);
                  lexer->mark_end(lexer);
                  lexer->result_symbol = PREPROC_END;
                  return true;
                }
              }
            }
          }
        } else if (lexer->lookahead == 'l') {
          advance(lexer);
          if (lexer->lookahead == 's') {
            advance(lexer);
            if (lexer->lookahead == 'e') {
              advance(lexer);
              // The innermost open directive was consumed as trivia, so its
              // `#else` branch is inactive: swallow everything through the
              // matching `#endif` as a single extra token. Only the active
              // (first) branch reaches the grammar, so a directive the
              // grammar has no rule for can never split a construct in two.
              if (top_preproc_is_stray(scanner) &&
                  !valid_symbols[ERROR_SENTINEL] &&
                  !is_word_char(lexer->lookahead)) {
                pop_preproc_kind(scanner);
                swallow_inactive_region(lexer);
                lexer->mark_end(lexer);
                lexer->result_symbol = PREPROC_INACTIVE;
                return true;
              }
              if (try_dedent_for_preproc(scanner, lexer)) {
                return true;
              }
              if (valid_symbols[PREPROC_ELSE]) {
                lexer->mark_end(lexer);
                lexer->result_symbol = PREPROC_ELSE;
                return true;
              }
              // Not emittable yet (an enclosing rule must close first):
              // remember we scanned past it so a trailing line comment
              // doesn't make the scanner decline (see the '/' case above).
              found_preproc_else = true;
            }
          }
        }
      } else if (lexer->lookahead == 'i') {
        advance(lexer);
        if (lexer->lookahead == 'n') {
          // `#indent "off"` / `#indent "on"`: a legacy verbose-syntax
          // directive with no grammar rule. Swallow the whole line as
          // inactive trivia (adding it to the grammar instead costs ~1 MB
          // of parser tables for two real-world occurrences).
          if (match_keyword_rest(lexer, "ndent") &&
              !valid_symbols[ERROR_SENTINEL]) {
            while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
              advance(lexer);
            }
            lexer->mark_end(lexer);
            lexer->result_symbol = PREPROC_INACTIVE;
            return true;
          }
          return false;
        }
        if (lexer->lookahead == 'f') {
          advance(lexer);
          found_preproc_if = true;
          // If an indented block is still open above this line AND the first
          // content line after the '#if' sits outside that block (less
          // indented), close the block before treating the directive.
          // The peek below only advances past mark_end (still at the scan
          // start), so the emitted DEDENT is zero-width and the directive
          // text is re-read by the next scan — nothing is consumed. Same
          // technique as is_type_application_open above. Otherwise —
          // otherwise the skip-line path below starves the parse that needs
          // the '#if' token after the dedent (e.g. a record '}' followed by
          // '#if' around a top-level binding). When the branch content stays
          // at block depth, keep the directive indentation-transparent.
          if (found_end_of_line && valid_symbols[DEDENT] &&
              scanner->indents.size > 1 &&
              indent_length < (uint32_t)peek_indent_length(scanner) &&
              (!peek_is_paren_indent(scanner) || indent_length == 0)) {
            while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
              advance(lexer);
            }
            uint32_t content_indent = 0;
            bool has_content = false;
            while (!lexer->eof(lexer)) {
              if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
                content_indent = 0;
                advance(lexer);
              } else if (lexer->lookahead == ' ') {
                content_indent++;
                advance(lexer);
              } else if (lexer->lookahead == '\t') {
                content_indent += 8;
                advance(lexer);
              } else if (lexer->lookahead == '/') {
                advance(lexer);
                if (lexer->lookahead != '/') { has_content = true; break; }
                while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
                  advance(lexer);
                }
              } else {
                has_content = true;
                break;
              }
            }
            if (has_content &&
                content_indent < (uint32_t)peek_indent_length(scanner)) {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
            // Branch content starts with a token that can never begin an
            // expression or declaration but *continues* the enclosing one
            // (a fluent-chain '.', a closing bracket, a tuple ','): a
            // structured branch could not parse it, so consume the directive
            // line — including the newline and the content line's indent, so
            // the content continues the previous expression exactly as if
            // the directive were not there — as inactive trivia, and
            // remember the stray open so `#else`/`#endif` are consumed
            // textually too.
            if (has_content && !valid_symbols[ERROR_SENTINEL] &&
                (lexer->lookahead == '.' || lexer->lookahead == ')' ||
                 lexer->lookahead == ']' || lexer->lookahead == '}' ||
                 lexer->lookahead == ',')) {
              push_preproc_kind(scanner, PREPROC_STRAY);
              lexer->mark_end(lexer);
              lexer->result_symbol = PREPROC_INACTIVE;
              return true;
            }
            // Branch content stays at block depth: treat the directive lines
            // as whitespace. The peek already consumed up to the content
            // char, so resume the whitespace loop from there.
            found_end_of_line = true;
            indent_length = content_indent;
            continue;
          }
          if ((valid_symbols[NEWLINE] || valid_symbols[INDENT]) &&
              !valid_symbols[PREPROC_IF]) {
            while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
              skip(lexer);
            }
          } else if (!valid_symbols[PREPROC_IF] &&
                     !valid_symbols[ERROR_SENTINEL] &&
                     !(scanner->indents.size > 0 && valid_symbols[DEDENT]) &&
                     !is_word_char(lexer->lookahead)) {
            // The grammar has no preproc rule at this position and no
            // zero-width token (NEWLINE/INDENT/DEDENT) can change that:
            // consume the directive line as inactive trivia and remember the
            // stray open, so the matching `#else`/`#endif` are consumed
            // textually too instead of reaching the grammar. Only the active
            // (first) branch is parsed, so a directive the grammar cannot
            // place never splits a construct in two. Previously this case
            // emitted a token the parser could not shift, producing an ERROR.
            push_preproc_kind(scanner, PREPROC_STRAY);
            while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
              advance(lexer);
            }
            lexer->mark_end(lexer);
            lexer->result_symbol = PREPROC_INACTIVE;
            return true;
          } else {
            if (scanner->indents.size > 0) {
              if (valid_symbols[PREPROC_IF]) {
                if (!valid_symbols[ERROR_SENTINEL] &&
                    !is_word_char(lexer->lookahead)) {
                  // The grammar can adopt the directive here, but peek the
                  // first branch line: if it starts with a token that can
                  // never begin an expression or declaration (a fluent-chain
                  // '.', a match-arm '|', a closing bracket, a tuple ','),
                  // the structured branch could not parse and would split
                  // the surrounding construct. Consume the directive line as
                  // inactive trivia instead: the branch content then parses
                  // inline as part of the enclosing construct, and a later
                  // stray `#else` swallows the alternative branch.
                  while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
                    advance(lexer);
                  }
                  lexer->mark_end(lexer);
                  int32_t branch_start = 0;
                  while (!lexer->eof(lexer)) {
                    if (lexer->lookahead == '\n' || lexer->lookahead == '\r' ||
                        lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                      advance(lexer);
                    } else if (lexer->lookahead == '/') {
                      advance(lexer);
                      if (lexer->lookahead != '/') {
                        branch_start = '/';
                        break;
                      }
                      while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
                        advance(lexer);
                      }
                    } else {
                      branch_start = lexer->lookahead;
                      break;
                    }
                  }
                  if (top_preproc_is_structured(scanner) ||
                      branch_start == '.' || branch_start == '|' ||
                      branch_start == ')' || branch_start == ']' ||
                      branch_start == '}' || branch_start == ',') {
                    // A `#if` nested inside a structured directive's active
                    // branch is consumed as stray trivia: structured-inside-
                    // structured mispairs the OUTER `#endif` (the nested
                    // adoption steals the close; DataContext's indented
                    // sibling `#if VENDOR` blocks, ProvidedTypes' nested
                    // record/class directives). Inlining the active branch
                    // keeps the enclosing books balanced.
                    push_preproc_kind(scanner, PREPROC_STRAY);
                    lexer->result_symbol = PREPROC_INACTIVE;
                    return true;
                  }
                  // Branch content is expression-shaped: decline, so the
                  // internal lexer adopts `#if` structurally. (The peek
                  // advanced past mark_end only; declining returns it all.)
                  return false;
                }
                uint16_t current_indent_length = peek_indent_length(scanner);
                array_push(&scanner->preprocessor_indents,
                           current_indent_length);
                push_preproc_kind(scanner, PREPROC_STRUCTURED);
              } else {
                pop_indent(scanner);
                lexer->result_symbol = DEDENT;
                return true;
              }
            } else {
              push_preproc_kind(scanner, PREPROC_STRUCTURED);
              lexer->mark_end(lexer);
              lexer->result_symbol = PREPROC_IF;
              return true;
            }
          }
        }
      } else {
        if (found_end_of_line) {
          if (valid_symbols[NEWLINE_NO_ALIGNED]) {
            lexer->result_symbol = NEWLINE_NO_ALIGNED;
            return true;
          }
          // '#line N' / '#N' / '#light' are extras, transparent for
          // indentation. Returning false resets the lexer to the scan start
          // (chars advanced past mark_end are returned to the input), so the
          // internal lexer re-reads and consumes the directive in place.
          if (lexer->lookahead == 'l') {
            advance(lexer);
            if (lexer->lookahead == 'i') {
              return false;
            }
          } else if (lexer->lookahead >= '0' && lexer->lookahead <= '9') {
            return false;
          }
          // Other directive lines (#r, #load, #nowarn, ...) are real syntax
          // nodes: fall through so an open indented block can DEDENT before
          // the directive token is lexed internally. Flag like a preproc line
          // so INDENT/NEWLINE stay suppressed (INDENT must fire at the next
          // real line instead).
          found_preprocessor_end = true;
          break;
        }
        return false;
      }
    } else {
      break;
    }
  }

  // We crossed onto a new line: it is not (yet) known to be stranded. The
  // stranded-NEWLINE emission below re-sets the flag in the same scan when
  // this line does hang between levels.
  if (found_end_of_line) {
    scanner->line_stranded = false;
  }

  // Top-level module-element separator, phase 1 of 2 (arm): a next line at
  // column 0 while only the base indent level is open separates module
  // elements, so an application expression cannot absorb the next element
  // (`f 1` followed by `let g ...` otherwise parses `let` as a let-in
  // argument with a MISSING body). Arming must happen here — at the end of
  // the whitespace walk, before any probe below moves the lexer — because
  // the token must CONSUME the walked newline: mark_end lands past it, so
  // an emission always makes progress. That is load-bearing: ELEM_SEP sits
  // in a repeat position, and a zero-width token emitted before the newline
  // can be shifted again at the same position forever, allocating without
  // bound. The EMISSION happens at phase 2, after the keyword probes, so
  // `and`/`then`/... keep their priority over the separator — a probe that
  // fires sets its own mark_end, and a probe that declines only advanced
  // past this mark (rolled back on emission). The next-line check is a
  // single-lookahead whitelist: only lines that begin like a fresh
  // declaration or expression (word char, '[' for attributes or lists,
  // '"') arm; anything else — a '|' union case or match arm, an infix or
  // fluent continuation, a closing bracket, a comment, a directive — takes
  // the normal paths. A blocked separator merely reverts that line pair to
  // the old absorbed-application parse; a wrong separator would split a
  // valid construct, so the whitelist errs toward blocking. Never at EOF
  // (nothing to separate), never after in-walk advances (the consumed span
  // must be pure walked whitespace), never during error recovery (recovery
  // marks every symbol valid regardless of grammar position). Accepted
  // side effect of arming: a zero-width token emitted below in an armed
  // scan (e.g. INDENT) carries this mark_end too and consumes the newline
  // as padding — same parse, slightly shifted extents.
  bool elem_sep_armed = false;
  if (valid_symbols[ELEM_SEP] && !valid_symbols[ERROR_SENTINEL] &&
      found_end_of_line && indent_length == 0 && !lexer->eof(lexer) &&
      scanner->indents.size == 1 && !advanced_in_ws_walk &&
      !found_preprocessor_end && !found_preproc_if && !found_preproc_else &&
      (is_word_char(lexer->lookahead) || lexer->lookahead == '[' ||
       lexer->lookahead == '"')) {
    elem_sep_armed = true;
    lexer->mark_end(lexer);
  }

  // Handle @> and @@> as external tokens to prevent them from being
  // tokenized as infix operators inside quotation expressions.
  // If a dedent is pending for a same-line expression block (e.g. fun x -> x),
  // emit the dedent first and leave the quote closer for the next scan.
  bool failed_at_sign_match = false;
  if (!valid_symbols[ERROR_SENTINEL] && lexer->lookahead == '@') {
    lexer->mark_end(lexer);
    advance(lexer);
    if (lexer->lookahead == '@') {
      advance(lexer);
      if (lexer->lookahead == '>') {
        if (valid_symbols[DEDENT] && scanner->indents.size > 1) {
          pop_indent(scanner);
          lexer->result_symbol = DEDENT;
          return true;
        }
        advance(lexer);
        lexer->mark_end(lexer);
        lexer->result_symbol = UNTYPED_QUOTED_CLOSE;
        return true;
      }
      // @@ not followed by > -- this is an infix operator, not a quotation closer.
      // The lexer has advanced past both @ chars; flag so we skip the keyword chain
      // and treat this like an infix op.
      failed_at_sign_match = true;
      found_start_of_infix_op = true;
    } else if (lexer->lookahead == '>') {
      if (valid_symbols[DEDENT] && scanner->indents.size > 1) {
        pop_indent(scanner);
        lexer->result_symbol = DEDENT;
        return true;
      }
      advance(lexer);
      lexer->mark_end(lexer);
      lexer->result_symbol = QUOTED_CLOSE;
      return true;
    } else {
      // @ not followed by > or @ -- this is an infix operator, not a quotation closer.
      // The lexer has advanced past @; flag so we skip the keyword chain
      // and treat this like an infix op.
      failed_at_sign_match = true;
      found_start_of_infix_op = true;
    }
  }

  // Emit any pending DEDENT/NEWLINE before probing for multi-dollar strings:
  // the probe's mark_end moves the token end past the consumed whitespace, so
  // a DEDENT emitted from inside it swallows the newline and starves the
  // remaining DEDENT/NEWLINE this line break still owes (e.g. before $"...").
  if (!valid_symbols[ERROR_SENTINEL] && lexer->lookahead == '$' &&
      found_end_of_line && scanner->indents.size > 0) {
    uint16_t current_indent_length = peek_indent_length(scanner);
    if (valid_symbols[DEDENT] && indent_length < current_indent_length &&
        (!peek_is_paren_indent(scanner) || indent_length == 0 ||
         lexer->eof(lexer))) {
      pop_indent(scanner);
      lexer->result_symbol = DEDENT;
      return true;
    }
    if (valid_symbols[NEWLINE] && indent_length == current_indent_length &&
        indent_length > 0) {
      lexer->result_symbol = NEWLINE;
      return true;
    }
  }

  if (!valid_symbols[ERROR_SENTINEL] && lexer->lookahead == '$' &&
      valid_symbols[MULTI_DOLLAR_TRIPLE_QUOTE_START]) {
    lexer->mark_end(lexer);

    {
      uint8_t dollar_count = 0;
      while (lexer->lookahead == '$' && dollar_count < UINT8_MAX) {
        advance(lexer);
        dollar_count++;
      }
      if (dollar_count > 1 && scan_n_chars(lexer, '"', 3)) {
        scanner->multi_dollar_count = dollar_count;
        lexer->result_symbol = MULTI_DOLLAR_TRIPLE_QUOTE_START;
        return true;
      }
      // Not a multi-dollar string. Before bailing out, check if DEDENT or
      // NEWLINE should be emitted -- the '$' might be the start of an
      // interpolated string on a new, less-indented line.
      if (found_end_of_line && scanner->indents.size > 0) {
        uint16_t current_indent_length = peek_indent_length(scanner);
        if (valid_symbols[DEDENT] && indent_length < current_indent_length) {
          bool can_dedent_paren_indent = !peek_is_paren_indent(scanner) || indent_length == 0 || lexer->eof(lexer);
          if (can_dedent_paren_indent) {
            pop_indent(scanner);
            lexer->result_symbol = DEDENT;
            return true;
          }
        }
        if (valid_symbols[NEWLINE] && indent_length == current_indent_length && indent_length > 0) {
          lexer->result_symbol = NEWLINE;
          return true;
        }
      }
      return false;
    }
  }

  bool failed_block_opener = false;
  {
    for (size_t i = 0; i < sizeof(block_openers) / sizeof(block_openers[0]); i++) {
      const BlockOpener *op = &block_openers[i];
      if (valid_symbols[op->token] && lexer->lookahead == op->first_char) {
        // No mark_end before the keyword is confirmed: a failed probe must
        // leave the token end at the scan start so a later zero-width DEDENT
        // doesn't consume the newline (which the next scan still needs for
        // NEWLINE/DEDENT decisions). The success path marks below.
        indent_length = lexer->get_column(lexer);
        advance(lexer);
        if (match_keyword_rest(lexer, op->rest)) {
          lexer->mark_end(lexer);
          lexer->result_symbol = op->token;
          return true;
        }
        failed_block_opener = true;
        break;
      }
    }
  }

  // Not gated on found_preprocessor_end: when '#endif' follows, this point is
  // only reached if PREPROC_END was not a valid symbol (a valid one returns
  // above), and the pending NEWLINE_NO_ALIGNED (e.g. an fsi directive's
  // terminator) must close its rule first. It IS deferred while inner indented
  // blocks still need to close — DEDENT comes first then.
  if (found_end_of_line && valid_symbols[NEWLINE_NO_ALIGNED] &&
      !found_start_of_infix_op) {
    bool dedent_first =
        valid_symbols[DEDENT] && scanner->indents.size > 1 &&
        indent_length < (uint32_t)peek_indent_length(scanner) &&
        (!peek_is_paren_indent(scanner) || indent_length == 0 ||
         lexer->eof(lexer));
    if (!dedent_first) {
      lexer->result_symbol = NEWLINE_NO_ALIGNED;
      return true;
    }
  }

  if (!failed_block_opener && !failed_at_sign_match) {

  if (valid_symbols[NEWLINE] && lexer->lookahead == ';') {
    advance(lexer);
    // `;;` is the fsi-style spelling of the same thing; consume both
    // semicolons as one token so the second one doesn't error.
    if (lexer->lookahead == ';') {
      advance(lexer);
    }
    lexer->mark_end(lexer);  // Token = just ';'/';;'; chars after are returned to input
    bool saw_newline = false;
    for (;;) {
      while (lexer->lookahead == ' ' || lexer->lookahead == '\n' ||
             lexer->lookahead == '\r') {
        if (lexer->lookahead == '\n') {
          saw_newline = true;
          indent_length = 0;
        } else if (lexer->lookahead == '\r') {
          // CRLF: skip without counting toward indentation.
        } else if (saw_newline) {
          indent_length++;
        }
        advance(lexer);  // Beyond mark_end: returned to input for next token
      }
      // A trailing line comment ("// ...") means the ';' is the last code on
      // its line, so skip over it (lookahead only — the comment stays in the
      // input) to reach the terminating newline. Without this, the loop would
      // stop at '/' and treat the ';' as a mid-line separator that demands a
      // following expression which isn't there (e.g. `then 1.; // note`).
      if (lexer->lookahead == '/') {
        advance(lexer);
        if (lexer->lookahead == '/') {
          while (lexer->lookahead != '\n' && !lexer->eof(lexer)) {
            advance(lexer);
          }
          continue;  // re-enter to consume the newline after the comment
        }
        break;  // a lone '/' is an operator, not a comment
      }
      break;
    }
    if (saw_newline) {
      found_end_of_line = true;
    }
    // A trailing ';' / ';;' TERMINATES a top-level element; it does not
    // separate two expressions. (';;' is just ';' here — the second semicolon
    // adds nothing but an fsi convention.) Emitting NEWLINE makes it a
    // sequential-expression separator that demands another expression on the
    // right, so `exit 0;` at end of file errors, and `exit 0;` followed by a
    // column-0 `let` absorbs that let as a let-in argument (MISSING `in`).
    // When the semicolon really does end a top-level element — only the base
    // indent is open and the next content is at column 0, or the file ends —
    // hand the position to the module-element separator instead (same
    // whitelist as the arming site above), which is what an element boundary
    // with no semicolon at all already produces; or at EOF decline so the ';'
    // extra takes the characters. Anything else (a ';' inside an indented
    // block or a bracket, mid-line, before a continuation line, before a
    // comment) keeps the separator behaviour.
    bool semi_terminator =
        !found_comment_start && scanner->indents.size == 1 &&
        (lexer->eof(lexer) || (saw_newline && indent_length == 0));
    if (semi_terminator) {
      if (!lexer->eof(lexer) && valid_symbols[ELEM_SEP] &&
          !valid_symbols[ERROR_SENTINEL] &&
          (is_word_char(lexer->lookahead) || lexer->lookahead == '[' ||
           lexer->lookahead == '"')) {
        // Consume the `;;` and the newline: the separator makes progress, so
        // it cannot be re-shifted at this position (see the arming comment).
        lexer->mark_end(lexer);
        elem_sep_armed = true;
      } else if (lexer->eof(lexer)) {
        return false;
      } else {
        semi_terminator = false;
      }
    }
    if (!semi_terminator) {
      found_end_of_line_semi_colon = true;
    }
  }

  if (lexer->lookahead == 't' &&
      (valid_symbols[THEN] || valid_symbols[DEDENT])) {
    advance(lexer);
    if (lexer->lookahead == 'h') {
      advance(lexer);
      if (lexer->lookahead == 'e') {
        advance(lexer);
        if (lexer->lookahead == 'n') {
          advance(lexer);
          if (!is_word_char(lexer->lookahead)) {
            // the 'THEN' token is only valid if we have popped the appropriate
            // amount of dedent tokens.
            // If 'THEN' is not valid we just continue to pop dedent tokens.
            if (valid_symbols[THEN]) {
              lexer->mark_end(lexer);
              lexer->result_symbol = THEN;
              return true;
            } else {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
          }
        }
      }
    }
  } else if (lexer->lookahead == 'a' &&
             (valid_symbols[AND] || valid_symbols[DEDENT])) {
    advance(lexer);
    if (lexer->lookahead == 'n') {
      advance(lexer);
      if (lexer->lookahead == 'd') {
        advance(lexer);
        if (lexer->lookahead == ' ') {
          // the 'AND' token is only valid if we have popped the appropriate
          // amount of dedent tokens.
          // If 'AND' is not valid we just continue to pop dedent tokens.
          if (valid_symbols[AND]) {
            // Greedy-`and` guard: a line-leading `and` sitting LEFT of the
            // current indent scope belongs to an outer construct (e.g.
            // `and B() = ...` closing a type whose last member was a property
            // accessor block) — unless it introduces another accessor
            // (`and set (v) = ...`), which legitimately continues the
            // accessor scope even though that scope's indent anchors at a
            // mid-line column. Peeking past `and` moves beyond a possible
            // mark_end position, so on the accessor path we decline and let
            // the internal lexer produce the string-literal `and` token.
            if (found_end_of_line && valid_symbols[DEDENT] &&
                scanner->indents.size > 0 &&
                indent_length < (uint32_t)peek_indent_length(scanner)) {
              while (lexer->lookahead == ' ') {
                advance(lexer);
              }
              char word[9];
              int word_len = 0;
              while (is_word_char(lexer->lookahead) && word_len < 8) {
                word[word_len++] = (char)lexer->lookahead;
                advance(lexer);
              }
              word[word_len] = '\0';
              bool accessor_next =
                  // `and [<Attr>] set (v) = ...`: an attribute set can only
                  // introduce another accessor here.
                  (word_len == 0 && lexer->lookahead == '[') ||
                  (!is_word_char(lexer->lookahead) &&
                   (keyword_equals(word, "get") || keyword_equals(word, "set") ||
                    keyword_equals(word, "inline") ||
                    keyword_equals(word, "private") ||
                    keyword_equals(word, "internal") ||
                    keyword_equals(word, "public")));
              if (!accessor_next) {
                pop_indent(scanner);
                lexer->result_symbol = DEDENT;
                return true;
              }
              return false;
            }
            lexer->mark_end(lexer);
            lexer->result_symbol = AND;
            return true;
          } else {
            pop_indent(scanner);
            lexer->result_symbol = DEDENT;
            return true;
          }
        } else if (!is_word_char(lexer->lookahead)) {
          // Handle 'and' followed by newline/EOF (not word char, not space).
          // Mirror the space-branch: emit AND when valid; otherwise trigger
          // a keyword-driven DEDENT to close intermediate scopes so the parser
          // retries AND at the correct level on the next scan.
          if (valid_symbols[AND]) {
            lexer->mark_end(lexer);
            lexer->result_symbol = AND;
            return true;
          } else if (valid_symbols[DEDENT]) {
            pop_indent(scanner);
            lexer->result_symbol = DEDENT;
            return true;
          }
        }
      }
    }
  } else if (lexer->lookahead == 'w' &&
             (valid_symbols[WITH] || valid_symbols[DEDENT])) {
    advance(lexer);
    if (lexer->lookahead == 'i') {
      advance(lexer);
      if (lexer->lookahead == 't') {
        advance(lexer);
        if (lexer->lookahead == 'h') {
          advance(lexer);
          if (!is_word_char(lexer->lookahead)) {
            // Force-close a try-body scope whose terminating 'with' begins a
            // new line at the same column as the body, so an ordinary dedent
            // won't fire. This must take priority over the same-indent NEWLINE
            // heuristic below: unlike a class/record body, a try body has no
            // 'with' augmentation of its own, so the 'with' must close it.
            if (valid_symbols[DEDENT] && !valid_symbols[WITH] &&
                peek_is_try_indent(scanner)) {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
            // If 'with' sits at the same indent as the current scope and the
            // grammar isn't yet expecting WITH, emit NEWLINE first so the
            // preceding statement closes before the augmentation opens.
            if (valid_symbols[NEWLINE] && found_end_of_line &&
                !valid_symbols[WITH] &&
                scanner->indents.size > 0 &&
                indent_length == (uint32_t)peek_indent_length(scanner)) {
              lexer->result_symbol = NEWLINE;
              return true;
            }
            // WITH only valid once the right number of DEDENTs have popped.
            if (valid_symbols[WITH]) {
              lexer->mark_end(lexer);
              lexer->result_symbol = WITH;
              return true;
            } else {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
          } else if (valid_symbols[WITH] && !is_word_char(lexer->lookahead)) {
            // Handle 'with' followed by newline/EOF (not word char, not space).
            // Only emit WITH here. Unlike 'and', do NOT force a keyword-driven
            // DEDENT when only DEDENT is valid: `with` on a new line inside a
            // class body is a CONTINUATION of the enclosing type (starting a
            // type_extension_elements), so closing the enclosing scope early
            // would truncate the type definition. Let the natural indent
            // tracking (line ~1050) emit any needed DEDENTs and NEWLINE.
            lexer->mark_end(lexer);
            lexer->result_symbol = WITH;
            return true;
          }
        }
      }
    }
  } else if (lexer->lookahead == 'd' && valid_symbols[DO_KEYWORD]) {
    advance(lexer);
    if (lexer->lookahead == 'o') {
      advance(lexer);
      // Exclude 'do!' so computation-expression do-bang is never claimed.
      if (!is_word_char(lexer->lookahead) && lexer->lookahead != '!') {
        lexer->mark_end(lexer);
        lexer->result_symbol = DO_KEYWORD;
        return true;
      }
    }
  } else if (lexer->lookahead == 'i' && valid_symbols[IN]) {
    advance(lexer);
    if (lexer->lookahead == 'n') {
      advance(lexer);
      if (!is_word_char(lexer->lookahead)) {
        // Produce the IN token to close an _expression_block_for_let.
        // Pop the indent that was pushed by the matching INDENT, since
        // _in replaces _dedent as the block terminator.
        pop_indent(scanner);
        lexer->mark_end(lexer);
        lexer->result_symbol = IN;
        return true;
      }
    }
  } else if (lexer->lookahead == 'e' &&
             (valid_symbols[ELSE] || valid_symbols[ELIF] ||
              valid_symbols[END] || valid_symbols[DEDENT])) {
    advance(lexer);
    int16_t token_indent_level = lexer->get_column(lexer);
    if (lexer->lookahead == 'l') {
      advance(lexer);
      if (lexer->lookahead == 's' &&
          (valid_symbols[ELSE] || valid_symbols[DEDENT])) {
        advance(lexer);
        if (lexer->lookahead == 'e') {
          advance(lexer);
          if (!is_word_char(lexer->lookahead)) {
            if (valid_symbols[ELSE]) {
              // Don't pop paren-kind scopes: an if/else inside a call's
              // parens may sit at any indentation; ')' closes that scope.
              if (scanner->indents.size > 0 &&
                  !peek_is_paren_indent(scanner) &&
                  token_indent_level < peek_indent_length(scanner)) {
                pop_indent(scanner);
                lexer->result_symbol = DEDENT;
                return true;
              } else {
                lexer->mark_end(lexer);
                // Only fold "else if" into ELIF when they share a line. If a
                // newline separates them this is an `else` whose block body
                // starts with an `if` (and may be followed by more statements),
                // so skip only same-line spacing here, never newlines.
                for (;;) {
                  if (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
                    advance(lexer);
                  } else {
                    break;
                  }
                }
                if (lexer->lookahead == 'i') {
                  advance(lexer);
                  if (lexer->lookahead == 'f') {
                    advance(lexer);
                    if (!is_word_char(lexer->lookahead)) {
                      lexer->mark_end(lexer);
                      lexer->result_symbol = ELIF;
                      return true;
                    }
                  }
                }
                lexer->result_symbol = ELSE;
                return true;
              }
            } else {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
          }
        }
      } else if (lexer->lookahead == 'i' &&
                 (valid_symbols[ELIF] || valid_symbols[DEDENT])) {
        advance(lexer);
        if (lexer->lookahead == 'f') {
          advance(lexer);
          if (!is_word_char(lexer->lookahead)) {
            if (valid_symbols[ELIF]) {
              if (scanner->indents.size > 0 &&
                  !peek_is_paren_indent(scanner) &&
                  token_indent_level < peek_indent_length(scanner)) {
                pop_indent(scanner);
                lexer->result_symbol = DEDENT;
                return true;
              } else {
                lexer->mark_end(lexer);
                lexer->result_symbol = ELIF;
                return true;
              }
            } else {
              pop_indent(scanner);
              lexer->result_symbol = DEDENT;
              return true;
            }
          }
        }
      }
    } else if (lexer->lookahead == 'n' &&
               (valid_symbols[END] || valid_symbols[DEDENT])) {
      advance(lexer);
      if (lexer->lookahead == 'd') {
        advance(lexer);
        if (!is_word_char(lexer->lookahead)) {
          if (valid_symbols[END]) {
            lexer->mark_end(lexer);
            lexer->result_symbol = END;
            return true;
          } else if (valid_symbols[DEDENT] && scanner->indents.size > 0) {
            pop_indent(scanner);
            lexer->result_symbol = DEDENT;
            return true;
          }
        }
      }
    }
  } else if (is_bracket_end(lexer)) {
    found_bracket_end = true;
  } else if (is_infix_op_start(lexer)) {
    found_start_of_infix_op = true;
  } else if (lexer->lookahead == '|') {
    skip(lexer);
    int32_t after_pipe = lexer->lookahead;
    if (after_pipe == ']' || after_pipe == '}') {
      found_bracket_end = true;
    } else if (after_pipe == '>') {
      found_start_of_infix_op = true;
    } else if (after_pipe == ' ' ||
               // A '|' glued to a pattern-start character (`|"A"`, `|_`,
               // `|(p, q)`, `|Some x`, `|[x]`, ``|``id`` ``) is a match arm
               // or union case exactly like the spaced form, not an infix
               // operator. Operator characters (`|>`, `||`, `|.`) keep the
               // infix treatment below.
               after_pipe == '"' || after_pipe == '(' || after_pipe == '[' ||
               after_pipe == '`' || is_word_char(after_pipe)) {
      if (!found_end_of_line) {
        found_start_of_infix_op = true;
        found_same_line_pipe_infix = true;
      } else {
        if (indent_length == 0) {
          indent_length = 1;
        }
        if (scanner->indents.size > 0) {
          uint16_t current_indent_length = peek_indent_length(scanner);
          if (found_end_of_line && indent_length == current_indent_length &&
              indent_length > 0 && !found_start_of_infix_op &&
              !found_bracket_end) {
            if (valid_symbols[NEWLINE] && !found_preprocessor_end) {
              lexer->result_symbol = NEWLINE;
              return true;
            }
          }
        }
      }
    } else {
      found_start_of_infix_op = true;
    }
  } else if (lexer->lookahead == '(') {
    skip(lexer);
    // Recorded for the SRTP typar-group veto below: this scan began *on* a
    // '(', so any group seen from here is inside that paren, i.e. the request
    // belongs to the enclosing expression's paren rather than to the group's.
    skipped_open_paren = true;
    if (lexer->lookahead == '*') {
      // `(*)` is the multiplication operator reference (F# spec 3.10), not a
      // comment opener — peek one further so it doesn't suppress INDENT and
      // force the comment interpretation (`let f = (*) 2 3`).
      skip(lexer);
      if (lexer->lookahead != ')') {
        found_comment_start = true;
      }
    }
  }

  } // end of !failed_block_opener && !failed_at_sign_match

  if (valid_symbols[NEWLINE] && found_end_of_line_semi_colon &&
      !found_comment_start && !found_bracket_end) {
    // If semicolon was followed by newlines that drop the indentation,
    // fall through to DEDENT logic instead of emitting NEWLINE
    bool needs_dedent = found_end_of_line && valid_symbols[DEDENT] &&
                        scanner->indents.size > 0 &&
                        indent_length < (uint32_t)peek_indent_length(scanner) &&
                        // Paren-kind scopes don't dedent on under-indented
                        // lines, so don't withhold the NEWLINE for them.
                        (!peek_is_paren_indent(scanner) ||
                         indent_length == 0 || lexer->eof(lexer));
    if (needs_dedent && indent_length > 0) {
      // Only defer to DEDENT when the next line lands exactly on an open
      // indentation level. A line "between" levels (e.g. record fields
      // wrapped at an arbitrary lower indent after `A = "x";`) continues
      // the current construct instead.
      bool has_matching_level = false;
      for (uint32_t lvl = 0; lvl + 1 < scanner->indents.size; lvl++) {
        if (*array_get(&scanner->indents, lvl) == indent_length) {
          has_matching_level = true;
          break;
        }
      }
      if (!has_matching_level) {
        needs_dedent = false;
      }
    }
    if (!needs_dedent) {
      lexer->result_symbol = NEWLINE;
      return true;
    }
  }

  if (valid_symbols[TRY_INDENT] && !valid_symbols[ERROR_SENTINEL] &&
      !found_bracket_end && !found_preprocessor_end &&
      !found_same_line_pipe_infix) {
    // Like INDENT, but the scope is tagged so it can be force-closed when its
    // terminating `with`/`finally` sits at the same column as the body.
    push_indent(scanner, indent_length, INDENT_TRY);
    lexer->result_symbol = TRY_INDENT;
    return true;
  }

  if (valid_symbols[INDENT] && !valid_symbols[ERROR_SENTINEL] &&
      !found_bracket_end && !found_preprocessor_end &&
      !found_same_line_pipe_infix &&
      // A block comment trailing the current line must not anchor the new
      // block: without this, `let f x = (* c *)` + an indented body pushes
      // the comment's column, and the body line immediately DEDENTs it back
      // out. Decline instead; after the comment is consumed as an extra, the
      // re-scan crosses the newline and INDENT anchors to the body line.
      !(found_comment_start && !found_end_of_line)) {
    uint8_t indent_flags = 0;
    if (!found_end_of_line) {
      indent_flags |= INDENT_KIND_MIDLINE_FLAG;
      if (scanner->line_stranded) {
        indent_flags |= INDENT_KIND_STRANDED_LINE_FLAG;
      }
    }
    push_indent(scanner, indent_length,
                (IndentKind)(INDENT_NORMAL | indent_flags));
    lexer->result_symbol = INDENT;
    return true;
  }

  if (valid_symbols[PAREN_INDENT] && !valid_symbols[ERROR_SENTINEL] &&
      !found_bracket_end &&
      !found_preprocessor_end && !found_same_line_pipe_infix) {
    // In `((^a or ^b) : (static member M : ^a -> ^b) x)` the inner '(' opens a
    // trait call's typar group, not a parenthesised expression. PAREN_INDENT is
    // zero-width and the parser takes it whenever it is valid, so emitting it
    // just inside that '(' would commit to paren_expression and kill the group
    // reading. Decline when what follows is exactly a typar group closed by
    // `) : (`, which the body of a parenthesised expression can never be.
    //
    // `skipped_open_paren` keeps the *enclosing* expression's own indent: this
    // same group is also what follows the outer '(', and that request — which
    // arrives with the scan sitting on the '(' the probe above skipped — must
    // still produce PAREN_INDENT, or the whole parenthesised expression dies.
    // No mark_end, so returning false discards the peek entirely.
    if (!skipped_open_paren &&
        (lexer->lookahead == '^' || lexer->lookahead == '\'') &&
        !found_end_of_line && !advanced_in_ws_walk &&
        is_srtp_typar_group_ahead(lexer)) {
      return false;
    }
    // Like INDENT, but tracked separately as a paren indent so DEDENT/NEWLINE
    // logic can be more lenient inside parenthesized expressions, where the
    // closing ')' determines scope rather than indentation alone.
    push_indent(scanner, indent_length, INDENT_PAREN);
    lexer->result_symbol = PAREN_INDENT;
    return true;
  }

  if (valid_symbols[BRACE_INDENT] && !valid_symbols[ERROR_SENTINEL] &&
      !found_bracket_end &&
      !found_preprocessor_end && !found_same_line_pipe_infix) {
    // Opens a '{...}' record/CE field block (see INDENT_BRACE).
    push_indent(scanner, indent_length, INDENT_BRACE);
    lexer->result_symbol = BRACE_INDENT;
    return true;
  }

  // Fires after '<' if the type args span multiple lines — either we just
  // consumed a newline, or peek-ahead shows one before the matching '>' (the
  // variant where the first arg shares the line with '<').
  if (valid_symbols[TYPE_APP_INDENT] && !valid_symbols[ERROR_SENTINEL] &&
      !found_bracket_end &&
      !found_preprocessor_end && !found_same_line_pipe_infix) {
    bool is_multiline = found_end_of_line || is_multiline_type_app_ahead(lexer);
    if (is_multiline) {
      push_indent(scanner, indent_length, INDENT_TYPE_APP);
      lexer->result_symbol = TYPE_APP_INDENT;
      return true;
    }
  }

  if (scanner->indents.size > 0) {
    bool is_paren_indent = peek_is_paren_indent(scanner);
    bool is_brace_indent = peek_is_brace_indent(scanner);
    uint16_t current_indent_length = peek_indent_length(scanner);

    // '>' closes a TYPE_APP_INDENT the same way ')' closes a PAREN_INDENT —
    // gated on the type-app bit so we don't pop ordinary paren scopes here.
    bool found_type_app_close = peek_is_type_app_indent(scanner) &&
                                lexer->lookahead == '>' &&
                                valid_symbols[DEDENT];

    if ((found_bracket_end || found_type_app_close) && valid_symbols[DEDENT]) {
      pop_indent(scanner);
      lexer->result_symbol = DEDENT;
      return true;
    }

    if (found_end_of_line) {
      // Inside a brace block, a line that is not more indented than the anchor
      // (including one left of the first item) separates items rather than
      // dedenting out — the '}' is what closes the block.
      bool brace_separator =
          is_brace_indent && indent_length < current_indent_length;
      if ((indent_length == current_indent_length || brace_separator) &&
          indent_length > 0 &&
          !found_start_of_infix_op && !found_bracket_end) {
        if (valid_symbols[NEWLINE] && !found_preprocessor_end &&
            !found_comment_start) {
          lexer->result_symbol = NEWLINE;
          return true;
        }
      }

      // Consume a pending stranded-dedent: the previous scan emitted a DEDENT
      // that closed a nested block, but this line sits above the enclosing
      // block's indent. Emit the NEWLINE the enclosing block owes so the line
      // starts a new element instead of being stranded. Guarded on a real
      // preceding stranded DEDENT so ordinary more-indented continuation lines
      // (e.g. a multi-line application argument) are untouched.
      if (prev_stranded_dedent && indent_length > current_indent_length &&
          valid_symbols[NEWLINE] && !found_start_of_infix_op &&
          !found_bracket_end && !found_preprocessor_end &&
          !found_comment_start) {
        // This line hangs between two open levels; scopes anchored mid-line
        // on it must not claim following same-column lines as continuations.
        scanner->line_stranded = true;
        lexer->result_symbol = NEWLINE;
        return true;
      }

      // Top-level module-element separator, phase 2 of 2 (emit). All
      // eligibility checks — and the mark_end that makes the token consume
      // the walked newline — happened at the arming site right after the
      // whitespace walk; see the comment there. Emitting this late keeps
      // the keyword probes' priority (an `and` continuation line must
      // produce AND, not a separator), and any lookahead they consumed past
      // the armed mark_end is returned to the input here.
      if (elem_sep_armed) {
        lexer->result_symbol = ELEM_SEP;
        return true;
      }

      bool can_dedent_preproc;

      if (scanner->preprocessor_indents.size > 0) {
        uint16_t current_preproc_length =
            *array_back(&scanner->preprocessor_indents);
        can_dedent_preproc = current_preproc_length < indent_length;
      } else {
        can_dedent_preproc = true;
      }

      bool can_dedent_infix_op;

      if (found_start_of_infix_op) {
        // A continuation line that starts with an infix operator is part of the
        // preceding expression (F#'s offside rule), so never DEDENT into it —
        // even when it sits several columns left of the expression it continues
        // (e.g. an `&&` line under a multi-line `if`/`elif` condition).
        can_dedent_infix_op = false;
      } else {
        can_dedent_infix_op = true;
      }

      // Inside paren-indented blocks, avoid DEDENT on ordinary under-indented
      // continuation lines. But still allow it at true EOF / column 0 so a
      // paren indent can't get stranded forever if its closing bracket was
      // consumed by the grammar rather than seen here at the start of a line.
      bool can_dedent_paren_indent = !(is_paren_indent || is_brace_indent) || indent_length == 0 || lexer->eof(lexer);

      // A line that sits strictly DEEPER than the line which opened a
      // mid-line-anchored scope continues the anchored expression, even when
      // it is left of the anchor column itself: `let a = Some <|` anchors at
      // `Some` (col 12), but F#'s offside rule measures the continuation
      // against the `let` line's indent (4), so an operand at col 8 belongs
      // to the expression and must not be DEDENTed into. A line at or left
      // of the anchor line's indent is a sibling/outer construct and still
      // dedents (`let c = ()` followed by `let d = ()` at the same column).
      // All conditions must hold, each excluding a shape where the dedent is
      // legitimate: a genuine pushed enclosing level (size > 2 — a top-level
      // module body is not indent-scoped and its members MUST dedent out),
      // of line-column kind (paren/brace/type-app indents are synthetic),
      // the line strictly deeper than that enclosing level, and the anchor
      // NOT opened on a stranded line (a stranded declaration like
      // `let c = ()` at col 8 under a col-4 block is followed by sibling
      // declarations at its own column, which must still dedent).
      bool can_dedent_midline_anchor = true;
      if (top_indent_is_midline_anchor(scanner) &&
          !top_indent_is_stranded_line(scanner) &&
          scanner->indents.size > 2) {
        uint8_t below_kind_raw =
            *array_get(&scanner->indent_kinds, scanner->indents.size - 2);
        IndentKind below_kind =
            (IndentKind)(below_kind_raw & ~INDENT_KIND_FLAGS_MASK);
        if ((below_kind == INDENT_NORMAL || below_kind == INDENT_TRY) &&
            indent_length >
                *array_get(&scanner->indents, scanner->indents.size - 2)) {
          can_dedent_midline_anchor = false;
        }
      }

      if (indent_length < current_indent_length && !found_bracket_end &&
          can_dedent_preproc && can_dedent_infix_op &&
          can_dedent_midline_anchor &&
          (!valid_symbols[TUPLE_MARKER] || valid_symbols[ERROR_SENTINEL]) && can_dedent_paren_indent) {
        pop_indent(scanner);
        // If this line closed a nested block but still sits above the enclosing
        // block's own indent (e.g. dedenting from a nested module body back to
        // an outer module whose next member is indented past the outer
        // module's offside column), the enclosing block owes an item
        // separator. Record that so the next scan emits the NEWLINE — a bare
        // DEDENT here would strand the line between two open levels.
        //
        // Require a genuine enclosing indent level (size > 1, i.e. we landed on
        // a pushed level, not the base level 0). A top-level `module M`/namespace
        // body is not indent-scoped, so an expression block (e.g. a `do ()` body)
        // closing back to the base level is an ordinary dedent, not a stranded
        // one — injecting a separator there wrongly glues sibling members into a
        // sequential_expression.
        if (scanner->indents.size > 1 &&
            indent_length > peek_indent_length(scanner)) {
          scanner->stranded_dedent = true;
        }
        lexer->result_symbol = DEDENT;
        return true;
      }
    }
  }

  return false;
}

static unsigned serialize(Scanner *scanner, char *buffer) {
  size_t size = 0;

  buffer[size++] = (char)scanner->multi_dollar_count;
  buffer[size++] = (char)scanner->stranded_dedent;
  buffer[size++] = (char)scanner->line_stranded;

  size_t preprocessor_count = scanner->preprocessor_indents.size;
  if (preprocessor_count > UINT8_MAX) {
    preprocessor_count = UINT8_MAX;
  }

  buffer[size++] = (char)preprocessor_count;

  for (size_t iter = 0; iter < preprocessor_count &&
                        size < TREE_SITTER_SERIALIZATION_BUFFER_SIZE;
       iter++) {
    char e = *array_get(&scanner->preprocessor_indents, iter);
    buffer[size++] = e;
  }

  size_t indent_count = scanner->indents.size > 0 ? scanner->indents.size - 1 : 0;
  if (indent_count > UINT8_MAX) {
    indent_count = UINT8_MAX;
  }
  buffer[size++] = (char)indent_count;

  uint32_t iter = 1;
  for (; iter <= indent_count && size < TREE_SITTER_SERIALIZATION_BUFFER_SIZE; ++iter) {
    buffer[size++] = (char)*array_get(&scanner->indents, iter);
  }

  // One byte per indent kind. Stack depth is bounded by source nesting (~10),
  // so the extra bytes vs bit-packing are negligible.
  for (uint32_t i = 1; i <= indent_count && size < TREE_SITTER_SERIALIZATION_BUFFER_SIZE; ++i) {
    buffer[size++] = (char)*array_get(&scanner->indent_kinds, i);
  }

  size_t preproc_kind_count = scanner->preproc_kinds.size;
  if (preproc_kind_count > UINT8_MAX) {
    preproc_kind_count = UINT8_MAX;
  }
  if (size < TREE_SITTER_SERIALIZATION_BUFFER_SIZE) {
    buffer[size++] = (char)preproc_kind_count;
  }
  for (size_t i = 0; i < preproc_kind_count &&
                     size < TREE_SITTER_SERIALIZATION_BUFFER_SIZE;
       i++) {
    buffer[size++] = (char)*array_get(&scanner->preproc_kinds, i);
  }

  return size;
}

static void deserialize(Scanner *scanner, const char *buffer, unsigned length) {
  // array_clear keeps the allocated capacity; deserialize runs on every
  // scanner-state restore (constantly, under GLR), and array_delete here
  // would free + re-malloc all four buffers each time.
  array_clear(&scanner->indents);
  array_push(&scanner->indents, 0);
  array_clear(&scanner->indent_kinds);
  array_push(&scanner->indent_kinds, (uint8_t)INDENT_NORMAL);

  array_clear(&scanner->preprocessor_indents);
  array_clear(&scanner->preproc_kinds);
  scanner->multi_dollar_count = 0;
  scanner->stranded_dedent = false;
  scanner->line_stranded = false;
  if (length > 0) {
    size_t size = 0;
    scanner->multi_dollar_count = (uint8_t)buffer[size++];

    if (size >= length) return;
    scanner->stranded_dedent = (uint8_t)buffer[size++];

    if (size >= length) return;
    scanner->line_stranded = (uint8_t)buffer[size++];

    if (size >= length) return;
    size_t preprocessor_count = (uint8_t)buffer[size++];

    size_t preproc_end = size + preprocessor_count;
    if (preproc_end > length) preproc_end = length;
    for (; size < preproc_end; size++) {
      array_push(&scanner->preprocessor_indents, (unsigned char)buffer[size]);
    }

    if (size >= length) return;
    size_t indent_count = (uint8_t)buffer[size++];
    size_t indent_bytes_end = size + indent_count;
    if (indent_bytes_end > length) indent_bytes_end = length;
    for (; size < indent_bytes_end; size++) {
      array_push(&scanner->indents, (unsigned char)buffer[size]);
    }

    size_t actual_indent_count = scanner->indents.size > 0 ? scanner->indents.size - 1 : 0;
    for (size_t i = 0; i < actual_indent_count; i++) {
      uint8_t kind = (size < length) ? (uint8_t)buffer[size++] : (uint8_t)INDENT_NORMAL;
      array_push(&scanner->indent_kinds, kind);
    }

    if (size >= length) return;
    size_t preproc_kind_count = (uint8_t)buffer[size++];
    size_t preproc_kind_end = size + preproc_kind_count;
    if (preproc_kind_end > length) preproc_kind_end = length;
    for (; size < preproc_kind_end; size++) {
      array_push(&scanner->preproc_kinds, (unsigned char)buffer[size]);
    }
  }
}

static Scanner *create() {
  Scanner *scanner = ts_calloc(1, sizeof(Scanner));
  array_init(&scanner->indents);
  array_init(&scanner->indent_kinds);
  array_init(&scanner->preprocessor_indents);
  array_init(&scanner->preproc_kinds);
  deserialize(scanner, NULL, 0);
  return scanner;
}

static void destroy(Scanner *scanner) {
  array_delete(&scanner->indents);
  array_delete(&scanner->indent_kinds);
  array_delete(&scanner->preprocessor_indents);
  array_delete(&scanner->preproc_kinds);
  ts_free(scanner);
}

#endif // TREE_SITTER_FSHARP_SCANNER_H_

