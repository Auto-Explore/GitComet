# Shared tree-sitter parser headers

Every grammar `tree-sitter generate` emits includes `tree_sitter/parser.h`, and
every external scanner also includes `tree_sitter/alloc.h` and
`tree_sitter/array.h`. The CLI writes a private copy of all three into each
grammar's `src/tree_sitter/`, so nine vendored grammars carried nine copies of the
same ~40 KB. They are kept here once instead.

The copy here is the one tree-sitter-cli 0.26.13 emits, which is what generated
every `src/parser.c` under `vendor/` except `tree-sitter-vue`'s — that grammar's
`grammar.js` was never vendored, so it cannot be regenerated.

## How a grammar finds these

`bindings/rust/build.rs` in each vendored grammar adds this directory to the
include path, *after* its own `src/`. That ordering is the override mechanism: a
grammar that still has a `src/tree_sitter/` of its own keeps using it, because
both `#include "tree_sitter/parser.h"` (relative to the including file) and the
`-I` search order reach `src/` first.

Nothing needs the override today, but `tree-sitter-vue` is where one would first
be needed and is worth knowing about: it is the only grammar here whose
`grammar.js` was not vendored, so it cannot be regenerated, and the `array.h` it
used to carry was an older implementation with a different macro contract
(`array_push` as an expression rather than a `do {} while (0)`, and no
`_array__cast`). Its `scanner.c` compiles against the current header anyway —
the call sites are unchanged — which is why there is one header set here and not
two. Only `tree-sitter-vue` and `tree-sitter-just` have a `scanner.c` at all; the
other seven include `parser.h` and nothing else.

## Using the tree-sitter CLI in a grammar directory

The CLI compiles `src/parser.c` itself and knows nothing about this directory, so
`tree-sitter parse` and `tree-sitter test` fail with
`fatal error: tree_sitter/parser.h: No such file or directory` in a grammar whose
private copy has been removed. Two ways round it, both fine:

```sh
# Point the C compiler at the shared headers for one command.
CPATH=../tree-sitter-headers tree-sitter parse foo.s

# Or just regenerate first -- `generate` writes src/tree_sitter/ back, so the
# usual edit loop already has them. Delete it again before committing.
tree-sitter generate && tree-sitter test && rm -rf src/tree_sitter
```

`cargo build` is unaffected either way: `vendor/tree-sitter-build` puts this
directory on the include path.

## Two CLI traps worth knowing

`tree-sitter generate` silently drops to ABI 14 when there is no
`tree-sitter.json` beside `grammar.js`, and `tree-sitter test --update` reindents
the whole corpus file, so diff it before and after or the real change is lost in
whitespace.

The CLI also caches compiled parsers in `~/.cache/tree-sitter/lib/<name>.so`,
keyed by the grammar's *name* and not by its path. Two checkouts of a grammar
called `asm` share one cache entry, so comparing a patched grammar against a
pristine copy silently compares it against itself. `rm ~/.cache/tree-sitter/lib/<name>.so`
between runs.

## After regenerating a grammar

`tree-sitter generate` writes `src/tree_sitter/` back. Delete it again unless you
meant to pin that grammar to a different header set; leaving it is not a build
error, just the duplication coming back one crate at a time.
