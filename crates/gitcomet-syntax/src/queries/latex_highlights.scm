; LaTeX.
;
; From nvim-treesitter's runtime/queries/latex/highlights.scm (Apache-2.0) at
; 692b051b09935653befdb8f7ba8afdb640adf17b, the last revision before nvim moved
; to a newer grammar: the vendored grammar (latex-lsp/tree-sitter-latex
; 7b06f6ed, Nov 2024) lacks the `curly_group_label`/`curly_group_value`/counter
; nodes that nvim's current query names. Re-take it from nvim when the grammar
; is bumped. The grammar ships no query of its own. Edits, marked `GitComet:`
; unless they are deletions:
;
;   * `\href`/`\url`: deleted `(#set! @_hyperlink url @markup.link.url)`, which
;     tree-sitter 0.27 rejects (a `#set!` takes one capture).
;   * Three `#lua-match?` -> `#match?` (never evaluated, so `\if...` matched every
;     command and `% !TeX` every comment). Patterns carried over unchanged.
;   * Math `@markup.math` -> `@markup.raw`, and verbatim/listing bodies added as
;     `@markup.raw`; `markup.math` resolves to uncoloured `markup`.
;   * Added `\%`, `\&`, `\$`, `\#`, `\_`, `\{`, `\}` as `@string.escape`.

; General syntax
(command_name) @function @nospell

(caption
  command: _ @function)

; Turn spelling on for text
(text) @spell

; \text, \intertext, \shortintertext, ...
(text_mode
  command: _ @function @nospell
  content: (curly_group
    (_) @none @spell))

; Variables, parameters
(placeholder) @variable

(key_value_pair
  key: (_) @variable.parameter @nospell
  value: (_))

(curly_group_spec
  (text) @variable.parameter)

(brack_group_argc) @variable.parameter

[
  (operator)
  "="
  "_"
  "^"
] @operator

"\\item" @punctuation.special

(delimiter) @punctuation.delimiter

(math_delimiter
  left_command: _ @punctuation.delimiter
  left_delimiter: _ @punctuation.delimiter
  right_command: _ @punctuation.delimiter
  right_delimiter: _ @punctuation.delimiter)

[
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket ; "(" ")" has no syntactical meaning in LaTeX

; General environments
(begin
  command: _ @module
  name: (curly_group_text
    (text) @label @nospell))

(end
  command: _ @module
  name: (curly_group_text
    (text) @label @nospell))

; Definitions and references
(new_command_definition
  command: _ @function.macro @nospell)

(old_command_definition
  command: _ @function.macro @nospell)

(let_command_definition
  command: _ @function.macro @nospell)

(environment_definition
  command: _ @function.macro @nospell
  name: (curly_group_text
    (_) @label @nospell))

(theorem_definition
  command: _ @function.macro @nospell
  name: (curly_group_text_list
    (_) @label @nospell))

(paired_delimiter_definition
  command: _ @function.macro @nospell
  declaration: (curly_group_command_name
    (_) @function))

(label_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link @nospell))

(label_reference_range
  command: _ @function.macro
  from: (curly_group_text
    (_) @markup.link)
  to: (curly_group_text
    (_) @markup.link))

(label_reference
  command: _ @function.macro
  names: (curly_group_text_list
    (_) @markup.link))

(label_number
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link)
  number: (_) @markup.link)

(citation
  command: _ @function.macro @nospell
  keys: (curly_group_text_list) @markup.link @nospell)

; GitComet: dropped the `@_hyperlink` wrapper and its `#set!`
(hyperlink
  command: _ @function @nospell
  uri: (curly_group_uri
    (_) @markup.link.url @nospell))

(glossary_entry_definition
  command: _ @function.macro @nospell
  name: (curly_group_text
    (_) @markup.link @nospell))

(glossary_entry_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link))

(acronym_definition
  command: _ @function.macro @nospell
  name: (curly_group_text
    (_) @markup.link @nospell))

(acronym_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link))

(color_definition
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link))

(color_reference
  command: _ @function.macro
  name: (curly_group_text
    (_) @markup.link)?)

; Sectioning
(title_declaration
  command: _ @module
  options: (brack_group
    (_) @markup.heading.1)?
  text: (curly_group
    (_) @markup.heading.1))

(author_declaration
  command: _ @module
  authors: (curly_group_author_list
    (author)+ @markup.heading.1))

(chapter
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.2)?
  text: (curly_group
    (_) @markup.heading.2))

(part
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.2)?
  text: (curly_group
    (_) @markup.heading.2))

(section
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.3)?
  text: (curly_group
    (_) @markup.heading.3))

(subsection
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.4)?
  text: (curly_group
    (_) @markup.heading.4))

(subsubsection
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.5)?
  text: (curly_group
    (_) @markup.heading.5))

(paragraph
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.6)?
  text: (curly_group
    (_) @markup.heading.6))

(subparagraph
  command: _ @module
  toc: (brack_group
    (_) @markup.heading.6)?
  text: (curly_group
    (_) @markup.heading.6))

; Beamer frames
(generic_environment
  (begin
    name: (curly_group_text
      (text) @label)
    (#any-of? @label "frame"))
  .
  (curly_group
    (_) @markup.heading))

((generic_command
  command: (command_name) @_name
  arg: (curly_group
    (_) @markup.heading))
  (#eq? @_name "\\frametitle"))

((generic_command
  command: (command_name) @_name
  arg: (curly_group
    (_) @markup.italic))
  (#any-of? @_name "\\emph" "\\textit" "\\mathit"))

((generic_command
  command: (command_name) @_name
  arg: (curly_group
    (_) @markup.strong))
  (#any-of? @_name "\\textbf" "\\mathbf"))

; GitComet: was #lua-match?
(generic_command
  (command_name) @keyword.conditional
  (#match? @keyword.conditional "^\\\\if[a-zA-Z@]+$"))

(generic_command
  (command_name) @keyword.conditional
  (#any-of? @keyword.conditional "\\fi" "\\else"))

; File inclusion commands
(class_include
  command: _ @keyword.import
  path: (curly_group_path) @string)

(package_include
  command: _ @keyword.import
  paths: (curly_group_path_list) @string)

(latex_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(verbatim_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(import_include
  command: _ @keyword.import
  directory: (curly_group_path) @string.special.path
  file: (curly_group_path) @string.special.path)

(bibstyle_include
  command: _ @keyword.import
  path: (curly_group_path) @string)

(bibtex_include
  command: _ @keyword.import
  paths: (curly_group_path_list) @string.special.path)

(biblatex_include
  "\\addbibresource" @keyword.import
  glob: (curly_group_glob_pattern) @string.regexp)

(graphics_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(svg_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(inkscape_include
  command: _ @keyword.import
  path: (curly_group_path) @string.special.path)

(tikz_library_import
  command: _ @keyword.import
  paths: (curly_group_path_list) @string)

[
  (label_reference)
  (label_reference_range)
  (label_number)
  (glossary_entry_reference)
  (acronym_reference)
  (color_definition)
  (color_reference)
  (class_include)
  (package_include)
  (latex_include)
  (verbatim_include)
  (import_include)
  (bibstyle_include)
  (bibtex_include)
  (biblatex_include)
  (graphics_include)
  (svg_include)
  (inkscape_include)
  (tikz_library_import)
] @nospell

; Math
; GitComet: @markup.math -> @markup.raw. GitComet resolves `markup.math` to
; `markup`, which renders uncoloured, so formulas read as prose.
[
  (displayed_equation)
  (inline_formula)
] @markup.raw @nospell

(math_environment
  (_) @markup.raw)

; GitComet: verbatim and code-listing bodies are literal text. nvim injects a
; language into the code ones instead; GitComet has no LaTeX injection query.
(verbatim_environment
  verbatim: (comment) @markup.raw)

(_
  code: (source_code) @markup.raw)

; GitComet: a backslash-escaped special character is an escape, not a command.
((command_name) @string.escape
  (#match? @string.escape "^\\\\[%&$#_{}]$"))

; Comments
[
  (line_comment)
  (block_comment)
  (comment_environment)
] @comment @spell

; GitComet: these two were #lua-match?
((line_comment) @keyword.directive @nospell
  (#match? @keyword.directive "^% !TeX"))

((line_comment) @keyword.directive @nospell
  (#match? @keyword.directive "^%&"))
