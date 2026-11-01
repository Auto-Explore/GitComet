; PHP.
;
; Vendored from tree-sitter-php 0.24.2's queries/injections.scm rather than used
; through `tree_sitter_php::INJECTIONS_QUERY`, because the rule that matters most
; is missing from it: a PHP file is two languages interleaved, and everything
; outside `<?php ... ?>` is inline HTML the grammar hands over as one structureless
; `text` node. Without an injection every such region renders as plain body text --
; 26 lines of fixtures/syntax_test/languages/php/templating.php, and 76 of
; comments.php, the latter because a `?>` inside a `//` comment really does end PHP
; mode, so most of that file genuinely *is* HTML.
;
; Upstream's `((comment) @injection.content (#set! injection.language "phpdoc"))`
; is dropped: GitComet wires no phpdoc grammar, so it resolved to nothing on every
; comment in every PHP file.
;
; The heredoc rules are upstream's and are kept as they are. They carry the target
; in an `@injection.language` *capture* -- the form queries/nix_injections.scm
; drops for its own rules, because a capture resolves arbitrary text against the
; alias table and is invisible to `warm_reachable_highlight_specs`. Here the text
; is a heredoc's own end tag, and `<<<SQL` / `<<<HTML` / `<<<JSON` is a real and
; common PHP idiom, so the feature is worth the cost; it is also what this grammar
; already did before the file was vendored.

((text) @injection.content
  (#set! injection.language "html"))

(heredoc
  (heredoc_body) @injection.content
  (heredoc_end) @injection.language)

(nowdoc
  (nowdoc_body) @injection.content
  (heredoc_end) @injection.language)
