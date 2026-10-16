; Written here rather than derived from `tree_sitter_svelte_ng::INJECTIONS_QUERY`.
; That query opens with `; inherits: html_tags` and then relies on a bare
; `((raw_text) @injection.content (#set! injection.language "javascript"))`
; catch-all, which matches the body of `<style>` too -- so a plain stylesheet gets
; parsed as JavaScript.
;
; The `lang` value is forwarded verbatim as @injection.language and resolved by
; injection_language_from_name -> diff_syntax_language_for_code_fence_info, the same
; alias table that backs fenced code blocks. Enumerating the servable values with
; `#any-of?` instead is a trap -- see the same argument in vue_injections.scm: a
; value matching no enumerated rule is *also* vetoed out of the default rule below
; by its `#not-match? "\\slang\\s*="` guard, so `lang="js"` and `lang="css"` would
; render with no highlighting at all rather than falling back.
;
; The veto on the two default rules is still load-bearing: without it a
; `<script lang="ts">` matches both the default rule and the forwarding rule over
; the same `raw_text`, and live.rs keeps both layers and interleaves their
; captures at the same depth.
;
; A `lang` the alias table does not know (`lang="stylus"`) injects nothing, which
; is how the rest of GitComet treats an unrecognised language name.

; <script>...</script>
((script_element
  (start_tag) @_no_lang
  (raw_text) @injection.content)
  (#not-match? @_no_lang "\\slang\\s*=")
  (#set! injection.language "javascript"))

((script_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (quoted_attribute_value
        (attribute_value) @injection.language)))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

; The unquoted form. The grammar permits `<script lang=ts>`, and without this arm
; it lands in the gap the veto opens: no default rule, no forwarding rule.
((script_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (attribute_value) @injection.language))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

; <style>...</style>
((style_element
  (start_tag) @_no_lang
  (raw_text) @injection.content)
  (#not-match? @_no_lang "\\slang\\s*=")
  (#set! injection.language "css"))

((style_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (quoted_attribute_value
        (attribute_value) @injection.language)))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

((style_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (attribute_value) @injection.language))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

; `{count}`, `{#if ready}`, `{@html body}`. `svelte_raw_text` is the expression
; body in every one of those forms, so this single rule covers the whole template
; side.
;
; Guarded against bare identifier paths, exactly as the interpolation and
; directive rules in vue_injections.scm are, and for the same two reasons. Every
; injection is its own layer holding its own cache entry --
; TS_INJECTION_CACHE_MAX_ENTRIES is 32 -- so an ordinary list render
; (`{#each rows as row}<li>{row.name}</li>`) otherwise emits one per row and
; evicts the cache on its own. And it buys nothing: a lone identifier is
; `@variable` in the JavaScript query too, which renders as plain text, so the
; skipped form looks identical to the injected one.
((svelte_raw_text) @injection.content
  (#not-match? @injection.content "^\\s*[A-Za-z_$][A-Za-z0-9_$]*(\\.[A-Za-z_$][A-Za-z0-9_$]*)*\\s*$")
  (#set! injection.language "javascript"))
