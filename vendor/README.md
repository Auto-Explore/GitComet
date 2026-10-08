# Vendored dependencies

## GPUI text storage

GPUI is pinned to Havunen/gpui-ce revision
`d0e01e8854d87ff749168b84ab16566b1e1919b6` in the root workspace manifest.
This revision stores shaped-line decorations in the shared layout instead of
reserving 32 decoration runs in every shaped line. It supersedes the compact
text-storage patch previously vendored in `gpui/`, so that crate and the Cargo
patch are no longer needed. Platform, renderer and test text-backend crates use
the same upstream revision.

## Tree-sitter grammars

Each `tree-sitter-*` directory here is a grammar GitComet compiles from source
rather than pulling from crates.io. Every one of them carries its reason in its
own `Cargo.toml` header. There are three reasons in play:

- **Unresolvable dependency.** The crate pins `tree-sitter ~0.20`, and the
  `links = "tree-sitter"` key makes that unresolvable alongside the workspace's
  0.27. Depending on `tree-sitter-language` instead drops the pin.
- **No release, or a needed fix.** No crates.io release exists, or upstream's
  grammar has a bug GitComet patches (`tree-sitter-asm`'s `mnemonic` token,
  `tree-sitter-ruby`'s `=begin` rule), or its published build wiring is not
  portable (`tree-sitter-v` hard-codes GNU object and archiver conventions).
- **The small-state retune.** Fifteen grammars are upstream's own source,
  regenerated only to make the parse tables smaller. That is what the rest of
  this file is about.

## The small-state retune

`tree-sitter generate` gives each LR state one of two representations:

- a **dense row** in `ts_parse_table` — `SYMBOL_COUNT` × 2 bytes, indexed
  directly, O(1);
- a **compact entry** in `ts_small_parse_table` — grouped by action and scanned.

It picks per state, in `tree-sitter-generate`'s `render.rs`:

```rust
let threshold = cmp::min(SMALL_STATE_THRESHOLD, self.parse_table.symbols.len() / 2);
//                       ^ 64
```

A state gets a dense row when it has more than `threshold` entries. The
`symbols.len() / 2` half is the real break-even — a dense row costs
`2 × SYMBOL_COUNT` bytes, so a state is worth making dense at roughly half that
many entries. The constant `64` only ever binds for a grammar with **more than
128 symbols**, and then it binds hard: F# has 538 symbols, so its break-even is
269, but the generator switches at 64 and hands dense 1076-byte rows to 9,268
states that did not need them.

Regenerating with the threshold raised to 128 took the 19 parsers that actually
reach the binary from **67.8 MB to 45.0 MB** of parse tables — a 22.75 MiB saving
that shows up 1:1 in the executable's `.rodata`:

| grammar | default | th=128 |
|---|---:|---:|
| fsharp | 11.37 MB | 6.17 MB |
| kotlin-sg | 5.48 | 2.96 |
| c-sharp | 5.06 | 3.41 |
| objc | 5.05 | 3.55 |
| ocaml (2 parsers) | 4.32 | 2.60 |
| julia | 5.91 | 4.45 |
| haskell | 3.57 | 2.16 |
| cpp | 3.37 | 2.19 |
| swift | 3.52 | 2.41 |
| typescript (2 parsers) | 2.71 | 1.83 |
| scala | 3.71 | 2.97 |
| sequel | 2.31 | 1.86 |
| rust | 1.06 | 0.67 |
| php | 1.01 | 0.71 |
| powershell | 0.88 | 0.63 |

The already-vendored `coffee` (6.47 → 4.73 MB) and `ruby` (2.00 → 1.72 MB) were
regenerated at the same setting and are included in that total.

Those numbers are from the original pass. Several grammars have been moved
forward to newer upstream revisions since, which changes both columns — `julia`
shrank by half when upstream halved its automaton, `sequel` grew with three
years of new statements — so treat the table as the shape of the win rather than
current figures. Each grammar's own `Cargo.toml` header carries what it is now.

`perl` is deliberately **not** on this list even though it would save 1.19 MB.
See "When regeneration is not safe" below.

### Why this is safe

The threshold is consumed only by the renderer. It selects how a state's actions
are *written*, never which states exist or what they do — so the automaton is
identical and every parse is identical.

That holds only while the CLI you regenerate with builds the same automaton
upstream shipped, which is **not** guaranteed by the ABI and not guaranteed by
staying inside one minor version. Compare the counts against upstream's own
`parser.c` — the one in their repository at the rev you vendored, or in the
crates.io tarball — and accept only `LARGE_STATE_COUNT` moving:

```sh
grep -E '^#define (STATE_COUNT|SYMBOL_COUNT|TOKEN_COUNT|EXTERNAL_TOKEN_COUNT|FIELD_COUNT|PRODUCTION_ID_COUNT)' \
  vendor/tree-sitter-<name>/src/parser.c
```

`grammar.json` and `node-types.json` should come out byte-identical too.

**This was assumed rather than checked once, and the drift shipped.** The
original pass compared `ts_node_string` on one corpus sample per grammar, which
is far too weak: `tree-sitter-c-sharp` went out with `STATE_COUNT` 8058 where
upstream ships 8053, and `case string when IsOid(x):` — valid C# — became two
`ERROR` nodes. `tree-sitter-typescript` drifted the same way (5878/5994 against
5870/5986), though 8,749 real files parsed identically there, so nothing was
visible. Both are regenerated with **tree-sitter-cli 0.26.5**, which reproduces
upstream's parsers exactly; 0.26.13 does not. Their `Cargo.toml` headers say so,
and `vendored_csharp_grammar_parses_a_when_clause_on_a_type_pattern` guards the
C# case.

So: pick the CLI by what reproduces upstream's counts, not by what is newest.
Upstream's `package.json` `devDependencies.tree-sitter-cli` is the first thing
to try. One caveat — a CLI older than 0.26 emits pre-0.26 parser source
(`.version`, `TSLexMode`), which does not compile against the shared headers in
`vendor/tree-sitter-headers`, so 0.26.x is the practical floor even when an
older CLI is what upstream used.

### When regeneration is not safe

**A newer CLI can build a different automaton.** Regenerating preserves the
representation only while the chosen CLI reproduces the automaton upstream
shipped. For `tree-sitter-perl`, 0.26.13 does not: it yields
`STATE_COUNT` 4634 where the crate ships 4698 — from `grammar.js` as readily as
from `grammar.json` — and `languages/perl/pod.pl` then parses with a
`scalar_variable` nested differently. No errors either way, but it is a real
parse change and not the one this exercise is meant to make, so perl stays a
crates.io dependency. Check `STATE_COUNT` and `SYMBOL_COUNT` against the shipped
`parser.c` before trusting a regeneration; only `LARGE_STATE_COUNT` should move.

**An old scanner may not survive the current `array.h`.** tree-sitter changed the
array API around 0.25: `_array__reserve` and `_array__grow` used to take an
`Array *` and mutate it, and now take and return `contents`, with the macros
assigning the result back. Code written against the old contract compiles clean
against the new header and then corrupts memory, with no diagnostic anywhere.

`tree-sitter-php` is the worked example. Its heredoc scanner popped and deleted
in a single expression:

```c
array_delete(&array_pop(&scanner->heredocs).word);
```

Taking the address of a member of the popped element is UB under the new
contract, because `array_delete` writes back into the storage the pop just
released — so the crates.io release segfaults on any heredoc:

```php
<?php
$x = <<<TEXT
  hi
  TEXT;
```

The fix is upstream's, not ours: commit `8b7d062` splits the pop from the delete,
and `1f30145` moves the grammar to the new header. At the September sweep neither
was released, so the crate was pinned to git rev `3f2465c`. The October sweep
moves it to 0.25.1, which includes both fixes and further heredoc improvements.
It uses the shared headers like every other grammar.

The general rule: if a vendored scanner misbehaves only under the shared header,
look upstream for the fix before pinning an old copy of `array.h`. Pinning works,
but it keeps a grammar on a header that will drift further every release.

### What it costs

Compact entries are scanned, not indexed, so parsing gets slower. Measured on a
1.4 MB F# file — the worst case here, because F# has the most symbols and so the
most states moved:

| threshold | parse tables | parse time |
|---|---:|---:|
| default (64) | 11.32 MB | 502.6 ms |
| **128** | **6.12 MB** | **536.1 ms  (+6.7%)** |
| 224 | 5.05 MB | 610.9 ms (+21.5%) |

Julia, with 243 symbols, costs +1.6% at 128. 128 is deliberate: it is close to
the break-even for a mid-sized grammar, and it keeps a dense fast path for the
hottest states instead of pushing everything into the scanned table. Going
further buys about 3 MB for three times the slowdown, which the diff pane's 1 ms
foreground parse budget cannot spend.

## Regenerating a grammar

The threshold is not exposed by the released CLI, so this needs a patched
`tree-sitter-generate`. The change is four lines:

```rust
// tree-sitter-generate/src/render.rs, in the fn that sets large_state_count
let threshold = std::env::var("TS_SMALL_STATE_THRESHOLD")
    .ok()
    .and_then(|v| v.parse::<usize>().ok())
    .unwrap_or_else(|| cmp::min(SMALL_STATE_THRESHOLD, self.parse_table.symbols.len() / 2));
```

```sh
# 1. Build the CLI recorded in the grammar's Cargo.toml against its patched
#    generator. Updated F# uses 0.27.0; C# and TypeScript still use 0.26.5.
cargo install tree-sitter-cli --version 0.27.0 --root /tmp/tsroot --locked \
  --config "patch.crates-io.tree-sitter-generate.path='/path/to/patched/tree-sitter-generate'"

# 2. Regenerate, from the grammar's own directory. The CLI writes into
#    <cwd>/src, so running this from the crate root of a multi-grammar crate
#    (fsharp, ocaml, php, typescript) creates a stray src/ and silently leaves
#    the real parser untouched.
cd vendor/tree-sitter-fsharp/fsharp
TS_SMALL_STATE_THRESHOLD=128 /tmp/tsroot/bin/tree-sitter generate --abi 15 src/grammar.json

# 3. Drop the private headers the CLI just wrote; the shared ones are used.
rm -rf src/tree_sitter
```

Pass `--abi` explicitly and keep the ABI upstream shipped — mixing an ABI change
into a regeneration would break the "representation only" guarantee above.
ABI 15 additionally requires a `tree-sitter.json`; `fsharp` and `swift` have one
written here because upstream's published crate omits it.

Confirm the result before committing: `LARGE_STATE_COUNT` in the new `parser.c`
should have dropped, every other count should match upstream's own `parser.c`
(see "Why this is safe"), and `cargo test -p gitcomet-ui-gpui --lib syntax`
should still pass — `vendored_grammars_keep_the_small_state_retune` holds the
`LARGE_STATE_COUNT` bounds, so a regeneration that moves one updates that list.

## Taking a newer upstream

The grammars here are pinned, so nothing tells you when upstream fixes a bug.
The last sweep was 2026-10-08; the results are below and in each updated
`Cargo.toml` header.
The routine that worked:

1. Find the rev each copy corresponds to by hashing its `grammar.js`,
   `scanner.c` and queries against upstream's history, rather than trusting the
   version number — several crates.io releases are not the tag they claim, and
   `tree-sitter-cue 0.0.1` is a snapshot of an unmerged PR.
2. Read every commit since that touches `grammar.js`, `grammar/`, `src/scanner.c`
   or `queries/`, and ignore the rest.
3. **Prove each fix**: build the pinned and the new grammar, parse the samples in
   `fixtures/syntax_test` plus a real corpus, and compare ERROR/MISSING counts
   and trees. Several "fixes" are tree-shape changes that cost more than they
   give — `cpp` and `ocaml` were declined on that basis.
4. Check the query. A grammar that renames or removes a node breaks the `.scm`
   that names it, and `language.rs` `.expect()`s that compile, so the app panics
   rather than losing a colour. `swift`, `cue` and `kdl` all needed their query
   updated in the same commit as the grammar.
5. Watch for a fork that lags: `tree-sitter-kotlin-sg` is ast-grep's fork of
   fwcd's grammar, and the fixes land in fwcd first.

## Upstream sweep: 2026-10-08

All 34 externally sourced grammar crates were checked against their upstream
Git histories, using the September 12 sweep as the previous review boundary.
Recorded revisions and crates.io VCS metadata were checked against the vendored
grammar/scanner inputs; generated parser formatting, build wrappers and local
patches were excluded from the source comparison. Merge ancestry was checked as
well as dates: SQL PR #361 was authored in April but only merged on September 18.

Five grammars are updated in full (F#, Haskell, OCaml, PHP, Swift), and the SQL
scanner gets its upstream serialization/deserialization fix. All six keep the
shared build wiring and headers. The 128-state retune is preserved. For the
updated parsers committed upstream, all automaton counts except
LARGE_STATE_COUNT, plus grammar.json and node-types.json, match upstream exactly.
Swift commits no parser.c; its regenerated node-types.json differs from upstream
only by omitting two anonymous entries (`?` and `??`). Each updated Cargo.toml records the complete
revision, generator and counts. The shared build helper now watches common/
scanner headers as well as the src/ wrappers that include them.

| Crate | Previously vendored sources | Upstream head checked | Result |
|---|---|---|---|
| asm | `839741fef4dab5128952334624905c82b40c7133` | [839741fef](https://github.com/RubixDev/tree-sitter-asm/commit/839741fef4dab5128952334624905c82b40c7133) | No new grammar, scanner or used-query changes since the last sweep. |
| c-sharp | `9150f7d` | [8c0abe0b8](https://github.com/tree-sitter/tree-sitter-c-sharp/commit/8c0abe0b84a3681d3e6852ce2e2a1eecb4f731c5) | Only repository/binding metadata changed; grammar, scanner and node types unchanged. |
| caddyfile | `7374a9d1080e4c5685397edcda7bf20b873d6e69` | [7374a9d10](https://github.com/matthewpi/tree-sitter-caddyfile/commit/7374a9d1080e4c5685397edcda7bf20b873d6e69) | No new grammar, scanner or used-query changes since the last sweep. |
| coffee | `3bb4dbd68ca926c76b3baadb529da4de3726ea37` | [3bb4dbd68](https://github.com/svkozak/tree-sitter-coffeescript/commit/3bb4dbd68ca926c76b3baadb529da4de3726ea37) | No new grammar, scanner or used-query changes since the last sweep. |
| cpp | `5cb9b693cfd7bfacab1d9ff4acac1a4150700609` | [c00922280](https://github.com/tree-sitter/tree-sitter-cpp/commit/c009222808634c1014f82438d4883753516a2c24) | Only a new query inheritance comment since the last sweep; product uses its own complete C++ query. Earlier grammar expansion remains declined. |
| css | 0.25.0 + local fixes | [dda5cfc57](https://github.com/tree-sitter/tree-sitter-css/commit/dda5cfc5722c429eaba1c910ca32c2c0c5bb1a3f) | No new grammar, scanner or used-query changes since the last sweep. |
| csv | `f6bf6e3` | [f6bf6e35e](https://github.com/amaanq/tree-sitter-csv/commit/f6bf6e35eb0b95fbadea4bb39cb9709507fcb181) | No new grammar, scanner or used-query changes since the last sweep. |
| cue | `dd7b90e` | [dd7b90e07](https://github.com/eonpatapon/tree-sitter-cue/commit/dd7b90e0770ff18070c515937ba3c3d6d93db00e) | No new grammar, scanner or used-query changes since the last sweep. |
| dhall | `62013259b26ac210d5de1abf64cf1b047ef88000` | [62013259b](https://github.com/jbellerb/tree-sitter-dhall/commit/62013259b26ac210d5de1abf64cf1b047ef88000) | No new grammar, scanner or used-query changes since the last sweep. |
| ebnf | 0.1.0 crate (`ae69c45`, crates/tree-sitter-ebnf) | [8e635b0b7](https://github.com/RubixDev/ebnf/commit/8e635b0b723c620774dfb8abf382a7f531894b40) | No new grammar, scanner or used-query changes since the last sweep. |
| fsharp | `0d3ccbb` | [aefd0c874](https://github.com/ionide/tree-sitter-fsharp/commit/aefd0c8741bdf3aeb827a228aa4a996a6536697e) | Updated to 0.3.12, grammar/scanner/queries; retune preserved. |
| gitignore | `f4685bf11ac466dd278449bcfe5fd014e94aa504` | [f4685bf11](https://github.com/shunsambongi/tree-sitter-gitignore/commit/f4685bf11ac466dd278449bcfe5fd014e94aa504) | No new grammar, scanner or used-query changes since the last sweep. |
| haskell | `v0.23.1` | [97288e585](https://github.com/tree-sitter/tree-sitter-haskell/commit/97288e585b0bd44199720280d067fa35bacd81ff) | Updated to 0.24.1; paired product query fixed for 0.27, scoped type-variable patch preserved. |
| html | 0.23.2 + local fix | [73a394732](https://github.com/tree-sitter/tree-sitter-html/commit/73a3947324f6efddf9e17c0ea58d454843590cc0) | No new grammar, scanner or used-query changes since the last sweep. |
| jinja-dialects | v0.1.1 + front matter | [4f832fe6f](https://github.com/bennypowers/tree-sitter-jinja-dialects/commit/4f832fe6feeae8c9e5963d835bec0272a8331b47) | No new grammar, scanner or used-query changes since the last sweep. |
| julia | v0.25.0 (`e0f9dcd`) | [e0f9dcd18](https://github.com/tree-sitter/tree-sitter-julia/commit/e0f9dcd180fdcfcfa8d79a3531e11d99e79321d3) | No new grammar, scanner or used-query changes since the last sweep. |
| just | `5685543a6e64f66335e25518c9ae8ffa1dae3d01` | [5685543a6](https://github.com/casey/tree-sitter-just/commit/5685543a6e64f66335e25518c9ae8ffa1dae3d01) | No new grammar, scanner or used-query changes since the last sweep. |
| kdl | v2.0.0 (`e8ff98a`) | [e8ff98a11](https://github.com/amaanq/tree-sitter-kdl/commit/e8ff98a113e175b84b92ec2096820517c1e10a66) | No new grammar, scanner or used-query changes since the last sweep. |
| kotlin-sg | ast-grep 0.4.1 + fwcd `1852ea1` | [1a6f9b1ee](https://github.com/ast-grep/tree-sitter-kotlin/commit/1a6f9b1ee1125a7357493eeb95da48d16ac302b4) | No new grammar/scanner changes in either ast-grep or fwcd. |
| objc | `v3.0.2` | [181a81b8f](https://github.com/tree-sitter-grammars/tree-sitter-objc/commit/181a81b8f23a2d593e7ab4259981f50122909fda) | No new grammar, scanner or used-query changes since the last sweep. |
| ocaml | `v0.25.0` | [3b2e14e06](https://github.com/tree-sitter/tree-sitter-ocaml/commit/3b2e14e0697d405c9aa0beddfa09b71f45abc504) | Updated to 0.26.0 + nested-comment scanner fix; use upstream query for all three parsers. |
| php | `3f2465c217d0a966d41e584b42d75522f2a3149e` | [58d564308](https://github.com/tree-sitter/tree-sitter-php/commit/58d5643086ae60e93a3746cca648363ee97c8761) | Updated to 0.25.1, grammar/scanner; retune preserved. |
| powershell | `v0.26.4` | [e7bd348c4](https://github.com/airbus-cert/tree-sitter-powershell/commit/e7bd348c49fdfd5c853a146a670965ba516a6239) | No new applicable changes; earlier region-fold query is unused by GitComet. |
| ron | 0.2.0 + regeneration compatibility | [78938553b](https://github.com/amaanq/tree-sitter-ron/commit/78938553b93075e638035f624973083451b29055) | No new grammar, scanner or used-query changes since the last sweep. |
| ruby | 0.23.1 + scanner `ad907a6` | [ad907a69d](https://github.com/tree-sitter/tree-sitter-ruby/commit/ad907a69da0c8a4f7a943a7fe012712208da6dee) | No new grammar, scanner or used-query changes since the last sweep. |
| rust | `v0.24.2` | [77a374726](https://github.com/tree-sitter/tree-sitter-rust/commit/77a3747266f4d621d0757825e6b11edcbf991ca5) | No new grammar, scanner or used-query changes since the last sweep. |
| scala | `db390f312` | [db390f312](https://github.com/tree-sitter/tree-sitter-scala/commit/db390f312a54b04b13790e1767bfac32665c17ac) | No new grammar, scanner or used-query changes since the last sweep. |
| sequel | `b7057b7` | [97614d051](https://github.com/derekstride/tree-sitter-sql/commit/97614d051eebfd3bc5d97c0bdb5a1638719ca811) | Applied PR #361 scanner fix only; local T-SQL exclusions and parser tables preserved. |
| spirv | `57032ecc3e8472593137910ecc154356420c26df` | [57032ecc3](https://github.com/JuliaGPU/tree-sitter-spirv/commit/57032ecc3e8472593137910ecc154356420c26df) | No new grammar, scanner or used-query changes since the last sweep. |
| swift | `00bbb0a` | [171fa3bc3](https://github.com/alex-pinkus/tree-sitter-swift/commit/171fa3bc343233fe07f5e17205aaada33d778825) | Updated to 0.7.4, Swift 6 grammar/scanner/query; retune preserved. |
| typescript | `v0.23.2` | [75b3874ed](https://github.com/tree-sitter/tree-sitter-typescript/commit/75b3874edb2dc714fb1fd77a32013d0f8699989f) | No new grammar, scanner or used-query changes since the last sweep. |
| v | `ca35516130a1e7044405e2e1c8fc84b7fb20d544` | [ed235d6c4](https://github.com/undivisible/tree-sitter-v/commit/ed235d6c43c9e29ebcabe8c237cc6823599201c5) | No new grammar, scanner or used-query changes since the last sweep. |
| vue | `ce8011a414fdf8091f4e4071752efc376f4afb08` | [ce8011a41](https://github.com/tree-sitter-grammars/tree-sitter-vue/commit/ce8011a414fdf8091f4e4071752efc376f4afb08) | No new grammar, scanner or used-query changes since the last sweep. |
| wat | `2ca28a9` + escape fix | [2ca28a9f9](https://github.com/wasm-lsp/tree-sitter-wasm/commit/2ca28a9f9d709847bf7a3de0942a84e912f59088) | No new grammar, scanner or used-query changes since the last sweep. |

The Kotlin parent [fwcd/tree-sitter-kotlin](https://github.com/fwcd/tree-sitter-kotlin/commit/1852ea17b7f60fb3f9d84e0b1555d56b46b39fb1)
was checked independently of ast-grep's fork. The shared parser.h, array.h and
alloc.h are byte-identical to the headers emitted by tree-sitter-cli 0.27.0;
Tree-sitter's runtime-private headers have a different allocator contract and
are not replacements for these generated-parser templates. tree-sitter-build,
CIL and crontab are local code and have no external upstream to update.

### Verification of this sweep

The five updated upstream suites passed: F# 582 parses, Haskell 725, OCaml 110,
PHP 148 and Swift 296, together with their supplied highlight/tag assertions.
Swift's legacy test/outline sample directory was excluded from the temporary
checkout because the 0.27 CLI treats it as a query-test directory and tries to
compile its Swift samples as .scm; its corpus and highlight suites ran in full.
The SQL scanner also passed an AddressSanitizer check with 1,024 repeated
deserialization/serialization cycles and no memory leaks. A 32,768-level nested
OCaml comment aborted the old parser with a 1 MiB stack and parsed cleanly with
the updated scanner. Product highlight and injection queries were checked
against the new parsers.
Haskell's old declaration query failed under 0.27 and was updated while retaining
the product's scoped type-variable rule. OCaml's upstream query now serves
implementations, interfaces and standalone types, so the obsolete product copy
was removed.

The old vendored parsers and candidates were also compared using the workspace's
tree-sitter 0.27 runtime, the project fixtures, and the source corpora below.
No previously clean file started failing. Counts include existing unsupported
syntax and deliberately invalid fixture sections; they are comparisons, not
claims that every file parses perfectly.

| Parser | Corpus and fixtures | Files | Files with errors before → after | Trees changed |
|---|---|---:|---:|---:|
| F# | dotnet/fsharp Compiler and FSharp.Core, .fs/.fsx | 260 | 94 → 93 | 23 |
| F# signature | same repository, .fsi | 220 | 148 → 145 | 35 |
| Haskell | haskell/text src | 56 | 0 → 0 | 9 (type_synonym rename) |
| OCaml | ocaml/ocaml compiler and stdlib, .ml | 243 | 0 → 0 | 210 |
| OCaml interface | same repository, .mli | 238 | 2 → 2 | 79 |
| PHP | WordPress wp-includes and wp-admin | 1318 | 2 → 2 | 0 |
| Swift | swift-collections Sources and Tests | 696 | 373 → 203 | 343 |
| SQL | project SQL fixtures | 5 | 5 → 5 | 0 |

F#'s signature grammar changes recovery in 12 already failing files, with three
other files becoming clean. GitComet currently uses the main F# grammar for .fsi;
the separate signature grammar's upstream highlight assertions also pass.
OCaml tree changes accompany the upstream OxCaml/fuzzer improvements, while all
checked implementation files remain clean and the interface error counts stay
unchanged. Targeted product tests cover let-in/quotations, the Haskell node rename
and cases keyword, deeply nested OCaml comments, PHP grouped uses/clone/nowdoc
closers, Swift sending/inline-array/unsafe syntax, and read-only SQL scanner
serialization. The product syntax suite passed: 536 tests, zero failures, one
ignored. It checks every vendored grammar's ABI and the retuned table bounds.
The standalone SQL binding tests and doc test, cargo fmt --all --check and
git diff --check passed as well.

Corpus revisions checked:

- [dotnet/fsharp `3917a24af`](https://github.com/dotnet/fsharp/commit/3917a24af543fd79527d06c1710a3ebd42207be8)
- [haskell/text `ba2c00651`](https://github.com/haskell/text/commit/ba2c00651f9059e0825f301328e5454dfcdeb530)
- [ocaml/ocaml `6471927db`](https://github.com/ocaml/ocaml/commit/6471927db51e6e153fcb591b1ade057f4ac9381a)
- [WordPress/WordPress `12fd26898`](https://github.com/WordPress/WordPress/commit/12fd2689863299e255366d484c184f96e4530af4)
- [apple/swift-collections `3b69cedba`](https://github.com/apple/swift-collections/commit/3b69cedbaa49957e97c092ae400bb57602ef941f)
