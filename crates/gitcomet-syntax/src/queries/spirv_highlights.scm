; SPIR-V assembly (.spvasm).
;
; Upstream's queries/highlights.scm from
; https://github.com/JuliaGPU/tree-sitter-spirv (MIT), which
; vendor/tree-sitter-spirv is vendored from. Its predicates are `#any-of?`,
; `#match?` and `#not-match?`, all of which tree-sitter's own query engine
; evaluates -- so unlike CMake's `#lua-match?` this needs no correction.

; SPIR-V assembly highlighting
;
; Opcodes are matched by family with regexes rather than enumerated, so new
; SPIR-V releases degrade to the generic @function capture instead of losing
; highlighting entirely.

(comment) @comment

(string) @string

(integer) @number
(float) @number
(raw_literal) @number

(id) @variable

(enumerant) @constant

"=" @operator

; module structure and debug/annotation instructions
((opcode) @keyword
 (#any-of? @keyword
  "OpCapability" "OpExtension" "OpExtInstImport" "OpMemoryModel"
  "OpEntryPoint" "OpExecutionMode" "OpExecutionModeId"
  "OpSource" "OpSourceContinued" "OpSourceExtension" "OpModuleProcessed"
  "OpName" "OpMemberName" "OpString" "OpLine" "OpNoLine"
  "OpDecorate" "OpMemberDecorate" "OpDecorationGroup" "OpGroupDecorate"
  "OpGroupMemberDecorate" "OpDecorateId" "OpDecorateString"
  "OpMemberDecorateString"
  "OpFunction" "OpFunctionParameter" "OpFunctionEnd" "OpLabel"))

; control flow
((opcode) @keyword.return
 (#any-of? @keyword.return
  "OpReturn" "OpReturnValue" "OpBranch" "OpBranchConditional" "OpSwitch"
  "OpKill" "OpUnreachable" "OpTerminateInvocation" "OpIgnoreIntersectionKHR"
  "OpTerminateRayKHR" "OpEmitMeshTasksEXT" "OpSelectionMerge" "OpLoopMerge"
  "OpPhi"))

; type declarations
((opcode) @type
 (#match? @type "^OpType"))

; all other opcodes
((opcode) @function
 (#not-match? @function "^OpType"))
