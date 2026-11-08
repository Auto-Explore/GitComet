(razor_comment) @comment
"razor_tag_name" @tag
(razor_tag_opt_out) @punctuation.special

[
  "at_page"
  "at_using"
  "at_model"
  "at_preservewhitespace"
  "at_attribute"
  "at_implements"
  "at_layout"
  "at_inherits"
  "at_typeparam"
  "at_inject"
  "at_namespace"
  "at_rendermode"
  "at_addtaghelper"
  "at_removetaghelper"
  "at_taghelperprefix"
  "at_block"
  "at_helper"
  "at_section"
  "at_if"
  "at_for"
  "at_foreach"
  "at_while"
  "at_do"
  "at_switch"
  "at_try"
  "at_lock"
] @keyword

[
  "at_implicit"
  "at_explicit"
  "at_template"
  "at_colon_transition"
  "at_at_escape"
] @punctuation.special

[
  (razor_attribute_name)
  (razor_literal_attribute_name)
] @attribute

(razor_attribute_value ["\"" "'"] @string)
(razor_attribute_literal ["\"" "'"] @string)
(razor_attribute_literal_text) @string

(razor_helper name: (identifier) @function)
(taghelper_wildcard) @operator
