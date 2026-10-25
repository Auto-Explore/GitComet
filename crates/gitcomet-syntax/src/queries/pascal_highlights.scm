; Object Pascal / Delphi.
;
; Authored here: `tree-sitter-pascal` 0.10.2 ships a parser and node types but no
; highlights query at all.
;
; Generated from the grammar's own node types rather than hand-listed, because
; this grammar spells every keyword as its own named node -- 157 of them, `kBegin`
; through `kXor`. Listing them by hand would be a transcription exercise that goes
; stale the moment the grammar adds one; the two groups below are the whole set,
; split only by whether the word is control flow.

(comment) @comment

[
  (literalString)
  (literalChar)
] @string

(literalNumber) @number

; A `{$MODE}` / `{$IFDEF}` compiler directive. Pascal writes these inside what
; would otherwise be a comment, which is why they are their own node.
(pp) @preproc

; Declared names, which is what a reader scans a unit for.
;
; This grammar names them with a `name:` field rather than by position, so these
; are field patterns; `defProc` carries no name of its own -- its `header:` is a
; `declProc`, which does.
(declProc name: (identifier) @function)
(declType name: (identifier) @type)
(declEnumValue name: (identifier) @constant)
(declField name: (identifier) @property)
(declProp name: (identifier) @property)
(declArg name: (identifier) @variable.parameter)
(moduleName (identifier) @namespace)

; A type reference in a signature or a field. Captured through `typeref` rather
; than the `type` node above it, because `type` also wraps whole `record` and
; `class` bodies and colouring one of those as a type name would paint the block.
(typeref (identifier) @type)

[
  (kBegin)
  (kCase)
  (kDo)
  (kDownto)
  (kElse)
  (kEnd)
  (kExcept)
  (kFinally)
  (kFor)
  (kGoto)
  (kIf)
  (kOf)
  (kRaise)
  (kRepeat)
  (kThen)
  (kTo)
  (kTry)
  (kUntil)
  (kWhile)
  (kWith)
] @keyword.control

[
  (kAbsolute)
  (kAbstract)
  (kAdd)
  (kAlias)
  (kAnd)
  (kArray)
  (kAs)
  (kAsm)
  (kAssembler)
  (kAssign)
  (kAssignAdd)
  (kAssignDiv)
  (kAssignMul)
  (kAssignSub)
  (kAt)
  (kCdecl)
  (kClass)
  (kConst)
  (kConstref)
  (kConstructor)
  (kCppdecl)
  (kCvar)
  (kDefault)
  (kDelayed)
  (kDeprecated)
  (kDestructor)
  (kDispId)
  (kDispInterface)
  (kDiv)
  (kDot)
  (kDynamic)
  (kEndDot)
  (kEq)
  (kExperimental)
  (kExport)
  (kExports)
  (kExternal)
  (kFalse)
  (kFar)
  (kFdiv)
  (kFile)
  (kFinalization)
  (kForward)
  (kFunction)
  (kGeneric)
  (kGt)
  (kGte)
  (kHardfloat)
  (kHat)
  (kHelper)
  (kImplementation)
  (kImplements)
  (kIn)
  (kIndex)
  (kInherited)
  (kInitialization)
  (kInline)
  (kInterface)
  (kInterrupt)
  (kIocheck)
  (kIs)
  (kLabel)
  (kLibrary)
  (kLocal)
  (kLt)
  (kLte)
  (kMessage)
  (kMod)
  (kMs_abi_cdecl)
  (kMs_abi_default)
  (kMul)
  (kMwpascal)
  (kName)
  (kNear)
  (kNeq)
  (kNil)
  (kNodefault)
  (kNoreturn)
  (kNostackframe)
  (kNot)
  (kObjccategory)
  (kObjcclass)
  (kObjcprotocol)
  (kObject)
  (kOn)
  (kOperator)
  (kOptional)
  (kOr)
  (kOut)
  (kOverload)
  (kOverride)
  (kPacked)
  (kPascal)
  (kPlatform)
  (kPrivate)
  (kProcedure)
  (kProgram)
  (kProperty)
  (kProtected)
  (kPublic)
  (kPublished)
  (kRead)
  (kRecord)
  (kReference)
  (kRegister)
  (kReintroduce)
  (kRequired)
  (kResourcestring)
  (kSafecall)
  (kSaveregisters)
  (kSealed)
  (kSet)
  (kShl)
  (kShr)
  (kSoftfloat)
  (kSpecialize)
  (kStatic)
  (kStdcall)
  (kStored)
  (kStrict)
  (kString)
  (kSub)
  (kSysv_abi_cdecl)
  (kSysv_abi_default)
  (kThreadvar)
  (kTrue)
  (kType)
  (kUnimplemented)
  (kUnit)
  (kUses)
  (kVar)
  (kVarargs)
  (kVectorcall)
  (kVirtual)
  (kWinapi)
  (kWrite)
  (kXor)
] @keyword

[
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ";"
  ":"
  "."
  ".."
] @punctuation.delimiter
