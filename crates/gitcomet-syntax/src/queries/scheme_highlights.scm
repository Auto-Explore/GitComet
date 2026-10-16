; Scheme.
;
; tree-sitter-scheme 0.24.7's queries/highlights.scm (MIT), edited to match the other
; Lisps here (Common Lisp, Racket, Emacs Lisp, Janet). It was written for last pattern
; wins, as GitComet resolves. Edits:
;
;   * (character) and (boolean) were @constant.builtin: now @character and @boolean.
;     (keyword) had no capture: now @constant. `#(` and `#vu8(` join the brackets.
;   * `(list . (symbol) @function)` coloured every list head, `let` and `do`
;     bindings and syntax-rules patterns included, and the head-position
;     @function.builtin regex (~200 names) coloured library calls. Both dropped: no
;     Lisp here colours calls. So is the `[...]` rule that undid the first.
;   * Quoting: upstream painted quoted data @constant four levels deep. Now the
;     markers are @operator and only a directly quoted symbol is @constant.
;   * The keyword regex became an #any-of? list (same match, readable) and gained
;     the R6RS/R7RS forms it lacked: define-record-type, define-values,
;     define-library, letrec*, case-lambda, parameterize, guard, include,
;     cond-expand, delay-force, syntax-case, with-syntax.
;   * Added definitions: procedure and syntax names @function with parameters,
;     value names plain @variable, record types @type, a named let's name
;     @function, and library names @namespace.
;   * `#;` comments: upstream's four hand-written levels became one
;     @comment.discarded rule, which GitComet greys whole at any depth.

["(" ")" "[" "]" "{" "}" "#(" "#vu8("] @punctuation.bracket

; ---- Literals ---------------------------------------------------------------

(number) @number
(character) @character
(boolean) @boolean
(keyword) @constant
(symbol) @variable

(string) @string

(escape_sequence) @escape

; ---- Quoting ----------------------------------------------------------------

(quote "'" @operator)
(quasiquote "`" @operator)
(unquote "," @operator)
(unquote_splicing ",@" @operator)
(syntax "#'" @operator)
(quasisyntax "#`" @operator)
(unsyntax "#," @operator)
(unsyntax_splicing "#,@" @operator)

; 'foo is a symbol literal.
(quote . (symbol) @constant)

((symbol) @operator
 (#match? @operator "^(\\+|-|\\*|/|=|>|<|>=|<=)$"))

; ---- Syntactic forms, in head position --------------------------------------

(list
  .
  (symbol) @keyword
  (#any-of? @keyword
    "define" "define-syntax" "define-record-type" "define-values" "define-library"
    "lambda" "λ" "case-lambda" "let" "let*" "letrec" "letrec*" "let-values"
    "let*-values" "let-syntax" "letrec-syntax" "syntax-rules" "syntax-case"
    "with-syntax" "identifier-syntax" "set!" "begin" "if" "cond" "case" "when"
    "unless" "do" "and" "or" "else" "=>" "delay" "delay-force" "parameterize"
    "guard" "assert" "quote" "quasiquote" "unquote" "unquote-splicing"
    "quote-splicing" "library" "export" "import" "include" "include-ci"
    "cond-expand" "rename" "only" "except" "prefix"))

; ---- Definitions ------------------------------------------------------------

; (define (name arg ...) body).
(list
  .
  (symbol) @_def
  .
  (list
    .
    (symbol) @function
    (symbol)* @variable.parameter)
  (#any-of? @_def "define" "define-syntax"))

; (define name value) binds a value; (define name (lambda ...)) a procedure.
; Same node, so the later pattern wins.
(list
  .
  (symbol) @_def
  .
  (symbol) @variable
  (#eq? @_def "define"))

(list
  .
  (symbol) @_def
  .
  (symbol) @function
  .
  (list
    .
    (symbol) @_lambda
    (#any-of? @_lambda "lambda" "λ" "case-lambda"))
  (#eq? @_def "define"))

(list
  .
  (symbol) @_def
  .
  (symbol) @function
  (#eq? @_def "define-syntax"))

(list
  .
  (symbol) @_def
  .
  (list
    (symbol) @variable)
  (#eq? @_def "define-values"))

(list
  .
  (symbol) @_def
  .
  (symbol) @type
  (#eq? @_def "define-record-type"))

; (lambda (arg ...) body) and (lambda args body).
(list
  .
  (symbol) @_lambda
  .
  [(list
     (symbol) @variable.parameter)
   (symbol) @variable.parameter]
  (#any-of? @_lambda "lambda" "λ"))

; A named let binds a local procedure.
(list
  .
  (symbol) @_let
  .
  (symbol) @function
  (#eq? @_let "let"))

; Library names: (library (demo matrix) ...) and the sets an import names.
(list
  .
  (symbol) @_lib
  .
  (list
    (symbol) @namespace)
  (#any-of? @_lib "library" "define-library"))

(list
  .
  (symbol) @_import
  (list
    .
    (symbol) @namespace
    (#not-any-of? @namespace "only" "except" "prefix" "rename" "for"))
  (#eq? @_import "import"))

(list
  .
  (symbol) @_set
  .
  (list
    .
    (symbol) @namespace)
  (#any-of? @_set "only" "except" "prefix" "rename" "for"))

;; datum comment ;;

(comment
  (_)) @comment.discarded

[(comment)
 (block_comment)
 (directive)] @comment
