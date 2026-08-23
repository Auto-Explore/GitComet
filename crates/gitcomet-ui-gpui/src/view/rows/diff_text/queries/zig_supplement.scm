; Appended to `tree_sitter_zig::HIGHLIGHTS_QUERY`.
;
; That query marks documentation comments with `(#lua-match? ... "^//!")`.
; `#lua-match?` is not one of tree-sitter's built-in predicates, so it is never
; evaluated and the pattern applies to *every* comment -- an ordinary `// note`
; rendered in the doc-comment colour. These two restate it with `#match?`, which
; the engine does evaluate, so only real doc comments are doc comments.

(comment) @comment

((comment) @comment.documentation
  (#match? @comment.documentation "^//[/!]"))
