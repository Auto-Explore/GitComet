# Vendored tree-sitter grammars

Each `tree-sitter-*` directory here is a grammar GitComet compiles from source
rather than pulling from crates.io. Every one of them carries its reason in its
own `Cargo.toml` header. There are three reasons in play:

- **Unresolvable dependency.** The crate pins `tree-sitter ~0.20`, and the
  `links = "tree-sitter"` key makes that unresolvable alongside the workspace's
  0.26. Depending on `tree-sitter-language` instead drops the pin.
- **No release, or a needed fix.** No crates.io release exists, or upstream's
  grammar has a bug GitComet patches (`tree-sitter-asm`'s `mnemonic` token,
  `tree-sitter-ruby`'s `=begin` rule).
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

`perl` is deliberately **not** on this list even though it would save 1.19 MB.
See "When regeneration is not safe" below.

### Why this is safe

The threshold is consumed only by the renderer. It selects how a state's actions
are *written*, never which states exist or what they do — so the automaton is
identical and every parse is identical.

That is checked rather than assumed. Every retuned grammar was built twice and
made to parse a real sample from `fixtures/syntax_test`: once as vendored here,
once from upstream's own `parser.c` compiled the way cargo used to compile it.
The `ts_node_string` output has to match byte for byte. That check is what found
both problems described below.

### When regeneration is not safe

**A newer CLI can build a different automaton.** Regenerating preserves the
representation only while tree-sitter-cli 0.26.13 reproduces the automaton
upstream shipped. For `tree-sitter-perl` it does not: 0.26.13 yields
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
and `1f30145` moves the grammar to the new header. Neither is in a release —
`v0.24.2` is still the newest tag — so this crate is pinned to git rev
`3f2465c`, the last commit that touches the grammar or the scanner. It uses the
shared headers like every other grammar.

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
# 1. Build a CLI against the patched generator.
cargo install tree-sitter-cli --version 0.26.13 --root /tmp/tsroot --locked \
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
should have dropped, and `cargo test -p gitcomet-ui-gpui --lib syntax_corpus`
should still pass.
