; A recipe body is a shell script, and make's own grammar says nothing about what
; is inside one: `shell_text` is a single node whose only children are the make
; expansions (`$@`, `$(CC)`) embedded in it. Upstream's highlights query does not
; capture it either, so every recipe line in every Makefile rendered as plain
; body text -- which is what "the highlighting breaks around line 73" was: line
; 73 is the last thing before the file's first tab-indented recipe.
;
; Deliberately NOT `injection.combined`, unlike queries/nix_injections.scm.
; Make runs each recipe line in its own shell, so one document per line is the
; semantics rather than a compromise, and it is also what keeps an unbalanced
; quote in one recipe from recolouring the next target's. The Nix queries combine
; because there a single `''...''` really is one script that a `${...}` merely
; interrupts.
;
; The injected range covers the node's children too (see
; `normalized_injection_content_byte_range` in ../syntax/prepared.rs, which
; strips children only for `string`/`template_string`), so bash sees the recipe
; line whole and its quotes balance. The cost is that the make expansions inside
; lose their host tokens -- `$@` and `$*` bash knows for itself, `$<` and `$^` it
; does not. Handing bash the gaps between them instead would keep those, at the
; price of feeding it `echo "target=` as a document; a broken quote is worse than
; two uncoloured sigils.
;
; TS_MAX_INJECTION_DEPTH is 1, so a heredoc'd python inside a recipe stays plain.
((shell_text) @injection.content
  (#set! injection.language "bash"))

; `VAR != date +%s` and the `$(shell ...)` function body are shell too.
((shell_command) @injection.content
  (#set! injection.language "bash"))
