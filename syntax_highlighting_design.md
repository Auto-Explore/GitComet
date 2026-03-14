# Robust Syntax Highlighting Design

## Goal

GitComet should adopt a document-based syntax highlighting model that behaves much closer to Zed:

- Large files should paint immediately without waiting for syntax highlighting.
- Full-document syntax parsing should continue asynchronously and apply later.
- Scrolling very large files should stay smooth even before all highlight slices are ready.
- Where GitComet has whole-file text, syntax should be based on the real document, not per-line heuristics.
- Rust highlighting should get much closer to Zed.
- HTML should support real HTML highlighting plus embedded CSS/JavaScript injections.
- XML should be a real language with its own grammar instead of being treated as HTML.

Important scope distinction:

- Parsing/highlighting ownership should become document-based.
- Rendering can remain row-based and virtualized.

That is the right compromise for GitComet. The UI does not need to stop being row-oriented, but syntax state must stop being line-local.

## Concrete Fixture Used In This Investigation

Primary real-world stress file:

- `/home/sampo/git/gitmess/gitmess/index.html`
- Size: `47,865,848` bytes
- Lines: `502,734`
- Max line length: `764` bytes

This matters because the current failure mode is driven mainly by line-count gating, not by pathological single-line length.

## Success Criteria

For the large HTML fixture above, GitComet should eventually satisfy all of these:

- The file becomes visible immediately with plain text or minimal diff styling.
- GitComet does not block first paint on full syntax parsing.
- Background parsing later enables syntax on the visible window.
- Scrolling remains smooth even while parse work or highlight-slice work is still catching up.
- HTML tags/attributes highlight correctly, and embedded `<script>`, `<style>`, `style=""`, and `on*=""` content can be highlighted via injections.

For language fidelity:

- Rust should stop using the current flattened, reduced query mapping and move closer to Zed’s query assets.
- XML should have first-class grammar support and extension mapping.

## Current GitComet State

### What already exists

GitComet is not starting from zero. It already has some good building blocks:

- A full-document tree-sitter parse path with a `1ms` foreground budget plus background fallback exists in `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1172-1243` and `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1246-1484`.
- Prepared syntax documents already retain full tree state, line lengths, and line starts in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:97-116`.
- Prepared documents already support lazy 64-row chunk tokenization in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:438-534`.
- Incremental reparse seeds already exist in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1218-1365`.
- `TextInput` already has a visible-window streamed highlight-run path in `crates/gitcomet-ui-gpui/src/kit/text_input.rs:510-553` and `crates/gitcomet-ui-gpui/src/kit/text_input.rs:3760-3783`.
- `TextModel` already has large-text-oriented chunked storage and line indexing in `crates/gitcomet-ui-gpui/src/kit/text_model.rs:6-217`.

This is enough to build a good system without copying Zed wholesale.

### Current blockers and failure modes

| Problem | Evidence | Why it matters |
| --- | --- | --- |
| Large files are hard-disabled from prepared syntax | `crates/gitcomet-ui-gpui/src/view/rows/mod.rs:5-6`, `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:879-885`, `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1039-1044`, `crates/gitcomet-ui-gpui/src/view/panes/main.rs:17-18` | The 50MB HTML fixture can never reach full syntax in current file/worktree/resolved-output paths. |
| Worktree/file diff async parse path only runs when syntax mode is `Auto` | `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1186-1189`, `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1251-1254` | The background parse pipeline already exists, but line-count gates prevent it from even starting on the cases that need it most. |
| Render-time fallback is still line-local | `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs:211-245`, `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs:482-540` | Even with a prepared document available, too much of the API surface is still designed around “give me one line’s tokens.” |
| Per-line tree-sitter fallback is capped at 512 bytes | `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1377-1379` | Any missing prepared-document state drops back to a line-local path with additional limits. |
| Resolved-output highlighting is built by splitting the whole document and highlighting line by line | `crates/gitcomet-ui-gpui/src/view/panes/main/helpers.rs:3-32`, `crates/gitcomet-ui-gpui/src/view/panes/main/core_impl.rs:592-599`, `crates/gitcomet-ui-gpui/src/view/panes/main/core_impl.rs:683-690` | This is exactly the opposite of the desired “parse once, slice later” model. |
| TextInput currently expects a full highlight vector | `crates/gitcomet-ui-gpui/src/kit/text_input.rs:456-466` | For very large documents, materializing a complete `(Range, HighlightStyle)` list is the wrong scaling point. |
| HTML has no injection system | `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1873-1878` | Script/style regions and style/on* attributes are not highlighted as embedded languages. |
| XML is not a real language | `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:709-712` | XML, SVG, XSL, XSD, and related files are all forced through HTML highlighting as a fallback. |
| Rust uses crate-default queries and drops `variable.*` captures | `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1891-1896`, `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1958-1977` | This explains why Rust does not look like Zed. The data has already been simplified away. |
| Many languages still have no tree-sitter grammar wired in | `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1808-1839` | The current syntax layer is still partly a language whitelist plus heuristics. |
| Inline file diff syntax is parsed over a mixed add/remove/context stream | `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1286-1305` | That stream is not a real file. Full-document syntax correctness is impossible there around edits. |
| Inline rows already carry real old/new line mapping, but syntax does not use it | `crates/gitcomet-core/src/diff.rs:4-10`, `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1615-1645` | This is an opportunity: inline syntax can be projected from real old/new documents instead of parsing synthetic inline text. |
| Background syntax completion currently clears whole row-style caches | `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1477-1479`, `crates/gitcomet-ui-gpui/src/view/panes/main/core_impl.rs:620-623` | For very large views, full cache clears are too blunt. |

## What Zed Does That Matters

Local Zed checkout used here: `/home/sampo/git/zed`

### 1. Parse pipeline: immediate foreground interpolation, short sync wait, background completion

Zed’s buffer reparsing flow in `../zed/crates/language/src/buffer.rs:1809-1875` does three important things:

- It immediately interpolates edits into the current foreground syntax state.
- It allows only a very short synchronous parse wait.
- It continues the real parse in the background if that short wait times out.

This is the correct model for GitComet too.

GitComet already has a partial equivalent. It should be expanded, not replaced.

### 2. Syntax is stored as layered trees, not as isolated line results

Zed’s `SyntaxMap` in `../zed/crates/language/src/syntax_map.rs:29-140`, `../zed/crates/language/src/syntax_map.rs:321-430`, and `../zed/crates/language/src/syntax_map.rs:525-620` keeps:

- a root parse tree,
- injection layers,
- incremental reparse state,
- range-oriented data structures for captures and reparsing.

GitComet does not need to port Zed’s exact `SumTree`/anchor implementation. But it does need the same architectural idea:

- parse whole documents,
- keep tree state around,
- support injections as first-class syntax layers,
- answer highlight queries by range.

### 3. Highlighting is range-based and streamed

Zed’s highlight retrieval in `../zed/crates/language/src/buffer.rs:3747-3792` and `../zed/crates/language/src/buffer.rs:5487-5525` works over buffer ranges and rope chunks, not one independent line at a time.

This is the most important direction for GitComet’s next architecture step.

### 4. Rendering stays viewport-oriented

Zed keeps rendering bounded:

- line shaping is capped in `../zed/crates/editor/src/editor.rs:245`,
- chunk-to-line shaping is built for currently needed content in `../zed/crates/editor/src/element.rs:8775-8835`.

GitComet already follows the same general idea in `TextInput` and row virtualization. The syntax layer should stop fighting that model by generating whole-document highlight vectors.

### 5. Language assets are real files, not only crate defaults

Relevant Zed assets:

- Rust query: `../zed/crates/languages/src/rust/highlights.scm`
- HTML query: `../zed/extensions/html/languages/html/highlights.scm`
- HTML injections: `../zed/extensions/html/languages/html/injections.scm`
- JavaScript query: `../zed/crates/languages/src/javascript/highlights.scm`

This matters because GitComet currently depends mostly on parser crate `HIGHLIGHTS_QUERY` constants. That is too limiting if the goal is “closer to Zed” rather than “generic tree-sitter output.”

### 6. XML note

The local Zed checkout does not include the XML extension implementation. It only documents the external extension in `../zed/docs/src/languages/xml.md:8-10`.

That still gives one useful design conclusion:

- Zed treats XML as its own language/extension path, not as HTML.

GitComet should do the same.

## Target Architecture For GitComet

### 1. Introduce document-owned syntax sessions

Add a syntax-session layer that represents “syntax state for one document source.”

Suggested shape:

```rust
struct SyntaxSessionKey {
    source_kind: SyntaxSourceKind,
    path: Option<PathBuf>,
    side: SyntaxDocumentSide,
    content_hash: u64,
}

enum SyntaxPhase {
    Plain,
    Parsing,
    Ready,
    Failed,
}

struct SyntaxSession {
    key: SyntaxSessionKey,
    language: SyntaxLanguageId,
    source_text: Arc<str>,
    line_starts: Arc<[usize]>,
    phase: SyntaxPhase,
    syntax_epoch: u64,
    parsed: Option<ParsedSyntaxDocument>,
    visible_slice_cache: SyntaxSliceCache,
}
```

The exact type names do not matter. The invariants do:

- the session owns a whole document,
- the session knows its line index,
- the session has a parse phase,
- the session answers range/viewport highlight requests,
- the session can increment an epoch when new syntax becomes available.

### 2. Make parsing always eligible for full-document views

Remove the “over 4,000 lines means no prepared syntax” policy for views that already have whole-file text:

- file diff old side,
- file diff new side,
- worktree preview,
- resolved output,
- future raw full-text preview/editor surfaces.

Do not replace that gate with a bigger gate. The problem is architectural, not numeric.

Correct behavior:

- if a grammar exists, create a syntax session,
- paint immediately in plain text,
- try a `1ms` foreground parse budget,
- continue in the background if needed,
- apply syntax later.

`HeuristicOnly` should remain as a fallback only when:

- no grammar/query asset exists,
- the session is not ready yet and the UI chooses to show a temporary minimal fallback,
- or the context is not a real whole document.

### 3. Change the highlight API from “line first” to “range first”

The new primary API should be byte-range based, with row helpers built on top.

Suggested API shape:

```rust
fn ensure_parse(session: &SyntaxSessionHandle, budget: ParseBudget);
fn request_visible_rows(session: &SyntaxSessionHandle, rows: Range<usize>);
fn highlight_spans_for_byte_range(
    session: &SyntaxSessionHandle,
    range: Range<usize>,
) -> HighlightSliceStatus;
```

Where `HighlightSliceStatus` can be something like:

- `Ready(Vec<HighlightSpan>)`
- `Pending`
- `Unsupported`

This enables the right runtime behavior:

- visible rows without a ready syntax slice can still render immediately,
- requesting a slice never has to block the render path,
- completed slices can invalidate only overlapping rows.

Current prepared-document line tokenization in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:438-534` is still useful as a transitional internal cache. But it should stop being the top-level contract.

### 4. Add a layered syntax model for injections

GitComet’s current `PreparedSyntaxTreeState` holds one tree in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:97-108`.

For HTML and future embedded-language support, that should become a layered parse model:

- one root tree,
- zero or more injected subtrees,
- compiled injection queries,
- merged capture iteration by byte range.

GitComet does not need Zed’s exact `SyntaxMap` data structures. For static or lightly edited documents, a simpler layered representation is enough:

- `root: ParsedLayer`
- `injections: Vec<ParsedLayer>`
- layers sorted by byte range and depth

The key requirement is that HTML can ask:

- where are `<script>` regions,
- where are `<style>` regions,
- where are `style=""` attributes,
- where are `on*=""` attributes,

and then parse those ranges with the correct embedded grammar.

### 5. Vendor language assets instead of relying on parser crate defaults

Add a small asset system in GitComet, for example under a new directory like:

- `crates/gitcomet-ui-gpui/src/syntax_assets/`
- or `crates/gitcomet-ui-gpui/src/view/rows/diff_text/queries/`

Each language asset should define:

- grammar loader,
- `highlights.scm`,
- optional `injections.scm`,
- optional config metadata later.

Suggested minimum first-class assets:

- Rust
- HTML
- CSS
- JavaScript
- XML

Why this is necessary:

- GitComet currently uses `tree_sitter_rust::HIGHLIGHTS_QUERY` and friends in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1842-1955`.
- GitComet also flattens capture names into a tiny enum and drops `variable.*` in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1958-1977`.
- Zed’s higher-quality output depends on richer query assets, not only on “using tree-sitter.”

### 6. Stop flattening all captures into the current 10-token model

Current token kinds live in `crates/gitcomet-ui-gpui/src/view/diff_text_model.rs:19-30`.

That model is too small for “rich” highlighting and directly causes loss of information. Examples already present in Zed assets:

- `@type.interface`
- `@type.builtin`
- `@function.special`
- `@function.definition`
- `@variable.parameter`
- `@comment.doc`
- `@string.escape`
- `@punctuation.delimiter.html`

Recommended approach:

- compile capture names to a small internal `HighlightClassId`,
- keep the canonical capture-name semantics,
- map those classes to `gpui::HighlightStyle` through a GitComet theme adapter,
- keep fallback grouping so unsupported classes still degrade gracefully.

This lets GitComet import Zed-style query assets without immediately needing a full Zed theme engine.

### 7. Fix inline file diff correctness by projecting from old/new documents

This is a separate correctness issue from performance, and it should be fixed.

Current inline syntax preparation uses `file_diff_inline_cache` in `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1286-1305`. That inline stream interleaves removed and added lines from different document versions. It is not a real parseable file.

Instead:

- parse only the real old document,
- parse only the real new document,
- remove the inline prepared syntax document as an independent parse target,
- render inline rows by projecting syntax from the correct side using `old_line` / `new_line`.

Projection rules:

- `Context` row: use either side’s real document line, preferably the new side for consistency.
- `Remove` row: highlight from old document line `old_line - 1`.
- `Add` row: highlight from new document line `new_line - 1`.
- `Header` / `Hunk` / meta rows: no syntax.

GitComet already carries the line mapping needed for this in:

- `crates/gitcomet-core/src/diff.rs:4-10`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs:1615-1645`

This is the cleanest way to get real full-document syntax semantics in inline diff view.

### 8. Extend TextInput with a highlight provider for large documents

`TextInput` already has good visible-window highlight-run machinery in `crates/gitcomet-ui-gpui/src/kit/text_input.rs:510-553` and `crates/gitcomet-ui-gpui/src/kit/text_input.rs:3760-3783`.

What it lacks is the right input contract.

Today it takes a full highlight vector through `set_highlights(...)` in `crates/gitcomet-ui-gpui/src/kit/text_input.rs:456-466`.

For large resolved output, add a second path:

- keep `set_highlights(Vec<...>)` for small docs,
- add `set_highlight_provider(...)` or similar for large docs,
- let prepaint request only the visible-window highlight spans,
- cache those runs by `(highlight_epoch, visible_start, visible_end)` as it already does.

This should be the long-term path for conflict resolved output and any future full-text editor/viewer surfaces.

### 9. Keep scrolling smooth by making missing highlight slices non-blocking

This is crucial.

Current missing prepared-document chunks are built synchronously in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:478-534`.

That is acceptable for small files, but it is the wrong policy for very large documents if slice construction becomes expensive.

Recommended runtime rule:

- if the parse is not ready, render plain text,
- if the parse is ready but the requested visible slice is missing, queue slice work in the background and render plain text or minimal syntax for now,
- when the slice arrives, invalidate only those visible rows.

This is the most important behavior for smooth scrolling under load.

A visible-slice prefetch policy should also exist:

- prefetch current visible range,
- prefetch a guard band above and below it,
- deprioritize non-visible slices,
- drop least-recently-used slices first.

### 10. Share source text where possible

Current tree-sitter input collection rebuilds a full `String` from lines in `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs:1191-1202`.

That is workable, but not ideal for very large files.

Recommended medium-term improvement:

- use `Arc<str>` or `SharedString` as the owned source text inside syntax sessions,
- reuse that shared text for parsing and range mapping,
- use `TextModel` / `TextModelSnapshot` directly for editable full-text contexts like resolved output.

This does not need to block the first implementation pass. But it should be planned, because 50MB documents make duplication visible.

## What To Implement, In Order

### Phase 0: Instrumentation and benchmark scaffolding

Before changing behavior, add measurement paths so regressions are visible.

Use existing benchmark infrastructure:

- `crates/gitcomet-ui-gpui/src/view/rows/benchmarks.rs`
- `crates/gitcomet-ui-gpui/benches/performance.rs`

Add fixture support for:

- the real HTML file via an env var or local-path config,
- a synthetic large HTML generator for CI-safe coverage.

Recommended benchmark scenarios:

- large file first paint with syntax disabled until background completion,
- background parse completion time,
- visible-window highlight slice build time, cold and warm,
- scroll step time before syntax is ready,
- scroll step time after syntax is ready,
- resolved-output incremental reparse after a small edit,
- inline diff syntax projection correctness and cost.

### Phase 1: Language assets and hard-gate removal

Files most likely touched:

- `Cargo.toml`
- `crates/gitcomet-ui-gpui/Cargo.toml`
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/mod.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/history.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/diff.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/conflict_resolver.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/main/helpers.rs`

Concrete work:

- Add a real asset loader around `include_str!` query files.
- Add `DiffSyntaxLanguage::Xml`.
- Stop mapping XML extensions to `Html`.
- Add a real JavaScript grammar instead of piggybacking on TSX.
- Import Rust and HTML query assets modeled on Zed.
- Add HTML injection queries.
- Remove the 4,000-line eligibility gates for full-document-capable views.
- Keep current plain-first async parse behavior, but make it reachable for large files.

Dependencies likely needed:

- `tree-sitter-javascript`
- `tree-sitter-xml` or equivalent maintained XML grammar crate

### Phase 2: Syntax sessions and inline-diff projection

Files most likely touched:

- `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/diff.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/history.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs`
- `crates/gitcomet-core/src/diff.rs` only if extra metadata becomes necessary

Concrete work:

- Introduce session handles for worktree preview and file diff old/new documents.
- Stop preparing a separate inline mixed-document syntax tree.
- Project inline syntax from the old/new sessions via `old_line` / `new_line`.
- Give prepared syntax state a per-session epoch rather than relying on broad cache clears.

### Phase 3: Visible-range highlight slices

Files most likely touched:

- `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/main/core_impl.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/main/helpers.rs`
- `crates/gitcomet-ui-gpui/src/kit/text_input.rs`

Concrete work:

- Add a byte-range/row-range highlight slice API.
- Build missing visible slices off the render path.
- Cache slices by range plus syntax epoch.
- Invalidate only overlapping rows when slices arrive.
- Add a `TextInput` highlight-provider path so resolved output stops building one huge highlight vector.

### Phase 4: Incremental editing and memory polish

Files most likely touched:

- `crates/gitcomet-ui-gpui/src/kit/text_model.rs`
- `crates/gitcomet-ui-gpui/src/kit/text_input.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/main/core_impl.rs`
- `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs`

Concrete work:

- Reuse `TextModelSnapshot` for editable full-text syntax sessions.
- Feed edit deltas into incremental reparsing for conflict resolved output.
- Reduce source-text duplication for very large documents.
- Tune slice sizes, cache limits, and prefetch policies with benchmarks.

## Recommended Initial Acceptance Tests

### Correctness

- Rust snippets that currently differ from Zed should gain expected token classes for identifiers, macros, types, fields, and variables.
- HTML should highlight tags/attributes/comments and embedded JavaScript/CSS in script/style and attribute injections.
- XML should highlight tags/attributes/comments/entities with real XML grammar support.
- Inline file diff rows should derive syntax from the correct old/new line, not from a mixed synthetic parse.
- A document with more than `4,000` lines should still create a full-document syntax session.

### Performance

- Opening the large HTML fixture should not wait for background parse completion.
- Scroll cost should remain bounded even when syntax slices are still being prepared.
- Background syntax completion should not force full style-cache rebuilds for unrelated rows.
- Resolved-output edits should use incremental reparse rather than rebuilding a full highlight vector each time.

## Risks and Open Questions

### 1. Patch diff vs full file diff

Not every diff surface in GitComet has whole-file text available.

Recommendation:

- full file diff, worktree preview, and resolved output should get the full document-based architecture first,
- patch-only surfaces can remain best-effort until upstream data includes whole-file snapshots.

Do not promise “full-file semantics” where GitComet only has patch hunks.

### 2. Theme richness

Moving from `SyntaxTokenKind` to richer capture classes will expose theme limitations.

Recommendation:

- add a fallback style map first,
- keep capture-class fidelity even if multiple classes initially share the same visual color,
- improve theme differentiation later.

### 3. Query asset provenance

If GitComet vendors Zed-inspired or upstream query assets, keep file provenance clear in comments or adjacent documentation.

### 4. Memory growth

Holding source text, parse trees, and visible-slice caches for multiple large documents can get expensive.

Recommendation:

- keep session LRU bounds,
- reuse text storage where possible,
- continue using deferred drop for large syntax payloads where helpful.

### 5. XML implementation source

The local Zed checkout only points to the external XML extension. It does not provide local XML query assets to copy directly.

Recommendation:

- add XML as a first-class GitComet language anyway,
- use a maintained XML tree-sitter grammar crate,
- author GitComet XML highlight queries directly if necessary,
- optionally later compare against the external Zed XML extension when that source is locally available.

## Where To Look Later

### GitComet references

- Syntax core: `crates/gitcomet-ui-gpui/src/view/rows/diff_text/syntax.rs`
- Syntax color/model layer: `crates/gitcomet-ui-gpui/src/view/diff_text_model.rs`, `crates/gitcomet-ui-gpui/src/view/rows/diff_text.rs`
- File/worktree async syntax prep: `crates/gitcomet-ui-gpui/src/view/panes/main/diff_cache.rs`
- Resolved-output line-by-line helper to replace: `crates/gitcomet-ui-gpui/src/view/panes/main/helpers.rs`
- TextInput visible-window highlight path: `crates/gitcomet-ui-gpui/src/kit/text_input.rs`
- Large text model: `crates/gitcomet-ui-gpui/src/kit/text_model.rs`
- Inline diff line mapping: `crates/gitcomet-core/src/diff.rs`

### Zed references

- Reparse pipeline: `../zed/crates/language/src/buffer.rs:1809-1875`
- Syntax layer model: `../zed/crates/language/src/syntax_map.rs:29-140`
- Edit interpolation: `../zed/crates/language/src/syntax_map.rs:321-430`
- Range reparsing and layered parse steps: `../zed/crates/language/src/syntax_map.rs:525-620`
- Range-based highlight retrieval: `../zed/crates/language/src/buffer.rs:3747-3792`
- Chunked buffer highlight streaming: `../zed/crates/language/src/buffer.rs:5487-5525`
- Rust query asset: `../zed/crates/languages/src/rust/highlights.scm`
- JavaScript query asset: `../zed/crates/languages/src/javascript/highlights.scm`
- HTML query asset: `../zed/extensions/html/languages/html/highlights.scm`
- HTML injections: `../zed/extensions/html/languages/html/injections.scm`
- HTML config/layout reference: `../zed/extensions/html/languages/html/config.toml`
- XML local note: `../zed/docs/src/languages/xml.md`

## Short implementation summary

If this work is started later without redoing the investigation, the correct order is:

1. Land language assets plus XML/JavaScript grammar support.
2. Remove line-count eligibility gates for full-document views while preserving plain-first async parse.
3. Replace inline synthetic parsing with old/new document projection.
4. Introduce visible-range syntax slices and a `TextInput` highlight-provider path.
5. Add incremental editing and memory optimizations after the architecture is already correct.
