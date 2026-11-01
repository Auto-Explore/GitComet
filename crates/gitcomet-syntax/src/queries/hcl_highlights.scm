; HCL / Terraform.
;
; Authored here rather than vendored: `tree-sitter-hcl` ships no highlights
; query, which is why `.tf` was heuristic-only. Capture order matters --
; overlapping captures resolve last-wins -- so the broad literal rules come
; before the structural ones that should take spans back off them.

(comment) @comment

; Literals.
(numeric_lit) @number
(bool_lit) @boolean
(null_lit) @constant.builtin
(string_lit) @string
; An interpolated string is a `quoted_template`, not a `string_lit`; without
; this its literal halves around `${...}` get no colour at all.
(quoted_template) @string
(template_literal) @string
(heredoc_template) @string
(heredoc_identifier) @string

; `${ ... }` inside a template is code again, so mark its fences.
(template_interpolation_start) @punctuation.special
(template_interpolation_end) @punctuation.special
(template_directive_start) @punctuation.special
(template_directive_end) @punctuation.special
(strip_marker) @punctuation.special

; References: `var.region`, `aws_instance.web.id`.
(variable_expr (identifier) @variable)
(get_attr (identifier) @property)
(attr_splat (get_attr (identifier) @property))

; `name = value` -- the left side names the setting.
(attribute (identifier) @property)

; Object keys: `tags = { Name = "web" }`.
(object_elem key: (expression (variable_expr (identifier) @property)))

; `jsonencode(...)`, `file(...)`.
(function_call (identifier) @function)

; Block headers: `resource "aws_instance" "web" {`. The leading identifier is
; the block type; the quoted labels are already `@string` above.
(block (identifier) @keyword)

(ellipsis) @punctuation.special

[
  "{"
  "}"
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  "."
  ","
  ":"
  "=>"
] @punctuation.delimiter

[
  "="
  "!"
  "!="
  "%"
  "&&"
  "*"
  "+"
  "-"
  "/"
  "<"
  "<="
  "=="
  ">"
  ">="
  "?"
  "||"
] @operator

[
  "for"
  "in"
  "if"
  "else"
  "endfor"
  "endif"
] @keyword.control
