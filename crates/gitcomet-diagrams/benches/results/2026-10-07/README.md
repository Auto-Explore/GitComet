# Mermaid performance — 2026-10-07

Linux x86_64, AMD Ryzen 9 5950X, Rust 1.98.1, x86-64-v3, optimized release
builds with fat LTO and one codegen unit. These are local exploratory results;
other builds/tests ran on the machine during measurement. They are not CI
thresholds. Probe processes ran sequentially, both pinned to logical CPU 31.
Criterion processes ran on logical CPU 15 with `--quick --noplot`.

## Rendering changed source

The allocation probe compares the original published mermaid-rs-renderer 0.3.1
`render_strict` plus XML validation with `gitcomet_diagrams::render`. Both use
System with stats_alloc, with 20 warm renders per graph. The revised path also
applies source configuration, uses bundled fonts, and validates natural SVG
sizes. Fonts and therefore layout/SVG output differ; this is a comparison of
the complete paths, not an isolated algorithm comparison. The CSVs include
first-render results; only the small case starts with a cold process.

| Case | Original warm ms | Revised warm ms | Original allocations | Revised allocations | Original allocated MiB | Revised allocated MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Small, 4 edges | 12.035 | 15.031 | 279,457 | 296,422 | 8.91 | 9.42 |
| Typical, 50 edges | 92.602 | 92.105 | 958,239 | 980,967 | 31.79 | 32.19 |
| Dense, 100 chain edges + 97 cross edges | 923.560 | 918.835 | 17,238,859 | 17,597,370 | 139.68 | 146.17 |
| Large, 500 edges | 1419.107 | 1178.024 | 3,410,087 | 3,413,187 | 51.08 | 50.07 |

The change does not produce a general full-layout speedup or allocation
reduction. Small source renders regress in this probe, typical/dense results
are close, and the large case improves. The benefit for interaction is sharing
prepared scenes and avoiding these passes entirely during pan, zoom and repeat
previews. Native layout remains the main cost when source changes.

Reproduce the probe (the published baseline crate must be in the Cargo cache):

```sh
python3 scripts/profiling/benchmark-mermaid-renderer.py
```

## Production stages and cache hits

Criterion uses the production release renderer without the System allocation
instrumentation. Representative central estimates are below; all 29 results
are in [core-criterion.txt](core-criterion.txt).

| Stage | Small | Typical | Dense | Large |
| --- | ---: | ---: | ---: | ---: |
| Parse/layout/SVG | 11.431 ms | 89.085 ms | 770.07 ms | 1101.9 ms |
| Parsing | 14.661 µs | 183.61 µs | 597.52 µs | 1.9713 ms |
| Layout | 11.536 ms | 88.849 ms | 769.28 ms | 1097.7 ms |
| SVG | 12.775 µs | 130.07 µs | 810.01 µs | 1.3493 ms |
| Prepare scene | 193.37 µs | 1.8774 ms | 4.7199 ms | 17.943 ms |
| Visible 512 px tile, 100%/DPR1 | 304.37 µs | 486.19 µs | 4.5027 ms | 2.0827 ms |
| Visible tile, 400%/DPR2 | 445.77 µs | 697.22 µs | 4.3145 ms | 3.1142 ms |

Header recognition is 185.27 ns. The UI production scene cache hit for the
typical case is 233.36 ns with zero allocation operations/bytes in its sidecar.
A warmed embedded preview frame is 96.757 µs on the GPUI test platform. This
measures layout/paint command generation, not GPU/compositor presentation.
These UI paths use the existing production mimalloc tracking allocator.
All 16 UI cases passed in release mode. The warmed typical pan/zoom tile batch
is 181.45 ns with two allocation operations (120 bytes); all requested tiles
are visible and cached after warmup. Tile misses are covered by the separate
raster timings above. The 100-diagram Markdown parse/cache path is 53.012 µs.
Allocation sidecars are retained as ui-cache-sidecar.json and ui-pan-sidecar.json.

Commands and the distinction between release and ci-bench profiles are in
[../../../README.md](../../../README.md). Raw before/after allocation
results are in [before.csv](before.csv) and [after.csv](after.csv).
