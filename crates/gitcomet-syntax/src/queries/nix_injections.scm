; Derived from tree-sitter-nix' queries/injections.scm
; (https://github.com/nix-community/tree-sitter-nix, MIT) as bundled with the
; v0.3.0 crate.
;
; Nix files carry shell script in indented strings all over the place -- every
; `shellHook`, `buildPhase`, `preInstall`, `writeShellScript` -- and without this
; the densest code in a derivation renders as one flat string.
;
; Every rule is `injection.combined`, and it matters for a reason unrelated to
; how many bindings a file has: a single `''…''` is split by the grammar into
; several `string_fragment` nodes wherever a `${…}` interpolation interrupts it.
; Combined, they parse as one shell script; separately, each fragment is its own
; document and a `if` opened before an interpolation never finds its `fi`.
; See apply_combined_injection_tokens in ../syntax/prepared.rs.
;
; Local changes vs upstream:
;   - upstream's first rule is dropped. It reads a language name out of a
;     preceding comment through an `@injection.language` capture, which resolves
;     arbitrary free text against the alias table, and -- being a capture rather
;     than a `#set!` literal -- is invisible to `warm_reachable_highlight_specs`,
;     so whatever it resolves to pays its query-compile cost on the draw path.
;     The four `bash` rules below all name their target as a literal.
;
; TS_MAX_INJECTION_DEPTH is 1, so the bash layer cannot inject further: a
; heredoc'd awk or python inside a shellHook stays plain.

; pkg.buildPhase / preInstall / installPhase / *Script / *Hook / *.startup
((binding
   attrpath: (attrpath (identifier) @_path)
   expression: (indented_string_expression
     (string_fragment) @injection.content))
 (#match? @_path "(^\\w*Phase|(pre|post)\\w*|(.*\\.)?\\w*([sS]cript|[hH]ook)|(.*\\.)?startup)$")
 (#set! injection.language "bash")
 (#set! injection.combined))

; pkgs.writeShellScript "name" '' … ''
((apply_expression
   function: (apply_expression function: (_) @_func)
   argument: (indented_string_expression (string_fragment) @injection.content))
 (#match? @_func "(^|\\.)writeShellScript(Bin)?$")
 (#set! injection.language "bash")
 (#set! injection.combined))

; pkgs.runCommand "name" { … } '' … ''
(apply_expression
  (apply_expression
    function: (apply_expression
      function: ((_) @_func)))
    argument: (indented_string_expression (string_fragment) @injection.content)
  (#match? @_func "(^|\\.)runCommand(((No)?(CC))?(Local)?)?$")
  (#set! injection.language "bash")
  (#set! injection.combined))

; pkgs.writeShellApplication { text = '' … ''; }
(apply_expression
  function: ((_) @_func)
  argument: (_ (_)* (_ (_)* (binding
    attrpath: (attrpath (identifier) @_path)
     expression: (indented_string_expression
       (string_fragment) @injection.content))))
  (#match? @_func "(^|\\.)writeShellApplication$")
  (#match? @_path "^text$")
  (#set! injection.language "bash")
  (#set! injection.combined))
