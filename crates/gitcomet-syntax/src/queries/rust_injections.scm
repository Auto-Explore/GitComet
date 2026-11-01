; Derived from
; gpui-component/crates/ui/src/highlighter/languages/rust/injections.scm
; (Apache-2.0).

((macro_invocation
  (token_tree) @injection.content)
 (#set! injection.language "rust")
 (#set! injection.include-children))

((macro_rule
  (token_tree) @injection.content)
 (#set! injection.language "rust")
 (#set! injection.include-children))
