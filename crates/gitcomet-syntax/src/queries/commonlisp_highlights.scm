; Common Lisp.
;
; tree-sitter-commonlisp 0.4.1 ships tags.scm only. Derived from nvim-treesitter's
; runtime/queries/commonlisp/highlights.scm (Apache-2.0) at commit
; ca581c3436fc2b236847f5b739ebc263739e5ca6, reshaped to match the other Lisps here
; (Racket, Scheme, Emacs Lisp, Janet; Clojure for head-position keywords). Edits:
;
;   * Its three `#lua-match?` rules (`*special*`, `+constant+`, `&key`) are
;     `#match?` now. Unevaluated, all three fired on every symbol and the last won:
;     every symbol rendered @string.special.symbol (53% of sample.lisp).
;   * Its two generated name lists are gone: 92 macros as @function.macro, then 752
;     functions as @function.builtin, which repainted 91 of those 92 macros
;     (`when`, `loop`, `defun`) as calls. No Lisp here colours calls. In their place,
;     one #any-of? list of special operators and syntax-like macros, @keyword.
;   * Captures renamed to GitComet's: @function.macro -> @keyword;
;     @string.special.symbol -> @constant (keywords, `#:sym`, 'sym) or @type
;     (defmethod specialisers); quote markers @string.escape -> @operator;
;     @variable.builtin -> @variable.special; @module -> @namespace; @spell dropped.
;   * `(include_reader_macro)` spanned the whole `#+feature form`; only the marker
;     and the feature are @preproc now.
;   * Dropped `(with_clause type: (_) @type)` (LOOP's `with x fixnum`): on this
;     5.9 MB grammar it alone cost 22 ms of a 78 ms release Query::new.
;   * Added: `#'fn` @function; the names defvar/defparameter (@variable.special),
;     defconstant (@constant) and defclass/defstruct/deftype/define-condition (@type)
;     bind; package-qualifier punctuation; `(char_lit "#")`, which otherwise lost to
;     the `"#"` rule because the child capture is emitted after its parent's.

(sym_lit) @variable

; ---- Literals ---------------------------------------------------------------

(comment) @comment
(block_comment) @comment
(dis_expr) @comment

(str_lit) @string
(format_specifier) @escape

(num_lit) @number
(array_dimension) @number
["#C" "#c" "#0A" "#0a"] @number

; `#:foo` is a `#` token beside a `kwd_lit`; colour it with the keyword.
"#" @constant
(kwd_lit) @constant
(self_referential_reader_macro) @constant

(char_lit) @character
(char_lit "#" @character)

(nil_lit) @constant.builtin

((sym_lit) @boolean
  (#any-of? @boolean "t" "T"))

(path_lit open: _ @string.special)

["(" ")"] @punctuation.bracket

(vec_lit open: _ @punctuation.bracket)

; ---- Quoting ----------------------------------------------------------------

(quoting_lit marker: _ @operator)
(syn_quoting_lit marker: _ @operator)
(unquoting_lit marker: _ @operator)
(unquote_splicing_lit marker: _ @operator)
(var_quoting_lit marker: _ @operator)

; 'foo is a symbol literal; #'foo names a function.
(quoting_lit value: (sym_lit) @constant)
(var_quoting_lit value: (sym_lit) @function)

; ---- Packages and feature expressions ---------------------------------------

(package_lit package: (sym_lit) @namespace)
(package_lit [":" "::"] @punctuation.delimiter)
"cl" @namespace

(include_reader_macro marker: _ @preproc)
(include_reader_macro condition: [(sym_lit) (kwd_lit) (nil_lit)] @preproc)

; ---- Special operators and standard macros, in head position ----------------

(list_lit
  .
  (sym_lit) @keyword
  (#any-of? @keyword
    ; special operators
    "block" "catch" "eval-when" "flet" "function" "go" "if" "labels" "let" "let*"
    "load-time-value" "locally" "macrolet" "multiple-value-call"
    "multiple-value-prog1" "progn" "progv" "quote" "return-from" "setq"
    "symbol-macrolet" "tagbody" "the" "throw" "unwind-protect"
    ; defining macros
    "defclass" "defconstant" "defgeneric" "defmacro" "defmethod" "defpackage"
    "defparameter" "defsetf" "defstruct" "deftype" "defun" "defvar"
    "define-compiler-macro" "define-condition" "define-method-combination"
    "define-modify-macro" "define-setf-expander" "define-symbol-macro"
    "in-package" "declaim" "declare" "lambda" "deftest"
    ; control and binding macros
    "and" "or" "when" "unless" "cond" "case" "ccase" "ecase" "typecase"
    "ctypecase" "etypecase" "do" "do*" "dolist" "dotimes" "do-symbols"
    "do-external-symbols" "do-all-symbols" "loop" "loop-finish" "return" "prog"
    "prog*" "prog1" "prog2" "multiple-value-bind" "multiple-value-setq"
    "destructuring-bind" "handler-case" "handler-bind" "ignore-errors"
    "restart-case" "restart-bind" "with-simple-restart" "with-condition-restarts"
    "with-open-file" "with-open-stream" "with-input-from-string"
    "with-output-to-string" "with-slots" "with-accessors" "with-standard-io-syntax"
    "with-hash-table-iterator" "with-package-iterator" "with-compilation-unit"
    "print-unreadable-object" "pprint-logical-block" "setf" "psetf" "psetq"))

; defun, defmacro, defgeneric and defmethod (and a bare lambda) have their own
; node, so the defining word is not a `sym_lit` head.
(defun_keyword) @keyword

; ---- LOOP -------------------------------------------------------------------

[
  (accumulation_verb)
  (for_clause_word)
  "loop" "for" "and" "finally" "thereis" "always" "never" "when" "if" "unless"
  "else" "do" "below" "above" "in" "on" "by" "from" "to" "upto" "downto"
  "upfrom" "downfrom" "then" "across" "repeat" "being" "into" "with" "as"
  "while" "until" "return" "initially" "using"
] @keyword

"=" @operator

(for_clause type: (sym_lit) @type)

; ---- Definitions ------------------------------------------------------------

(defun_header function_name: (sym_lit) @function)
(defun_header function_name: (package_lit symbol: (sym_lit) @function))

(defun_header
  lambda_list: (list_lit (sym_lit) @variable.parameter))

; (name default) and (name class): the first symbol is the parameter.
(defun_header
  lambda_list: (list_lit
    (list_lit
      .
      (sym_lit) @variable.parameter)))

; defmethod's specialiser: (it item) dispatches on the class `item`.
(defun_header
  keyword: (defun_keyword "defmethod")
  lambda_list: (list_lit
    (list_lit
      .
      (sym_lit)
      .
      (sym_lit) @type)))

(defun_header specifier: _ @constant)

(list_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @function
  (#any-of? @_def
    "define-compiler-macro" "define-modify-macro" "define-setf-expander" "defsetf"
    "deftest"))

(list_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @type
  (#any-of? @_def "defclass" "defstruct" "deftype" "define-condition"))

(list_lit
  .
  (sym_lit) @_def
  .
  (list_lit
    .
    (sym_lit) @type)
  (#eq? @_def "defstruct"))

(list_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @variable.special
  (#any-of? @_def "defvar" "defparameter"))

(list_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @constant
  (#eq? @_def "defconstant"))

; ---- Naming conventions -----------------------------------------------------

; &optional, &key, &rest, &body ...: lambda-list keywords, not parameters.
((sym_lit) @keyword
  (#match? @keyword "^&"))

; *special* variables and +constants+.
((sym_lit) @variable.special
  (#match? @variable.special "^\\*.+\\*$"))

((sym_lit) @constant
  (#match? @constant "^\\+.+\\+$"))

((sym_lit) @operator
  (#any-of? @operator "+" "-" "*" "/" "=" "/=" "<" ">" "<=" ">=" "1+" "1-"))
