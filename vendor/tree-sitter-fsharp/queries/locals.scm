;; Scopes, definitions and references for scope-aware highlighting and for
;; `tree-sitter tags`, which loads this file too and accepts only the plain
;; @local.scope / @local.definition / @local.reference names (no
;; `.parameter`-style suffixes).

(identifier) @local.reference

[
  (namespace)
  (named_module)
  (module_defn)
  (function_or_value_defn)
  (member_defn)
  (fun_expression)
  (for_expression)
  (match_expression)
  (rule)
] @local.scope

;; `let x = ...`: the bound name can sit several pattern wrappers deep
;; (paren/typed/tuple/as), so match it at any of those depths.
(value_declaration_left
  .
  [
   (_ (identifier) @local.definition)
   (_ (_ (identifier) @local.definition))
   (_ (_ (_ (identifier) @local.definition)))
   (_ (_ (_ (_ (identifier) @local.definition))))
   (_ (_ (_ (_ (_ (identifier) @local.definition)))))
   (_ (_ (_ (_ (_ (_ (identifier) @local.definition))))))
  ])

;; `let f x = ...`: f is visible in the enclosing scope (so recursive and
;; later uses resolve), its parameters only inside the definition.
(function_declaration_left
  .
  ((_) @local.definition
   (#set! "definition.function.scope" "parent")))

;; Parameters live in `argument_patterns`, which appears under function
;; declarations, lambdas and property accessors alike.
(argument_patterns
  [
   (_ (identifier) @local.definition)
   (_ (_ (identifier) @local.definition))
   (_ (_ (_ (identifier) @local.definition)))
   (_ (_ (_ (_ (identifier) @local.definition))))
   (_ (_ (_ (_ (_ (identifier) @local.definition)))))
   (_ (_ (_ (_ (_ (_ (identifier) @local.definition))))))
  ])

;; Tuple-form member parameters: `member this.M(a, b: int) = ...`.
(method_or_prop_defn
  args: [
   (_ (identifier) @local.definition)
   (_ (_ (identifier) @local.definition))
   (_ (_ (_ (identifier) @local.definition)))
   (_ (_ (_ (_ (identifier) @local.definition))))
  ])

;; The self identifier of an instance member (`member this.M`).
(property_or_ident
  instance: (identifier) @local.definition)

;; `for i in ...` / `for i = a to b` loop variable.
(for_expression
  (identifier) @local.definition)
