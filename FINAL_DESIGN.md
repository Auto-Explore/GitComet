## Implementation Progress

### External Diff/Merge Usage Design (`external_usage.md`)

- ✅ CLI subcommands and argument model (`gitgpui-app difftool`, `gitgpui-app mergetool`) implemented in `crates/gitgpui-app/src/cli.rs`.
- ✅ Arg/env resolution + validation implemented for `LOCAL`, `REMOTE`, `MERGED`, `BASE`, labels, missing-input and missing-path errors.
- ✅ Exit code constants aligned to design (`0`, `1`, `>=2`) defined in app CLI module.
- ✅ Foundational conflict-marker label formatter implemented in `crates/gitgpui-core/src/conflict_labels.rs` (`empty tree`, `<short-sha>:<path>`, merged-ancestors, rebase-parent shapes), ready for focused merge-mode integration.
- 🔧 Focused command-mode execution path is partially implemented:
  - ✅ `difftool` mode now executes a dedicated runtime path in `crates/gitgpui-app/src/difftool_mode.rs` (delegates to `git diff --no-index`, supports labels/display-path headers, and maps git exit `1`/diff-present to app success exit `0`).
  - ⬜ Focused `mergetool` UI/runtime path is still not implemented; `main.rs` still exits with explicit not-yet-implemented messaging for mergetool mode.
- ✅ External mergetool backend launch exists (`launch_mergetool`) with stage materialization (`BASE/LOCAL/REMOTE`), trust-exit behavior, unresolved-marker rejection, and staging semantics.
- ✅ Mergetool GUI selection and path override support implemented:
  - `merge.guitool` + `mergetool.guiDefault` precedence logic
  - `mergetool.<tool>.path` executable override (when `.cmd` is not set)
  - unit + integration test coverage added
- ✅ `mergetool.writeToTemp` parity implemented in `crates/gitgpui-git-gix/src/repo/mergetool.rs` with Git-like stage-file naming for both modes:
  - `writeToTemp=true`: absolute temp files under `gitgpui-mergetool-*`
  - `writeToTemp=false`: workdir-prefixed paths (`./...`) with `<base>_{BASE,LOCAL,REMOTE}_<pid><ext>` naming
  - stage file cleanup for workdir mode and unit/integration coverage
- 🔧 Git behavior parity matrix coverage is partial. Implemented/covered: spaced paths, no-base handling for stage extraction (including empty `BASE` file for add/add), trust-exit semantics, deleted output handling, and writeToTemp path semantics. Remaining explicit coverage: symlink, submodule conflict invocation paths, CRLF preservation assertions, dir-diff mode, cancel/close exit semantics.
- 🔧 Git-like scenario porting is partial. Existing and new tests cover a subset of t7610-style behavior (`trustExitCode`, custom cmd with braced env, gui preference, writeToTemp, no-base stage-file contract); `--tool-help`, full gui-default parity flow, order-file, delete/delete interaction prompts, and submodule-specific flows remain.
- 🔧 Dedicated difftool mode tests are partially implemented:
  - ✅ Runtime/unit coverage added in `crates/gitgpui-app/src/difftool_mode.rs` (identical files, changed files with exit normalization, display-path and explicit labels, missing-input error handling, directory diff).
  - ⬜ Full git-invoked integration tests (`git difftool` end-to-end) are still pending.
- ⬜ End-to-end tests that invoke `git difftool`/`git mergetool` with global-like config and `gitgpui-app` as the tool are not implemented yet.
- ✅ KDiff3-style fixture harness implemented in `crates/gitgpui-core/tests/merge_fixture_harness.rs` with fixture data in `crates/gitgpui-core/tests/fixtures/merge/`. Auto-discovers `*_base.*` fixtures, runs merge algorithm, validates invariants (marker well-formedness, content integrity, context preservation), and compares against expected results. 7 seed fixtures + harness discovery test = 8 tests.
- ✅ Generated permutation corpus integration (Phase 3A) added in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`: ports KDiff3’s 11-option line-state table, runs deterministic sampled corpus (`r=3`, `seed=0`, 243 cases) in default test runs, and includes an ignored exhaustive run (11^5 = 161,051 cases).

### Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)

- ✅ Phase 1A (git `t6403` algorithm-focused cases): 3-way merge algorithm implemented in `crates/gitgpui-core/src/merge.rs` with 22 unit tests. Integration test suite `crates/gitgpui-core/tests/merge_algorithm.rs` ports 18 t6403-style test cases covering: identity merge, non-overlapping clean merge, overlapping conflict detection, conflict marker format with labels, delete-vs-modify, ours/theirs/union strategies, EOF trailing newline preservation, CRLF marker handling, configurable marker width, diff3 output, Myers C-code merge, binary content, identical-change dedup, and single-side-only changes.
- ✅ Phase 1B (git `t6427` `zdiff3` 4-case portability set): all 4 zdiff3 test cases ported (`zdiff3_basic`, `zdiff3_middle_common`, `zdiff3_interesting`, `zdiff3_evil`). Tests verify common prefix/suffix extraction outside conflict markers and correct inner conflict content.
- ✅ Phase 1C (conflict marker label formatting cases): implemented in `crates/gitgpui-core/src/conflict_labels.rs` with portability tests in `crates/gitgpui-core/tests/conflict_label_formatting.rs`:
  - `label_no_base` -> `empty tree`
  - `label_unique_base` -> `<short-sha>:<path>`
  - `label_unique_base_rename` -> `<short-sha>:<original-path>`
  - `label_merged_ancestors` -> `merged common ancestors:<path>`
  - `label_rebase_parent` -> `parent of <desc>`
- ✅ Phase 2 (KDiff3 fixture harness with `*_base/*_contrib/*_expected_result` discovery + invariants):
  - 2A: Fixture format adopted — `tests/fixtures/merge/{prefix}_{base,contrib1,contrib2,expected_result}.txt`
  - 2B: Test runner in `tests/merge_fixture_harness.rs` — auto-discovers `*_base.*`, loads triplets, runs `merge_file`, validates 3 algorithm-independent invariants (marker well-formedness, content integrity, context preservation), compares against expected output, writes `*_actual_result.*` on mismatch
  - 2C: 7 seed test cases ported: simpletest (KDiff3), prefer-identical (KDiff3), nonoverlapping changes, overlapping conflict, identical changes, delete-vs-modify, add/add conflict
- ✅ Phase 3A (permutation corpus generation): implemented in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`.
  - Ports KDiff3 permutation option table from `generate_testdata_from_permutations.py`.
  - Generates corpus at test time (no fixture file bloat).
  - Default run executes deterministic sampled corpus (`r=3`, `seed=0`) for 243 cases.
  - Includes ignored exhaustive corpus test for all 161,051 cases.
  - Validates generated outputs with marker well-formedness, content integrity, context preservation, and reported conflict-count parity.
- ✅ Phase 3C (real-world merge extraction harness): implemented in `crates/gitgpui-core/tests/merge_git_extraction.rs`.
  - Ports KDiff3's `generate_testdata_from_git_merges.py` concept to Rust.
  - Walks merge commits via `git rev-list --merges --parents`, finds merge-base, extracts base/contrib1/contrib2 file contents.
  - Skips trivial merges (base == either contrib, or contribs identical) and binary files (non-UTF-8).
  - Validates algorithm-independent invariants (marker well-formedness, content integrity) on each extracted case.
  - Default test runs against gitgpui's own repo; ignored test supports arbitrary external repos via `GITGPUI_MERGE_EXTRACTION_REPO` env var.
  - Includes fixture file generation (`write_fixtures`) compatible with the existing Phase 2 fixture harness format.
  - 8 tests (+ 2 ignored): discovery, trivial skip, nontrivial conflict, clean merge, binary skip, fixture writing, multifile merge, self-repo regression.
- 🔧 Phase 4A (critical `t7610-mergetool` E2E): partially implemented in `gitgpui-git-gix` tests:
  - ✅ trust-exit behavior and content-change semantics
  - ✅ custom command invocation and braced env variables
  - ✅ gui tool preference path via `merge.guitool` + `mergetool.guiDefault=true`
  - ✅ tool path override via `mergetool.<tool>.path`
  - ✅ writeToTemp stage-file path behavior (`true` temp paths, `false` `./`-prefixed workdir paths)
  - ✅ no-base file contract in add/add conflicts (tool receives an empty `BASE` file)
  - ⬜ remaining cases (tool-help, nonexistent tool messaging parity, orderFile/delete-delete prompt flow/submodule matrix, and full E2E via `git mergetool` command) still pending
- 🔧 Phase 4B (critical `t7800-difftool` E2E): partially implemented.
  - ✅ Foundational difftool runtime added in `gitgpui-app` (`difftool_mode.rs`) with Git-compatible exit semantics and label/display-path handling.
  - ✅ Targeted difftool runtime tests added (unit-level behavior parity for changed/unchanged files, label handling, directory diff, and error path).
  - ⬜ Remaining: full `git difftool` command E2E parity scenarios (`guiDefault`, subdirectory invocation, trust-exit config interactions, and global-like config wiring).

### Latest Component Delivered (Iteration 7)

- Implemented foundational `difftool` runtime path in `gitgpui-app`:
  - Added `crates/gitgpui-app/src/difftool_mode.rs` with `run_difftool()` that executes `git diff --no-index -- <LOCAL> <REMOTE>`.
  - Added Git-compatible exit mapping: diff-present (`git` exit `1`) is normalized to app success (`0`), while operational failures return app error semantics.
  - Added header label support for `--label-left`, `--label-right`, and `--path`/display name by rewriting unified-diff file headers (`---` / `+++`) deterministically.
  - Wired `AppMode::Difftool` in `crates/gitgpui-app/src/main.rs` to this runtime path (removed previous not-implemented hard error for difftool mode).
  - Added 7 unit tests covering unchanged/changed files, label behavior, directory diff mode, and missing-input error handling.
- ✅ Phase 5A/5B/5C (Meld-derived matcher/interval/newline test ports): implemented in `crates/gitgpui-core/src/text_utils.rs` with tests in `crates/gitgpui-core/tests/meld_algorithm_tests.rs`:
  - 5A: Myers matching blocks extraction (`matching_blocks_chars`, `matching_blocks_lines`) with 8 tests (4 ported from Meld's `test_matchers.py` inputs + 4 line-level tests). Sync point tests noted as Meld-specific (not applicable to our standard Myers engine).
  - 5B: Interval merging (`merge_intervals`) with 8 tests (6 ported from Meld's `test_misc.py` + 2 edge cases).
  - 5C: Newline-aware text operations (`delete_last_line`) with 12 tests (7 ported from Meld's `test_chunk_actions.py` + 5 edge cases).

### Latest Component Delivered (Iteration 6)

- Implemented Phase 3C real-world merge extraction harness:
  - Added `crates/gitgpui-core/tests/merge_git_extraction.rs` porting KDiff3's `generate_testdata_from_git_merges.py` concept to Rust.
  - **Merge commit discovery**: walks `git rev-list --merges --parents HEAD` to find standard 2-parent merge commits (skips octopus merges).
  - **File extraction**: for each merge, finds merge-base via `git merge-base`, identifies files changed in both parents, extracts file contents at base/contrib1/contrib2 via `git show`.
  - **Trivial skip**: filters out cases where base == either contrib or contribs are identical.
  - **Binary skip**: filters out non-UTF-8 files that the text merge algorithm cannot process.
  - **Invariant validation**: runs `merge_file()` on each extracted case and validates marker well-formedness + content integrity.
  - **Fixture generation**: `write_fixtures()` writes extracted cases to disk in the Phase 2 fixture harness format (`{sha}_{path}_{base,contrib1,contrib2,expected_result}.txt`).
  - **Self-repo regression**: default test runs against gitgpui's own repo; ignored tests support external repos via `GITGPUI_MERGE_EXTRACTION_REPO` env var (e.g., linux kernel).
  - 8 tests passing + 2 ignored: `extraction_discovers_merge_commits`, `extraction_skips_trivial_merges`, `extraction_finds_nontrivial_conflict`, `extraction_handles_clean_merge`, `extraction_skips_binary_files`, `extraction_writes_fixture_files`, `extraction_handles_multifile_merge`, `extraction_regression_on_gitgpui_repo`.

### Iteration 5 Component Delivered

- Implemented Phase 5A/5B/5C Meld-derived algorithm tests and utilities:
  - Added `crates/gitgpui-core/src/text_utils.rs` with three utility groups:
    1. **Matching block extraction** (`matching_blocks_chars`, `matching_blocks_lines`): converts Myers diff edit scripts into `MatchingBlock` tuples `(a_start, b_start, length)` for both character-level and line-level sequences.
    2. **Interval merging** (`merge_intervals`): coalesces overlapping/adjacent `(start, end)` intervals into non-overlapping sorted output.
    3. **Newline-aware line deletion** (`delete_last_line`): removes the last line respecting `\n`, `\r\n`, and `\r` line endings.
  - Added `crates/gitgpui-core/tests/meld_algorithm_tests.rs` with 28 tests:
    - 5A (8 tests): 4 character-level matching block tests ported from Meld's `test_matchers.py` (basic, postprocess, inline, no-sync-points) with invariant verification (valid content, ordering, non-overlapping) + 4 line-level matching block tests.
    - 5B (8 tests): 6 interval merging tests ported from Meld's `test_misc.py` (dominated, disjoint, two-groups, unsorted, duplicate, chain) + 2 edge cases.
    - 5C (12 tests): 7 newline-aware deletion tests ported from Meld's `test_chunk_actions.py` (CRLF, LF, CR, trailing, mixed) + 5 edge cases.
  - Note: Meld's `sync_point_one` and `sync_point_two` tests exercise Meld-specific sync point alignment constraints not present in our standard Myers engine; these are documented but not ported.

### Iteration 4 Component Delivered

- Implemented Phase 3A generated permutation corpus regression coverage:
  - Added `crates/gitgpui-core/tests/merge_permutation_corpus.rs`.
  - Ported the KDiff3 11-option line-state permutation model used by `generate_testdata_from_permutations.py`.
  - Added deterministic sampled corpus execution (`r=3`, `seed=0`) and validation across 243 generated cases.
  - Added ignored exhaustive run for full 11^5 coverage (161,051 generated cases) to support deep local/CI sweeps when desired.
  - Added algorithm-independent validation checks per generated case:
    1. Conflict marker well-formedness (balanced and ordered markers; no nesting)
    2. Content integrity (every non-marker output line comes from base/local/remote inputs)
    3. Context preservation (lines common to all three inputs remain present)
    4. `conflict_count` parity with emitted `<<<<<<<` marker blocks

### Iteration 3 Component Delivered

- Implemented Phase 2 KDiff3-style fixture harness for merge algorithm regression testing:
  - Created `crates/gitgpui-core/tests/fixtures/merge/` directory with KDiff3 naming convention (`{prefix}_base.txt`, `{prefix}_contrib1.txt`, `{prefix}_contrib2.txt`, `{prefix}_expected_result.txt`).
  - Built auto-discovery test runner in `crates/gitgpui-core/tests/merge_fixture_harness.rs` that scans for `*_base.*` files, loads all triplets, runs `merge_file`, and validates three algorithm-independent invariants:
    1. Conflict marker well-formedness (balanced `<<<<<<<`/`=======`/`>>>>>>>`, proper ordering, no nesting)
    2. Content integrity (every non-marker output line traceable to base, contrib1, or contrib2)
    3. Context preservation (lines common to all three inputs appear in output)
  - On mismatch, writes `*_actual_result.*` for manual comparison.
  - Ported 7 seed fixtures: 2 from KDiff3 (`1_simpletest`, `2_prefer_identical`) + 5 additional merge scenarios (`3_nonoverlapping_changes`, `4_overlapping_conflict`, `5_identical_changes`, `6_delete_vs_modify`, `7_add_add_conflict`).
  - Total: 8 new tests (7 individual fixtures + 1 harness discovery test).

### Iteration 2 Component Delivered

- Implemented Phase 1C conflict marker label formatting support in `gitgpui-core`:
  - Added `BaseLabelScenario` model + formatter API (`format_base_label`) in `crates/gitgpui-core/src/conflict_labels.rs`.
  - Added deterministic short-SHA formatting (`7` chars by default) and git-path normalization.
  - Added 5 portability tests in `crates/gitgpui-core/tests/conflict_label_formatting.rs`.

### Iteration 1 Component Delivered

- Implemented standalone 3-way merge-file algorithm in `crates/gitgpui-core/src/merge.rs`:
  - Full `merge_file(base, ours, theirs, options) -> MergeResult` public API.
  - Myers diff-based hunk detection with overlapping-region expansion.
  - Three conflict styles: `Merge` (2-section), `Diff3` (3-section with base), `Zdiff3` (with common prefix/suffix extraction).
  - Four merge strategies: `Normal` (markers), `Ours`, `Theirs`, `Union`.
  - Configurable marker size, per-side labels, CRLF-aware marker emission.
  - Trailing newline preservation matching git semantics.
  - 22 unit tests + 30 integration tests (total: 52 new merge tests).
- Ported t6403 and t6427 test suites in `crates/gitgpui-core/tests/merge_algorithm.rs`.

### Earlier Components Delivered

- Implemented foundational mergetool selection and executable resolution parity improvements in `crates/gitgpui-git-gix/src/repo/mergetool.rs`:
  - Added `mergetool.guiDefault` parsing (`true`/`false`/`auto`) with deterministic tool selection.
  - Added `merge.guitool` preference when GUI-default resolution requires it.
  - Added `mergetool.<tool>.path` support for non-`cmd` tool invocation.
  - Added targeted unit tests and integration tests in `status_integration.rs`.
