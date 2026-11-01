; Caddyfile.
;
; Upstream's queries/highlights.scm from
; https://github.com/matthewpi/tree-sitter-caddyfile (MIT, archived), which
; vendor/tree-sitter-caddyfile is vendored from. The trailing commented-out block
; of directive-argument constants is upstream's and is left as it found it.

[
	(url)
	(unix_socket)
	(network_address)
] @type
(placeholder) @constant

(site_address) @keyword
(snippet_name) @property

(directive (directive_name) @property)

(named_matcher (matcher_name) @function.method)

(matcher) @function.call

[
	(interpreted_string_literal)
	(raw_string_literal)
] @string

(escape_sequence) @escape

(int_literal) @number

(comment) @comment

;[
;	"on"
;	"off"
;	"first"
;	"last"
;	"before"
;	"after"
;	"internal"
;	"strip_prefix"
;	"strip_suffix"
;	"replace"
;] @constant

; Added here, not upstream: a Caddyfile is blocks inside blocks, and upstream
; captures neither brace -- so every line that holds only a `}` rendered as body
; text, 28 of the 168 in the corpus sample. `syntax/pairs.rs` reads the tree
; rather than a query, so this is the colour only; the pairing already worked.
[
  "{"
  "}"
] @punctuation.bracket
