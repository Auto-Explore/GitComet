# tree-sitter-haskell

[![CI][ci]](https://github.com/tree-sitter/tree-sitter-haskell/actions/workflows/ci.yml)
[![discord][discord]](https://discord.gg/w7nTvsVJhm)
[![matrix][matrix]](https://matrix.to/#/#tree-sitter-chat:matrix.org)
[![crates][crates]](https://crates.io/crates/tree-sitter-haskell)
[![npm][npm]](https://www.npmjs.com/package/tree-sitter-haskell)
[![pypi][pypi]](https://pypi.org/project/tree-sitter-haskell)

Haskell grammar for [tree-sitter].

# References

- [Haskell 2010 Language Report – Syntax References][ref]
- [GHC Language Extensions][ext]

# Supported Language Extensions

These extensions are supported ✅, unsupported ❌ or not applicable because they don't involve parsing ➖️:

- AllowAmbiguousTypes ➖️
- ApplicativeDo ➖️
- Arrows ❌
- BangPatterns ✅
- BinaryLiterals ✅
- BlockArguments ✅
- CApiFFI ✅
- ConstrainedClassMethods ✅
- ConstraintKinds ✅
- CPP ✅
- CUSKs ✅
- DataKinds ✅
- DatatypeContexts ✅
- DefaultSignatures ✅
- DeriveAnyClass ➖️
- DeriveDataTypeable ➖️
- DeriveFoldable ➖️
- DeriveFunctor ➖️
- DeriveGeneric ➖️
- DeriveLift ➖️
- DeriveTraversable ➖️
- DerivingStrategies ✅
- DerivingVia ✅
- DisambiguateRecordFields ➖️
- DuplicateRecordFields ➖️
- EmptyCase ✅
- EmptyDataDecls ✅
- EmptyDataDeriving ✅
- ExistentialQuantification ✅
- ExplicitForAll ✅
- ExplicitNamespaces ✅
- ExtendedDefaultRules ➖️
- FlexibleContexts ✅
- FlexibleInstances ✅
- ForeignFunctionInterface ✅
- FunctionalDependencies ✅
- GADTs ✅
- GADTSyntax ✅
- GeneralisedNewtypeDeriving ➖️
- GHCForeignImportPrim ✅
- Haskell2010 ➖️
- Haskell98 ➖️
- HexFloatLiterals ✅
- ImplicitParams ✅
- ImplicitPrelude ➖️
- ImportQualifiedPost ✅
- ImpredicativeTypes ➖️
- IncoherentInstances ➖️
- InstanceSigs ✅
- InterruptibleFFI ✅
- KindSignatures ✅
- LambdaCase ✅
- LexicalNegation ❌
- LiberalTypeSynonyms ✅
- LinearTypes ✅
- ListTuplePuns ✅
- MagicHash ✅
- Modifiers ❌
- MonadComprehensions ➖️
- MonadFailDesugaring ➖️
- MonoLocalBinds ➖️
- MonomorphismRestriction ➖️
- MultiParamTypeClasses ✅
- MultiWayIf ✅
- NamedFieldPuns ✅
- NamedWildCards ✅
- NegativeLiterals ➖️
- NondecreasingIndentation ✅
- NPlusKPatterns ➖️
- NullaryTypeClasses ✅
- NumDecimals ➖️
- NumericUnderscores ✅
- OverlappingInstances ➖️
- OverloadedLabels ✅
- OverloadedLists ➖️
- OverloadedRecordDot ✅
- OverloadedRecordUpdate ✅
- OverloadedStrings ➖️
- PackageImports ✅
- ParallelListComp ✅
- PartialTypeSignatures ✅
- PatternGuards ✅
- PatternSynonyms ✅
- PolyKinds ➖️
- PostfixOperators ➖️
- QualifiedDo ✅
- QuantifiedConstraints ✅
- QuasiQuotes ✅
- Rank2Types ✅
- RankNTypes ✅
- RebindableSyntax ➖️
- RecordWildCards ➖️
- RecursiveDo ✅
- RequiredTypeArguments ✅
- RoleAnnotations ✅
- Safe ➖️
- ScopedTypeVariables ✅
- StandaloneDeriving ✅
- StandaloneKindSignatures ✅
- StarIsType ✅
- StaticPointers ❌
- Strict ➖️
- StrictData ✅
- TemplateHaskell ✅
- TemplateHaskellQuotes ✅
- TraditionalRecordSyntax ➖️
- TransformListComp ✅
- Trustworthy ➖️
- TupleSections ✅
- TypeAbstractions ✅
- TypeApplications ✅
- TypeData ✅
- TypeFamilies ✅
- TypeFamilyDependencies ✅
- TypeInType ✅
- TypeOperators ✅
- TypeSynonymInstances ➖️
- UnboxedSums ✅
- UnboxedTuples ✅
- UndecidableInstances ➖️
- UndecidableSuperClasses ➖️
- UnicodeSyntax ✅
- UnliftedFFITypes ➖️
- UnliftedNewtypes ✅
- Unsafe ➖️
- ViewPatterns ✅

# Bugs

## CPP

Preprocessor `#elif` and `#else` directives cannot be handled correctly, since the parser state would have to be
manually reset to what it was at the `#if`.
As a workaround, the code blocks in the alternative branches are parsed as part of the directives.

# Querying

The grammar contains several [supertypes](https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types),
which group multiple other node types under a single name.

Supertype names do not occur as extra nodes in parse trees, but they can be used in queries in special ways:

- As an alias, matching any of their subtypes
- As prefix for one of their subtypes, matching its symbol only when it occurs as a production of the supertype

For example, the query `(expression)` matches the nodes `infix`, `record`, `projection`, `constructor`, and the second
and third `variable` in this tree for `cats <> Cat {mood = moods.sleepy}`:

```
(infix
  (variable)
  (operator)
  (record
    (constructor)
    (field_update
      (field_name (variable))
      (projection (variable) (field_name (variable)))))))))
```

The two occurrences of `variable` in `field_name` (`mood` and `sleepy`) are not expressions, but record field names part
of a composite `record` expression.

Matching `variable` nodes specifically that are expressions is possible with the second special form.
A query for `(expression/variable)` will match only the other two, `cats` and `moods`.

The grammar's supertypes consist of the following sets:

- [`expression`](./grammar/exp.js)

  Rules that are valid in any expression position, excluding type applications, explicit types and expression
  signatures.

- [`pattern`](./grammar/pat.js)

  Rules that are valid in any pattern position, excluding type binders, explicit types and pattern signatures.

- [`type`](./grammar/type.js)

  Types that are either atomic (have no ambiguous associativity, like bracketed constructs, variables and type
  constructors), applied types or infix types.

- [`quantified_type`](./grammar/type.js)

  Types prefixed with a `forall`, context or function parameter.

- [`constraint`](./grammar/constraint.js)

  Almost the same rules as `type`, but mirrored for use in contexts.

- [`constraints`](./grammar/constraints.js)

  Analog of `quantified_type`, for constraints with `forall` or context.

- [`type_param`](./grammar/type.js)

  Atomic nodes in type and class heads, like the three nodes following `A` in `data A @k a (b :: k)`.

- [`declaration`](./grammar/module.js)

  All top-level declarations, like functions and data types.

- [`decl`](./grammar/decl.js)

  Shorthand for declarations that are also valid in local bindings (`let` and `where`) and in class and instance bodies,
  except for fixity declarations.
  Consists of `signature`, `function` and `bind`.

- [`class_decl` and `instance_decl`](./grammar/class.js)

  All declarations that are valid in classes and instances, which includes associated type and data families.

- [`statement`](./grammar/exp.js)

  Different forms of `do`-notation statements.

- [`qualifier`](./grammar/exp.js)

  Different forms of list comprehension qualifiers.

- [`guard`](./grammar/exp.js)

  Different forms of guards in function equations and case alternatives.

# Development

The main driver for generating and testing the parser for this grammar is the [tree-sitter CLI][cli].
Other components of the project require additional tools, described below.

Some are made available through `npm` – for example, `npx tree-sitter` runs the CLI.
If you don't have `tree-sitter` available otherwise, prefix all the commands in the following sections with `npx`.

## Output path

The CLI writes the shared library containing the parser to the directory denoted by `$TREE_SITTER_LIBDIR`.
If that variable is unset, it defaults to `$HOME/.cache/tree-sitter/lib`.

In order to avoid clobbering this global directory with development versions, you can set the env var to a local path:

```
export TREE_SITTER_LIBDIR=$PWD/.lib
```

## The grammar

The javascript file `grammar.js` contains the entry point into the grammar's production rules.
Please consult the [tree-sitter documentation][grammar-docs] for a comprehensive introduction to the syntax and
semantics.

Parsing starts with the first item in the `rules` field:

```javascript
{
  rules: {
    haskell: $ => seq(
      optional($.header),
      optional($._body),
    ),
  }
}
```

## Generating the parser

The first step in the development workflow converts the javascript rule definitions to C code in `src/parser.c`:

```
$ tree-sitter generate
```

Two byproducts of this process are written to `src/grammar.json` and `src/node-types.json`.

## Compiling the parser

The C code is automatically compiled by most of the test tools mentioned below, but you can instruct tree-sitter to do
it in one go:

```
$ tree-sitter generate --build
```

If you've set `$TREE_SITTER_LIBDIR` as mentioned above, the shared object will be written to `$PWD/.lib/haskell.so`.

Aside from the generated `src/parser.c`, tree-sitter will also compile and link `src/scanner.c` into this object.
This file contains the _external scanner_, which is a custom extension of the built-in lexer whose purpose is to handle
language constructs that cannot be expressed (efficiently) in the javascript grammar, like Haskell layouts.

### WebAssembly

The parser can be compiled to WebAssembly as well, which requires `emscripten`:

```
$ tree-sitter build --wasm
```

The resulting binary is written to `$PWD/tree-sitter-haskell.wasm`.

## Testing the parser

The most fundamental test infrastructure for tree-sitter grammars consists of a set of code snippets with associated
reference ASTs stored in `./test/corpus/*.txt`.

```
$ tree-sitter test
```

Individual tests can be run by specifying (a substring of) their description with `-f`:

```
$ tree-sitter test -f 'module: exports empty'
```

The project contains several other types of tests:

- `test/parse/run.bash [update] [test names ...]` parses the files in `test/parse/*.hs` and compares the output with
  `test/parse/*.target`.
  If `update` is specified as the first argument, it will update the `.target` file for the first failing test.

- `test/query/run.bash [update] [test names ...]` parses the files in `test/query/*.hs`, applies the queries in
  `test/query/*.query` and compares the output with `test/query/*.target`, similar to `test/parse`.

- `test/rust/parse-test.rs` contains a few tests that use tree-sitter's Rust API to extract the test ranges for
  terminals in a slightly more convenient way.
  This requires `cargo` to be installed, and can be executed with `cargo test` (which also runs the tests in
  `bindings/rust`).

- `test/parse-libs [wasm]` clones a set of Haskell libraries to `test/libs` and parses the entire codebase.
  When invoked as `test/parse-libs wasm`, it will use the WebAssembly parser.
  This requires `bc` to be installed.

- `test/parse-lib name [wasm]` parses only the library `name` in that directory (without cloning the repository).

### Debugging

The shared library built by `tree-sitter test` includes debug symbols, so if the scanner segfaults you can just run
`coredumpctl debug` to inspect the backtrace and memory:

```
newline_lookahead () at src/scanner.c:2583
2583                ((Newline *) 0)->indent = 5;
(gdb) bt
#0  newline_lookahead () at src/scanner.c:2583
#1  0x00007ffff7a0740e in newline_start () at src/scanner.c:2604
#2  scan () at src/scanner.c:2646
#3  eval () at src/scanner.c:2684
#4  tree_sitter_haskell_external_scanner_scan (payload=<optimized out>, lexer=<optimized out>,
    valid_symbols=<optimized out>) at src/scanner.c:2724
#5  0x0000555555772488 in ts_parser.lex ()
```

For more control, launch `gdb tree-sitter` and start the process with `run test -f 'some test'`, and set a breakpoint
with `break tree_sitter_haskell_external_scanner_scan`.

To disable optimizations, run `tree-sitter test --debug-build`.

#### Tracing

The `test` and `parse` commands offer two modes for obtaining detailed information about the parsing process.

With `tree-sitter test --debug`, every lexer step and shift/reduce action is printed to stderr.

With `tree-sitter test --debug-graph`, the CLI will generate an HTML file showing a graph representation of every step.
This requires `graphviz` to be installed.

[tree-sitter]: https://github.com/tree-sitter/tree-sitter
[ref]: https://www.haskell.org/onlinereport/haskell2010/haskellch10.html
[ext]: https://downloads.haskell.org/~ghc/latest/docs/html/users_guide/exts/table.html
[cli]: https://github.com/tree-sitter/tree-sitter/tree/master/cli
[grammar-docs]: https://tree-sitter.github.io/tree-sitter/creating-parsers#writing-the-grammar
[ci]: https://img.shields.io/github/actions/workflow/status/tree-sitter/tree-sitter-haskell/ci.yml?logo=github&label=CI
[discord]: https://img.shields.io/discord/1063097320771698699?logo=discord&label=discord
[matrix]: https://img.shields.io/matrix/tree-sitter-chat%3Amatrix.org?logo=matrix&label=matrix
[npm]: https://img.shields.io/npm/v/tree-sitter-haskell?logo=npm
[crates]: https://img.shields.io/crates/v/tree-sitter-haskell?logo=rust
[pypi]: https://img.shields.io/pypi/v/tree-sitter-haskell?logo=pypi&logoColor=ffd242
