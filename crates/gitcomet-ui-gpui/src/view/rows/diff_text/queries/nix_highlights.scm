; Derived from tree-sitter-nix' queries/highlights.scm
; (https://github.com/nix-community/tree-sitter-nix, MIT) as bundled with the
; v0.3.0 crate, so every node kind and field name here is known-valid against
; the grammar this compiles against.
;
; REORDERED. Upstream is written for tree-sitter-highlight, where the *first*
; matching pattern wins. This engine resolves overlaps differently
; (`normalize_non_overlapping_tokens` in ../syntax/prepared.rs): it splits every
; overlap into atomic slices and gives each slice to the **last containing
; capture in emission order**. The query cursor emits captures in node start-byte
; order, which makes that rule shake out as:
;
;   - different nodes -> the inner one wins on its own bytes, because it starts
;     later. `(escape_sequence)` inside a `(string_expression)`, or the `${` of an
;     interpolation, need no help from this file's ordering; they are handled by
;     position no matter where their rules sit.
;   - the *same* node captured by two patterns -> the start bytes tie, so the tie
;     breaks on pattern index, i.e. **the later rule in this file wins**.
;
; Only the second case bites, and in this grammar it is entirely the `(identifier)`
; family: upstream ends with a blanket `(identifier) @variable`, which under that
; rule buries `@variable.builtin`, `@function.builtin`, `@variable.parameter` and
; both `@property` rules -- every one of which captures an identifier too. So the
; identifier rules run generic-first, specific-last:
;
;   `(identifier) @variable`  ->  property / parameter / function  ->  builtins
;
; `nix_specific_captures_survive_the_generic_identifier_rule` is the guard, and it
; does fail against upstream's ordering. Anything re-synced from upstream has to
; be re-sorted the same way.
;
; The non-identifier rules below are grouped for readability only; their order
; carries no meaning.
;
; Local changes beyond the reordering:
;   - upstream's bare `(identifier) @property` catch-all is dropped. It exists to
;     catch attrpath positions that the `binding` / `select_expression` /
;     `inherit_from` rules already name explicitly, and as a second generic
;     fallback it just fights `(identifier) @variable` for every identifier in
;     the file.
;   - upstream's `(variable_expression (identifier)) @variable` is dropped as
;     subsumed by the generic rule, which now runs first and assigns the same kind.
;
; Two upstream constructs are kept verbatim and are inert here by design:
;   - `(#is-not? local)` needs a locals query, which GitComet does not implement.
;     tree-sitter routes unknown predicates to `general_predicates` rather than
;     failing to compile, so this neither breaks `Query::new` nor has any effect;
;     the builtin rules simply always fire. That is what we want -- `builtins`,
;     `map` and `import` stay coloured even where a local shadows them. `#offset!`
;     is already ignored the same way elsewhere in the tree.
;   - `(_) @embedded` marks the interpolated expression. `embedded` maps to `None`
;     in `syntax_kind_for_capture_name` and the token loop skips `None` captures,
;     so it emits nothing and leaves the interior to the rules above.

; --- literals, keywords, operators, punctuation -----------------------------

(comment) @comment

[
  "if"
  "then"
  "else"
  "let"
  "inherit"
  "in"
  "rec"
  "with"
  "assert"
  "or"
] @keyword

[
  (integer_expression)
  (float_expression)
] @number

(unary_expression
  operator: _ @operator)

(binary_expression
  operator: _ @operator)

[
  ";"
  "."
  ","
  "="
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; --- strings, and what nests inside them ------------------------------------

[
  (string_expression)
  (indented_string_expression)
] @string

[
  (path_expression)
  (hpath_expression)
  (spath_expression)
] @string.special.path

(uri_expression) @string.special.uri

(escape_sequence) @escape
(dollar_escape) @escape

(interpolation
  "${" @punctuation.special
  (_) @embedded
  "}" @punctuation.special)

; --- identifiers: generic base ... -----------------------------------------

(identifier) @variable

; --- ... then structural roles ... ------------------------------------------

(select_expression
  attrpath: (attrpath (identifier)) @property)

(binding
  attrpath: (attrpath (identifier)) @property)

(inherit_from attrs: (inherited_attrs attr: (identifier) @property) )

(function_expression
  universal: (identifier) @variable.parameter
)

(formal
  name: (identifier) @variable.parameter
  "?"? @punctuation.delimiter)

(apply_expression
  function: [
    (variable_expression (identifier)) @function
    (select_expression
      attrpath: (attrpath
        attr: (identifier) @function .))])

; --- ... then builtins, which must win outright -----------------------------

((identifier) @variable.builtin
 (#match? @variable.builtin "^(__currentSystem|__currentTime|__langVersion|__nixPath|__nixVersion|__storeDir|builtins|false|null|true)$")
 (#is-not? local))

((identifier) @function.builtin
 (#match? @function.builtin "^(__add|__addErrorContext|__all|__any|__appendContext|__attrNames|__attrValues|__bitAnd|__bitOr|__bitXor|__catAttrs|__ceil|__compareVersions|__concatLists|__concatMap|__concatStringsSep|__deepSeq|__div|__elem|__elemAt|__fetchurl|__filter|__filterSource|__findFile|__flakeRefToString|__floor|__foldl'|__fromJSON|__functionArgs|__genList|__genericClosure|__getAttr|__getContext|__getEnv|__getFlake|__groupBy|__hasAttr|__hasContext|__hashFile|__hashString|__head|__intersectAttrs|__isAttrs|__isBool|__isFloat|__isFunction|__isInt|__isList|__isPath|__isString|__length|__lessThan|__listToAttrs|__mapAttrs|__match|__mul|__parseDrvName|__parseFlakeRef|__partition|__path|__pathExists|__readDir|__readFile|__readFileType|__replaceStrings|__seq|__sort|__split|__splitVersion|__storePath|__stringLength|__sub|__substring|__tail|__toFile|__toJSON|__toPath|__toXML|__trace|__traceVerbose|__tryEval|__typeOf|__unsafeDiscardOutputDependency|__unsafeDiscardStringContext|__unsafeGetAttrPos|__zipAttrsWith|abort|baseNameOf|break|derivation|derivationStrict|dirOf|fetchGit|fetchMercurial|fetchTarball|fetchTree|fromTOML|import|isNull|map|placeholder|removeAttrs|scopedImport|throw|toString)$")
 (#is-not? local))
