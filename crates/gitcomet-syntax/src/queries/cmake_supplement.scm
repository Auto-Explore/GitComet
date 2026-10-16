; Appended to `tree_sitter_cmake::HIGHLIGHTS_QUERY`.
;
; That query ends with a shebang rule:
;
;   ((source_file . (line_comment) @keyword.directive @nospell)
;     (#lua-match? @keyword.directive "^#!/"))
;
; `#lua-match?` is a Neovim predicate, not one of tree-sitter's built-in ones, so
; the engine never evaluates it and the pattern applies unconditionally -- see
; queries/zig_highlights.scm, which is the same bug in a different grammar and
; had to be vendored whole because a supplement could not narrow it. The
; `.` anchor limits the damage to the *first* comment in the file, which in a
; CMakeLists.txt is almost always the copyright or summary header: the top line
; of nearly every one rendered in the keyword colour.
;
; Restating it with `#match?` is not the fix here, because the two captures on
; that pattern are what makes it apply -- a genuine `#!/usr/bin/cmake -P` shebang
; would then be a keyword and nothing else would. Simply re-asserting the comment
; is enough, and costs only that a real shebang reads as the comment it also is.
(line_comment) @comment
