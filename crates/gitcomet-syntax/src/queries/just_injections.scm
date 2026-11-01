; Adapted from queries/just/injections.scm in
; https://github.com/casey/tree-sitter-just (Apache-2.0).
;
; A recipe body is a shell script and `just` says nothing about what is inside
; one, so without this every recipe in a justfile renders as plain body text --
; the same gap queries/makefile_injections.scm fills for Make, and the larger
; half of what a justfile is.
;
; Two of upstream's rules are dropped. Both handle `set shell := [...]` naming
; the language for the whole file, and both carry the target in an
; `@injection.language` *capture* rather than a `#set!` literal. The capture form
; resolves arbitrary text against the alias table and is invisible to
; `warm_reachable_highlight_specs` in ../syntax/language.rs, so whatever it
; resolves to pays its whole query-compile cost on the draw path -- the same
; reason queries/nix_injections.scm drops upstream's first rule. Recipes in such
; a justfile fall through to the bash default below.
;
; Upstream's `(#set! injection.language "comment")` rule is dropped too: GitComet
; wires no `comment` grammar, so it resolves to nothing.
;
; TS_MAX_INJECTION_DEPTH is 1, so a heredoc inside a recipe stays plain.

; The right-hand side of `=~`.
((regex_literal
  (_) @injection.content)
  (#set! injection.language "regex"))

; A recipe with no shebang runs under the shell.
(recipe_body
  !shebang
  (#set! injection.language "bash")
  (#set! injection.include-children)) @injection.content

; `` `...` `` and ```` ```...``` ```` command substitution.
(external_command
  (command_body) @injection.content
  (#set! injection.language "bash"))

; A shebang recipe is whatever it says it is, by the name of the interpreter.
(recipe_body
  (shebang
    (language) @injection.language)
  (#not-any-of? @injection.language "python3" "nodejs" "node" "uv")
  (#set! injection.include-children)) @injection.content

(recipe_body
  (shebang
    (language) @_lang)
  (#any-of? @_lang "python3" "uv")
  (#set! injection.language "python")
  (#set! injection.include-children)) @injection.content

(recipe_body
  (shebang
    (language) @_lang)
  (#any-of? @_lang "node" "nodejs")
  (#set! injection.language "javascript")
  (#set! injection.include-children)) @injection.content
