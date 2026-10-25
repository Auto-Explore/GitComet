; Appended to `tree_sitter_make::HIGHLIGHTS_QUERY`.
;
; Upstream captures a target name only when it is one of GNU make's ~25
; conventional ones (`all`, `clean`, `install`, ...) or a builtin `.PHONY`-style
; directive. Every other target -- which in a real Makefile is most of them --
; is captured by nothing at all, so `order-only-prerequisite:` and `prefixes:`
; rendered as a bare `:` with the name beside it in body text.
;
; Overlapping captures resolve last-wins and a supplement is appended, so this
; rule wins over upstream's `@constant.macro` for the conventional names. That is
; deliberate: a Makefile where `clean` is one colour and `build` is another reads
; as a bug in the highlighter rather than as information. The two exclusions keep
; the cases where upstream's label really does say something the name does not:
;
;   `^[.]`   -- `.PHONY`, `.DEFAULT_GOAL`, `.SUFFIXES` are directives, not
;               targets, and stay `@constant.builtin` / `@constant.macro`.
;   `[%*?]`  -- `%.o` is a pattern, and upstream's `@string.regex` is the right
;               thing to say about it.
((targets (word) @function)
  (#not-match? @function "^[.]|[%*?]"))
