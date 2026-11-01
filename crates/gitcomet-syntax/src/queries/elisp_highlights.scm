; Emacs Lisp.
;
; tree-sitter-elisp 1.7.2's queries/highlights.scm (MIT), edited to match the other
; Lisps here (Common Lisp, Racket, Scheme, Janet); everything below `GitComet
; additions` is new. Edits:
;
;   * Its `docstring: (string) @comment` rules sat before `(string) @string`, an order
;     that only works where the first pattern for a node wins. GitComet takes the
;     last, so they never applied. Docstrings are @string.documentation now, after
;     (string) so the label stands; they render as strings, as Python's do.
;   * (char) was @number: now @character. "t" was @constant.builtin with "nil":
;     now @boolean.
;   * Added: 'sym @constant; syntax-like macros (when, dolist, pcase, cl-defun, ...)
;     as @keyword in head position, since the grammar gives only the C special forms
;     their own node; the names defvar/defcustom (@variable.special), defconst
;     (@constant), cl-defun/define-minor-mode/defalias/... (@function) and
;     cl-defstruct (@type) bind; lambda and cl-defun parameters; `&optional`/`&rest`
;     as @keyword; arithmetic symbols as @operator.

;; Special forms
[
  "and"
  "catch"
  "cond"
  "condition-case"
  "defconst"
  "defvar"
  "function"
  "if"
  "interactive"
  "lambda"
  "let"
  "let*"
  "or"
  "prog1"
  "prog2"
  "progn"
  "quote"
  "save-current-buffer"
  "save-excursion"
  "save-restriction"
  "setq"
  "setq-default"
  "unwind-protect"
  "while"
] @keyword

;; Function definitions
[
 "defun"
 "defsubst"
 ] @keyword
(function_definition name: (symbol) @function)
(function_definition parameters: (list (symbol) @variable.parameter))

;; Highlight macro definitions the same way as function definitions.
"defmacro" @keyword
(macro_definition name: (symbol) @function)
(macro_definition parameters: (list (symbol) @variable.parameter))

;; A sharp quoted symbol is a function reference, e.g. #'foo.
(function_quote (symbol) @function)

(comment) @comment

(integer) @number
(float) @number
(char) @character

(string) @string

; Docstrings render as strings, as Python's do; after (string) so the label stands.
(function_definition docstring: (string) @string.documentation)
(macro_definition docstring: (string) @string.documentation)

[
  "("
  ")"
  "#["
  "["
  "]"
] @punctuation.bracket

[
  "`"
  "#'"
  "'"
  ","
  ",@"
] @operator

;; Highlight nil and t as constants, unlike other symbols
"nil" @constant.builtin
"t" @boolean

;; Keywords evaluate to themselves, so highlight them as constants too.
;; https://www.gnu.org/software/emacs/manual/html_node/elisp/Constant-Variables.html
((symbol) @constant
  (#match? @constant "^:"))

; ---- GitComet additions -----------------------------------------------------

; 'foo is a symbol literal.
(quote "'" (symbol) @constant)

; Standard macros that read as syntax, in head position. The grammar only gives
; the C-level special forms above their own node.
(list
  .
  (symbol) @keyword
  (#any-of? @keyword
    "when" "unless" "dolist" "dotimes" "pcase" "pcase-let" "pcase-let*"
    "pcase-dolist" "if-let" "if-let*" "when-let" "when-let*" "and-let*"
    "while-let" "named-let" "letrec" "dlet" "cl-loop" "cl-case" "cl-ecase"
    "cl-typecase" "cl-flet" "cl-labels" "cl-letf" "cl-block" "cl-return"
    "cl-return-from" "cl-destructuring-bind" "cl-defun" "cl-defmacro"
    "cl-defgeneric" "cl-defmethod" "cl-defstruct" "defgroup" "defcustom" "defface"
    "defalias" "defvar-local" "defvar-keymap" "define-minor-mode"
    "define-derived-mode" "define-globalized-minor-mode" "define-error" "declare"
    "declare-function" "ignore-errors" "with-temp-buffer" "with-current-buffer"
    "with-eval-after-load" "with-demoted-errors" "save-match-data" "setf"
    "eval-when-compile" "eval-and-compile" "ert-deftest"))

; The name each defining form binds.
(special_form
  "defvar"
  .
  (symbol) @variable.special)

(special_form
  "defconst"
  .
  (symbol) @constant)

(list
  .
  (symbol) @_def
  .
  (symbol) @variable.special
  (#any-of? @_def "defcustom" "defvar-local" "defface"))

(list
  .
  (symbol) @_def
  .
  (symbol) @function
  (#any-of? @_def
    "cl-defun" "cl-defmacro" "cl-defgeneric" "cl-defmethod" "define-minor-mode"
    "define-derived-mode" "define-globalized-minor-mode" "ert-deftest"))

(list
  .
  (symbol) @_def
  .
  (symbol)
  .
  (list
    [(symbol) @variable.parameter
     (list . (symbol) @variable.parameter)])
  (#any-of? @_def "cl-defun" "cl-defmacro" "cl-defgeneric"))

(special_form
  "lambda"
  .
  (list
    (symbol) @variable.parameter))

(list
  .
  (symbol) @_def
  .
  (quote
    (symbol) @function)
  (#eq? @_def "defalias"))

(list
  .
  (symbol) @_def
  .
  [(symbol) @type
   (list . (symbol) @type)]
  (#eq? @_def "cl-defstruct"))

; &optional and &rest are lambda-list keywords, not parameters.
((symbol) @keyword
  (#match? @keyword "^&"))

((symbol) @operator
  (#any-of? @operator "+" "-" "*" "/" "=" "/=" "<" ">" "<=" ">=" "1+" "1-"))
