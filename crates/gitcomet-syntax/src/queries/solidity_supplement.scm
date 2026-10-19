; Appended to `tree_sitter_solidity::HIGHLIGHT_QUERY`, which captures `returns`
; with `return` as `@keyword.return`. `return` is control flow; `returns`
; declares a function's outputs, so it reads as a plain keyword.
"returns" @keyword
