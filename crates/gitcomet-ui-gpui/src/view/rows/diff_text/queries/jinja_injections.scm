; The template grammar is "template-first": it parses the `{{ }}` / `{% %}` /
; `{# #}` tags and exposes everything between them as opaque `text` nodes. All
; of the HTML in a `.njk` or `.j2` file lives in those nodes.
;
; `injection.combined` is what makes that work. Without it every text run would
; be its own layer: an entry each in the 32-slot TS_INJECTION_CACHE, and an
; independent HTML parse, so a `<ul>` opened before `{% for %}` would be unknown
; by the time `<li>` appeared after it. Combined, the whole set is parsed once
; with `set_included_ranges`, which is both one parse per document and a single
; coherent HTML document. See apply_prepared_combined_layer_tokens in
; ../syntax/prepared/query_tokens.rs.
;
; The target is a `#set!` literal rather than an `@injection.language` capture
; on purpose: warm_reachable_highlight_specs (../syntax/language.rs) discovers
; injection targets by reading literal `#set! injection.language` values off the
; compiled query, so this is what gets the HTML spec compiled on the warm-up
; thread instead of inline on the first draw.
;
; The HTML layer is depth 1, so its own `<script>`/`<style>` bodies are depth 2,
; which both engines allow (TS_MAX_INJECTION_DEPTH).

((text) @injection.content
 (#set! injection.language "html")
 (#set! injection.combined))

; `front_matter` is a GitComet addition to the vendored grammar: the leading
; `---` YAML block Eleventy templates open with. YAML reads the `---` lines as
; document markers, as it does for markdown.
((front_matter) @injection.content
 (#set! injection.language "yaml"))
