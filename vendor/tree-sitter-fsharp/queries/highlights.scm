;; Highlights for the fsharp grammar. Capture names follow the nvim-treesitter
;; convention (@keyword.conditional, @variable.member, ...), which every other
;; consumer maps from. Three resolution rules shape the order of this file:
;;
;;  1. When several patterns capture the same node, the LAST one in the file
;;     wins (tree-sitter-highlight and Neovim both resolve that way).
;;  2. A capture on a child node overrides one on its parent. There is
;;     deliberately no `(identifier) @variable` fallback: it would override
;;     every parent-level capture (`(argument_patterns) @variable.parameter`,
;;     `(_type) @type`) from inside, so a plain value reference is left to the
;;     editor's default colour.
;;  3. tree-sitter-highlight (the CLI and Helix, not Neovim) drops the remaining
;;     captures of a match once a LATER pattern captures the same node as that
;;     match's FIRST capture. `tree-sitter query` still prints them, so only a
;;     highlight assertion catches this. The identifier rules at the end of the
;;     file are ordered with this in mind.

;; ----------------------------------------------------------------------------
;; Literals and comments

;; `@spell` marks text for spell checking; it is listed first so the colour
;; capture is the one that wins (rule 1).
[
  (line_comment)
  (xml_doc)
  (block_comment)
] @spell @comment

(xml_doc) @spell @comment.documentation

(const
  [
   (_) @constant
   (unit) @constant.builtin
  ])

(primary_constr_args (_) @variable.parameter)

(class_as_reference
  (_) @variable.parameter.builtin)


((argument_patterns (long_identifier (identifier) @character.special))
 (#match? @character.special "^_"))

;; ----------------------------------------------------------------------------
;; Types and definitions

(type_name type_name: (_) @type.definition)
(exception_definition exception_name: (_) @type.definition)

[
 (_type)
 (atomic_type)
] @type

;; Type parameters: `'T` / `^a` in declarations (`type Box<'T>`), constraints
;; and annotations. `type_argument` is not a `_type`, so it needs its own rule.
(type_argument) @type

;; Units of measure: `1.0<m/s>`, `float<kg>`.
(measure_atom (simple_type) @type)

;; The member named by an SRTP constraint: `(static member Zero : ^a)`.
(trait_member_constraint (identifier) @function.member)

(member_signature
  .
  (identifier) @function.member)

(member_signature
  (curried_spec
    (arguments_spec
      "*"* @operator
      (argument_spec
        (argument_name_spec
          "?"? @character.special
          name: (_) @variable.parameter)))))

;; Union and enum cases are constructors (`| Circle of ...`, `| A = 1`).
(union_type_case (identifier) @constructor)
(enum_type_case . (identifier) @constructor)

;; Named union-case fields: `| Circle of radius: float`.
(union_type_field . (identifier) @variable.member (_))

(rules
  (rule
    pattern: (_) @constant
    block: (_)))

(wildcard_pattern) @character.special

(identifier_pattern
  .
  (_) @constructor
  .
  (_) @variable)

(optional_pattern
  "?" @character.special)

(fsi_directive_decl . (string) @module)

(import_decl . (_) @module)
(named_module
  name: (_) @module)
(namespace
  name: (_) @module)
;; The name is the identifier child, not the first child: a nested module may
;; carry attributes (`[<AutoOpen>] module Inner = ...`).
(module_defn
  (identifier) @module)

(ce_expression
  .
  (_) @constant.macro)

(field_initializer
  field: (_) @property)

(record_fields
  (record_field
    .
    (identifier) @property))

(value_declaration_left . (_) @variable)

(function_declaration_left
  . (_) @function)

(argument_patterns) @variable.parameter
(typed_pattern
  (_pattern) @variable.parameter
  (_type) @type)

;; A member name has two mutually-exclusive shapes, matched separately so the
;; highlight is deterministic regardless of tree-sitter's alternation-match order
;; (0.26.11 changed which branch of an overlapping `[...]` wins for a node that
;; matched both). Bare `member M(x)` -> M is the method; instance `member this.M`
;; -> `this` is the self parameter, M the method.
(member_defn
  (method_or_prop_defn
    (property_or_ident . (identifier) @function .)
    args: (_)* @variable.parameter))

(member_defn
  (method_or_prop_defn
    (property_or_ident
      instance: (identifier) @variable.parameter.builtin
      method: (identifier) @function.method)
    args: (_)* @variable.parameter))

;; Auto-property: `member val P = 1 with get, set` has no method_or_prop_defn.
(member_defn
  (property_or_ident (identifier) @function.member))

;; Member access on a compound expression: `(f x).Name`, `arr.[0].Length`.
(dot_expression
  base: (_) @variable.member
  field: (long_identifier_or_op
    (identifier) @property))

;; ----------------------------------------------------------------------------
;; Literals

[
  (xint)
  (int)
  (int16)
  (uint16)
  (int32)
  (uint32)
  (int64)
  (uint64)
  (nativeint)
  (unativeint)
] @number

[
  (ieee32)
  (ieee64)
  (float)
  (decimal)
] @number.float

(bool) @boolean

([
  (string)
  (triple_quoted_string)
  (verbatim_string)
  (char)
  (format_string)
  (format_triple_quoted_string)
] @spell @string)

(compiler_directive_decl) @keyword.directive

(preproc_line
  "#line" @keyword.directive)

(attribute
  target: (identifier)? @keyword
  (_type) @attribute)

;; ----------------------------------------------------------------------------
;; Punctuation and operators

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "[|"
  "|]"
  "{|"
  "|}"
] @punctuation.bracket

[
  "[<"
  ">]"
] @punctuation.special

;; Code quotations `<@ ... @>` / `<@@ ... @@>`.
(literal_expression
  [
    "<@"
    "@>"
    "<@@"
    "@@>"
  ] @punctuation.special)

(format_string_eval
  [
    "{"
    "}"
  ] @punctuation.special)

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

[
  "|"
  "="
  ">"
  "<"
  "-"
  "~"
  "->"
  "<-"
  "&"
  "&&"
  "|"
  "||"
  ":>"
  ":?>"
  ".."
  (infix_op)
  (prefix_op)
  (op_identifier)
] @operator

(generic_type
  [
   "<"
   ">"
  ] @punctuation.bracket)

(typed_expression
  ">" @punctuation.bracket)

;; ----------------------------------------------------------------------------
;; Keywords

[
  "if"
  "then"
  "else"
  "elif"
  "when"
  "match"
  "match!"
] @keyword.conditional

[
  "and"
  "or"
  "not"
  "upcast"
  "downcast"
] @keyword.operator

[
  "return"
  "return!"
  "yield"
  "yield!"
] @keyword.return

[
  "for"
  "while"
  "downto"
  "to"
] @keyword.repeat


[
  "open"
  "#r"
  "#load"
] @keyword.import

[
  "abstract"
  "delegate"
  "extern"
  "static"
  "inline"
  "mutable"
  "override"
  "rec"
  "global"
  (access_modifier)
] @keyword.modifier

[
  "let"
  "let!"
  "use"
  "use!"
  "and!"
  "member"
] @keyword.function

[
  "enum"
  "type"
  "exception"
  "inherit"
  "interface"
  "and"
  "class"
  "struct"
] @keyword.type

[
  "as"
  "assert"
  "begin"
  "end"
  "done"
  "default"
  "in"
  "do"
  "do!"
  "fun"
  "function"
  "get"
  "set"
  "lazy"
  "new"
  "of"
  "struct"
  "val"
  "module"
  "namespace"
  "with"
] @keyword

[
  "null"
] @constant.builtin

(match_expression "with" @keyword.conditional)

(try_expression
  [
    "try"
    "with"
    "finally"
  ] @keyword.exception)

(preproc_if
  [
    "#if" @keyword.directive
    "#endif" @keyword.directive
  ]
  condition: (_)? @keyword.directive)

(preproc_else
  "#else" @keyword.directive)

; Inactive branch of a directive the grammar could not place structurally;
; render like a comment, the way C/C++ editors gray out inactive regions.
(preproc_inactive) @comment

;; ----------------------------------------------------------------------------
;; Identifiers in expressions
;;
;; Everything from here on captures identifier nodes that earlier rules may
;; also capture, so the order follows rules 1 and 3 from the header: general
;; first, specific last, and no rule's first capture may be re-captured by a
;; later rule unless the later one is meant to win outright.

;; Leading segments of a dotted path are members of the root (`a.b.c`: a, b).
((long_identifier
  (identifier)+ @variable.member
  .
  (identifier)))

;; Every segment of a module path (`namespace Company.Product`,
;; `open System.IO`), a type path (`System.String`) or an attribute path is
;; part of that name, not a member access; re-capture the segments the rule
;; above marked as members.
(namespace name: (long_identifier (identifier) @module))
(named_module name: (long_identifier (identifier) @module))
(import_decl (long_identifier (identifier) @module))
(module_abbrev (long_identifier (identifier) @module))
(simple_type (long_identifier (identifier) @type))
(attribute (simple_type (long_identifier (identifier) @attribute)))

((simple_type
   (long_identifier
     (identifier) @type.builtin))
 (#any-of? @type.builtin "bool" "byte" "sbyte" "int16" "uint16" "int" "uint" "int64" "uint64" "nativeint" "unativeint" "decimal" "float" "double" "float32" "single" "char" "string" "unit"))

;; `base.M(...)`, `this.M(...)`: the current object. `base` is a keyword in
;; F#; `this`/`self` are conventional self-identifier names. Only in
;; expression paths, so the declaration site (`member this.M`) keeps its
;; @variable.parameter.builtin capture.
((long_identifier . (identifier) @variable.builtin)
 (#any-of? @variable.builtin "base" "this" "self"))
((long_identifier_or_op (identifier) @variable.builtin)
 (#any-of? @variable.builtin "base" "this" "self"))

((identifier) @module.builtin
 (#any-of? @module.builtin "Array" "Async" "Directory" "File" "List" "Option" "Path" "Map" "Set" "Lazy" "Seq" "Task" "String" "Result" ))

;; The last segment of a value-rooted path is a property (`ex.Message`,
;; `item.Tags.Count`); a call head (`x.M y`) is re-captured below. The root
;; is re-captured with the same name the member rule gave it because the
;; predicate needs a capture, and a throwaway `@_root` capture would take the
;; root's colour away under rules 1 and 3; the self identifiers are excluded
;; so they keep @variable.builtin.
((long_identifier_or_op
   (long_identifier . (identifier) @variable.member (identifier) @property .))
 (#match? @variable.member "^[a-z_]")
 (#not-any-of? @variable.member "this" "base" "self"))

;; The head of an application is the function being called: a bare name
;; (`f x`) or the last segment of a qualified one (`List.map f`, `x.M y`).
(application_expression
  .
  (long_identifier_or_op
    (identifier) @function.call)
  .
  (_))

(application_expression
  .
  (long_identifier_or_op
    (long_identifier
      (identifier) @function.call .))
  .
  (_))

(application_expression
  .
  (dot_expression
    field: (long_identifier_or_op
      (identifier) @function.call))
  .
  (_))

(application_expression
  .
  (typed_expression
    (long_identifier_or_op
      (identifier) @function.call)
    (_))
  .
  (_))

(application_expression
  .
  (typed_expression
    (long_identifier_or_op
      (long_identifier
        (identifier) @function.call .))
    (_))
  .
  (_))

(application_expression
  .
  (typed_expression
    (dot_expression
      field: (long_identifier_or_op
        (identifier) @function.call))
    (_))
  .
  (_))

;; The function side of a pipe or composition (`x |> f`, `x |> List.map g`,
;; `f >> string`, `xs |> Seq.sum`): applied even though it is not the head of
;; an application node (the grammar parses `xs |> List.map g` as
;; `(xs |> List.map) g`, so the bare qualified-name alternatives matter).
;;
;; These sit after the operator list (rule 3: their `@operator` capture on
;; `|>` is the match's first capture, and the list's later capture of the
;; same node would otherwise silence the callee capture), and each operator
;; family has exactly one rule for the same reason.
((infix_expression
  .
  (_)
  .
  (infix_op) @operator
  .
  [
    (long_identifier_or_op (identifier) @function.call)
    (long_identifier_or_op (long_identifier (identifier) @function.call .))
    (application_expression . (long_identifier_or_op (identifier) @function.call))
    (application_expression . (long_identifier_or_op (long_identifier (identifier) @function.call .)))
    (application_expression . (dot_expression field: (long_identifier_or_op (identifier) @function.call)))
  ])
 (#any-of? @operator "|>" "||>" "|||>"))

((infix_expression
  .
  [
    (long_identifier_or_op (identifier) @function.call)
    (long_identifier_or_op (long_identifier (identifier) @function.call .))
    (application_expression . (long_identifier_or_op (identifier) @function.call))
    (application_expression . (long_identifier_or_op (long_identifier (identifier) @function.call .)))
    (application_expression . (dot_expression field: (long_identifier_or_op (identifier) @function.call)))
  ]
  .
  (infix_op) @operator
  .
  (_))
 (#any-of? @operator "<|" "<||" "<|||"))

;; Composition applies both sides.
((infix_expression
  .
  [
    (long_identifier_or_op (identifier) @function.call)
    (long_identifier_or_op (long_identifier (identifier) @function.call .))
    (application_expression . (long_identifier_or_op (identifier) @function.call))
    (application_expression . (long_identifier_or_op (long_identifier (identifier) @function.call .)))
  ]
  .
  (infix_op) @operator
  .
  [
    (long_identifier_or_op (identifier) @function.call)
    (long_identifier_or_op (long_identifier (identifier) @function.call .))
    (application_expression . (long_identifier_or_op (identifier) @function.call))
    (application_expression . (long_identifier_or_op (long_identifier (identifier) @function.call .)))
  ])
 (#any-of? @operator ">>" "<<"))

;; A capitalised name used on its own, as an application head (`Some x`,
;; `Ok value`, `Circle 1.0`) or as a pattern head (`| Some v ->`) is a union
;; case, exception or active-pattern case by F# convention. Module- and
;; type-qualified heads (`List.map`, `Foo.Bar`) are excluded by the anchors
;; and keep their own colours. After the call rules so it wins for heads.
((long_identifier_or_op . (identifier) @constructor .)
 (#match? @constructor "^[A-Z]"))

;; Intrinsics that take a type argument (`typeof<int>`, `sizeof<T>`) and
;; `nameof`; reserved names, so matching the identifier anywhere is safe.
((identifier) @function.builtin
 (#any-of? @function.builtin "typeof" "typedefof" "sizeof" "nameof"))

((identifier) @keyword.exception
 (#any-of? @keyword.exception "failwith" "failwithf" "raise" "reraise"))

;; `not` is a library function, but reads as an operator like `&&`; the
;; keyword list above only covers the places the grammar lexes it as a token.
((long_identifier_or_op (identifier) @keyword.operator)
 (#eq? @keyword.operator "not"))

;; `query { ... }` custom operations whose names are unambiguous (they are not
;; ordinary F# functions/values). Common-named operations (zip, head, count,
;; where, select, sortBy, groupBy, ...) are intentionally omitted: they cannot
;; be scoped to a `query` builder with the current query language without
;; highlighting the same names everywhere.

;; Operations that take an argument: matched only as an application head, so
;; module-qualified names (List.sortByDescending) and member access on an
;; expression ((expr).sortByDescending) are left untouched.
((application_expression
   .
   (long_identifier_or_op (identifier) @keyword.operator))
 (#any-of? @keyword.operator
   "leftOuterJoin" "groupJoin" "groupValBy"
   "sortByDescending" "thenBy" "thenByDescending"
   "sortByNullable" "sortByNullableDescending"
   "thenByNullable" "thenByNullableDescending"
   "sumByNullable" "minByNullable" "maxByNullable" "averageByNullable"))

;; Zero-argument terminal operations: matched only as a bare statement inside a
;; computation-expression body, which likewise excludes member access.
((sequential_expression (long_identifier_or_op (identifier) @keyword.operator))
 (#any-of? @keyword.operator
   "headOrDefault" "lastOrDefault" "exactlyOne" "exactlyOneOrDefault"))

;; `[<Literal>] let maxSize = 100`: the bound name is a constant. A top-level
;; `let` followed by more declarations is a `declaration_expression`; only the
;; last one (and class-level lets) is a `value_declaration`.
((value_declaration
   (attributes
     (attribute
       (simple_type
         (long_identifier
           (identifier) @attribute))))
   (function_or_value_defn
     (value_declaration_left
       .
       (identifier_pattern (long_identifier_or_op (identifier) @constant)))))
 (#eq? @attribute "Literal"))

((declaration_expression
   (attributes
     (attribute
       (simple_type
         (long_identifier
           (identifier) @attribute))))
   (function_or_value_defn
     (value_declaration_left
       .
       (identifier_pattern (long_identifier_or_op (identifier) @constant)))))
 (#eq? @attribute "Literal"))
