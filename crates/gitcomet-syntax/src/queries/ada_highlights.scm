; Ada.
;
; Vendored from tree-sitter-ada 0.1.0's queries/highlights.scm (MIT,
; https://github.com/briot/tree-sitter-ada; the crate ships no LICENSE file, its
; Cargo.toml declares MIT). Edits, all marked `GitComet:` below:
;
;   * Dropped a duplicate `(loop_parameter_specification "in" @keyword.repeat)`.
;   * Package names, instantiations included, `@function` -> `@namespace`,
;     matching the `with`/`use` names.
;   * gnatprep `#if` blocks: upstream captured the whole statement, so every
;     identifier and bracket inside the block took the preproc colour. Now only
;     the directive tokens are captured.
;   * Dropped the two `@attribute` rules that "gray" an expression function's
;     body and a whole aspect specification; the aspect name alone is kept.
;   * Added type names (declarations and subtype marks), `'Attribute`
;     designators and procedure-call names. Upstream colours none of them.
;     Function calls stay uncoloured: `F (X)` is also array indexing and a
;     type conversion, which the grammar cannot tell apart.

[
   "abort"
   "abs"
   "abstract"
   "accept"
   "access"
   "all"
   "array"
   "at"
   "begin"
   "declare"
   "delay"
   "delta"
   "digits"
   "do"
   "end"
   "entry"
   "exit"
   "generic"
   "interface"
   "is"
   "limited"
   "null"
   "of"
   "others"
   "out"
   "pragma"
   "private"
   "range"
   "synchronized"
   "tagged"
   "task"
   "terminate"
   "until"
   "when"
] @keyword
[
   "aliased"
   "constant"
   "renames"
] @storageclass
[
   "mod"
   "new"
   "protected"
   "record"
   "subtype"
   "type"
] @keyword.type
[
   "with"
   "use"
] @include
[
   "body"
   "function"
   "overriding"
   "procedure"
   "package"
   "separate"
] @keyword.function
[
   "and"
   "in"
   "not"
   "or"
   "xor"
] @keyword.operator
[
   "while"
   "loop"
   "for"
   "parallel"
   "reverse"
   "some"
] @repeat
[
   "return"
] @keyword.return
[
   "case"
   "if"
   "else"
   "then"
   "elsif"
   "select"
] @conditional
[
   "exception"
   "raise"
] @exception
(comment) @comment @spell
(string_literal) @string
(character_literal) @string
(numeric_literal) @number

;; Highlight the name of subprograms
(procedure_specification name: (_) @function)
(function_specification name: (_) @function)
;; GitComet: packages were @function
(package_declaration name: (_) @namespace)
(package_body name: (_) @namespace)
(generic_instantiation name: (_) @function)

; GitComet: a package instantiation names a package, not a subprogram.
(generic_instantiation
  "package"
  name: (_) @namespace)

(entry_declaration . (identifier) @function)

;; Some keywords should take different categories depending on the context
(use_clause "use"  @include "type" @include)
(with_clause "private" @include)
(with_clause "limited" @include)
(use_clause (_) @namespace)
(with_clause (_) @namespace)

(loop_statement "end" @keyword.repeat)
(if_statement "end" @conditional)
(loop_parameter_specification "in" @keyword.repeat) ; GitComet: listed twice upstream
(iterator_specification ["in" "of"] @keyword.repeat)
(range_attribute_designator "range" @keyword.repeat)

(raise_statement "with" @exception)

;; GitComet: upstream captured the two whole `#if` statements; directive tokens only
["#if" "#elsif" "#else" "#end"]      @preproc
(gnatprep_declarative_if_statement ["then" "if"] @preproc)
(gnatprep_if_statement ["then" "if"] @preproc)
(gnatprep_identifier)                @preproc

(subprogram_declaration "is" @keyword.function "abstract"  @keyword.function)
(aspect_specification "with" @keyword.function)

(full_type_declaration "is" @keyword.type)
(subtype_declaration "is" @keyword.type)
(record_definition "end" @keyword.type)
(full_type_declaration (_ "access" @keyword.type))
(array_type_definition "array" @keyword.type "of" @keyword.type)
(access_to_object_definition "access" @keyword.type)
(access_to_object_definition "access" @keyword.type
   [
      (general_access_modifier "constant" @keyword.type)
      (general_access_modifier "all" @keyword.type)
   ]
)
(range_constraint "range" @keyword.type)
(signed_integer_type_definition "range" @keyword.type)
(index_subtype_definition "range" @keyword.type)
(record_type_definition "abstract" @keyword.type)
(record_type_definition "tagged" @keyword.type)
(record_type_definition "limited" @keyword.type)
(record_type_definition (record_definition "null" @keyword.type))
(private_type_declaration "is" @keyword.type "private" @keyword.type)
(private_type_declaration "tagged" @keyword.type)
(private_type_declaration "limited" @keyword.type)
(task_type_declaration "task" @keyword.type "is" @keyword.type)

;; GitComet: dropped upstream's "gray the body of expression functions" and
;; whole-`aspect_specification` @attribute rules; colour the aspect name only.
(aspect_association . (identifier) @attribute)

;; GitComet: type names, `'Attribute` designators and procedure calls.
(full_type_declaration . (identifier) @type)
(subtype_declaration . (identifier) @type)
(private_type_declaration . (identifier) @type)
(private_extension_declaration . (identifier) @type)
(incomplete_type_declaration . (identifier) @type)
(task_type_declaration . (identifier) @type)
(protected_type_declaration . (identifier) @type)
(_ subtype_mark: (identifier) @type)
(_ subtype_mark: (selected_component selector_name: (identifier) @type))
(qualified_expression subtype_name: (identifier) @type)
(attribute_designator (identifier) @attribute)
(procedure_call_statement name: (identifier) @function)
(procedure_call_statement name: (selected_component selector_name: (identifier) @function))

;; Highlight full subprogram specifications
;(subprogram_body
;    [
;       (procedure_specification)
;       (function_specification)
;    ] @function.spec
;)

((comment) @comment.documentation
  . [
      (entry_declaration)
      (subprogram_declaration)
      (parameter_specification)
    ])

(compilation_unit 
  . (comment) @comment.documentation)

(component_list
  (component_declaration)
    . (comment) @comment.documentation)

(enumeration_type_definition 
  (identifier)
  . (comment) @comment.documentation)

;; Highlight errors in red. This is not very useful in practice, as text will
;; be highlighted as user types, and the error could be elsewhere in the code.
;; This also requires defining    :hi @error guifg=Red    for instance.
(ERROR) @error

