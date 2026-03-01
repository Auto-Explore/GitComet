## STATUS: COMPLETE

## Implementation Progress

### External Diff/Merge Usage Design (`external_usage.md`)

- ✅ CLI subcommands and argument model (`gitgpui-app difftool`, `gitgpui-app mergetool`) implemented in `crates/gitgpui-app/src/cli.rs`.
- ✅ Arg/env resolution + validation implemented for `LOCAL`, `REMOTE`, `MERGED`, `BASE`, labels, missing-input and missing-path errors.
- ✅ Exit code constants aligned to design (`0`, `1`, `>=2`) defined in app CLI module.
- ✅ Foundational conflict-marker label formatter implemented in `crates/gitgpui-core/src/conflict_labels.rs` (`empty tree`, `<short-sha>:<path>`, merged-ancestors, rebase-parent shapes), ready for focused merge-mode integration.
- ✅ Focused command-mode execution paths fully implemented:
  - ✅ `difftool` mode executes a dedicated runtime path in `crates/gitgpui-app/src/difftool_mode.rs` (delegates to `git diff --no-index --no-ext-diff`, strips recursive `GIT_EXTERNAL_DIFF` env, supports labels/display-path headers, and maps git exit `1`/diff-present to app success exit `0`).
  - ✅ `mergetool` mode executes a dedicated runtime path in `crates/gitgpui-app/src/mergetool_mode.rs` using the built-in 3-way merge algorithm (`merge_file`). Reads base/local/remote files, performs automatic merge, writes result to MERGED path. Exits 0 on clean merge, 1 on unresolved conflicts. Supports labels, no-base (add/add) scenarios, binary file detection (copies local side), CRLF preservation, and paths with spaces. 19 unit tests.
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
- ✅ `mergetool.keepBackup` delete/delete parity scenario covered by dedicated git-invoked E2E assertion (`git_mergetool_keep_backup_delete_delete_no_errors`).
- ✅ Git behavior parity matrix coverage is complete. All items covered: spaced and Unicode paths, no-base handling for stage extraction (including empty `BASE` file for add/add), trust-exit semantics, deleted output handling, writeToTemp path semantics, difftool `--dir-diff`, difftool `guiDefault` selection (`auto` + `DISPLAY`, `--gui`, `--no-gui`), difftool `--tool-help` discoverability, mergetool `guiDefault` selection (`auto` + `DISPLAY`, `--gui`, `--no-gui`), mergetool `--tool-help` discoverability, mergetool GUI fallback (no guitool → merge.tool), nonexistent tool error handling, delete/delete conflict handling, modify/delete conflict handling, symlink conflict resolution (l/r/a prompts, coexistence with normal file conflicts, difftool target diff), and submodule conflict handling (l/r resolution, coexistence with normal file conflicts, file-vs-submodule, deleted-vs-modified submodule, submodule in subdirectory).
- ✅ Git-like scenario porting is complete. All listed t7610/t7800 parity items are covered: `trustExitCode`, custom cmd with braced env, gui preference, writeToTemp/keepTemporaries, keepBackup delete/delete, no-base stage-file contract, difftool gui-default/trust/tool-help parity, mergetool gui-default/trust/tool-help parity, GUI fallback, nonexistent tool error, delete/delete, modify/delete, order-file invocation ordering (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), and submodule conflicts (l/r resolution, deleted-vs-modified, file-vs-submodule, subdirectory submodule, coexistence with normal files).
- ✅ Dedicated difftool mode tests are implemented with parity-focused coverage:
  - ✅ Runtime/unit coverage in `crates/gitgpui-app/src/difftool_mode.rs` (identical files, changed files with exit normalization, display-path and explicit labels, missing-input error handling, directory diff).
  - ✅ Full git-invoked integration coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs` (basic invocation, spaced and Unicode paths, subdirectory invocation, `--dir-diff`, `guiDefault`/`--gui`/`--no-gui` selection precedence, trust-exit-code matrix, `--tool-help` discoverability, and symlink target diff).
- ✅ End-to-end tests that invoke `git difftool`/`git mergetool` with global-like config and `gitgpui-app` as the tool are fully implemented:
  - ✅ `git difftool` E2E in `crates/gitgpui-app/tests/difftool_git_integration.rs` (14 tests).
  - ✅ `git mergetool` E2E in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (30 tests): overlapping conflict processing, trust-exit-code semantics (clean merge resolved / conflict preserved), spaced and Unicode path handling, subdirectory invocation, add/add (no-base) conflict, multiple conflicted files, CRLF preservation, `--tool-help` discoverability, `guiDefault=auto` selection (with/without DISPLAY), `--gui` and `--no-gui` flag overrides, GUI fallback when no guitool configured, nonexistent tool error handling, delete/delete conflict, delete/delete with keepBackup=true (no-error parity), modify/delete conflict, invocation ordering parity (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), and submodule conflicts (l/r resolution, deleted-vs-modified, file-vs-submodule, subdirectory submodule, coexistence with normal files).
- ✅ KDiff3-style fixture harness implemented in `crates/gitgpui-core/tests/merge_fixture_harness.rs` with fixture data in `crates/gitgpui-core/tests/fixtures/merge/`. Auto-discovers `*_base.*` fixtures, runs merge algorithm, validates invariants (marker well-formedness, content integrity, context preservation), and compares against expected results. 7 seed fixtures + harness discovery test = 8 tests.
- ✅ Generated permutation corpus integration (Phase 3A) added in `crates/gitgpui-core/tests/merge_permutation_corpus.rs`: ports KDiff3’s 11-option line-state table, runs deterministic sampled corpus (`r=3`, `seed=0`, 243 cases) in default test runs, and includes an ignored exhaustive run (11^5 = 161,051 cases).

### Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)

- 🔧 Phase 1A (git `t6403` algorithm-focused cases): 3-way merge algorithm implemented in `crates/gitgpui-core/src/merge.rs` with focused portability coverage in `crates/gitgpui-core/tests/merge_algorithm.rs`.
  - ✅ Added zealous conflict coalescing in core merge flow (`coalesce_zealous_conflicts`) for adjacent conflicts and blank-only separators.
  - ✅ Added portability tests: `t6403_merge_zealous_coalesces_adjacent_conflict_lines`, `t6403_merge_zealous_alnum_coalesces_across_blank_separator`, and non-blank separator guard.
  - ✅ Existing t6403-style coverage remains for identity/non-overlap/conflicts, conflict markers + labels, delete-vs-modify, ours/theirs/union, EOF/trailing-newline behavior, CRLF markers, marker width, diff3 output, Myers C-code case, identical changes, and single-side-only changes.
  - ✅ `merge_histogram_clean` parity implemented: patience/histogram diff algorithm in `file_diff.rs` (`histogram_edits`), `DiffAlgorithm` enum in `MergeOptions`, and 4 portability tests demonstrating clean merge on C code that produces spurious conflicts with Myers.
  - ✅ Strict `merge_binary_rejected` contract implemented: `MergeError::BinaryContent` error type and `merge_file_bytes(&[u8], &[u8], &[u8], &MergeOptions) -> Result<MergeResult, MergeError>` entry point with null-byte and non-UTF-8 detection. 3 portability tests covering PNG rejection, null-byte-in-UTF-8 rejection, and text-API backward compatibility.
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
- ✅ Phase 4A (critical `t7610-mergetool` E2E): fully implemented across `gitgpui-git-gix` tests and `gitgpui-app` E2E:
  - ✅ trust-exit behavior and content-change semantics
  - ✅ custom command invocation and braced env variables
  - ✅ gui tool preference path via `merge.guitool` + `mergetool.guiDefault=true`
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
  - ✅ modify/delete conflict handling: pipeline completes without crash
  - ✅ orderFile invocation order parity (`diff.orderFile` and CLI `-O...`) in `crates/gitgpui-app/tests/mergetool_git_integration.rs`
  - ✅ symlink conflict resolution: l/r prompt handling, coexistence with normal file conflicts
  - ✅ submodule conflict resolution: l/r prompt, deleted-vs-modified, file-vs-submodule, subdirectory submodule, coexistence with normal files
  - ✅ `mergetool.keepTemporaries` stage-file retention semantics (`true` retains, default `false` cleans up) in backend launch path
  - ✅ `mergetool.keepBackup=true` delete/delete E2E assertion: rename/rename conflict with keepBackup produces no stderr errors
  - ✅ difftool symlink target diff: `git difftool` shows diff between symlink targets
  - ✅ full E2E via `git mergetool` command in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (30 tests)
  - ✅ full E2E via `git difftool` command in `crates/gitgpui-app/tests/difftool_git_integration.rs` (14 tests)
- ✅ Phase 4B (critical `t7800-difftool` E2E): implemented in `crates/gitgpui-app/tests/difftool_git_integration.rs`.
  - ✅ Foundational difftool runtime with Git-compatible exit semantics and label/display-path handling.
  - ✅ Git-invoked E2E coverage for basic invocation, subdirectory execution, spaced path handling, and `--dir-diff`.
  - ✅ Explicit `difftool.guiDefault` selection-path parity (`auto` with/without `DISPLAY`, `--gui`, `--no-gui`).
  - ✅ Dedicated trust-exit interaction matrix assertions (`difftool.trustExitCode`, `--trust-exit-code`, `--no-trust-exit-code`).
  - ✅ `git difftool --tool-help` discoverability assertion for configured `gitgpui` tool.

### Latest Component Delivered (Iteration 12) — Phase 1A completion

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
