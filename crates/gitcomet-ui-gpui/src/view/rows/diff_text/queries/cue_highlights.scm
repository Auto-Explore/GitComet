; CUE.
;
; Upstream's queries/highlights.scm from https://github.com/eonpatapon/tree-sitter-cue (MIT),
; the release vendor/tree-sitter-cue is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.

; Includes

[
  "package"
	"import"
] @include

; Namespaces

(package_identifier) @namespace

(import_spec ["." "_"] @punctuation.special)

[
  (attr_path)
  (package_path)
] @text.uri ;; In attributes

; Attributes

(attribute) @attribute

; Conditionals

"if" @conditional

; Repeats

[
  "for"
] @repeat

(for_clause "_" @punctuation.special)

; Keywords

[
  "let"
] @keyword

[
  "in"
] @keyword.operator

; Operators

[
  "+"
	"-"
	"*"
	"/"
	"|"
	"&"
  "||"
  "&&"
	"=="
	"!="
  "<"
  "<="
  ">"
  ">="
  "=~"
  "!~"
	"!"
	"="
] @operator

; Fields & Properties

(field 
	 (label 
		 (identifier) @field))

(selector_expression
	  (_)
		(identifier) @property)

; Functions

(call_expression
	function: (identifier) @function.call)
(call_expression
	function: (selector_expression
	  (_)
		(identifier) @function.call))
(call_expression
	function: (builtin_function) @function.call)

(builtin_function) @function.builtin

; Variables

(identifier) @variable

; Types

(primitive_type) @type.builtin

(field
	(label (identifier) @type)
	(value (struct_lit)))

((identifier) @type
 (#match? @type "^(#|_#)"))

(source_file
  (field
    (label
      (identifier) @type))) ;; Top level identifiers are types

[
  (slice_type)
	(pointer_type)
] @type ;; In attributes

; Punctuation

[
  ","
  ":"
] @punctuation.delimiter

[ "{" "}" ] @punctuation.bracket

[ "[" "]" ] @punctuation.bracket

[ "(" ")" ] @punctuation.bracket

[ "<" ">" ] @punctuation.bracket

[
  (ellipsis)
	"?"
] @punctuation.special

; Literals

(string) @string

[
  (escape_char)
	(escape_unicode)
] @string.escape

(number) @number

(float) @float

(si_unit
	(float)
  (_) @symbol)

(boolean) @boolean

[
  (null)
	(top)
	(bottom)
] @constant.builtin

; Interpolations

(interpolation "\\(" @punctuation.special (_) ")" @punctuation.special) @none

(interpolation "\\(" (identifier) @variable ")")

; Commments

(comment) @comment @spell

; Errors

(ERROR) @error
