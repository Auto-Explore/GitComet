# Performance Runbook

## Scope

This runbook exists so `local_agent_loop.sh` can investigate the current large-file incident before optimize mode starts.

Current incident: GitComet crashed while opening or diffing an approximately 150 MB `.json` file.

Treat this as a large-text and syntax-path regression until the measurements prove otherwise.

## Goals

1. Reproduce the failure with the narrowest command that is still credible.
2. Capture CPU timing plus allocation bytes and allocation ops for that same command.
3. Identify whether the failure is dominated by syntax preparation, diff materialization, or a later follow-on pass.
4. Only switch into optimize mode after there is at least one stable benchmark or harness reproduction to track.

## Measurement Rules

- Always compare before and after with the same benchmark label and the same environment variables.
- Prefer one narrow benchmark over a full-suite run while reproduction is still uncertain.
- Treat a crash, abort, or OOM as a valid result. Record the exact command and the observed failure mode in the handoff.
- Do not weaken budgets, remove syntax work, or disable instrumentation just to make the workload complete.

## Allocation Metric Semantics

- CPU: Criterion mean or harness-reported timing for the exact same benchmark label.
- Allocation bytes: `alloc_bytes` from the sidecar payload captured by `PerfTrackingAllocator`.
- Allocation ops: `alloc_ops` from the same sidecar payload.
- For Criterion syntax benches, sidecars now also emit `tree_sitter_alloc_*` when the benchmark uses the benchmark-only native tree-sitter allocator shim. Use those fields for native parser heap comparisons; the base `alloc_*` fields remain Rust-side only.
- On the current worktree, prepared whole-document tree-sitter is skipped above `8 MiB`, so the two large-JSON proxy commands below now measure that retained heuristic-fallback gate instead of the old parser-path outlier unless the threshold changes again.
- When a command crashes before Criterion finishes, keep the task unchecked and record the crash together with any partial timing or stderr output.

## Large-JSON Proxy Commands

These are the first commands to try because they are already wired into the current performance suite.

### Syntax prepare proxy

This now uses a JSON-grammar synthetic document and approximates a roughly
150 MB single-document workload:

```bash
GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES=12200 \
GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES=12288 \
cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
  "file_diff_syntax_prepare/file_diff_syntax_prepare_cold" --exact
```

### Query-stress proxy

This now uses a JSON-grammar nested synthetic document and is the best existing
stress case for large nested syntax inputs and long lines:

```bash
GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINES=3072 \
GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINE_BYTES=49152 \
GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_NESTING=192 \
cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
  "file_diff_syntax_query_stress/nested_long_lines_cold" --exact
```

### Diff-open proxies

Use these after the syntax-only proxies if the crash looks tied to file-diff row creation rather than parser preparation:

```bash
GITCOMET_BENCH_FILE_DIFF_LINES=1200000 \
GITCOMET_BENCH_FILE_DIFF_WINDOW=200 \
cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
  "diff_open_file_inline_first_window/200" --exact
```

```bash
GITCOMET_BENCH_FILE_DIFF_LINES=1200000 \
GITCOMET_BENCH_FILE_DIFF_WINDOW=200 \
cargo bench -p gitcomet-ui-gpui --features benchmarks --bench performance -- --noplot \
  "diff_open_file_split_first_window/200" --exact
```

These diff-open cases currently use synthetic Rust-like file text, so they are secondary proxies. If they do not resemble the crash, add a JSON-specific benchmark instead of overfitting to them.

## When To Add A New Benchmark

Add a new benchmark or harness case when any of these are true:

- the existing syntax proxies do not reproduce the crash, timeout, or allocation spike
- the crash only happens for JSON-specific parser behavior
- the crash requires a real diff path rather than standalone document preparation
- the workload depends on minified JSON or another shape the current synthetic fixtures do not model

Preferred location for new benchmark code:

- fixture code: `crates/gitcomet-ui-gpui/src/view/rows/benchmarks/`
- benchmark harness: `crates/gitcomet-ui-gpui/benches/performance/`

## Triage Heuristics

- If syntax-only proxies crash first, inspect prepared syntax document creation, chunk workers, cache drops, and deferred cleanup paths.
- If diff-open proxies fail first, inspect row provider construction, side-by-side planning, inline projection, and cached text materialization.
- If the app survives the narrow benchmarks but still crashes in the real UI path, instrument the missing stage rather than optimizing blind.

## Handoff Minimums

Each loop turn should append these facts to the handoff:

- exact command run
- benchmark label or harness path
- CPU timing if available
- allocation bytes and allocation ops if available
- crash type or success result
- changed files, if any
- next recommended measurement
