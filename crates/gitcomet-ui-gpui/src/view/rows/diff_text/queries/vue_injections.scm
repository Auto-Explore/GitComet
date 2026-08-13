; Derived from nvim-treesitter's queries for
; tree-sitter-grammars/tree-sitter-vue (`queries/vue/injections.scm` plus the
; `; inherits: html_tags` base it pulls in, which is nvim-treesitter's
; html/injections.scm).
;
; The html base comes first so the Vue `lang="..."` rules below take precedence.
;
; Local changes vs upstream:
;   - Neovim-only predicates rewritten for core tree-sitter: `#not-lua-match?`
;     -> `#not-match?`, `#lua-match?` -> `#match?`, and the `<script type>`
;     `#gsub!` rule replaced with an explicit `#any-of?` MIME list.
;   - the two lit-html `${...}` rules were dropped; they need `#offset!`.
;   - the `comment` and `pug` injections were dropped: GitComet has no such
;     languages, so they were dead patterns. Comments are already coloured by
;     the `(comment) @comment` rule in vue_highlights.scm.
;   - the `<script type="importmap">` -> json rule was dropped. Json's grammar
;     is gated behind `syntax-data` but tree-sitter-vue behind `syntax-web`, so
;     under `--features syntax-web` alone the rule would resolve to no grammar
;     and inject nothing. `cfg(test)` force-enables every grammar, so no test
;     can catch that; not naming a cross-feature language is the only reliable
;     guard. Every other language named here lives in `syntax-web` alongside
;     Vue.
;   - the inline `<a style="…">` -> css rule was dropped. The CSS grammar parses
;     an attribute body as a stylesheet, so `style="color: red"` came out as a
;     type selector (`color` -> Tag, `red` -> Type) rather than a declaration.
;     It also injects unconditionally, so a template full of static inline
;     styles would evict TS_INJECTION_CACHE on its own. Falling back to the html
;     `(attribute_value) @string` rule is both cheaper and less wrong.
;     queries/html_injections.scm still carries this rule and still has the bug.
;
; Note that injection language names resolve through
; diff_syntax_language_for_identifier, so "scss"/"sass"/"less" land on Css,
; "ts"/"tsx" on TypeScript/Tsx, and "regex" on Regex.
;
; This base is NOT queries/html_injections.scm, and cannot be: every rule here
; needs the `lang`/`type` guards that let the vue rules below take over, which
; the html file has no reason to carry. The upshot is that .vue and .html
; templates do not highlight identically -- `<input pattern>` -> regex fires
; only here, inline `style=` only there. That is deliberate, not drift; if
; html_injections.scm grows a rule, decide explicitly whether it belongs here.

; --- html base -------------------------------------------------------------

; <style>...</style>
; <style blocking>...</style>
; The `lang` check is here so vue can inherit this without capturing the
; element twice.
((style_element
  (start_tag) @_no_type_lang
  (raw_text) @injection.content)
  (#not-match? @_no_type_lang "\\slang\\s*=")
  (#not-match? @_no_type_lang "\\stype\\s*=")
  (#set! injection.language "css"))

; The `lang` veto is repeated on every `type=` rule below. Without it a tag
; carrying both attributes -- `<script type="module" lang="ts">` -- matches the
; `type=` rule here *and* the `lang=` rule in the vue section, over the same
; `raw_text`. The prepared path happens to survive that (its injections sort by
; range and the last write wins), but live.rs keeps both layers, parses both,
; and interleaves their captures at the same depth, so the editor colours the
; block arbitrarily.
((style_element
  (start_tag
    (attribute
      (attribute_name) @_type
      (quoted_attribute_value
        (attribute_value) @_css))) @_no_lang
  (raw_text) @injection.content)
  (#eq? @_type "type")
  (#eq? @_css "text/css")
  (#not-match? @_no_lang "\\slang\\s*=")
  (#set! injection.language "css"))

; <script>...</script>
; <script defer>...</script>
((script_element
  (start_tag) @_no_type_lang
  (raw_text) @injection.content)
  (#not-match? @_no_type_lang "\\slang\\s*=")
  (#not-match? @_no_type_lang "\\stype\\s*=")
  (#set! injection.language "javascript"))

; <script type="text/javascript">
; <script type="application/javascript">
((script_element
  (start_tag
    (attribute
      (attribute_name) @_attr
      (#eq? @_attr "type")
      (quoted_attribute_value
        (attribute_value) @_type))) @_no_lang
  (raw_text) @injection.content)
  (#any-of? @_type "text/javascript" "application/javascript" "text/ecmascript" "application/ecmascript")
  (#not-match? @_no_lang "\\slang\\s*=")
  (#set! injection.language "javascript"))

; <script type="module">
((script_element
  (start_tag
    (attribute
      (attribute_name) @_attr
      (#eq? @_attr "type")
      (quoted_attribute_value
        (attribute_value) @_module))) @_no_lang
  (raw_text) @injection.content)
  (#eq? @_module "module")
  (#not-match? @_no_lang "\\slang\\s*=")
  (#set! injection.language "javascript"))

; <input pattern="[0-9]">
(element
  (_
    (tag_name) @_tag
    (attribute
      (attribute_name) @_name
      (quoted_attribute_value
        (attribute_value) @injection.content)))
  (#eq? @_tag "input")
  (#eq? @_name "pattern")
  (#set! injection.language "regex"))

; <button onclick="alert('hi')">
(attribute
  (attribute_name) @_name
  (#match? @_name "^on[a-z]+$")
  (quoted_attribute_value
    (attribute_value) @injection.content)
  (#set! injection.language "javascript"))

; --- vue -------------------------------------------------------------------

; <style lang="scss">, <script lang="ts">, and every other `lang` value.
;
; One rule per element rather than upstream's `#any-of?` list per target
; language. The value is forwarded verbatim as @injection.language and resolved
; by injection_language_from_name -> diff_syntax_language_for_code_fence_info,
; the same alias table that backs fenced code blocks, so `ts`/`typescript`/
; `mts`, `js`/`mjs`, `tsx`, `jsx`, `css`/`scss`/`less`/`sass`/`postcss` all land
; on a grammar without being enumerated here.
;
; Enumerating them is not just verbose, it is a trap: a value that matched no
; vue rule would *also* be vetoed out of the html base rules above by their
; `#not-match? "\\slang\\s*="` guard, so the entire block would render with no
; highlighting at all rather than falling back. A value the alias table does not
; know (lang="stylus") still injects nothing, which is how the rest of GitComet
; treats an unrecognised language name.
((style_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (quoted_attribute_value
        (attribute_value) @injection.language)))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

((script_element
  (start_tag
    (attribute
      (attribute_name) @_lang
      (quoted_attribute_value
        (attribute_value) @injection.language)))
  (raw_text) @injection.content)
  (#eq? @_lang "lang"))

; {{ count + 1 }}, and v-if="ok" / :prop="a && b" / @click="handler($event)".
;
; Both are guarded against bare identifier paths. Every injection is its own
; layer holding its own cache entry -- TS_INJECTION_CACHE_MAX_ENTRIES is 32 --
; and, on the editor path, its own draw from a single shared parse budget. A
; dense template otherwise emits one per directive and interpolation, which
; thrashes the cache and exhausts the budget on every keystroke.
;
; The skipped cases are exactly the ones an injection adds nothing to:
; `{{ msg }}` and :class="wrapperClass" are already coloured by the @none and
; @variable rules in vue_highlights.scm. Anything with an operator, call,
; literal or keyword in it -- which is what the parse is actually for -- still
; injects.
((interpolation
  (raw_text) @injection.content)
  (#not-match? @injection.content "^\\s*[A-Za-z_$][A-Za-z0-9_$]*(\\.[A-Za-z_$][A-Za-z0-9_$]*)*\\s*$")
  (#set! injection.language "typescript"))

((directive_attribute
  (quoted_attribute_value
    (attribute_value) @injection.content))
  (#not-match? @injection.content "^\\s*[A-Za-z_$][A-Za-z0-9_$]*(\\.[A-Za-z_$][A-Za-z0-9_$]*)*\\s*$")
  (#set! injection.language "typescript"))

; The grammar also permits the unquoted form, `v-if=ok`. `attribute_value` is a
; direct child of `directive_attribute` there rather than of a
; `quoted_attribute_value`, so it needs its own arm in both query files.
((directive_attribute
  (attribute_value) @injection.content)
  (#not-match? @injection.content "^\\s*[A-Za-z_$][A-Za-z0-9_$]*(\\.[A-Za-z_$][A-Za-z0-9_$]*)*\\s*$")
  (#set! injection.language "typescript"))
