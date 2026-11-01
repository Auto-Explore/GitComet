; Derived from
; gpui-component/crates/ui/src/highlighter/languages/html/highlights.scm
; (Apache-2.0).

(tag_name) @tag
(erroneous_end_tag_name) @tag.error
(doctype) @constant
(attribute_name) @attribute
(attribute_value) @string
(comment) @comment

[
  "<"
  ">"
  "</"
  "/>"
] @punctuation.bracket
