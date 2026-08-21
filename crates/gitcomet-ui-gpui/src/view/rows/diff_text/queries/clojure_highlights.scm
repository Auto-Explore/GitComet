; Built on `tree_sitter_clojure_orchard::HIGHLIGHTS_QUERY` (CC0-1.0), which is
; seven patterns wide: literals, comments and the quasiquotation operators. On its
; own it leaves every symbol -- including `defn`, `let` and the head of every form
; -- rendered as plain text, which is most of what a Clojure file is.
;
; The `; --- upstream ---` section is that constant verbatim; everything under
; `; --- gitcomet ---` is written here.
; `clojure_highlights_query_embeds_the_upstream_base_verbatim` keeps the copy
; honest against a grammar bump.
;
; Symbols are matched by name via `#any-of?` rather than by position. Clojure has
; no grammar-level notion of a special form -- `(defn f [x] x)` and `(f x)` are
; both `list_lit` with a `sym_lit` head -- so the alternative is colouring every
; list head as a function, which paints local calls and macro names alike.
;
; The quoting literals (`quoting_lit`, `syn_quoting_lit`, ...) are deliberately not
; captured as nodes: each spans the whole quoted form, so `'(alpha beta)` would be
; painted end to end. Upstream's operator rule below captures the marker instead.

; --- upstream --------------------------------------------------------------

;; Literals

(num_lit) @number

[
  (char_lit)
  (str_lit)
] @string

[
 (bool_lit)
 (nil_lit)
] @constant.builtin

(kwd_lit) @constant

;; Comments

(comment) @comment

;; Treat quasiquotation as operators for the purpose of highlighting.

[
 "'"
 "`"
 "~"
 "@"
 "~@"
] @operator

; --- gitcomet --------------------------------------------------------------

(regex_lit) @string.regex

; `#_` discards the form after it, and the node spans the whole discarded form.
; Greying all of it out is the point.
(dis_expr) @comment

;; Special forms and defining macros, in head position only.

(list_lit
  .
  (sym_lit
    name: (sym_name) @keyword)
  (#any-of? @keyword
    "def" "defn" "defn-" "defmacro" "defmulti" "defmethod" "defprotocol"
    "defrecord" "deftype" "definterface" "defstruct" "defonce" "declare"
    "ns" "in-ns" "require" "import" "use" "refer"
    "let" "letfn" "binding" "loop" "recur" "fn" "if" "if-let" "if-some"
    "if-not" "when" "when-let" "when-some" "when-not" "when-first" "cond"
    "condp" "case" "do" "doto" "for" "doseq" "dotimes" "while"
    "try" "catch" "finally" "throw" "quote" "var" "set!" "new"
    "monitor-enter" "monitor-exit" "reify" "proxy" "extend" "extend-type"
    "extend-protocol" "deftest" "testing" "are" "is"))

;; The name a `def`-like form binds.

(list_lit
  .
  (sym_lit
    name: (sym_name) @_def)
  .
  (sym_lit
    name: (sym_name) @function)
  (#any-of? @_def
    "def" "defn" "defn-" "defmacro" "defmulti" "defmethod" "defprotocol"
    "defrecord" "deftype" "definterface" "defstruct" "defonce" "declare"
    "deftest"))

;; A namespaced symbol reads as `namespace/name`; the namespace half is the part
;; worth telling apart.

(sym_lit
  namespace: (sym_ns) @namespace)

(kwd_lit
  namespace: (kwd_ns) @namespace)

;; The reader macro marker on `#inst "..."` / `#uuid "..."`, which is a token
;; rather than a whole form.

(tagged_or_ctor_lit
  marker: _ @punctuation.special)

;; Brackets. Clojure's three literal delimiters carry meaning that parentheses do
;; not, so they are worth seeing.

(list_lit
  open: _ @punctuation.bracket
  close: _ @punctuation.bracket)

(vec_lit
  open: _ @punctuation.bracket
  close: _ @punctuation.bracket)

(map_lit
  open: _ @punctuation.bracket
  close: _ @punctuation.bracket)

(set_lit
  open: _ @punctuation.bracket
  close: _ @punctuation.bracket)
