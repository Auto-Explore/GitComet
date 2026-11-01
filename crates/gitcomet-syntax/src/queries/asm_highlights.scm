; Assembly -- GAS, Intel/MASM, NASM and the 8-bit dialects alike.
;
; Derived from queries/asm/highlights.scm in
; https://github.com/RubixDev/tree-sitter-asm (MIT), which the grammar in
; vendor/tree-sitter-asm is vendored from. Kept here rather than in the vendored
; crate for the reason vendor/tree-sitter-vue gives: the grammar tracks upstream,
; the query is ours.
;
; Two changes from upstream:
;
;   * `(meta kind:)` is `@preproc`, not `@function.builtin`. A `.section` is an
;     assembler directive, not a call, and upstream painted it the same colour as
;     `mov` -- so the two things in an assembly file that could not be more
;     different read as one. Instructions keep `@function.builtin`.
;   * `{` and `}` join the bracket list. An ARMv7 register list `{r0, r4-r7}` is
;     the one place assembly has a third bracket pair, and leaving it out meant
;     clicking one end highlighted nothing (see syntax/pairs.rs).

; General
(label
  [(ident) (word)] @label)

; A dotted local label -- `.LBB0_1:`, `.Lfunc_end0:`. The grammar files these as
; `meta_ident` rather than `ident`, so upstream's rule above misses them, and
; clang emits one per basic block.
(label
  (meta_ident) @label)

; GAS local label references: `2f` is "the next label named `2:`, forwards", `1b`
; the previous one backwards. They are the jump targets in
;
;     1:      cmpq    $0, %r13
;             je      2f
;             jmp     1b
;
; and the grammar files them as `reg` like every other bare operand, so a jump
; target was painted the same colour as `%r13`.
;
; Matched by shape rather than by the branch mnemonic. A mnemonic test is the
; obvious approach and is a trap: `^(j|b|call)` also catches ARM's `bic`, x86's
; `bsf`/`bswap`/`bt` and every other non-branch that happens to start with those
; letters, and their operands really are registers. `digits + f|b` cannot be a
; register in any dialect -- nothing names one starting with a digit -- so this
; costs no false positives.
;
; The general case is not fixable here: `(reg (word))` is a bare word, which is a
; register in Intel/ARM/RISC-V (`rax`, `x0`, `sp`) and a symbol in AT&T (`printf`,
; `FRAME`), and the grammar is deliberately dialect-agnostic. Telling them apart
; needs to know the target, which no query can see.
((instruction
   (ident
     (reg
       (word) @label)))
  (#match? @label "^[0-9]+[fb]$"))

; ...and a branch to one. `b.eq .Lloop`.
(instruction
  (ident
    (meta_ident) @label))

; A directive's own operands: the `.rodata` in `.section .rodata`. Upstream
; captures only the directive's `kind`, so the section name -- the half of the
; line that says which section -- had no colour at all.
(meta
  (ident
    (meta_ident) @constant))

(reg) @variable.builtin

; Assembler directives: `.section`, `.align`, `.p2align`, `.globl`.
(meta
  kind: (_) @preproc)

; Mnemonics, including the dotted arm64 forms (`b.eq`, `csel.w`) that the
; vendored grammar's `mnemonic` token exists to keep whole.
(instruction
  kind: (_) @function.builtin)

(const
  name: (word) @constant)

; Comments
[
  (line_comment)
  (block_comment)
] @comment @spell

; Literals
(int) @number

(float) @number.float

(string) @string

; Keywords
[
  "byte"
  "word"
  "dword"
  "qword"
  "ptr"
  "rel"
  "label"
  "const"
] @keyword

; Operators & Punctuation
[
  "+"
  "-"
  "*"
  "/"
  "%"
  "|"
  "^"
  "&"
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ":"
] @punctuation.delimiter
