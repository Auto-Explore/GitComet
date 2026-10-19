; Newick / New Hampshire X (.nwk, .nh, .nhx).
;
; Authored here from tree-sitter-newick 1.1.0's grammar.js and node-types.json.
; The crate ships a seven-line queries/highlights.scm, but under CeCILL-C, so it
; is not copied; it also leaves `:`, `,`, `;` and `=` uncaptured and paints every
; node name in the function colour.

; A leaf's name is the taxon; an inner node's name labels the clade. An inner
; name that is a number is the clade's support value (bootstrap or posterior).
(leaf
  name: (name) @type)

(clade
  (name) @label)

((clade
  (name) @number)
  (#match? @number "^[0-9]+(\\.[0-9]+)?$"))

(length) @number

; `[&&NHX:S=human:B=100]`: key-value annotations on a node.
(nhx_entry
  key: (nhx_val) @property)

(nhx_entry
  value: (nhx_val) @string)

((nhx_entry
  value: (nhx_val) @number)
  (#match? @number "^-?[0-9]+(\\.[0-9]+)?([eE][-+]?[0-9]+)?$"))

[
  "("
  ")"
  "[&&NHX"
  "]"
] @punctuation.bracket

[
  ":"
  ","
  ";"
] @punctuation.delimiter

"=" @operator
