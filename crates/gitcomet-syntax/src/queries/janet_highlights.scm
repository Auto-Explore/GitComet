; Janet.
;
; Built like clojure_highlights.scm, whose grammar family this is. The `; --- upstream
; ---` section is vendor/tree-sitter-janet-simple's queries/highlights.scm (CC0-1.0,
; difftastic's copy of sogaiu/tree-sitter-janet-simple) verbatim; everything under
; `; --- gitcomet ---` is written here to match the other Lisps (Common Lisp,
; Racket, Scheme, Emacs Lisp). nvim-treesitter's janet_simple query (Apache-2.0)
; informed the special-form and macro list; no rule is copied from it.
;
; Symbols are matched by name in head position, as in Clojure: `(defn f [x] x)` and
; `(f x)` are both `par_tup_lit`, so colouring every head would paint calls. The
; quoting literals are not captured whole; only their markers are.

; --- upstream --------------------------------------------------------------

(num_lit) @number

[
 (buf_lit)
 (long_buf_lit)
 (long_str_lit)
 (str_lit)
] @string

[
 (bool_lit)
 (nil_lit)
] @constant.builtin

(kwd_lit) @constant

(comment) @comment

;; Treat quasiquotation as operators for the purpose of highlighting.

[
 "'"
 "~"
 ","
] @operator

; --- gitcomet --------------------------------------------------------------

; true/false are booleans, as in the other Lisps; nil stays a builtin constant.
(bool_lit) @boolean

[
  "(" ")" "[" "]" "{" "}" "@(" "@[" "@{"
] @punctuation.bracket

; ,;body splices; |(...) is a short fn.
(splice_lit ";" @operator)
(short_fn_lit "|" @operator)

; 'foo is a symbol literal.
(quote_lit (sym_lit) @constant)

;; Special forms and core macros that read as syntax, in head position only.

(par_tup_lit
  .
  (sym_lit) @keyword
  (#any-of? @keyword
    ; special forms
    "def" "var" "fn" "do" "quote" "if" "splice" "while" "break" "set"
    "quasiquote" "unquote" "upscope"
    ; defining macros
    "defn" "defn-" "defmacro" "defmacro-" "def-" "var-" "varfn" "defglobal"
    "varglobal" "defdyn"
    ; binding, control flow and error handling
    "let" "when" "unless" "cond" "case" "match" "if-let" "when-let" "if-not"
    "if-with" "when-with" "and" "or" "loop" "for" "forv" "each" "eachk" "eachp"
    "eachy" "repeat" "forever" "seq" "catseq" "tabseq" "generate" "try" "protect"
    "defer" "edefer" "with" "with-dyns" "with-syms" "with-vars" "with-env"
    "label" "prompt" "coro" "fiber-fn" "default" "assert" "compif" "comptime"
    "compwhen" "import" "use" "->" "->>" "-?>" "-?>>" "as->" "as?->" "as-macro"
    "ev/spawn" "ev/do-thread" "ev/spawn-thread" "ev/gather" "ev/with-deadline"
    "ev/with-lock" "ev/with-rlock" "ev/with-wlock" "ffi/defbind"))

;; The name each defining form binds.

; (def name value) and (var name value); a destructuring tuple binds each symbol.
(par_tup_lit
  .
  (sym_lit) @_def
  .
  [(sym_lit) @variable
   (sqr_tup_lit (sym_lit) @variable)]
  (#any-of? @_def "def" "def-" "var" "var-" "defglobal" "varglobal" "defdyn"))

; (def name (fn ...)) binds a function. Same node, so the later pattern wins.
(par_tup_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @function
  .
  (par_tup_lit
    .
    (sym_lit) @_fn
    (#eq? @_fn "fn"))
  (#any-of? @_def "def" "def-" "var" "var-"))

(par_tup_lit
  .
  (sym_lit) @_def
  .
  (sym_lit) @function
  (#any-of? @_def "defn" "defn-" "defmacro" "defmacro-" "varfn"))

; The parameter tuple: after the name and any docstring or metadata.
(par_tup_lit
  .
  (sym_lit) @_def
  .
  (sym_lit)
  .
  [(str_lit) (long_str_lit) (kwd_lit)]*
  .
  (sqr_tup_lit
    (sym_lit) @variable.parameter)
  (#any-of? @_def "defn" "defn-" "defmacro" "defmacro-" "varfn"))

; (fn [args] ...) and (fn name [args] ...).
(par_tup_lit
  .
  (sym_lit) @_fn
  .
  (sym_lit)?
  .
  (sqr_tup_lit
    (sym_lit) @variable.parameter)
  (#eq? @_fn "fn"))

; The module an import or use names.
(par_tup_lit
  .
  (sym_lit) @_import
  .
  (sym_lit) @namespace
  (#any-of? @_import "import" "use"))

;; Naming conventions.

; &opt, &keys, &named and a bare & are parameter-list markers.
((sym_lit) @keyword
  (#match? @keyword "^&"))

; *dynamic* names.
((sym_lit) @variable.special
  (#match? @variable.special "^\\*.+\\*$"))

((sym_lit) @operator
  (#any-of? @operator
    "+" "-" "*" "/" "%" "=" "not=" "<" ">" "<=" ">=" "++" "--" "+=" "-=" "*="
    "/=" "%="))
