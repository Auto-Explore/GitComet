# Native Mermaid previews

GitComet renders Mermaid offline with Rust. File previews recognize `.mmd` and
`.mermaid`; Markdown recognizes those fence labels and unlabelled fences with
an explicit native Mermaid header. Other diagram languages remain source code.
Their syntax fixtures are retained.

The native renderer supports 23 diagram families: architecture, block, C4,
class, ER, flowchart, Gantt, GitGraph, journey, Kanban, mindmap, packet, pie,
quadrant, radar, requirement, Sankey, sequence, state, timeline, treemap,
XY chart and ZenUML. This is the native renderer's language subset, not complete
mermaid.js compatibility. Unsupported configuration settings appear as preview
diagnostics. Supported init directives and YAML `config` frontmatter are applied
before layout. Malformed input shows an error and its source.

Rendering uses bundled IBM Plex Sans, Lilex, Fira Code and Noto Emoji fonts.
It never scans system fonts or launches another renderer. Missing font families
fall back to the bundled fonts. The source limit is 1 MiB; there are no additional
node, edge or nesting limits. Very large graph layout can still be expensive.

## Viewport and caching

Initial and reset zoom is 100%. Fitting axes are centered; oversized axes start
with 16 px padding. Primary-button dragging pans, including outside the viewport.
Zoom controls use 50, 75, 100, 125, 150, 200, 300 and 400 percent. Wheel scrolling
continues to scroll the containing document. Embedded viewports are 320 px tall;
standalone and file previews fill their available space.

Source preparation parses, lays out and produces SVG once, then prepares a
shared immutable usvg scene. Panning and zooming request only visible 512 px tiles
at the actual device scale, with a cropped halo for seamless boundaries. They do
not repeat parsing or layout or upscale a capped whole-diagram bitmap.

The scene LRU retains at most 32 entries and 16 MiB of SVG bytes. This budget
measures retained SVG, not the complete usvg tree allocation. Larger scenes can
render without entering that LRU. The pixel LRU retains at most 64 MiB; GPU image
release runs on the UI thread. Active previews can continue to hold evicted
scenes/images. Duplicate scene and tile work is shared; at most two expensive
jobs run concurrently, with waiting visible tiles prioritized. Source and
viewport generations cancel obsolete queued work and reject stale completion.
Each preview keeps its own viewport state.

## Regression checks

```sh
cargo test -p gitcomet-diagrams --locked --offline
cargo test -p gitcomet-ui-gpui --lib --locked --offline
cargo clippy -p gitcomet-diagrams -p gitcomet-fonts --all-targets --locked --offline -- -D warnings
python3 scripts/ci/boundaries.py
```

The corpus has 93 source fixtures covering all families, plus parameterized
aliases, bounded generated graphs, source/configuration errors and UI/cache
regressions. Checks include labels, finite geometry, visible pixels,
deterministic SVG, natural dimensions, tile seams, cache budgets, concurrent
request deduplication and stale requests. Linux x86_64 compares 23 reviewed
family PNGs exactly. Other platforms check visibility, geometry and dimensions.
Mismatch images are written under `target/mermaid-golden-failures`.

Regenerate PNGs explicitly, inspect every changed image, then rerun tests:

```sh
cargo run -p gitcomet-diagrams --example update_goldens --release --locked --offline -- --write
```

Tests never bless golden images. Fixture attribution is in
[tests/fixtures/README.md](tests/fixtures/README.md); renderer changes are in
[../../vendor/mermaid-rs-renderer/GITCOMET_PATCHES.md](../../vendor/mermaid-rs-renderer/GITCOMET_PATCHES.md).

## Performance measurements

The measured release latency/allocation comparison and stage results are in
[benches/results/2026-10-07/README.md](benches/results/2026-10-07/README.md).

The core Criterion benchmark exercises the production renderer and separately
measures parsing, layout, SVG, scene preparation, visible tiles and header
recognition for small, typical, dense and large flowcharts:

```sh
cargo bench -p gitcomet-diagrams --bench mermaid --locked --offline -- --quick
```

The UI benchmark uses the production scene cache, tile cache and GPUI test
platform. Select exact cases to avoid setting up unrelated benchmark fixtures:

```sh
cargo bench -p gitcomet-ui-gpui --bench performance --features benchmarks --locked --offline -- --exact mermaid_preview/scene_cache_hit/typical --quick
cargo bench -p gitcomet-ui-gpui --bench performance --features benchmarks --locked --offline -- --exact mermaid_preview/preview_frame/typical --quick
cargo bench -p gitcomet-ui-gpui --bench performance --features benchmarks --locked --offline -- --exact mermaid_preview/pan_zoom_tiles/typical --quick
```

Additional cases cover cold preparation, concurrent identical requests and a
Markdown document with 100 diagrams. Allocation sidecars accompany cache-hit
and tile cases. Frame timings use the GPUI test platform and do not measure
GPU presentation latency. Use the default release benchmark profile for timing;
`ci-bench` is suitable for a compile/run smoke check, not production comparison.
