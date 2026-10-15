; Derived from
; gpui-component/crates/ui/src/highlighter/languages/html/injections.scm
; (Apache-2.0). Local additions preserve inline `style=` and `on*=` injections.

; A `<script>` is JavaScript only when its `type` says so -- or says nothing.
;
; `<script type="text/template">` holds raw text a framework reads later, not
; code: the corpus sample has HTML prose in one, and injecting JavaScript into it
; coloured `raw`, `text` and `unparsed` as variables and `--` as an operator.
;
; The test is `#not-match?` against the *start tag's text* rather than against a
; captured attribute value. A query cannot express "has no attribute whose value
; is X": a pattern that binds the type attribute matches once per attribute, so
; `<script type="text/template" id="tpl">` would still match on `id` and inject.
; Matching the whole tag sidesteps that -- and `#not-match?` is one of the
; predicates tree-sitter actually evaluates.
((script_element
  (start_tag) @_start
  (raw_text) @injection.content)
 (#not-match? @_start "type\\s*=\\s*[\"']?(text/template|text/x-|text/html|application/json|application/ld\\+json|importmap|speculationrules)")
 (#set! injection.language "javascript"))

; ...and the data blocks that really are JSON. `application/ld+json` (structured
; data), `importmap` and `speculationrules` are all JSON by specification.
((script_element
  (start_tag) @_start
  (raw_text) @injection.content)
 (#match? @_start "type\\s*=\\s*[\"']?(application/json|application/ld\\+json|importmap|speculationrules)")
 (#set! injection.language "json"))

((style_element
  (raw_text) @injection.content)
 (#set! injection.language "css"))

(attribute
  (attribute_name) @_attribute_name
  (#match? @_attribute_name "^style$")
  (quoted_attribute_value
    (attribute_value) @injection.content)
  (#set! injection.language "css"))

(attribute
  (attribute_name) @_attribute_name
  (#match? @_attribute_name "^on[a-z]+$")
  (quoted_attribute_value
    (attribute_value) @injection.content)
  (#set! injection.language "javascript"))
