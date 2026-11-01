; HCL heredoc bodies.
;
; A `<<EOT` block is where the dense part of a Terraform file lives -- the cloud-init
; script in `user_data`, the IAM document in `policy` -- and HCL says nothing about
; what is inside one. Without this the body is a flat `@string`, one colour for
; thirty lines of shell, which is what "EOT blocks are not correctly handled" means
; in practice.
;
; The language comes from the *attribute name*, not from the content. Terraform has
; no syntax for declaring it, and the two content tests that suggest themselves both
; fail: a `#!` shebang can only be predicated on the *first* `template_literal`, and
; a pattern that captures every literal generates one match per literal, so the
; predicate would drop all the later fragments and defeat the combining below. An
; attribute name is a single node, constant across the match, so it can gate it.
; The names here are conventions the whole Terraform ecosystem follows.
;
; `injection.combined`, unlike queries/makefile_injections.scm and
; queries/just_injections.scm: a `${...}` interpolation splits one heredoc into
; several `template_literal` nodes, and those are one script, not several -- a
; `for` opened before an interpolation has to find its `done` after it. Same
; reasoning as queries/nix_injections.scm, which has the same shape for the same
; reason. The cost is documented there too: matches of one pattern share a
; document, so two `user_data` blocks in the same 64-row chunk parse as one script.
;
; The interpolations themselves stay out of the injected ranges, so they keep the
; HCL colours `hcl_highlights.scm` gives them -- `${local.name_prefix}` reads as
; Terraform inside the shell, which is what it is.
;
; TS_MAX_INJECTION_DEPTH is 1, so a heredoc inside the injected bash stays plain.

((attribute
   (identifier) @_name
   (expression
     (template_expr
       (heredoc_template
         (template_literal) @injection.content))))
  (#match? @_name "^(user_data|command|script|.*_script|.*_command)$")
  (#set! injection.language "bash")
  (#set! injection.combined))

((attribute
   (identifier) @_name
   (expression
     (template_expr
       (heredoc_template
         (template_literal) @injection.content))))
  (#match? @_name "^(policy|.*_policy|policy_document|.*_definitions?)$")
  (#set! injection.language "json")
  (#set! injection.combined))

; A `%{ if }` / `%{ for }` directive nests its body two levels deeper -- the
; directive wraps a `template_if`/`template_for`, and that holds the literals --
; so those fragments are not direct children of the heredoc and the patterns above
; miss them. The `(_)` is that inner wrapper, either of the two. This repeats the
; bash rule for that level, one level only, since
; fragments are not direct children of the heredoc and the patterns above miss
; directives can nest arbitrarily and a query has no descendant axis.
;
; Deliberately NOT combined, unlike the two above. Combined, every directive body
; in the chunk joins one document, and two `%{ if }` blocks each holding
; `echo "..."` parse as `echo` taking a second `echo` as its argument -- the second
; one comes out a parameter rather than a command. A directive body is complete
; lines, so one document each is both simpler and more accurate.
((attribute
   (identifier) @_name
   (expression
     (template_expr
       (heredoc_template
         (template_directive
           (_
             (template_literal) @injection.content))))))
  (#match? @_name "^(user_data|command|script|.*_script|.*_command)$")
  (#set! injection.language "bash"))
