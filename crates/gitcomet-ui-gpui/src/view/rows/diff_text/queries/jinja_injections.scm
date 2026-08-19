; The template grammar is "template-first": it parses the `{{ }}` / `{% %}` /
; `{# #}` tags and exposes everything between them as opaque `text` nodes. All
; of the HTML in a `.njk` or `.j2` file lives in those nodes.
;
; `injection.combined` is what makes that work. Without it every text run would
; be its own layer: an entry each in the 32-slot TS_INJECTION_CACHE, and an
; independent HTML parse, so a `<ul>` opened before `{% for %}` would be unknown
; by the time `<li>` appeared after it. Combined, the whole set is parsed once
; with `set_included_ranges`, which is both one cache-free parse per window and a
; single coherent HTML document. See apply_combined_injection_tokens in
; ../syntax/prepared.rs.
;
; The target is a `#set!` literal rather than an `@injection.language` capture
; on purpose: warm_reachable_highlight_specs (../syntax/language.rs) discovers
; injection targets by reading literal `#set! injection.language` values off the
; compiled query, so this is what gets the HTML spec compiled on the warm-up
; thread instead of inline on the first draw.
;
; HTML is in the `syntax-web` feature bucket, and so is this grammar. Naming a
; language from another bucket would resolve to no grammar under
; `--features syntax-web` alone -- and `cfg(test)` force-enables every grammar,
; so no test could catch it. queries/vue_injections.scm documents that trap at
; length.
;
; TS_MAX_INJECTION_DEPTH is 1, so the HTML layer cannot itself inject: `<script>`
; and `<style>` *bodies* inside a template stay uncoloured. Their tags and
; attributes still highlight.

((text) @injection.content
 (#set! injection.language "html")
 (#set! injection.combined))
