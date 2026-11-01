; Appended to `tree_sitter_xml::XML_HIGHLIGHT_QUERY`, which tags the angle
; brackets `@punctuation.delimiter`. HTML calls the same characters brackets,
; and a document switching colour for `<` depending on which of the two
; grammars claimed the file reads as a bug.

[
  "<"
  ">"
  "</"
  "/>"
  "<?"
  "?>"
  "<!"
  "<!["
  "]]>"
] @punctuation.bracket
