## STATUS: COMPLETE

All components from both design documents are fully implemented. Iteration 50 maintains completeness; the last full-suite verification (Iteration 48) passed 453+ tests, Iteration 49 added one difftool E2E case (22 difftool git-invoked E2E tests total), and Iteration 50 adds standalone behavior-matrix hardening for no-base mergetool and directory difftool direct-invocation paths.

## Implementation Progress

### Progress Snapshot (Iteration 50)

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ Dedicated CLI modes (`difftool`, `mergetool`) and arg/env validation are implemented.
- ✅ KDiff3-style compatibility fallback is implemented for no-subcommand invocations (positional args + `--L1/--L2/--L3` + `--base` + `-o/--output/--out` + optional `--auto`), enabling Git built-in tool flows that invoke the binary directly via `*.path`.
- ✅ Meld-style compatibility fallback is implemented for no-subcommand invocations (`--output`/`-o` + optional `--auto-merge` with `LOCAL BASE REMOTE` positional ordering), enabling Git built-in `meld` path-override flows (`mergetool.meld.path`) to invoke GitGpui directly.
- ✅ Meld-style pane labels are now supported in no-subcommand compatibility invocations via `-L` / `--label` (including attached-value forms like `-LLEFT` / `--label=LEFT`) for both diff and merge flows.
- ✅ Strict compatibility validation is implemented for no-subcommand invocation: invalid combinations now fail fast with actionable errors (`--auto` requires output path, merge mode positional count validation, merge-mode `--L3`-without-`BASE` rejection in 2-path mode, `--base` ambiguity guards, diff-mode `--L3`/`--base` rejection, and too-many-positional guards).
- ✅ CLI empty-path hardening is implemented: required `LOCAL`/`REMOTE`/`MERGED` inputs now reject empty path values with actionable parse-time errors, optional display-path env vars ignore empty strings, and empty `BASE` from env is normalized to no-base while explicit empty `--base` is rejected.
- ✅ Mergetool path-kind validation is hardened: `LOCAL`, `REMOTE`, and explicit `BASE` now reject directories (must be files), and existing-directory `MERGED` targets fail fast with actionable parse-time errors.
- ✅ Difftool env compatibility is complete: display-path resolution now honors optional `MERGED` and `BASE` compatibility vars with explicit precedence (`--path` > `MERGED` > `BASE`).
- ✅ Mergetool CLI compatibility aliases are implemented: `-o`/`--output`/`--out` for output path and `--L1`/`--L2`/`--L3` for labels (KDiff3/Meld-style command compatibility).
- ✅ Dedicated mergetool marker-width control is now exposed end-to-end via `--marker-size <N>` with strict validation (`N > 0`), runtime propagation into merge options, and standalone/CLI regression coverage.
- ✅ Standalone mergetool output-target behavior is implemented: `MERGED` may be a new path, and runtime creates parent directories before writing.
- ✅ Focused difftool/mergetool runtimes are implemented with Git-compatible exit semantics.
- ✅ Standalone binary-level exit contract is now explicitly covered by direct `gitgpui-app` E2E tests (`difftool`/`mergetool` success, unresolved conflict, and invalid-input paths mapped to exit codes `0/1/2`).
- ✅ Git-invoked E2E coverage exists for `git difftool` and `git mergetool` parity scenarios (GUI selection, trust-exit handling, spaced/unicode paths, subdir invocation, `--tool-help`, symlink/submodule/delete-modify edge cases, order-file behavior, explicit `mergetool.writeToTemp` path-shape parity, and binary file conflict handling).
- ✅ GUI-default parity matrix is now explicit for both Git entry points: `difftool.guiDefault` and `mergetool.guiDefault` are covered for `auto`, `true`, and `false`, plus `--gui` / `--no-gui` overrides.
- ✅ Pathspec flow parity is now explicit in git-invoked E2E tests: pathspec-targeted `git difftool` only diffs selected paths, and pathspec-targeted `git mergetool` resolves only selected conflicts while leaving non-selected conflicts unresolved.
- ✅ Explicit `t7610` custom-command parity is now covered in git-invoked mergetool tests: `cat "$REMOTE" > "$MERGED"` resolves conflicts, writes expected output, and clears unmerged index entries.
- ✅ Git-invoked add/add no-base parity is now explicit: dedicated mergetool E2E coverage asserts `$BASE` is passed as an existing empty stage file for no-base conflicts.
- ✅ Difftool binary/non-UTF8 behavior-matrix coverage is now explicit in both dedicated runtime tests and `git difftool` E2E tests.
- ✅ Difftool submodule behavior-matrix coverage is now explicit in git-invoked E2E tests: submodule gitlink-only changes are diffed via temporary `Subproject commit <sha>` files, and output includes both old/new commit pointers.
- ✅ Automated git config setup: `gitgpui-app setup` subcommand writes all recommended git config entries (merge.tool, diff.tool, mergetool.gitgpui.cmd, difftool.gitgpui.cmd, `mergetool.trustExitCode`, `mergetool.gitgpui.trustExitCode`, `difftool.trustExitCode`, `difftool.gitgpui.trustExitCode`, prompt suppression, GUI tool aliases, guiDefault=auto). Both mergetool and difftool sides now have symmetric generic + per-tool trust keys. Supports `--dry-run` (print commands without executing) and `--local` (repo-scoped instead of global). Dry-run output is shell-runnable with robust quoting for nested command values and literal `$BASE/$LOCAL/$REMOTE/$MERGED` placeholders. Covered by unit tests and standalone setup integration tests.
- ✅ Dedicated mergetool conflict-marker labels now have Git-style runtime fallback semantics: missing labels default to input filenames, and no-base diff3/zdiff3 base labels default to `empty tree` (with focused unit coverage).
- ✅ Automatic git config fallback: mergetool reads `merge.conflictstyle` and `diff.algorithm` from git config when no CLI flag is provided, mirroring `git merge-file` behavior. CLI flags take priority over git config, and git config takes priority over defaults. Unknown config values are gracefully ignored. Iteration 37 extends this parity to no-subcommand compatibility invocations (`kdiff3`-style `--auto/-o/--L*`), which previously bypassed fallback.
- ✅ Delete/delete conflict choice matrix parity is now explicit in git-invoked tests (`d` delete, `m` modified destination, `a` abort non-zero) for path-targeted mergetool flows.
- ✅ Deleted-vs-modified submodule choice parity is now explicit in git-invoked tests: path-targeted `git mergetool submod` assertions now verify `r` keeps the modified submodule gitlink commit and `l` keeps deletion (no gitlink/path), matching the t7610 contract.
- ✅ Submodule abort/cancel parity is now explicit in git-invoked tests: path-targeted `git mergetool` submodule conflicts now assert `a` aborts with non-zero exit and leaves the conflict unresolved.
- ✅ Parity-focused CI regression gates implemented in `.github/workflows/rust.yml` (Phase 3, rollout item #2): separate CI jobs for clippy, merge algorithm parity, fixture/corpus regression, git mergetool/difftool E2E (including standalone tool-mode exit-code tests), and backend integration.
- ✅ Mergetool backend parity features are implemented (`mergetool.<tool>.path`, `writeToTemp`, `keepTemporaries`, unresolved-marker rejection, deleted-output staging).
- ✅ `keepTemporaries=true` abort-path parity is now explicit in backend integration coverage (external tool exit non-zero keeps stage files in both workdir and temp modes).
- ✅ Git built-in path-override E2E coverage added for `kdiff3` and `meld` mergetool flows plus both `kdiff3` and `meld` difftool flows to validate direct executable invocation compatibility.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A implemented: core 3-way merge algorithm + t6403 portability set (including histogram and binary-reject paths).
- ✅ Phase 1B implemented: all 4 t6427 `zdiff3` portability cases.
- ✅ Phase 1C implemented: conflict marker label formatting portability cases.
- ✅ Phase 1C runtime portability hardening: dedicated mergetool mode now applies default marker-label fallbacks (filename defaults + `empty tree` no-base base label) with regression tests.
- ✅ Phase 2 implemented: fixture harness supports both merged-output goldens and KDiff3-style alignment index triples, with auto-discovery, expected-result comparison, and invariants for both formats (merge-output marker/content/context checks + alignment monotonicity/consistency checks).
- ✅ Phase 3A implemented: generated permutation corpus test runner (sampled + ignored exhaustive mode).
- ✅ Phase 3B implemented: generated permutation corpus now enforces KDiff3-style alignment invariants (sequence monotonicity + content consistency) for every generated case.
- ✅ Phase 3C implemented: real-world merge extraction harness from Git history.
- ✅ Phase 3C portability hardening implemented: extraction fixture tests now run with `commit.gpgsign=false` in helper git invocations, so the suite is stable on hosts with global commit-signing enabled.
- ✅ Phase 4A implemented: critical `t7610` mergetool E2E scenarios, including `trustExitCode=false` unchanged-output and changed-output behavior, plus explicit no-base add/add stage-file assertions in git-invoked coverage.
- ✅ Phase 4A deleted-vs-modified submodule hardening: explicit `l`/`r` resolution assertions for path-targeted `git mergetool submod` flows now validate kept-module vs kept-deletion outcomes.
- ✅ Phase 4A submodule abort hardening: explicit `a` abort-path assertion for submodule conflicts now validates non-zero exit and unresolved-index preservation.
- ✅ Phase 4B implemented: critical `t7800` difftool E2E scenarios.
- ✅ Phase 5 implemented: Meld-derived matcher/interval/newline portability suites.
- ✅ Phase 5A exact portability parity is now implemented for Meld matching blocks: `myers_basic`, `myers_postprocess`, and `myers_inline_trigram` now assert exact tuple outputs via a Meld-style matcher pipeline in `crates/gitgpui-core/src/text_utils.rs` (prefix/suffix trimming, optional discard preprocessing, postprocess cleanup, and inline trigram mode).
- ✅ Phase 1A marker-size portability is now wired through dedicated mergetool command mode (`--marker-size`), not only the core merge API tests.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### External Diff/Merge Usage Design (`external_usage.md`)

- ✅ CLI subcommands and argument model (`gitgpui-app difftool`, `gitgpui-app mergetool`) implemented in `crates/gitgpui-app/src/cli.rs`.
- ✅ Arg/env resolution + validation implemented for `LOCAL`, `REMOTE`, `MERGED`, `BASE`, labels, missing-input and missing-path errors. `MERGED` is treated as an output target and can be non-existent at parse time. Difftool display-path fallback now follows `--path` > `MERGED` > `BASE`.
- ✅ Arg/env validation hardening for empty values implemented in `crates/gitgpui-app/src/cli.rs`: required paths reject empty inputs early, optional display-path env vars ignore empty strings, env-provided empty `BASE` is treated as no-base (add/add-compatible), and explicit empty `--base` errors with actionable text.
- ✅ Mergetool file-path kind validation hardening implemented in `crates/gitgpui-app/src/cli.rs`: `LOCAL`/`REMOTE`/explicit `BASE` must resolve to files (not directories), and existing directory `MERGED` targets are rejected up front with actionable errors.
- ✅ Mergetool compatibility aliases implemented in `crates/gitgpui-app/src/cli.rs`:
  - `-o`/`--output`/`--out` as aliases for `--merged`
  - `--L1`/`--L2`/`--L3` as aliases for `--label-base`/`--label-local`/`--label-remote`
  - coverage: parser unit tests + git-invoked integration test (`git_mergetool_accepts_kdiff3_alias_flags_in_cmd`)
- ✅ KDiff3/Meld-style no-subcommand compatibility parser implemented in `crates/gitgpui-app/src/cli.rs`:
  - accepts direct external-tool invocation with positional paths and compatibility flags (`--auto`, `--auto-merge`, `--L1/--L2/--L3`, Meld-style `-L/--label`, `--base`, `-o/--output/--out`)
  - maps to validated `difftool`/`mergetool` app modes
  - enforces strict compatibility validation with actionable errors (`--auto`/`--auto-merge` output-path requirements, merge positional count checks, merge-mode `--L3` requires `BASE` in compatibility mode, `--base` conflict guards, diff-mode `--L3`/`--base` rejection, too-many-positional checks, and label-arity overflow rejection)
  - coverage: CLI parser tests (including Meld `-L/--label` diff+merge cases and over-arity rejection) + git-invoked `kdiff3`/`meld` path-override integration tests
- ✅ Exit code constants aligned to design (`0`, `1`, `>=2`) defined in app CLI module.
- ✅ Conflict-marker label formatter and runtime integration implemented: `crates/gitgpui-core/src/conflict_labels.rs` provides `empty tree`/`<short-sha>:<path>`/merged-ancestors/rebase-parent formatting, and `crates/gitgpui-app/src/mergetool_mode.rs` now applies filename/`empty tree` fallback labels in dedicated mergetool flows.
- ✅ Focused command-mode execution paths fully implemented:
  - ✅ `difftool` mode executes a dedicated runtime path in `crates/gitgpui-app/src/difftool_mode.rs` (delegates to `git diff --no-index --no-ext-diff`, strips recursive `GIT_EXTERNAL_DIFF` env, supports labels/display-path headers, and maps git exit `1`/diff-present to app success exit `0`).
  - ✅ `mergetool` mode executes a dedicated runtime path in `crates/gitgpui-app/src/mergetool_mode.rs` using the built-in 3-way merge algorithm (`merge_file_bytes`). Reads base/local/remote files, performs automatic merge, writes result to MERGED path (creating parent directories as needed). Exits 0 on clean merge, 1 on unresolved conflicts. Supports labels (including default filename fallbacks and `empty tree` no-base diff3/zdiff3 base label fallback), no-base (add/add) scenarios, byte-level binary file detection (null-byte and non-UTF-8 detection) with base-aware auto-resolution heuristics (clean when sides are identical or one side matches base; conflict fallback keeps local bytes), CRLF preservation, paths with spaces, configurable conflict style (`--conflict-style merge|diff3|zdiff3`), diff algorithm selection (`--diff-algorithm myers|histogram`), and marker width control (`--marker-size <N>`). 35 unit tests.
- ✅ External mergetool backend launch exists (`launch_mergetool`) with stage materialization (`BASE/LOCAL/REMOTE`), trust-exit behavior, unresolved-marker rejection, and staging semantics.
- ✅ Mergetool GUI selection and path override support implemented:
  - `merge.guitool` + `mergetool.guiDefault` precedence logic
  - `mergetool.<tool>.path` executable override (when `.cmd` is not set)
  - unit + integration test coverage added
- ✅ `mergetool.writeToTemp` parity implemented in `crates/gitgpui-git-gix/src/repo/mergetool.rs` with Git-like stage-file naming for both modes:
  - `writeToTemp=true`: absolute temp files under `gitgpui-mergetool-*`
  - `writeToTemp=false`: workdir-prefixed paths (`./...`) with `<base>_{BASE,LOCAL,REMOTE}_<pid><ext>` naming
  - stage file cleanup for workdir mode and unit/integration coverage
- ✅ `mergetool.keepTemporaries` parity implemented in `crates/gitgpui-git-gix/src/repo/mergetool.rs`:
  - reads `mergetool.keepTemporaries` from git config
  - keeps stage files when enabled for both `writeToTemp=true` and `writeToTemp=false`
  - preserves default cleanup behavior when disabled
  - covered by unit tests in `repo/mergetool.rs` and integration tests in `tests/status_integration.rs`
  - abort-path retention is now explicitly tested with non-zero external tool exits:
    - `launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files_on_abort`
    - `launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files_on_abort`
- ✅ `mergetool.keepBackup` delete/delete parity scenario covered by dedicated git-invoked E2E assertion (`git_mergetool_keep_backup_delete_delete_no_errors`).
- ✅ `mergetool.keepTemporaries=true` delete/delete abort parity is now explicit in git-invoked E2E coverage (`git_mergetool_keep_temporaries_delete_delete_abort_keeps_stage_files`), matching `t7610` expectations for preserved stage temp files.
- ✅ Git behavior parity matrix coverage is complete. All items covered: spaced and Unicode paths, invocation from subdirectories, pathspec-targeted invocations, no-base handling for stage extraction (including empty `BASE` file for add/add), binary/non-UTF8 content handling in both difftool and mergetool flows, trust-exit semantics, deleted output handling, writeToTemp path semantics, difftool `--dir-diff`, difftool `guiDefault` selection (`true`/`false`/`auto` + `DISPLAY`, `--gui`, `--no-gui`), difftool `--tool-help` discoverability, mergetool `guiDefault` selection (`true`/`false`/`auto` + `DISPLAY`, `--gui`, `--no-gui`), mergetool `--tool-help` discoverability, mergetool GUI fallback (no guitool → merge.tool), nonexistent tool error handling, delete/delete conflict handling, modify/delete conflict handling, symlink conflict resolution (l/r/a prompts, coexistence with normal file conflicts, difftool target diff), and submodule conflict handling (l/r/a prompt semantics, coexistence with normal file conflicts, file-vs-submodule, directory-vs-submodule, deleted-vs-modified submodule, submodule in subdirectory).
- ✅ Git-like scenario porting is complete. All listed t7610/t7800 parity items are covered: `trustExitCode`, custom cmd (`cat "$REMOTE" > "$MERGED"` + braced env variants), gui preference, writeToTemp/keepTemporaries, keepBackup delete/delete, no-base stage-file contract, difftool gui-default/trust/tool-help parity, mergetool gui-default/trust/tool-help parity, GUI fallback, nonexistent tool error, delete/delete, modify/delete, pathspec-targeted runs, order-file invocation ordering (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), and submodule conflicts (l/r/a prompt semantics, deleted-vs-modified, file-vs-submodule, directory-vs-submodule, subdirectory submodule, coexistence with normal files).
- ✅ Dedicated difftool mode tests are implemented with parity-focused coverage:
  - ✅ Runtime/unit coverage in `crates/gitgpui-app/src/difftool_mode.rs` (identical files, changed files with exit normalization, display-path and explicit labels, missing-input error handling, directory diff, binary content, and non-UTF8 content).
  - ✅ Full git-invoked integration coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs` (basic invocation, spaced and Unicode paths, subdirectory invocation, pathspec filtering, `--dir-diff`, `guiDefault` true/false/auto + `--gui`/`--no-gui` selection precedence, trust-exit-code matrix, `--tool-help` discoverability, symlink target diff, submodule gitlink-pointer diff, binary content, and non-UTF8 content).
- ✅ End-to-end tests that invoke `git difftool`/`git mergetool` with global-like config and `gitgpui-app` as the tool are fully implemented:
  - ✅ `git difftool` E2E in `crates/gitgpui-app/tests/difftool_git_integration.rs` (22 tests, including pathspec filtering parity, explicit `difftool.guiDefault` true/false/auto matrix coverage, submodule gitlink-pointer diff coverage, and both `kdiff3`/`meld` path-override compatibility invocation).
  - ✅ `git mergetool` E2E in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (56 tests): overlapping conflict processing, explicit custom command parity (`cat "$REMOTE" > "$MERGED"`), trust-exit-code semantics (clean merge resolved / conflict preserved), no-trust exit behavior (unchanged output stays unresolved, changed output resolves), spaced and Unicode path handling, subdirectory invocation, pathspec-targeted invocation parity, add/add (no-base) conflict + explicit empty-`BASE` stage-file contract assertion, multiple conflicted files, CRLF preservation, `--tool-help` discoverability, `guiDefault` true/false/auto selection (with/without DISPLAY), `--gui` and `--no-gui` flag overrides, GUI fallback when no guitool configured, nonexistent tool error handling, delete/delete conflict, delete/delete with keepBackup=true (no-error parity), delete/delete abort with `keepTemporaries=true` stage-file retention parity, modify/delete conflict, explicit `mergetool.writeToTemp` `true`/`false` stage-path-shape assertions, invocation ordering parity (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), submodule conflicts (l/r/a prompt semantics, deleted-vs-modified, file-vs-submodule, directory-vs-submodule, subdirectory submodule, coexistence with normal files), and `kdiff3`/`meld` path-override compatibility invocation.
- ✅ Direct standalone command-mode E2E coverage for `gitgpui-app` subcommands is implemented in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - ✅ `mergetool` clean merge exits `0` and writes merged output
  - ✅ `mergetool` unresolved conflict exits `1` and writes conflict markers
  - ✅ `mergetool` invalid input exits `2` with actionable validation error text
  - ✅ `mergetool` rejects existing directory `MERGED` targets with exit `2` and actionable error text
  - ✅ `difftool` changed-file invocation exits `0` and emits unified diff output
  - ✅ `difftool` invalid input exits `2` with actionable validation error text
  - ✅ no-subcommand compatibility E2E for Meld-style `-L/--label` diff and merge invocations
- ✅ Standalone behavior-matrix hardening is explicit for direct command-mode invocation:
  - ✅ no-base mergetool clean add/add path (identical additions) exits `0` and writes clean output
  - ✅ no-base mergetool conflict path with `--conflict-style zdiff3` exits `1` and uses `||||||| empty tree` base label fallback
  - ✅ standalone `difftool` supports direct directory-diff invocation (directory inputs, exit `0`, file-level diff output)
- ✅ KDiff3-style fixture harness implemented in `crates/gitgpui-core/tests/merge_fixture_harness.rs` with fixture data in `crates/gitgpui-core/tests/fixtures/merge/`. Auto-discovers `*_base.*` fixtures, runs merge algorithm, validates invariants, and compares against expected results in two formats:
  - merged-output expected files: marker well-formedness, content integrity, context preservation
  - alignment-triple expected files: sequence monotonicity + equality consistency across aligned line indices
  - seed fixtures: 7 merged-output fixtures + 2 KDiff3-style alignment fixtures
- ✅ Generated permutation corpus integration (Phase 3A + 3B) added in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`: ports KDiff3’s 11-option line-state table, runs deterministic sampled corpus (`r=3`, `seed=0`, 243 cases) in default test runs, includes an ignored exhaustive run (11^5 = 161,051 cases), and now validates alignment monotonicity + cross-side content consistency invariants for each generated case.
- ✅ Iteration 12 hardening: dedicated mergetool runtime now routes through `merge_file_bytes` so binary detection matches the core portability contract (including embedded NUL-byte data), with regression coverage in `crates/gitgpui-app/src/mergetool_mode.rs`.

### Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)

- ✅ Phase 1A (git `t6403` algorithm-focused cases): 3-way merge algorithm implemented in `crates/gitgpui-core/src/merge.rs` with focused portability coverage in `crates/gitgpui-core/tests/merge_algorithm.rs`.
  - ✅ Added zealous conflict coalescing in core merge flow (`coalesce_zealous_conflicts`) for adjacent conflicts and blank-only separators.
  - ✅ Added portability tests: `t6403_merge_zealous_coalesces_adjacent_conflict_lines`, `t6403_merge_zealous_alnum_coalesces_across_blank_separator`, and non-blank separator guard.
  - ✅ Existing t6403-style coverage remains for identity/non-overlap/conflicts, conflict markers + labels, delete-vs-modify, ours/theirs/union, EOF/trailing-newline behavior, CRLF markers, marker width, diff3 output, Myers C-code case, identical changes, and single-side-only changes.
  - ✅ `merge_histogram_clean` parity implemented: patience/histogram diff algorithm in `file_diff.rs` (`histogram_edits`), `DiffAlgorithm` enum in `MergeOptions`, and 4 portability tests demonstrating clean merge on C code that produces spurious conflicts with Myers.
  - ✅ Strict `merge_binary_rejected` contract implemented: `MergeError::BinaryContent` error type and `merge_file_bytes(&[u8], &[u8], &[u8], &MergeOptions) -> Result<MergeResult, MergeError>` entry point with null-byte and non-UTF-8 detection. 3 portability tests covering PNG rejection, null-byte-in-UTF-8 rejection, and text-API backward compatibility.
  - ✅ `merge_missing_lf_at_eof` parity implemented: improved trailing-newline handling that applies 3-way merge logic to the trailing LF based on which input contributed the output's last line. Handles git's `test_expect_failure` case (missing LF at EOF with non-overlapping changes) cleanly — an improvement over git's merge-file. 2 new tests: `t6403_merge_missing_lf_at_eof`, `t6403_merge_missing_lf_at_eof_away_from_change`.
- ✅ Phase 1B (git `t6427` `zdiff3` 4-case portability set): all 4 zdiff3 test cases ported (`zdiff3_basic`, `zdiff3_middle_common`, `zdiff3_interesting`, `zdiff3_evil`). Tests verify common prefix/suffix extraction outside conflict markers and correct inner conflict content.
- ✅ Phase 1C (conflict marker label formatting cases): implemented in `crates/gitgpui-core/src/conflict_labels.rs` with portability tests in `crates/gitgpui-core/tests/conflict_label_formatting.rs`:
  - `label_no_base` -> `empty tree`
  - `label_unique_base` -> `<short-sha>:<path>`
  - `label_unique_base_rename` -> `<short-sha>:<original-path>`
  - `label_merged_ancestors` -> `merged common ancestors:<path>`
  - `label_rebase_parent` -> `parent of <desc>`
- ✅ Phase 2 (KDiff3 fixture harness with `*_base/*_contrib/*_expected_result` discovery + invariants):
  - 2A: Fixture format adopted — `tests/fixtures/merge/{prefix}_{base,contrib1,contrib2,expected_result}.txt`
  - 2B: Test runner in `tests/merge_fixture_harness.rs` — auto-discovers `*_base.*`, loads triplets, runs `merge_file`, supports dual expected-result modes, and writes `*_actual_result.*` on mismatch:
    - merged-output mode validates marker well-formedness, content integrity, and context preservation
    - alignment-triple mode validates sequence monotonicity and aligned-content consistency
  - 2C: 9 seed test cases ported:
    - merged-output fixtures: simpletest (KDiff3), prefer-identical (KDiff3), nonoverlapping changes, overlapping conflict, identical changes, delete-vs-modify, add/add conflict
    - alignment fixtures: kdiff3_simple_alignment, kdiff3_prefer_identical_alignment
- ✅ Phase 3A (permutation corpus generation): implemented in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`.
  - Ports KDiff3 permutation option table from `generate_testdata_from_permutations.py`.
  - Generates corpus at test time (no fixture file bloat).
  - Default run executes deterministic sampled corpus (`r=3`, `seed=0`) for 243 cases.
  - Includes ignored exhaustive corpus test for all 161,051 cases.
  - Validates generated outputs with marker well-formedness, content integrity, context preservation, and reported conflict-count parity.
- ✅ Phase 3B (permutation invariants): implemented in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`.
  - Builds deterministic three-way alignments for each generated `(base, contrib1, contrib2)` case.
  - Enforces sequence monotonicity in each alignment column (`base`, `contrib1`, `contrib2`).
  - Enforces equality consistency for aligned base/contrib1, base/contrib2, and contrib1/contrib2 rows.
- ✅ Phase 3C (real-world merge extraction harness): implemented in `crates/gitgpui-core/tests/merge_git_extraction.rs`.
  - Ports KDiff3's `generate_testdata_from_git_merges.py` concept to Rust.
  - Walks merge commits via `git rev-list --merges --parents`, finds merge-base, extracts base/contrib1/contrib2 file contents.
  - Skips trivial merges (base == either contrib, or contribs identical) and binary files (non-UTF-8).
  - Validates algorithm-independent invariants (marker well-formedness, content integrity) on each extracted case.
  - Default test runs against gitgpui's own repo; ignored test supports arbitrary external repos via `GITGPUI_MERGE_EXTRACTION_REPO` env var.
  - Includes fixture file generation (`write_fixtures`) compatible with the existing Phase 2 fixture harness format.
  - Portability hardening: test helper git invocations force `-c commit.gpgsign=false`, avoiding environment-dependent failures when global git config enables signed commits.
  - 8 tests (+ 2 ignored): discovery, trivial skip, nontrivial conflict, clean merge, binary skip, fixture writing, multifile merge, self-repo regression.
- ✅ Phase 4A (critical `t7610-mergetool` E2E): fully implemented across `gitgpui-git-gix` tests and `gitgpui-app` E2E:
  - ✅ trust-exit behavior and content-change semantics
  - ✅ custom command invocation and braced env variables
  - ✅ gui tool preference path via `merge.guitool` + `mergetool.guiDefault=true`
  - ✅ explicit `mergetool.guiDefault=true` and `mergetool.guiDefault=false` baseline selection parity
  - ✅ tool path override via `mergetool.<tool>.path`
  - ✅ writeToTemp stage-file path behavior (`true` temp paths, `false` `./`-prefixed workdir paths)
  - ✅ no-base file contract in add/add conflicts (tool receives an empty `BASE` file)
  - ✅ `--tool-help` discoverability (gitgpui listed in `git mergetool --tool-help`)
  - ✅ `guiDefault=auto` with/without DISPLAY selects correct tool (CLI vs GUI)
  - ✅ `--gui` flag overrides `guiDefault=false` to select GUI tool
  - ✅ `--no-gui` flag overrides `guiDefault=true` to select CLI tool
  - ✅ GUI fallback: `--gui` with no `merge.guitool` falls back to `merge.tool`
  - ✅ nonexistent tool error: tool with invalid command reports failure
  - ✅ delete/delete conflict handling: both-deleted files resolved correctly
  - ✅ delete/delete path-targeted choice parity (`git mergetool a/a/file.txt`): `d` removes original path, `m` keeps modified destination (`b/b/file.txt`), `a` aborts with non-zero
  - ✅ modify/delete conflict handling: pipeline completes without crash
  - ✅ orderFile invocation order parity (`diff.orderFile` and CLI `-O...`) in `crates/gitgpui-app/tests/mergetool_git_integration.rs`
  - ✅ pathspec-targeted invocation parity: `git mergetool -- <path>` resolves only selected conflict paths, leaving non-selected unmerged entries intact
  - ✅ symlink conflict resolution: l/r prompt handling, coexistence with normal file conflicts
  - ✅ submodule conflict resolution: l/r/a prompt behavior, deleted-vs-modified, file-vs-submodule, directory-vs-submodule, subdirectory submodule, coexistence with normal files
  - ✅ `mergetool.keepTemporaries` stage-file retention semantics (`true` retains, default `false` cleans up) in backend launch path
  - ✅ `mergetool.keepTemporaries=true` delete/delete abort parity in git-invoked E2E (`git mergetool a/a/file.txt` with `a` keeps `file_{BASE,LOCAL,REMOTE}_<pid>.txt` stage files)
  - ✅ `mergetool.keepBackup=true` delete/delete E2E assertion: rename/rename conflict with keepBackup produces no stderr errors
  - ✅ difftool symlink target diff: `git difftool` shows diff between symlink targets
  - ✅ full E2E via `git mergetool` command in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (56 tests, including explicit add/add no-base stage-file assertions, `guiDefault` true/false/auto matrix coverage, pathspec filtering parity, binary file conflict handling, explicit custom-command parity, delete/delete `keepTemporaries` abort parity, submodule `a` abort-path parity, `kdiff3`/`meld` path-override compatibility invocation, and compatibility-mode git-config fallback parity)
  - ✅ full E2E via `git difftool` command in `crates/gitgpui-app/tests/difftool_git_integration.rs` (22 tests, including `guiDefault` true/false/auto matrix coverage, pathspec filtering parity, `kdiff3` + `meld` path-override compatibility invocation, submodule gitlink-pointer diff coverage, plus binary and non-UTF8 content coverage)
- ✅ Phase 4B (critical `t7800-difftool` E2E): implemented in `crates/gitgpui-app/tests/difftool_git_integration.rs`.
  - ✅ Foundational difftool runtime with Git-compatible exit semantics and label/display-path handling.
  - ✅ Git-invoked E2E coverage for basic invocation, subdirectory execution, pathspec filtering, spaced path handling, `--dir-diff`, binary content, and non-UTF8 content.
  - ✅ Explicit `difftool.guiDefault` selection-path parity (`true`/`false`/`auto` with/without `DISPLAY`, `--gui`, `--no-gui`).
  - ✅ Dedicated trust-exit interaction matrix assertions (`difftool.trustExitCode`, `--trust-exit-code`, `--no-trust-exit-code`).
- ✅ `git difftool --tool-help` discoverability assertion for configured `gitgpui` tool.

### Latest Component Delivered (Iteration 49) — Difftool Submodule Gitlink E2E Parity

- Added `git_difftool_shows_submodule_gitlink_change` in `crates/gitgpui-app/tests/difftool_git_integration.rs`.
- The new test builds a real submodule repo, advances the submodule commit, runs `git difftool -- submod`, and asserts GitGpui surfaces both old and new `Subproject commit <sha>` pointers.
- This closes explicit difftool-side submodule path coverage in the `external_usage.md` behavior matrix.
- Verification:
  - `cargo test --offline -p gitgpui-app --test difftool_git_integration git_difftool_shows_submodule_gitlink_change -- --nocapture`
  - `cargo test --offline -p gitgpui-app --test difftool_git_integration -- --nocapture`

### Previous Component Delivered (Iteration 48) — Meld Phase 5A Exact Matching-Block Parity

- Implemented Meld-style matching-block preprocessing/postprocessing pipeline in `crates/gitgpui-core/src/text_utils.rs`:
  - common prefix/suffix stripping before diff
  - discard preprocessing parity (`index_matching`) plus inline trigram mode parity (`index_matching_kmers`)
  - discarded-index remapping back to original coordinates
  - backward cleanup pass (`postprocess_blocks`) to reduce alignment chaff
- Added `matching_blocks_chars_inline(...)` for explicit inline trigram parity coverage.
- Hardened Meld portability tests in `crates/gitgpui-core/tests/meld_algorithm_tests.rs`:
  - `myers_matching_blocks_basic` now asserts exact tuples `[(0,2,3),(4,5,3),(10,8,2)]`
  - `myers_matching_blocks_postprocess` now asserts exact tuples `[(0,2,3),(4,6,3),(7,12,2)]`
  - `myers_matching_blocks_inline` now uses inline trigram matcher and asserts exact tuple `[(17,16,7)]`
- Verification:
  - `cargo test --offline -p gitgpui-core --test meld_algorithm_tests -- --nocapture`
  - `cargo test --offline -p gitgpui-core`

### Latest Verification (Iteration 49) — Difftool Submodule Coverage

- `cargo test --offline -p gitgpui-app --test difftool_git_integration git_difftool_shows_submodule_gitlink_change -- --nocapture` passed.
- `cargo test --offline -p gitgpui-app --test difftool_git_integration -- --nocapture` passed (22/22).

### Previous Verification (Iteration 48) — Full Completeness Confirmed

- Verified all design document items from both `external_usage.md` and `docs/REFERENCE_TEST_PORTABILITY.md` are implemented.
- Full test suite: 453+ tests pass, 0 failures, 0 clippy warnings, clean compilation.
  - 56 mergetool git-invoked E2E tests
  - 21 difftool git-invoked E2E tests
  - 13 standalone tool-mode E2E tests
  - 41 merge algorithm portability tests (t6403/t6427)
  - 11 fixture harness tests
  - 32 Meld-derived algorithm tests
  - 8 real-world merge extraction tests (+2 ignored opt-in)
  - 1 permutation corpus test (+1 ignored exhaustive)
  - 71 backend integration tests (status/conflict/submodule/upstream)
  - 194 unit tests
  - 5 conflict label formatting tests
- Fixed stale test count in progress documentation (55 → 56 mergetool E2E).

### Previous Component Delivered (Iteration 47) — Submodule Abort E2E Parity

- Added `git_mergetool_submodule_conflict_choice_a_aborts_with_nonzero` to harden Phase 4A submodule cancel/abort semantics.

### Previous Component Delivered (Iteration 46) — Vendored GPUI Line Wrapper Test Fix

- Fixed 3 failing tests in `crates/vendor/gpui/src/text_system/line_wrapper.rs`:
  - `test_truncate_line`: restored `"…"` ellipsis in second case (was `""` with stale expected output)
  - `test_truncate_multiple_runs`: restored `"…"` ellipsis in all cases (run length expectations assumed 3-byte suffix)
  - `test_update_run_after_truncation`: restored `"…"` ellipsis and result strings (expected lengths assumed suffix present)
- Root cause: tests were adapted from upstream Zed (`TruncateFrom::End` variant) but the `"…"` ellipsis character was stripped without updating expected values, creating impossible expectations (e.g., identical inputs yielding different outputs, run lengths exceeding result length)
- All 991 workspace tests now pass (0 failures)

### Previous Component Delivered (Iteration 45) — Binary 3-Way Auto-Resolution Hardening

- Implemented dedicated mergetool-mode binary resolution heuristics in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - clean auto-merge when `LOCAL == REMOTE` (binary-identical sides)
  - clean auto-merge when a real `BASE` exists and exactly one side changed (`LOCAL == BASE` -> choose `REMOTE`, `REMOTE == BASE` -> choose `LOCAL`)
  - conservative conflict fallback when both binary sides changed differently (keeps local bytes in `MERGED`, exits `1`)
  - preserves existing no-base behavior by not treating synthetic empty-base as a real ancestor for binary resolution
- Expanded dedicated runtime coverage in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - `binary_identical_sides_auto_merge_success`
  - `binary_with_base_local_matches_base_chooses_remote`
  - `binary_with_base_remote_matches_base_chooses_local`
  - `binary_without_base_does_not_treat_empty_side_as_unchanged`
  - `null_byte_content_with_single_side_change_auto_merges`
  - `null_byte_content_conflicting_changes_still_conflicts`
- Verification:
  - `cargo test -p gitgpui-app --tests -- --nocapture`

### Latest Component Delivered (Iteration 44) — Mergetool Directory-Path Validation Hardening

- Implemented parse-time mergetool path-kind validation in `crates/gitgpui-app/src/cli.rs`:
  - reject existing directory `MERGED` targets (`--merged` / `-o` / `--output` / `--out`)
  - reject directory-valued `LOCAL`, `REMOTE`, and explicit `BASE` inputs (must be files)
  - preserve existing no-base behavior and non-existent `MERGED` output-target support
- Added dedicated parser/unit coverage in `crates/gitgpui-app/src/cli.rs`:
  - `mergetool_existing_merged_directory_errors`
  - `mergetool_local_directory_errors`
  - `mergetool_remote_directory_errors`
  - `mergetool_base_directory_errors_when_explicitly_provided`
- Added standalone exit-contract E2E coverage in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - `standalone_mergetool_rejects_directory_merged_target_with_exit_two`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app cli::tests::mergetool_ -- --nocapture`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration standalone_mergetool_rejects_directory_merged_target_with_exit_two -- --nocapture`
  - `cargo test -p gitgpui-app --tests`

### Latest Component Delivered (Iteration 43) — GUI Default Boolean E2E Parity Matrix

- Added explicit boolean `guiDefault` matrix coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs`:
  - `git_difftool_gui_default_true_prefers_gui_tool_without_display`
  - `git_difftool_gui_default_false_prefers_cli_tool_with_display`
- Added explicit boolean `guiDefault` matrix coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_gui_default_true_prefers_gui_tool_without_display`
  - `git_mergetool_gui_default_false_prefers_cli_tool_with_display`
- This closes the remaining GUI-selection parity gap by validating `true`/`false` in addition to existing `auto` + `--gui`/`--no-gui` assertions.
- Verification:
  - `cargo test -p gitgpui-app --test difftool_git_integration gui_default_ -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration gui_default_ -- --nocapture`

### Latest Component Delivered (Iteration 42) — End-to-End Mergetool Marker-Width Parity

- Implemented dedicated mergetool marker-size support in `crates/gitgpui-app/src/cli.rs`:
  - added `--marker-size <N>` CLI flag for `gitgpui-app mergetool`
  - added strict validation (`N > 0`) with actionable parse error on `0`
  - wired validated value into `MergetoolConfig` as `marker_size`
- Wired runtime propagation in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - mergetool mode now forwards `marker_size` into `MergeOptions`
  - added unit coverage asserting 10-character conflict markers are emitted when configured
- Added standalone E2E coverage in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - `standalone_mergetool_marker_size_flag_controls_marker_width`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app -- --nocapture`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration -- --nocapture`

### Latest Component Delivered (Iteration 41) — No-Base Add/Add Git E2E Stage-File Parity

- Added explicit git-invoked no-base stage-file parity coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_add_add_provides_empty_base_stage_file`
  - creates an add/add conflict, runs `git mergetool`, records tool stage-file inputs, and asserts:
    - `$BASE` is a non-empty stage-file path
    - the recorded base stage-file size is `0` (`BASE_SIZE=0` contract for no-base add/add)
- This closes a remaining Phase 4A parity gap by validating no-base behavior in the real `git mergetool` invocation path (not only backend stage materialization tests).
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_add_add_provides_empty_base_stage_file -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration -- --nocapture`

### Latest Component Delivered (Iteration 40) — Meld `-L/--label` Compatibility Flags

- Implemented Meld-style pane-label parsing in no-subcommand external compatibility mode in `crates/gitgpui-app/src/cli.rs`:
  - supports `-L <label>`, `-L<label>`, `--label <label>`, and `--label=<label>`
  - maps labels in-order into existing compatibility slots while preserving KDiff3-vs-Meld positional disambiguation
  - adds explicit overflow validation (`>3` labels) with actionable error text
- Added parser regression coverage in `crates/gitgpui-app/src/cli.rs`:
  - `compat_parses_meld_style_difftool_short_labels`
  - `compat_parses_meld_style_mergetool_labels`
  - `compat_rejects_too_many_label_flags`
- Added standalone end-to-end coverage in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - `standalone_compat_difftool_accepts_meld_style_label_flags`
  - `standalone_compat_mergetool_meld_label_order_maps_to_local_base_remote`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app cli::tests::compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration standalone_compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --tests`

### Latest Component Delivered (Iteration 39) — Meld Difftool Path-Override E2E Parity

- Added `git difftool` built-in `meld` path-override integration coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs`:
  - `configure_meld_path_override_to_gitgpui` helper configures `diff.tool=meld` + `difftool.meld.path=<gitgpui-app>`
  - `git_difftool_meld_path_override_invokes_compat_mode` validates direct executable invocation through no-subcommand compatibility parsing and asserts emitted diff output
- Verification:
  - `cargo test -p gitgpui-app --test difftool_git_integration git_difftool_meld_path_override_invokes_compat_mode -- --nocapture`

### Latest Component Delivered (Iteration 38) — Meld Path-Override Compatibility Mode

- Implemented no-subcommand external-tool compatibility support for Meld-style mergetool invocation in `crates/gitgpui-app/src/cli.rs`:
  - accepts `--auto-merge`
  - supports `--output/-o/--out <MERGED> <LOCAL> <BASE> <REMOTE>` positional ordering
  - preserves KDiff3 parsing precedence (`--auto`/`--L*` keeps `BASE LOCAL REMOTE` semantics)
  - adds strict validation for `--auto-merge` without output target
- Added parser regression coverage in `crates/gitgpui-app/src/cli.rs`:
  - `compat_parses_meld_style_mergetool_with_output`
  - `compat_parses_meld_style_mergetool_with_auto_merge_flag`
  - `compat_auto_merge_requires_output_path`
- Added git-invoked E2E path-override coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_meld_path_override_invokes_compat_mode`
  - configures `merge.tool=meld`, `mergetool.meld.path=<gitgpui-app>`, `mergetool.meld.hasOutput=true`, `mergetool.meld.useAutoMerge=true`
- Verification:
  - `cargo test -p gitgpui-app compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_meld_path_override_invokes_compat_mode -- --nocapture`

### Latest Component Delivered (Iteration 37) — Compatibility-Mode Git Config Fallback Parity

- Fixed a parity gap in `crates/gitgpui-app/src/cli.rs`: KDiff3-style no-subcommand compatibility invocations (`--auto/-o/--L*`) now route through the same git-config fallback as explicit `mergetool` subcommand parsing.
  - Previously, compatibility mode resolved through `resolve_mergetool_with_env` only, so `merge.conflictstyle` and `diff.algorithm` were ignored in path-override flows.
  - Now, compatibility mode uses `resolve_mergetool_with_config`, so both `merge.conflictstyle` and `diff.algorithm` are honored consistently.
- Added parser-level regression coverage in `crates/gitgpui-app/src/cli.rs`:
  - `compat_mergetool_applies_merge_conflictstyle_from_git_config`
  - `compat_mergetool_applies_diff_algorithm_from_git_config`
- Added git-invoked E2E regression coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_kdiff3_path_override_respects_merge_conflictstyle_diff3_from_git_config`
  - validates that built-in `kdiff3` path-override invocation (compatibility mode) now emits diff3 base sections (`|||||||`) when `merge.conflictstyle=diff3` is configured.
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app cli::tests::compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration kdiff3_path_override -- --nocapture`
  - `cargo test -p gitgpui-app --tests`

### Latest Component Delivered (Iteration 36) — Pathspec E2E Parity Coverage

- Added explicit pathspec flow parity tests required by `external_usage.md` behavior matrix (`pathspec and dir-diff flows`):
  - `crates/gitgpui-app/tests/difftool_git_integration.rs`:
    - `git_difftool_pathspec_limits_invocation_to_selected_path`
    - validates `git difftool -- <path>` only emits diff output for selected path and excludes non-selected paths.
  - `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
    - `git_mergetool_pathspec_resolves_only_selected_conflict`
    - validates `git mergetool -- <path>` resolves only the targeted conflicted path, keeps other conflicted paths unresolved, and preserves unmerged index entries for non-selected paths.
- Verification:
  - `cargo test -p gitgpui-app --test difftool_git_integration git_difftool_pathspec_limits_invocation_to_selected_path -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_pathspec_resolves_only_selected_conflict -- --nocapture`

### Latest Component Delivered (Iteration 35) — CI Coverage/Setup Symmetry + Empty-Path Validation Hardening

- **CI fix**: wired `standalone_tool_mode_integration` tests into `.github/workflows/rust.yml` `tool-integration` job. Previously these tests existed but were not run in CI, creating a blind spot for standalone exit-code and validation-error assertions.
- **Setup config symmetry**: added generic `mergetool.trustExitCode=true` to `gitgpui-app setup` config entries, matching the symmetric pattern already used on the difftool side (both generic + per-tool keys). Both mergetool and difftool now write `<scope>.trustExitCode` and `<scope>.gitgpui.trustExitCode`.
- **Empty-path validation hardening** (`crates/gitgpui-app/src/cli.rs`): required path inputs now reject empty values (`LOCAL`, `REMOTE`, `MERGED`) with actionable parse-time errors; optional display-path compatibility vars ignore empty strings; env-provided empty `BASE` now maps to no-base, while explicit empty `--base` is rejected.
- Updated regression coverage:
  - unit: `build_config_entries_contains_all_required_keys` now asserts both `mergetool.trustExitCode` and `mergetool.gitgpui.trustExitCode`
  - integration: `setup_dry_run_prints_commands_without_writing` asserts both mergetool trust keys
  - integration: `setup_local_writes_config_to_repo` asserts both mergetool trust keys are written to repo config
  - unit: added CLI regression tests for empty-path handling (`difftool_empty_local_path_errors`, `difftool_empty_merged_env_is_ignored_for_display_path`, `mergetool_empty_base_env_treated_as_missing`, `mergetool_empty_base_flag_errors`, `mergetool_empty_merged_path_errors`)
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app setup_mode::tests:: -- --nocapture` (11 passed)
  - `cargo test -p gitgpui-app --bin gitgpui-app cli::tests:: -- --nocapture` (64 passed)
  - `cargo test -p gitgpui-app --tests -- --nocapture` (187 passed)
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration -- --nocapture` (9 passed)

### Previous: Iteration 34 — Setup TrustExitCode Config-Parity Hardening

### Latest Component Delivered (Iteration 33) — `t7610` KeepTemporaries Delete/Delete Abort E2E Parity

- Added explicit git-invoked parity coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - new test: `git_mergetool_keep_temporaries_delete_delete_abort_keeps_stage_files`
  - scenario: rename/rename setup that produces delete/delete at `a/a/file.txt`, with `mergetool.keepTemporaries=true` and abort choice `a`
  - assertions: `git mergetool` returns non-zero on abort and preserves workdir stage files matching `file_{BASE,LOCAL,REMOTE}_<pid>.txt`
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration keep_temporaries_delete_delete_abort -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration`

### Latest Component Delivered (Iteration 32) — Explicit `t7610` Custom-Cmd Git E2E Parity

- Added explicit `git mergetool` custom-command portability coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - new test: `git_mergetool_custom_cmd_copies_remote_to_merged`
  - configures a plain custom tool command (`cat "$REMOTE" > "$MERGED"`) as required by `t7610`-style parity scenarios
  - verifies end-to-end behavior: command resolves conflict, writes expected merged file bytes, and clears unmerged index entries (`git ls-files -u` empty)
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_custom_cmd_copies_remote_to_merged -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration`

### Latest Component Delivered (Iteration 31) — Mergetool Label-Fallback Portability

- Implemented Git-style default label fallback in dedicated mergetool runtime (`crates/gitgpui-app/src/mergetool_mode.rs`):
  - unresolved markers now default missing local/remote labels to input filenames
  - diff3/zdiff3 base label now defaults to base filename when present
  - diff3/zdiff3 base label now defaults to `empty tree` when base is absent
- Added focused regression coverage in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - `conflict_without_explicit_labels_defaults_to_filenames`
  - `conflict_with_partial_labels_defaults_missing_side_to_filename`
  - `diff3_style_defaults_base_label_to_filename`
  - `diff3_style_no_base_uses_empty_tree_label`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app mergetool_mode::tests:: -- --nocapture`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration standalone_mergetool_ -- --nocapture`

### Latest Component Delivered (Iteration 30) — Setup Dry-Run Shell-Quoting Parity

- Hardened setup command serialization in `crates/gitgpui-app/src/setup_mode.rs`:
  - added shell-safe single-quote escaping helper used for binary path embedding in `*.cmd` config values
  - fixed `--dry-run` rendering to shell-quote full config values, avoiding broken nested quoting in printed commands
- Added targeted regression coverage:
  - unit tests: `shell_single_quote_wraps_plain_text`, `shell_single_quote_escapes_embedded_single_quote`, `mergetool_cmd_escapes_single_quote_in_binary_path`
  - integration test: `setup_dry_run_commands_execute_verbatim_in_shell` in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs` executes dry-run lines via `sh -c` and verifies literal `$BASE/$LOCAL/$REMOTE/$MERGED` placeholders are preserved in git config values
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app setup_mode::tests:: -- --nocapture`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration setup_ -- --nocapture`

### Latest Component Delivered (Iteration 28) — Difftool Binary/Non-UTF8 Behavior-Matrix Coverage

- Added dedicated difftool runtime-unit coverage in `crates/gitgpui-app/src/difftool_mode.rs`:
  - `run_difftool_binary_content_returns_success`
  - `run_difftool_non_utf8_text_content_returns_success`
- Added git-invoked difftool E2E coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs`:
  - `git_difftool_handles_binary_content_change`
  - `git_difftool_handles_non_utf8_content_change`
- This closes the remaining explicit behavior-matrix test gap from `external_usage.md` item `binary and non-UTF8 content` for difftool flows.
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app run_difftool_ -- --nocapture`
  - `cargo test -p gitgpui-app --test difftool_git_integration`

### Latest Component Delivered (Iteration 27) — Phase 3C Portability Hardening

- Hardened `merge_git_extraction` test helpers in `crates/gitgpui-core/tests/merge_git_extraction.rs` to force `commit.gpgsign=false` for helper-run git commands.
- This removes host-environment coupling where global `commit.gpgsign=true` previously caused temporary test-repo commits to fail.
- Verification:
  - `cargo test -p gitgpui-core --test merge_git_extraction`
  - `cargo test -p gitgpui-core`

### Latest Component Delivered (Iteration 26) — Strict Merge `--L3` Arity Validation in Compat Mode

- Hardened no-subcommand KDiff3-compat merge parsing in `crates/gitgpui-app/src/cli.rs`:
  - merge-mode compatibility invocation with 2 positional paths (`LOCAL REMOTE`) now rejects `--L3` unless a base side is provided (`--base <BASE>` or 3 positional paths `BASE LOCAL REMOTE`)
  - prevents silent dropping of `--L3` labels in no-base merge mode
- Added parser regression test:
  - `compat_merge_without_base_rejects_l3_label`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --bin gitgpui-app`

### Latest Component Delivered (Iteration 25) — Standalone Command-Mode Exit Contract E2E

- Added direct binary-level integration coverage in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs` to validate `external_usage.md` exit-code policy for dedicated command modes (`difftool`/`mergetool`) without going through `git *tool`.
- Added 5 new tests:
  - `standalone_mergetool_clean_merge_exits_zero_and_writes_output`
  - `standalone_mergetool_conflict_exits_one_and_writes_markers`
  - `standalone_mergetool_invalid_path_exits_two`
  - `standalone_difftool_changed_files_exits_zero_and_prints_diff`
  - `standalone_difftool_missing_input_exits_two`
- Verification:
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration -- --nocapture`
  - `cargo test -p gitgpui-app`

### Latest Component Delivered (Iteration 24) — KDiff3 `--base` Compatibility in No-Subcommand Mode

- Extended no-subcommand compatibility parsing in `crates/gitgpui-app/src/cli.rs`:
  - accepts `--base <path>` and `--base=<path>` for merge-mode compatibility invocation (`-o/--output/--out` path provided)
  - preserves existing positional-base support and adds clear ambiguity guards when `--base` and positional BASE are both supplied
  - rejects `--base` in diff-mode compatibility invocation with actionable error text
- Added 3 regression tests in the CLI parser suite:
  - `compat_parses_kdiff3_style_mergetool_with_base_flag`
  - `compat_merge_rejects_base_flag_with_extra_positionals`
  - `compat_diff_rejects_base_without_output_path`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app compat_ -- --nocapture`
  - `cargo test -p gitgpui-app --bin gitgpui-app`
  - `cargo test -p gitgpui-app --test difftool_git_integration git_difftool_kdiff3_path_override_invokes_compat_mode -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_kdiff3_path_override_invokes_compat_mode -- --nocapture`

### Latest Component Delivered (Iteration 23) — Strict External Compat Validation

- Hardened no-subcommand external-tool parsing in `crates/gitgpui-app/src/cli.rs`:
  - `--auto` now requires `-o`/`--output`/`--out <MERGED>` (merge mode constraint).
  - merge-mode compatibility invocation now fails fast on invalid positional counts (must be 2 or 3).
  - diff-mode compatibility invocation now rejects merge-only `--L3` usage and extra positional args with actionable errors.
- Added 5 regression tests in CLI parser suite:
  - `compat_auto_requires_output_path`
  - `compat_merge_requires_two_or_three_positionals_after_output_flag`
  - `compat_merge_rejects_too_many_positionals`
  - `compat_diff_rejects_l3_without_output_path`
  - `compat_diff_rejects_too_many_positionals`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app compat_`
  - `cargo test -p gitgpui-app --bin gitgpui-app`
  - `cargo test -p gitgpui-app --test difftool_git_integration git_difftool_kdiff3_path_override_invokes_compat_mode -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_kdiff3_path_override_invokes_compat_mode -- --nocapture`

### Latest Component Delivered (Iteration 22) — KDiff3 Positional Compatibility + `*.path` Git E2E

- Implemented no-subcommand compatibility parsing in `crates/gitgpui-app/src/cli.rs`:
  - supports direct KDiff3-style external invocation with positional paths
  - accepts compatibility flags: `--auto`, `--L1/--L2/--L3`, `-o/--output/--out`
  - maps these inputs into validated `difftool`/`mergetool` app modes
- Added CLI unit coverage (4 tests):
  - `compat_parses_positional_difftool_invocation`
  - `compat_parses_kdiff3_style_difftool_labels`
  - `compat_parses_kdiff3_style_mergetool_with_base`
  - `compat_parses_kdiff3_style_mergetool_without_base`
- Added git-invoked E2E coverage for built-in `kdiff3` path override:
  - `git_difftool_kdiff3_path_override_invokes_compat_mode`
  - `git_mergetool_kdiff3_path_override_invokes_compat_mode`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app`
  - `cargo test -p gitgpui-app --test difftool_git_integration git_difftool_kdiff3_path_override_invokes_compat_mode -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_kdiff3_path_override_invokes_compat_mode -- --nocapture`

### Latest Component Delivered (Iteration 21) — Difftool `BASE` Env Compatibility Fallback

- Closed the remaining difftool contract gap in `crates/gitgpui-app/src/cli.rs`:
  - display-path resolution now honors optional `BASE` when `MERGED` is not set.
  - explicit precedence is now locked: `--path` > `MERGED` > `BASE`.
- Added regression coverage in CLI resolver tests:
  - `difftool_uses_base_env_as_display_path_fallback`
  - `difftool_prefers_merged_over_base_for_display_path`
  - `difftool_path_flag_overrides_merged_and_base_display_env`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app difftool_ -- --nocapture`

### Latest Component Delivered (Iteration 20) — Standalone `MERGED` Output-Target Support

- Implemented standalone output-target parity for `gitgpui-app mergetool`:
  - `resolve_mergetool_with_env` no longer requires `MERGED` to pre-exist.
  - `MERGED` is treated as an output path (aligned with `-o`/`--output`/`--out` semantics).
- Hardened output writes in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - added shared `write_merged_output` helper
  - creates parent directories for `MERGED` before writing (clean merges and binary conflict fallback paths)
- Added regression tests:
  - `cli::tests::mergetool_nonexistent_merged_is_allowed`
  - `mergetool_mode::tests::merged_output_path_can_be_created_when_missing`
  - `mergetool_mode::tests::merged_output_parent_dirs_created_for_binary_conflict`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app` (75 tests, all passing)
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_resolves_overlapping_conflict -- --nocapture`

### Earlier Iteration 20 Pass — Binary File Conflict E2E Coverage

- Added 2 git-invoked mergetool E2E tests in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_binary_conflict_keeps_local_version`: creates a binary file conflict (null-byte content) across branches, runs `git mergetool` with gitgpui, and verifies that the binary conflict is detected, the local version is kept in MERGED, and the tool reports the binary conflict.
  - `git_mergetool_binary_conflict_alongside_text_conflict`: creates both a binary file conflict and a text file conflict in the same merge, verifies both are processed correctly — text conflict gets markers, binary conflict keeps local version.
- Closes the last gap in the behavior matrix from `external_usage.md`: item #4 (binary and non-UTF8 content) is now covered end-to-end through `git mergetool` invocation, in addition to existing unit-level coverage.
- Mergetool E2E suite expanded from 43 to 45 tests.
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_binary -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration` (45 tests, all passing)
  - `cargo clippy -p gitgpui-app -- -D warnings` (clean)

### Previous: Iteration 19 — Mergetool CLI Alias Compatibility

- Added KDiff3/Meld-style mergetool argument aliases in `crates/gitgpui-app/src/cli.rs`:
  - output path aliases: `-o`, `--output`, `--out` -> `--merged`
  - label aliases: `--L1`, `--L2`, `--L3` -> `--label-base`, `--label-local`, `--label-remote`
- Added parser-level coverage in `crates/gitgpui-app/src/cli.rs`:
  - `clap_parses_mergetool_output_aliases`
  - `clap_parses_mergetool_kdiff3_label_aliases`
- Added git-invoked integration coverage in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_accepts_kdiff3_alias_flags_in_cmd`
- Verification:
  - `cargo test -p gitgpui-app --bin gitgpui-app`
  - `cargo test -p gitgpui-app --test mergetool_git_integration git_mergetool_accepts_kdiff3_alias_flags_in_cmd -- --nocapture`

### Iteration 19 (Earlier Pass) — Git Config Fallback for Merge Preferences

- Added automatic git config fallback to `resolve_mergetool()` in `crates/gitgpui-app/src/cli.rs`:
  - When `--conflict-style` is not provided via CLI, reads `merge.conflictstyle` from git config (`merge`, `diff3`, `zdiff3`).
  - When `--diff-algorithm` is not provided via CLI, reads `diff.algorithm` from git config (`myers`, `histogram`, `patience`, `default`, `minimal`).
  - CLI flags always take priority over git config values. Unknown config values are silently ignored, preserving defaults.
  - Uses `git config --get` to read config from the current working directory's repo, which is the correct context when invoked by `git mergetool`.
- Added testable internal architecture:
  - `read_git_config(key) -> Option<String>`: lightweight git config reader via subprocess.
  - `apply_git_config_fallback()`: applies config values only when CLI flags are absent.
  - `resolve_mergetool_with_config()`: internal function accepting a mock config reader for unit tests.
- Added 8 unit tests in `crates/gitgpui-app/src/cli.rs`:
  - `git_config_fallback_reads_merge_conflictstyle_zdiff3`
  - `git_config_fallback_reads_merge_conflictstyle_diff3`
  - `git_config_fallback_reads_diff_algorithm_histogram`
  - `git_config_fallback_reads_diff_algorithm_patience_as_histogram`
  - `git_config_fallback_explicit_cli_overrides_git_config`
  - `git_config_fallback_no_git_config_uses_defaults`
  - `git_config_fallback_unknown_values_ignored`
  - `git_config_fallback_combined_style_and_algorithm`
- Added 4 E2E integration tests in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_respects_merge_conflictstyle_zdiff3_from_git_config`
  - `git_mergetool_respects_merge_conflictstyle_diff3_from_git_config`
  - `git_mergetool_respects_diff_algorithm_histogram_from_git_config`
  - `git_mergetool_cli_flag_overrides_git_config`
- Verification:
  - `cargo test -p gitgpui-app` (42 integration + 71 binary tests, all passing)
  - `cargo clippy -p gitgpui-app -- -D warnings` (clean)

### Previous: Iteration 18 — Delete/Delete `d` / `m` / `a` Mergetool Parity

- Added a dedicated rename/rename conflict setup helper in `crates/gitgpui-app/tests/mergetool_git_integration.rs` that reproduces t7610's delete/delete-at-original-path scenario (`a/a/file.txt` while branches rename to `b/b/file.txt` and `c/c/file.txt`).
- Added 3 new git-invoked mergetool E2E tests:
  - `git_mergetool_delete_delete_choice_d_deletes_original_path`
  - `git_mergetool_delete_delete_choice_m_keeps_modified_destination`
  - `git_mergetool_delete_delete_choice_a_aborts_with_nonzero`
- Coverage now explicitly verifies the path-targeted `git mergetool a/a/file.txt` choice matrix from `t7610-mergetool.sh`:
  - `d` choice deletes original path
  - `m` choice keeps modified destination (`b/b/file.txt`)
  - `a` choice aborts and returns non-zero
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration delete_delete -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration`

### Latest Component Delivered (Iteration 17) — Phase 3B Permutation Alignment Invariants

- Implemented the remaining Phase 3B portability invariant checks in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`.
- Added deterministic three-way alignment construction for each generated corpus case.
- Added two algorithm-independent alignment assertions for every case:
  1. sequence monotonicity for `base`, `contrib1`, and `contrib2` index columns
  2. content consistency whenever two concrete indices align on a row
- Existing Phase 3A checks (marker well-formedness, content integrity, context preservation, conflict-count parity) remain intact and now run alongside the new alignment checks.
- Verification:
  - `cargo test -p gitgpui-core --test merge_permutation_corpus`
  - `cargo test -p gitgpui-core --test merge_fixture_harness`

### Iteration 17 (Earlier Pass) — Parity-Focused CI Regression Gates

- Replaced monolithic `cargo test` CI workflow with 5 focused jobs in `.github/workflows/rust.yml`:
  1. **Clippy**: lint gate for core crates (`-D warnings`)
  2. **Merge algorithm parity**: t6403/t6427 portability, conflict labels, Meld algorithm, core lib tests
  3. **Merge regression suite**: KDiff3-style fixture harness, 243-case permutation corpus, real-world merge extraction
  4. **Git mergetool/difftool E2E**: t7610/t7800 parity (35 mergetool + 14 difftool integration tests)
  5. **Backend integration**: mergetool launcher, status, conflict checkout tests
- This fulfills Phase 3 rollout item "parity-focused regression gates in CI" from `external_usage.md`.
- Each job targets specific crate/test targets, avoiding vendored gpui test failures while providing clear per-domain pass/fail signals.

### Latest Component Delivered (Iteration 16) — `mergetool.writeToTemp` Git E2E Path-Parity Tests

- Added stage-path capture helpers in `crates/gitgpui-app/tests/mergetool_git_integration.rs` for git-invoked mergetool runs.
- Added 2 new end-to-end tests:
  - `git_mergetool_write_to_temp_true_uses_absolute_stage_paths`
  - `git_mergetool_write_to_temp_false_uses_workdir_prefixed_stage_paths`
- Coverage verifies the Phase 4A portability edge cases through `git mergetool` execution:
  - `mergetool.writeToTemp=true` passes absolute `BASE/LOCAL/REMOTE` stage paths.
  - `mergetool.writeToTemp=false` passes `./`-prefixed workdir stage paths.
- Verification:
  - `cargo test -p gitgpui-app --test mergetool_git_integration write_to_temp -- --nocapture`
  - `cargo test -p gitgpui-app --test mergetool_git_integration`

### Latest Component Delivered (Iteration 15) — KDiff3 Alignment Triple Fixture Support

- Extended `crates/gitgpui-core/tests/merge_fixture_harness.rs` to support dual expected-result formats:
  - merged-output goldens (existing behavior)
  - KDiff3-style alignment triples (`base_idx contrib1_idx contrib2_idx`, `-1` gaps)
- Implemented a deterministic three-way alignment builder for fixture validation:
  - pairwise LCS projection from base -> contrib1/contrib2
  - insertion alignment via LCS between contrib insertion runs
- Added alignment-specific algorithm-independent invariant checks:
  - sequence monotonicity per column (strictly increasing indices)
  - equality consistency for rows that align two/three concrete lines
- Added 2 new KDiff3-style alignment fixtures:
  - `8_kdiff3_simple_alignment`
  - `9_kdiff3_prefer_identical_alignment`
- Kept backward compatibility with existing merged-output fixtures and mismatch artifact writing (`*_actual_result.*`).
- Verification:
  - `cargo test -p gitgpui-core --test merge_fixture_harness`
  - `cargo test -p gitgpui-core --test merge_algorithm --test meld_algorithm_tests --test merge_fixture_harness`

### Latest Component Delivered (Iteration 14) — Mergetool `trustExitCode=false` E2E Parity

- Added 2 git-invoked mergetool integration tests in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - `git_mergetool_no_trust_exit_code_unchanged_output_stays_unresolved`
  - `git_mergetool_no_trust_exit_code_changed_output_resolves_conflict`
- New coverage verifies `t7610` no-trust semantics end-to-end:
  - unchanged MERGED output with `trustExitCode=false` remains unresolved and returns non-zero
  - changed MERGED output with `trustExitCode=false` is accepted even when tool exits non-zero
- Verification run:
  - `cargo test -p gitgpui-app --test mergetool_git_integration no_trust_exit_code -- --nocapture`

### Iteration 14 (Earlier Pass) — Mergetool Conflict Style & Diff Algorithm CLI Options

- Added `--conflict-style` CLI flag to `gitgpui-app mergetool` (values: `merge`, `diff3`, `zdiff3`; defaults to `merge`).
- Added `--diff-algorithm` CLI flag to `gitgpui-app mergetool` (values: `myers`, `histogram`; defaults to `myers`).
- Wired both options through `MergetoolConfig` → `MergeOptions` so the mergetool runtime uses the user's preferred conflict marker format and diff algorithm.
- CLI validation produces actionable error messages for invalid values.
- Added 10 new tests:
  - 7 CLI validation tests: default values, diff3/zdiff3/invalid conflict style, histogram/invalid diff algorithm, clap parsing
  - 3 functional tests: diff3 base-section inclusion, zdiff3 common prefix/suffix extraction, histogram clean merge on structural code
- Applied pending `rustfmt` formatting across all modified files.
- All tests pass: 63 binary tests, 14 difftool E2E, 31 mergetool E2E, 262+ core tests.

### Iteration 13 — Meld Sync-Point Matcher Portability

- Implemented sync-point-aware matching in `crates/gitgpui-core/src/text_utils.rs`:
  - Added `matching_blocks_chars_with_sync_points` and `matching_blocks_lines_with_sync_points`.
  - Added strict sync point validation (`SyncPointError`) for out-of-bounds and non-monotonic inputs.
  - Preserves default behavior of existing APIs (`matching_blocks_chars`, `matching_blocks_lines`) while enabling deterministic sync-point-constrained alignment.
- Ported missing Meld sync-point coverage in `crates/gitgpui-core/tests/meld_algorithm_tests.rs`:
  - `sync_point_none` parity through the new API with empty sync points.
  - one-sync-point case (`(3, 6)`) and two-sync-point case (`(3, 2)`, `(8, 6)`).
  - validation tests for invalid sync-point inputs.
- Verification:
  - `cargo test -p gitgpui-core --test meld_algorithm_tests` passes (32/32).
  - `cargo test -p gitgpui-core` runs green for core suites touched by this change; unrelated `merge_git_extraction` tests fail in this environment due GPG signing policy (`commit.gpgsign`) rather than merge logic.

### Iteration 12 — Phase 1A completion

- Implemented patience/histogram diff algorithm in `crates/gitgpui-core/src/file_diff.rs`:
  - `histogram_edits`: patience diff that anchors on unique lines via longest increasing subsequence, with Myers fallback for regions with no unique lines
  - `patience_recurse`: recursive diff with common prefix/suffix stripping
  - `find_patience_anchors`: unique-line matching between two ranges
  - `patience_lis`: longest increasing subsequence for anchor ordering
- Added `DiffAlgorithm` enum (`Myers`, `Histogram`) in `crates/gitgpui-core/src/merge.rs`:
  - New field `diff_algorithm` on `MergeOptions` (defaults to `Myers` for backward compatibility)
  - `merge_file` dispatches to the selected algorithm
- Added `MergeError` and `merge_file_bytes` for binary detection:
  - `MergeError::BinaryContent` with `Display` and `Error` impls
  - `merge_file_bytes(base: &[u8], ours: &[u8], theirs: &[u8], options) -> Result<MergeResult, MergeError>` — checks for null bytes and non-UTF-8 before delegating to `merge_file`
- Added 7 new tests in `crates/gitgpui-core/tests/merge_algorithm.rs`:
  - `t6403_merge_histogram_clean`: C code test case that produces clean merge with histogram vs spurious conflicts with Myers
  - `t6403_merge_histogram_identity`: identity merge with histogram
  - `t6403_merge_histogram_nonoverlapping`: non-overlapping changes with histogram
  - `t6403_merge_histogram_conflict`: true conflict detection with histogram
  - `t6403_merge_binary_rejected`: PNG/null-byte binary rejection via `merge_file_bytes`
  - `t6403_merge_binary_null_byte_in_utf8`: null-byte-in-UTF-8 rejection
  - `t6403_merge_binary_content_text_api_no_panic`: backward-compatible text API
- Verified with `cargo test -p gitgpui-core` (255 tests passing, all suites green).

### Iteration 11

- Implemented zealous conflict coalescing in `crates/gitgpui-core/src/merge.rs`:
  - adjacent conflict hunks are coalesced
  - conflict hunks separated only by blank base context are coalesced
  - non-blank separators are intentionally preserved as separate conflicts
- Added portability tests in `crates/gitgpui-core/tests/merge_algorithm.rs` covering both zealous scenarios and a non-blank regression guard.
- Verified with `cargo test -p gitgpui-core --test merge_algorithm` (33/33 passing).

### Iteration 9

- Completed symlink and submodule conflict E2E test coverage, closing the last remaining gaps in the behavior parity matrix:
  - Added 3 symlink mergetool tests: `git_mergetool_symlink_conflict_resolved_via_local`, `git_mergetool_symlink_conflict_resolved_via_remote`, `git_mergetool_symlink_alongside_normal_file_conflict`
  - Added 6 submodule mergetool tests: `git_mergetool_submodule_conflict_resolved_via_local`, `git_mergetool_submodule_conflict_resolved_via_remote`, `git_mergetool_submodule_alongside_normal_file_conflict`, `git_mergetool_file_replaced_by_submodule_conflict`, `git_mergetool_submodule_in_subdirectory_conflict`, `git_mergetool_deleted_submodule_conflict`
  - Added 1 symlink difftool test: `git_difftool_shows_symlink_target_change`
  - Added `run_git_with_stdin` helper for interactive prompt testing (piping l/r/d answers to git's symlink/submodule resolution prompts)
  - Mergetool suite expanded from 19 to 28 tests; difftool suite expanded from 12 to 13 tests
  - All behavior matrix items now covered: symlink conflicts (#6) and submodule path conflicts (#7) verified
  - All Phase 4A submodule-specific flows implemented: l/r resolution, deleted-vs-modified, file-vs-submodule, subdirectory submodule
  - Added Unicode path parity coverage:
    - `git_difftool_handles_unicode_path`
    - `git_mergetool_handles_unicode_path`
  - Suite totals now: mergetool 29 tests, difftool 14 tests

### Previous: Iteration 8

- Implemented Phase 4A order-file invocation-order parity tests in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - Added `git_mergetool_honors_diff_order_file_configuration` to assert `diff.orderFile` controls merge processing order (`b`, then `a`).
  - Added `git_mergetool_o_flag_overrides_diff_order_file` to assert CLI `-O...` overrides configured `diff.orderFile`.
  - Added reusable test helpers:
    - `setup_order_file_conflict` for deterministic two-file conflict setup.
    - `configure_recording_mergetool` and `read_recorded_merge_order` for deterministic invocation-order capture.
  - Verified via `cargo test -p gitgpui-app --test mergetool_git_integration` (19/19 passing).

### Iteration 10

- Implemented `mergetool.keepTemporaries` support in the backend external-tool launcher:
  - Added config parsing in `crates/gitgpui-git-gix/src/repo/mergetool.rs` (`mergetool.keepTemporaries`, default `false`).
  - Wired stage-file lifecycle so temporary stage files are retained when enabled and cleaned by default when disabled.
  - Extended write-to-temp path handling so retained temp stage files persist after tool execution in `writeToTemp=true` mode.
  - Added backend unit coverage for config resolution (`test_resolve_mergetool_config_reads_keep_temporaries`).
  - Added integration coverage in `crates/gitgpui-git-gix/tests/status_integration.rs`:
    - `launch_mergetool_write_to_temp_false_keep_temporaries_preserves_stage_files`
    - `launch_mergetool_write_to_temp_true_keep_temporaries_preserves_stage_files`
    - strengthened existing `writeToTemp` tests to assert default cleanup behavior.
  - Verified with `cargo test -p gitgpui-git-gix` (all tests passing).

### Iteration 9 Component Delivered

- Implemented full Git-invoked mergetool E2E integration tests in `crates/gitgpui-app/tests/mergetool_git_integration.rs`:
  - 8 tests covering the full `git mergetool` → `gitgpui-app mergetool` pipeline:
    1. `git_mergetool_resolves_overlapping_conflict` — tool processes conflicted file via git mergetool
    2. `git_mergetool_with_trust_exit_code_marks_clean_merge_resolved` — clean auto-merge accepted by git
    3. `git_mergetool_trust_exit_code_conflict_preserves_unmerged_state` — unresolved conflict correctly leaves file unmerged
    4. `git_mergetool_handles_path_with_spaces` — spaced filenames passed correctly through git→tool pipeline
    5. `git_mergetool_works_from_subdirectory` — invocation from repo subdirectory resolves paths correctly
    6. `git_mergetool_handles_add_add_conflict` — both-added file (no base) scenario
    7. `git_mergetool_multiple_conflicted_files` — tool invoked for each conflicted file
    8. `git_mergetool_crlf_content_preserved` — CRLF line endings preserved through merge pipeline
  - Tests create real git repos with merge conflicts, configure gitgpui-app as `mergetool.gitgpui.cmd` with `trustExitCode=true`, and invoke `git mergetool --no-prompt` to verify end-to-end behavior.

### Iteration 8 Component Delivered

- Implemented full Git-invoked difftool integration coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs`:
  - `git difftool` basic invocation (`LOCAL`/`REMOTE` wiring)
  - spaced-path file handling
  - invocation from repo subdirectories
  - `--dir-diff` directory-mode execution
- Fixed an external-tool recursion bug in `crates/gitgpui-app/src/difftool_mode.rs`:
  - switched command to `git diff --no-index --no-ext-diff`
  - removed inherited `GIT_EXTERNAL_DIFF` before spawning nested git diff
  - prevents recursive `git-difftool--helper` loops when Git launches GitGpui as `difftool.<tool>.cmd`

### Iteration 8 Component Delivered (Mergetool Runtime)

- Implemented focused `mergetool` runtime path in `gitgpui-app`:
  - Added `crates/gitgpui-app/src/mergetool_mode.rs` with `run_mergetool()` that performs 3-way merge using the built-in `merge_file()` algorithm from `gitgpui-core`.
  - Reads base/local/remote file contents, runs automatic merge, writes result to MERGED output path.
  - Exit code semantics: 0 on clean merge (all conflicts auto-resolved), 1 on remaining conflicts (markers written to output), ≥2 on I/O errors.
  - Supports label forwarding (`--label-local`, `--label-remote`, `--label-base`) to conflict markers.
  - Handles no-base (add/add) scenarios by treating missing base as empty content.
  - Detects binary content (non-UTF-8) and falls back to copying local side to MERGED.
  - Preserves CRLF line endings, trailing newline semantics, and handles paths with spaces.
  - Wired `AppMode::Mergetool` in `main.rs` to this runtime (replaced previous not-implemented error).
  - Added 19 unit tests covering: clean merge, identical files, conflicts with markers, labels, no-base, binary detection, file I/O errors, CRLF, empty files, trailing newlines, multi-region conflicts, spaced paths, output overwrite.

### Iteration 7 Component Delivered

- Implemented foundational `difftool` runtime path in `gitgpui-app`:
  - Added `crates/gitgpui-app/src/difftool_mode.rs` with `run_difftool()` that executes `git diff --no-index -- <LOCAL> <REMOTE>`.
  - Added Git-compatible exit mapping: diff-present (`git` exit `1`) is normalized to app success (`0`), while operational failures return app error semantics.
  - Added header label support for `--label-left`, `--label-right`, and `--path`/display name by rewriting unified-diff file headers (`---` / `+++`) deterministically.
  - Wired `AppMode::Difftool` in `crates/gitgpui-app/src/main.rs` to this runtime path (removed previous not-implemented hard error for difftool mode).
  - Added 7 unit tests covering unchanged/changed files, label behavior, directory diff mode, and missing-input error handling.
- ✅ Phase 5A/5B/5C (Meld-derived matcher/interval/newline test ports): implemented in `crates/gitgpui-core/src/text_utils.rs` with tests in `crates/gitgpui-core/tests/meld_algorithm_tests.rs`:
  - 5A: Myers matching blocks extraction (`matching_blocks_chars`, `matching_blocks_lines`) plus sync-point-constrained variants (`matching_blocks_chars_with_sync_points`, `matching_blocks_lines_with_sync_points`) with 12 tests (including `sync_point_none`, one-sync-point, and two-sync-point parity cases from Meld).
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
    1. **Matching block extraction** (`matching_blocks_chars`, `matching_blocks_lines`, `matching_blocks_*_with_sync_points`): converts Myers diff edit scripts into `MatchingBlock` tuples `(a_start, b_start, length)` for both character-level and line-level sequences, with optional sync-point constraints.
    2. **Interval merging** (`merge_intervals`): coalesces overlapping/adjacent `(start, end)` intervals into non-overlapping sorted output.
    3. **Newline-aware line deletion** (`delete_last_line`): removes the last line respecting `\n`, `\r\n`, and `\r` line endings.
  - Added `crates/gitgpui-core/tests/meld_algorithm_tests.rs` with 32 tests:
    - 5A (12 tests): 4 character-level matching block tests ported from Meld's `test_matchers.py` (basic, postprocess, inline, no-sync-points), 3 sync-point parity tests (`sync_point_none`, one sync point, two sync points), sync-point validation tests, and line-level matching block tests.
    - 5B (8 tests): 6 interval merging tests ported from Meld's `test_misc.py` (dominated, disjoint, two-groups, unsorted, duplicate, chain) + 2 edge cases.
    - 5C (12 tests): 7 newline-aware deletion tests ported from Meld's `test_chunk_actions.py` (CRLF, LF, CR, trailing, mixed) + 5 edge cases.
  - Sync-point support now mirrors Meld's constrained-matching concept with explicit validation for out-of-bounds and non-monotonic sync point input.

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
