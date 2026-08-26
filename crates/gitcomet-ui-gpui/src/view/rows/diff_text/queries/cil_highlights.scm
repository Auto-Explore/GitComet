; CIL (MSIL).
;
; Authored here alongside vendor/tree-sitter-cil, which was written for GitComet
; because no CIL grammar exists anywhere. That grammar is a lexer rather than a
; parser -- see its header -- so this query separates the token kinds by shape
; rather than by position, and the ordering matters: overlapping captures resolve
; last-wins, so the two broad `(identifier)` rules come first and the exact lists
; take their words back afterwards.

[
  (line_comment)
  (block_comment)
] @comment

; `.assembly`, `.class`, `.method`, `.maxstack`. These outnumber the
; instructions, which is the thing about CIL that surprises people.
(directive) @preproc

(string) @string

; `'a field with spaces'`, `'class'` -- a quoted identifier can hold anything,
; including a reserved word, which is why it is not `@string`.
(quoted_identifier) @string.special

(number) @number

; Broad rules first. An opcode is lower-case and may carry dots (`ldc.i4.0`,
; `br.s`); a type name starts with a capital (`System.Object`).
((identifier) @function.builtin
  (#match? @function.builtin "^[a-z][a-z0-9_.]*$"))

((identifier) @type
  (#match? @type "^[A-Z]"))

; The primitive types, which are lower-case and so were caught as opcodes above.
((identifier) @type.builtin
  (#any-of? @type.builtin
    "bool" "char" "float32" "float64" "int" "int8" "int16" "int32" "int64"
    "native" "object" "string" "typedref" "unsigned" "void"))

; Modifiers and declaration words, likewise lower-case.
((identifier) @keyword
  (#any-of? @keyword
    "abstract" "ansi" "assembly" "auto" "beforefieldinit" "cil" "class" "extends"
    "extern" "family" "famandassem" "famorassem" "final" "hidebysig" "implements"
    "initonly" "instance" "literal" "managed" "nested" "newslot" "private"
    "privatescope" "public" "rtspecialname" "sealed" "serializable" "specialname"
    "static" "valuetype" "virtual"))

; Last of the identifier rules, because it must win over all of them: a label is
; written `IL_0000:`, which starts with a capital and so was taken by the type
; rule above. The `:` is what makes it a label, and only this pattern sees it.
(label
  name: (identifier) @label)

(operator) @operator

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ":"
  "::"
] @punctuation.delimiter

"=" @operator
