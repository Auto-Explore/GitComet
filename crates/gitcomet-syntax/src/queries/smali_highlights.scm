; Smali (Dalvik assembly).
;
; From vendor/tree-sitter-smali's queries/highlights.scm (MIT), the copy difftastic
; 0.71.0 ships (git.sr.ht/~yotam/tree-sitter-smali at 5ae51e15). Edits, each
; marked `GitComet:` below:
;   * `#lua-match?` -> `#match?` for the `^invoke` and constant rules (Lua's
;     `^[%u_]*$` becomes `^[A-Z_][A-Z0-9_]*$`).
;   * one `@keyword.control` rule for the opcodes that transfer control --
;     `if-*`, `goto`, `return*`, `throw`, `packed-switch`, `sparse-switch` --
;     replaces the four opcode rules; `cmp-*` stays a plain opcode.
;   * `(access_modifier)` is `@keyword`, not `@type.qualifier` (a type colour).
;   * `"L"` is `@punctuation.special`, not `@character.special` (a string colour).
;   * dropped: `.line N`'s and `.source`'s markup captures, the class-level
;     doc-comment rule (Smali has none), and the rule painting `nop` as a comment.

; Types

(class_identifier
  (identifier) @type)

(primitive_type) @type.builtin

((class_identifier
   . (identifier) @_first @type.builtin
   (identifier) @type.builtin)
  (#any-of? @_first "android" "dalvik" "java" "kotlinx"))

((class_identifier
   . (identifier) @_first @type.builtin
   .  (identifier) @_second @type.builtin
   (identifier) @type.builtin)
  (#eq? @_first "com")
  (#any-of? @_second "android" "google"))

; Methods

(method_definition
  (method_signature (method_identifier) @method))

(expression
  (opcode) @_invoke
	(body
	  (full_method_signature
      (method_signature (method_identifier) @method.call)))
  (#match? @_invoke "^invoke")) ; GitComet: was #lua-match?

(method_handle
  (full_method_signature
	(method_signature (method_identifier) @method.call)))

(custom_invoke
  . (identifier) @method.call
  (method_signature (method_identifier) @method.call))

(annotation_value
  (body
    (method_signature (method_identifier) @method.call)))

(annotation_value
  (body
    (full_method_signature
      (method_signature (method_identifier) @method.call))))

(field_definition
	(body
		(method_signature (method_identifier) @method.call)))

(field_definition
	(body
	  (full_method_signature
		  (method_signature (method_identifier) @method.call))))

((method_identifier) @constructor
  (#any-of? @constructor "<init>" "<clinit>"))

"constructor" @constructor

; Fields

[
  (field_identifier)
  (annotation_key)
] @field

; GitComet: was `#lua-match? "^[%u_]*$"`.
((field_identifier) @constant
  (#match? @constant "^[A-Z_][A-Z0-9_]*$"))

; Variables

(variable) @variable.builtin

(local_directive
  (identifier) @variable)

; Parameters

(parameter) @parameter.builtin
(param_identifier) @parameter

; Labels

[
  (label)
  (jmp_label)
] @label

; Operators

(opcode) @keyword.operator

; GitComet: replaces four `#lua-match?` rules (^return, ^if, ^cmp, ^throw).
((opcode) @keyword.control
  (#match? @keyword.control "^(if-|goto|return|throw|packed-switch|sparse-switch)"))

; GitComet: dropped `((opcode) @comment (#eq? @comment "nop"))`.

[
  "="
  ".."
] @operator

; Keywords

[
  ".class"
  ".super"
  ".implements"
  ".field"
  ".end field"
  ".annotation"
  ".end annotation"
  ".subannotation"
  ".end subannotation"
  ".param"
  ".end param"
  ".parameter"
  ".end parameter"
  ".line"
  ".locals"
  ".local"
  ".end local"
  ".restart local"
  ".registers"
  ".packed-switch"
  ".end packed-switch"
  ".sparse-switch"
  ".end sparse-switch"
  ".array-data"
  ".end array-data"
  ".enum"
  (prologue_directive)
  (epilogue_directive)
] @keyword

[
  ".source"
] @include

[
  ".method"
  ".end method"
] @keyword.function

[
  ".catch"
  ".catchall"
] @exception

; Literals

(string) @string
; GitComet: dropped `(source_directive (string "\"" _ @text.uri "\""))`.
(escape_sequence) @string.escape

(character) @character

"L" @punctuation.special ; GitComet: was @character.special

(number) @number

[
 (float)
 (NaN)
 (Infinity)
] @float

(boolean) @boolean

(null) @constant.builtin

; Misc

(annotation_visibility) @storageclass

(access_modifier) @keyword ; GitComet: was @type.qualifier

(array_type
  "[" @punctuation.special)

["{" "}"] @punctuation.bracket

["(" ")"] @punctuation.bracket

[
  "->"
  ","
  ":"
  ";"
  "@"
  "/"
] @punctuation.delimiter

; GitComet: dropped `(line_directive (number) @text.underline @text.literal)`.

; Comments

(comment) @comment @spell

; GitComet: dropped `(class_definition (comment) @comment.documentation)`.

; Errors

(ERROR) @error
