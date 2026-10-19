(function_definition) @local.scope
(block_statement) @local.scope

(function_definition (parameter name: (_) @local.definition))

; still have to support tuple assignments
(assignment_expression left: (_) @local.definition)

(identifier) @local.reference
