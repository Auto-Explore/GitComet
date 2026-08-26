; CoffeeScript.
;
; Upstream's queries/highlights.scm from https://github.com/svkozak/tree-sitter-coffeescript (MIT),
; the revision vendor/tree-sitter-coffee is vendored from. Kept here rather than in
; the vendored crate for the reason vendor/tree-sitter-vue gives: the grammar
; tracks upstream, the query is ours.

;; Comments
(comment) @comment

;; Literals
(number) @number
(string) @string
(regex) @string.regexp
(boolean) @constant.builtin
(null_literal) @constant.builtin
(undefined_literal) @constant.builtin

;; Variables
(instance_variable (identifier) @variable.other.member)

;; Functions and methods
(function_definition
  (identifier) @function)

(method_definition
  (identifier) @function.method)

(class_property_method
  (identifier) @function.method)

;; Parameters (covers identifiers and instance variables)
(parameters
  (parameter
    (pattern
      (identifier) @variable.parameter)))

(parameters
  (parameter
    (pattern
      (instance_variable (identifier) @variable.parameter))))

(function_expression
  (pattern
    (identifier) @variable.parameter))

(function_expression
  (pattern
    (instance_variable (identifier) @variable.parameter)))

(function_call
  (expression
    (identifier) @function.call))

(function_call
  (expression
    (member_expression
      (expression) @variable.other
      (identifier) @function.method)))

;; Classes and properties
(class_definition
  (identifier) @type)

(class_definition
  "extends"
  (identifier) @type.builtin)

(class_property_block
  (identifier) @property)

(class_property_assignment
  (identifier) @property)

(pair
  (identifier) @property)

;; Control flow keywords
"class" @keyword.control
"extends" @keyword.control
"if" @keyword.control
"else" @keyword.control
"unless" @keyword.control
"when" @keyword.control
"switch" @keyword.control
"for" @keyword.control
"while" @keyword.control
"until" @keyword.control
"then" @keyword.control
"do" @keyword.control
"by" @keyword.control
"try" @keyword.control
"catch" @keyword.control
"finally" @keyword.control
"loop" @keyword.control
(break_statement) @keyword.control
(continue_statement) @keyword.control
(return_statement) @keyword.control
(throw_statement) @keyword.control

;; Imports / exports
"import" @keyword.control.import
"export" @keyword.control.import
"from" @keyword.control.import
"as" @keyword.control.import
"default" @keyword.control.import

;; Operators and miscellaneous keywords
"and" @keyword.operator
"or" @keyword.operator
"not" @keyword.operator
"is" @keyword.operator
"isnt" @keyword.operator
"in" @keyword.operator
"of" @keyword.operator
"instanceof" @keyword.operator
"new" @keyword.operator
"typeof" @keyword.operator
"delete" @keyword.operator
"await" @keyword.operator
"async" @keyword.operator
"yield" @keyword.operator

;; Fallback variable highlight
(identifier) @variable
