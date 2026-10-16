; WebAssembly text format (.wat, .wast).
;
; Authored here: vendor/tree-sitter-wat is the `wat/` half of an npm-only
; repository and ships no query.
;
; Wasm text is s-expressions where the first symbol of a form is the whole of its
; meaning, and the grammar spells those as anonymous tokens -- so most of this is
; a literal list rather than node patterns.

[
  (comment_line)
  (comment_block)
] @comment

; `(@name ...)` annotations are comments that tooling reads.
[
  (comment_line_annot)
  (comment_block_annot)
] @comment.doc

; `$add`, `$0` -- every name in wasm text is `$`-prefixed.
(identifier) @variable

(string) @string

[
  (int)
  (float)
  (dec_float)
  (hex_float)
  (nat)
  (dec_nat)
  (hex_nat)
] @number

; Instructions. The grammar names them by operand shape rather than by mnemonic
; -- `op_nullary` is `i32.mul`, `op_index` is `local.get $x` -- so this is the
; list of shapes, not of opcodes, and it keeps working as wasm grows proposals.
[
  (op_nullary)
  (op_index)
  (op_index_opt)
  (op_index_opt_offset_opt_align_opt)
  (op_const)
  (op_func_bind)
  (op_let)
  (op_select)
  (op_simd_const)
  (op_simd_lane)
  (op_simd_offset_opt_align_opt)
  (op_table_copy)
  (op_table_init)
] @function.builtin

; `i32`, `f64`, `v128`, `funcref`, `externref`.
[
  (num_type_i32)
  (num_type_i64)
  (num_type_f32)
  (num_type_f64)
  (num_type_v128)
  (ref_type_funcref)
  (ref_type_externref)
] @type.builtin

; Module structure: the forms that declare rather than compute.
[
  "module"
  "func"
  "param"
  "result"
  "local"
  "global"
  "memory"
  "table"
  "elem"
  "data"
  "type"
  "start"
  "import"
  "export"
  "mut"
  "declare"
  "item"
  "offset"
  "align"
] @keyword

; Control flow.
[
  "block"
  "loop"
  "if"
  "then"
  "else"
  "end"
  "br_table"
  "call_indirect"
  "select"
  "let"
] @keyword.control

[
  "null"
  "inf"
  "ref.null"
  "ref.extern"
] @constant.builtin

"=" @operator

[
  "("
  ")"
] @punctuation.bracket

[
  "."
  "_"
] @punctuation.delimiter
