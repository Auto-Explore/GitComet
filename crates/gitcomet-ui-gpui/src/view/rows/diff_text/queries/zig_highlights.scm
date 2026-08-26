; Zig.
;
; Vendored from tree-sitter-zig 1.1.2's queries/highlights.scm (MIT) rather than
; used through `tree_sitter_zig::HIGHLIGHTS_QUERY`, because three of its patterns
; are gated on `#lua-match?` and had to be edited in place.
;
; `#lua-match?` is a Neovim predicate. `tree_sitter::Query::new` accepts it as a
; general predicate and the engine **never evaluates it**, so each of those three
; patterns applied unconditionally -- it compiles clean and fails silently, in
; colour only. What that cost here:
;
;   * `(identifier) @type` and `(identifier) @constant` both fired on *every*
;     identifier. `@constant` is the later of the two, so it won: every
;     identifier in every Zig file rendered as a constant, `const multiline =`
;     included.
;   * `@comment.documentation` fired on every comment, so an ordinary `// note`
;     rendered in the italic doc-comment style.
;
; A supplement could not fix the first two the way queries/zig_supplement.scm
; fixed the third. Undoing an unconditional `(identifier)` capture needs a base
; `(identifier) @variable` to reset it, and appended-last that would also override
; upstream's legitimate `(parameter type: (identifier) @type)` and the function
; and namespace captures. Owning the query is the only way to narrow the rules
; rather than fight them, which is why solidity, julia and ocaml are vendored too.
;
; Three edits, all marked `GitComet:` below. Two are `#lua-match?` -> `#match?`
; with the Lua pattern carried over unchanged (these three are plain enough to be
; identical in both syntaxes); the third also widens `^//!` to `^//[/!]`, which is
; what zig_supplement.scm did before it was folded in here -- Zig writes `///` for
; a doc comment and `//!` for a module-level one, and upstream matched only the
; second.

; Variables

(identifier) @variable

; Parameters

(parameter
  name: (identifier) @variable.parameter)

; Types

(parameter
  type: (identifier) @type)

((identifier) @type
  ; GitComet: `#lua-match?` -> `#match?`; see the header.
  (#match? @type "^[A-Z_][a-zA-Z0-9_]*"))

(variable_declaration
  (identifier) @type
  "="
  [
    (struct_declaration)
    (enum_declaration)
    (union_declaration)
    (opaque_declaration)
  ])

[
  (builtin_type)
  "anyframe"
] @type.builtin

; Constants

((identifier) @constant
  ; GitComet: `#lua-match?` -> `#match?`; see the header.
  (#match? @constant "^[A-Z][A-Z_0-9]+$"))

[
  "null"
  "unreachable"
  "undefined"
] @constant.builtin

(field_expression
  .
  member: (identifier) @constant)

(enum_declaration
  (container_field
    type: (identifier) @constant))

; Labels

(block_label (identifier) @label)

(break_label (identifier) @label)

; Fields

(field_initializer
  .
  (identifier) @variable.member)

(field_expression
  (_)
  member: (identifier) @variable.member)

(container_field
  name: (identifier) @variable.member)

(initializer_list
  (assignment_expression
      left: (field_expression
              .
              member: (identifier) @variable.member)))

; Functions

(builtin_identifier) @function.builtin

(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (field_expression
    member: (identifier) @function.call))

(function_declaration
  name: (identifier) @function)

; Modules

(variable_declaration
  (identifier) @module
  (builtin_function
    (builtin_identifier) @keyword.import
    (#any-of? @keyword.import "@import" "@cImport")))

; Builtins

[
  "c"
  "..."
] @variable.builtin

((identifier) @variable.builtin
  (#eq? @variable.builtin "_"))

(calling_convention
  (identifier) @variable.builtin)

; Keywords

[
  "asm"
  "defer"
  "errdefer"
  "test"
  "error"
  "const"
  "var"
] @keyword

[
  "struct"
  "union"
  "enum"
  "opaque"
] @keyword.type

[
  "async"
  "await"
  "suspend"
  "nosuspend"
  "resume"
] @keyword.coroutine

"fn" @keyword.function

[
  "and"
  "or"
  "orelse"
] @keyword.operator

"return" @keyword.return

[
  "if"
  "else"
  "switch"
] @keyword.conditional

[
  "for"
  "while"
  "break"
  "continue"
] @keyword.repeat

[
  "usingnamespace"
  "export"
] @keyword.import

[
  "try"
  "catch"
] @keyword.exception

[
  "volatile"
  "allowzero"
  "noalias"
  "addrspace"
  "align"
  "callconv"
  "linksection"
  "pub"
  "inline"
  "noinline"
  "extern"
  "comptime"
  "packed"
  "threadlocal"
] @keyword.modifier

; Operator

[
  "="
  "*="
  "*%="
  "*|="
  "/="
  "%="
  "+="
  "+%="
  "+|="
  "-="
  "-%="
  "-|="
  "<<="
  "<<|="
  ">>="
  "&="
  "^="
  "|="
  "!"
  "~"
  "-"
  "-%"
  "&"
  "=="
  "!="
  ">"
  ">="
  "<="
  "<"
  "&"
  "^"
  "|"
  "<<"
  ">>"
  "<<|"
  "+"
  "++"
  "+%"
  "-%"
  "+|"
  "-|"
  "*"
  "/"
  "%"
  "**"
  "*%"
  "*|"
  "||"
  ".*"
  ".?"
  "?"
  ".."
] @operator

; Literals

(character) @character

([
  (string)
  (multiline_string)
] @string
  (#set! "priority" 95))

(integer) @number

(float) @number.float

(boolean) @boolean

(escape_sequence) @string.escape

; Punctuation

[
  "["
  "]"
  "("
  ")"
  "{"
  "}"
] @punctuation.bracket

[
  ";"
  "."
  ","
  ":"
  "=>"
  "->"
] @punctuation.delimiter

(payload "|" @punctuation.bracket)

; Comments

(comment) @comment @spell

((comment) @comment.documentation
  ; GitComet: `#lua-match?` -> `#match?`, and `^//!` -> `^//[/!]` so a `///` doc
  ; comment counts too; see the header.
  (#match? @comment.documentation "^//[/!]"))
