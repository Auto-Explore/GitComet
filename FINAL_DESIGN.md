## STATUS: COMPLETE

## Implementation Progress

### Progress Snapshot (Iteration 10, Independent Verification & Dead Code Cleanup — March 2, 2026)

Independent verification audit by a fresh agent across three parallel investigations (production code quality, behavior matrix test coverage, edge-case handling) confirms all design document components remain fully implemented and robust.

Implemented this iteration:
- ✅ Removed unnecessary `#[allow(dead_code)]` annotation on `label_base` field in `crates/gitgpui-ui-gpui/src/focused_merge.rs` — the field IS actively used in `build_output()` for conflict marker label rendering; the annotation was a leftover.
- ✅ Removed truly dead `validate_repo_path()` function and its unused imports (`Error`, `ErrorKind`, `Path`, `PathBuf`) from `crates/gitgpui-state/src/store/executor.rs`.

Verification scope (this iteration):
- ✅ `cargo test --workspace --no-default-features --features gix`: **1106 passed, 0 failed, 5 ignored**.
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`: **0 warnings** (headless CI mode).
- ✅ `cargo clippy --workspace -- -D warnings`: **0 warnings** (full-UI mode with all features).
- ✅ Conducted parallel deep audit via three independent exploration agents:
  - **Production code quality audit**: No TODO/FIXME/HACK/unimplemented markers in production code. Two `unreachable!()` usages are logically sound (guarded by prior conditions). All `.unwrap()` calls in production code are properly guarded by prior checks. No `unsafe` blocks (workspace-level `unsafe_code = "forbid"`). No panicking patterns.
  - **Behavior matrix test coverage audit**: All 10 behavior matrix items from `external_usage.md` have EXCELLENT test coverage with specific, named tests. Total: 127+ integration tests across mergetool/difftool/standalone modes, plus core algorithm and fixture harness tests.
  - **Edge-case handling audit**: Read-only output files, broken symlinks, empty directories, symlink cycles, unknown CLI arguments, file existence validation, env var fallback, and setup mode config generation are all properly handled. No critical gaps found.
- ✅ 5 ignored tests are all intentional: 2 require external repo env var, 1 is exhaustive 161K-case corpus, 2 are performance benchmarks.

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 9, Dir-Diff Symlink-Cycle Hardening — March 2, 2026)

Implemented this iteration:
- ✅ Hardened `difftool --dir-diff` staging in `crates/gitgpui-app/src/difftool_mode.rs` to detect and reject symlink cycles instead of recursing indefinitely.
  - Added canonical active-directory tracking during recursive staging.
  - Added explicit cycle error reporting (`Detected symlink cycle while staging directory diff inputs ...`).
- ✅ Added regression coverage:
  - `difftool_mode::tests::run_difftool_directory_diff_rejects_symlink_cycles` (Unix-only).

Verification scope (this iteration):
- ✅ `cargo test -p gitgpui-app --no-default-features --features gix` (all unit and integration suites passing).

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified, including hardened symlink edge-case handling in directory diff staging.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 8, TrustExitCode Fallback Hardening — March 2, 2026)

Implemented this iteration:
- ✅ External mergetool trust-exit-code parity hardening in `crates/gitgpui-git-gix/src/repo/mergetool.rs`: `resolve_mergetool_config` now resolves trust semantics in precedence order `mergetool.<tool>.trustExitCode` → `mergetool.trustExitCode` → default `false`.
- ✅ Added targeted regression tests for precedence and fallback:
  - `test_resolve_mergetool_config_trust_exit_code_falls_back_to_global_setting`
  - `test_resolve_mergetool_config_tool_specific_trust_exit_overrides_global`

Verification scope (this iteration):
- ✅ `cargo test -p gitgpui-git-gix`
- ✅ `cargo test --workspace --no-default-features --features gix`
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified, including trust-exit-code fallback handling for internal mergetool backend configuration parity.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 8 — March 2, 2026)

Independent verification audit by a fresh agent confirms all design document components remain fully implemented. No new components to add, no gaps found.

Implemented this iteration:
- ✅ Fixed clippy `collapsible_if` lint in `crates/gitgpui-app/src/cli.rs`: collapsed nested `if` in `normalize_mergetool_args` empty-base handling into a single compound condition with `let`-chain, satisfying the `collapsible_if` lint under `-D warnings`.

Verification scope (this iteration):
- ✅ `cargo test --workspace --no-default-features --features gix`: **1103 passed, 0 failed, 5 ignored** (6 new tests since iteration 7).
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`: **0 warnings** (fixed collapsible_if regression).
- ✅ Conducted parallel deep audit of all major subsystems via three independent exploration agents:
  - **External usage design audit**: Verified all CLI subcommands (`difftool`, `mergetool`, `setup`) with full flag sets, env fallbacks, KDiff3/Meld compatibility parsing, exit code policy (0/1/≥2), setup mode config generation, and all 10 behavior matrix items. All FULLY IMPLEMENTED.
  - **Reference test portability audit**: Cross-referenced every Phase (1A–5C) from `REFERENCE_TEST_PORTABILITY.md` against actual test names and files — every planned test is present and passing, with many phases exceeding planned counts (e.g., Phase 4A: 63 tests vs 20 planned, Phase 4B: 26 vs 6 planned, Phase 5: 32 vs 19 planned).
  - **Code quality audit**: Searched for TODO/FIXME/HACK, `unimplemented!()`, `todo!()`, unsafe `unwrap()`, dead code, and robustness issues in all 7 production crates. **No issues found** — all production code is clean with no markers, no panicking patterns, and proper error handling throughout.
- ✅ Verified `parse_reflog_index` in `gitgpui-git-gix/src/util.rs` is safe despite string slicing — `?` operators guarantee valid bounds before any slice operation.

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 7 — March 2, 2026)

Implemented this iteration:
- ✅ External tool compatibility hardening for no-base merge invocations that pass an explicit empty base argument (`--base ""` / `--base=`):
  - `crates/gitgpui-app/src/cli.rs`: `resolve_mergetool_with_env` now treats empty explicit base as `None` (no-base) instead of failing validation.
  - `crates/gitgpui-app/src/cli.rs`: added pre-parse normalization so `mergetool --base ""` no longer fails in clap before resolver logic runs.
  - Added regression tests:
    - `cli::tests::mergetool_empty_base_flag_treated_as_missing`
    - `cli::tests::parse_mode_mergetool_drops_empty_base_value_before_clap`
    - `cli::tests::parse_mode_mergetool_drops_empty_attached_base_value_before_clap`
    - `standalone_mergetool_empty_base_flag_treated_as_no_base`

Verification scope (this iteration):
- ✅ `cargo test -p gitgpui-app --no-default-features --features gix` (all unit + integration suites passing).

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified, including robust handling of empty-base external invocations from shell-expanded git tool commands.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 6 — March 2, 2026)

Implemented this iteration:
- ✅ Closed a remaining Phase 2B portability gap in the KDiff3-style fixture harness: expected-result files are now optional, matching the design contract ("compare against expected when present").
  - `crates/gitgpui-core/tests/merge_fixture_harness.rs` now discovers fixtures when `{base,contrib1,contrib2}` exist, regardless of `*_expected_result.*`.
  - Harness execution now always runs invariants and only performs golden/alignment comparisons when expected data exists.
  - Added regression tests:
    - `discover_fixtures_includes_cases_without_expected_result`
    - `run_fixture_without_expected_result_succeeds`
    - `actual_result_path_without_expected_uses_base_directory`

Verification run:
- ✅ `cargo test -p gitgpui-core --test merge_fixture_harness`
- ✅ `cargo test -p gitgpui-core`

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures + optional expected-result support).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 6, Independent Verification — March 2, 2026)

Independent verification audit by a fresh agent confirms all design document components remain fully implemented. No new components to add, no gaps found.

Verification scope (this iteration):
- ✅ `cargo test --workspace --no-default-features --features gix`: **1097 passed, 0 failed, 5 ignored** (unchanged from iteration 5).
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`: **0 warnings**.
- ✅ Conducted parallel deep audit of all major subsystems via three independent exploration agents:
  - **CLI & tool modes audit**: Verified all CLI subcommands (`difftool`, `mergetool`, `setup`) with full flag sets, env fallbacks, KDiff3/Meld compatibility parsing, exit code policy (0/1/≥2), and setup mode config generation. All FULLY IMPLEMENTED.
  - **Core merge & reference test audit**: Verified merge algorithm (3-way, conflict styles, strategies, zealous coalescing, CRLF, binary detection), all Phase 1A–5C test suites against `REFERENCE_TEST_PORTABILITY.md` requirements. All FULLY IMPLEMENTED.
  - **E2E test & GUI audit**: Verified test counts (63 mergetool + 26 difftool + 37 standalone = 126 E2E integration tests), test coverage of all behavior matrix categories, and confirmed GUI focused windows are real GPUI implementations (1,185 + 349 lines, not stubs).
- ✅ Searched for TODO/FIXME/HACK markers in production code: **none found** (all matches are in vendor crates or test data string literals).
- ✅ Searched for `unimplemented!()` / `todo!()` in production code: **none found** (all matches are in vendor crates or test mock trait stubs).
- ✅ Verified CI configuration (`.github/workflows/rust.yml`) properly gates all phases: clippy, build, merge-algorithm (Phase 1A/1B/1C/5), merge-regression (Phase 2/3A/3C), tool-integration (Phase 4A/4B), and backend-integration.

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 5, Verification Refresh — March 2, 2026)

Independent implementation audit for this loop found no remaining unimplemented components from either design document.

Verification run (this iteration):
- ✅ `cargo test --workspace --no-default-features --features gix` passed (all tests green, including difftool/mergetool E2E and portability suites).
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings` passed (0 warnings).

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + invariants + seed fixtures).
- ✅ Phase 3A–3C complete (permutation corpus + real-world merge extraction).
- ✅ Phase 4 complete (t7610/t7800 mergetool+difftool E2E parity).
- ✅ Phase 5 complete (Meld-derived algorithm tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 5 — March 2, 2026)

Independent verification audit: all design document components remain fully implemented. No new components to add.

Verification scope:
- ✅ Cross-referenced every Phase (1A–5C) from `REFERENCE_TEST_PORTABILITY.md` and every section from `external_usage.md` against actual test names and source files — no gaps found.
- ✅ Verified GUI interactive mode is fully implemented (not stubbed): `focused_merge.rs` (1,185 lines, 21 tests) and `focused_diff.rs` (349 lines, 3 tests) provide complete GPUI conflict resolution and diff windows.
- ✅ Verified CI configuration (`.github/workflows/rust.yml`) runs headless tests with `--no-default-features --features gix`.
- ✅ Core source code (`gitgpui-core`, `gitgpui-app`, `gitgpui-ui-gpui`, `gitgpui-git-gix`) contains zero TODO/FIXME/HACK markers. The `unimplemented!()` calls in `gitgpui-state/src/store/tests.rs` are standard test mock trait stubs — not production gaps.
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`: **0 warnings**
- ✅ `cargo test --workspace --no-default-features --features gix`: **1097 passed, 0 failed, 5 ignored**
  - Ignored tests (intentional): exhaustive 161K permutation corpus, external repo extraction, fixture generation utility, 2 perf benchmarks in ui-gpui.

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + seed fixtures + invariants).
- ✅ Phase 3A–3B complete (permutation corpus generation + invariants).
- ✅ Phase 3C complete (production extraction module + integration tests using public API, hardened against fixture-stem collisions).
- ✅ Phase 4 complete (t7610/t7800 mergetool/difftool E2E parity suites).
- ✅ Phase 5 complete (Meld-derived matcher/interval/newline behavior tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 4, Follow-up — March 2, 2026)

Implemented this iteration:
- ✅ Phase 3C fixture-generation hardening in `crates/gitgpui-core/src/merge_extraction.rs`: `write_fixture_files` now disambiguates colliding sanitized fixture stems (for example, `src/a-b.txt` vs `src/a/b.txt`) with deterministic numeric suffixes, preventing silent fixture overwrites during real-world merge corpus extraction.
- ✅ Added regression coverage in `merge_extraction::tests::write_fixture_files_disambiguates_colliding_sanitized_paths`.

Verification run:
- ✅ `cargo test -p gitgpui-core`
- ✅ `cargo test --workspace --no-default-features --features gix`: **1097 passed, 0 failed, 5 ignored**
- ✅ `cargo clippy -p gitgpui-core -- -D warnings`

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + seed fixtures + invariants).
- ✅ Phase 3A–3B complete (permutation corpus generation + invariants).
- ✅ Phase 3C complete (production extraction module + integration tests using public API, now hardened against fixture-stem collisions).
- ✅ Phase 4 complete (t7610/t7800 mergetool/difftool E2E parity suites).
- ✅ Phase 5 complete (Meld-derived matcher/interval/newline behavior tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 4 — March 2, 2026)

Independent verification audit: all design document components remain fully implemented.

Implemented this iteration:
- ✅ Refactored `crates/gitgpui-core/tests/merge_git_extraction.rs` to use the production `merge_extraction` module API instead of duplicate local implementations. This eliminates ~240 lines of duplicated extraction/git helper code (`discover_merge_commits`, `extract_merge_cases`, `write_fixtures`, `ExtractedMerge` struct, `git_output`, `git_bytes`), fixes a sanitization inconsistency between the test file and the module, and ensures the public API is integration-tested alongside merge algorithm invariant checks.
- ✅ All 8 extraction tests now exercise the production `merge_extraction::discover_merge_commits`, `extract_merge_cases`, `extract_merge_cases_from_repo`, and `write_fixture_files` APIs, confirming real integration coverage.
- ✅ The `generate_fixtures_from_repo` utility test now uses `extract_merge_cases_from_repo` + `write_fixture_files` for consistent fixture naming.

Verification run:
- ✅ `cargo test --workspace --no-default-features --features gix`: **1096 passed, 0 failed, 5 ignored**
- ✅ `cargo clippy --workspace --no-default-features --features gix -- -D warnings`: **0 warnings**
- ✅ Cross-referenced every Phase (1A–5C) from `REFERENCE_TEST_PORTABILITY.md` and every section from `external_usage.md` against actual test names and source files — no gaps found.

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ All components implemented and verified. No remaining gaps.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + seed fixtures + invariants).
- ✅ Phase 3A–3B complete (permutation corpus generation + invariants).
- ✅ Phase 3C complete (production extraction module + integration tests using public API).
- ✅ Phase 4 complete (t7610/t7800 mergetool/difftool E2E parity suites).
- ✅ Phase 5 complete (Meld-derived matcher/interval/newline behavior tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 3 — March 2, 2026)

Implemented this iteration:
- ✅ Ported Phase 3C extraction flow into production core code via new module `crates/gitgpui-core/src/merge_extraction.rs`.
- ✅ Added reusable data models and APIs: `MergeCommit`, `ExtractedMergeCase`, `MergeExtractionOptions`, `discover_merge_commits`, `extract_merge_cases`, `extract_merge_cases_from_repo`, and `write_fixture_files`.
- ✅ Added robust error handling (`MergeExtractionError`) and deterministic fixture naming/sorting for stable outputs.
- ✅ Added unit coverage for merge discovery, text-only extraction with binary skipping, and fixture writing semantics (including preserving existing expected-result files).

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ Dedicated difftool/mergetool CLI modes, validation, runtime behavior, and git-invoked E2E parity are implemented and verified.
- ✅ Behavior matrix coverage remains complete (spaces/unicode, subdir, no-base, binary/non-UTF8, delete/delete, symlink, submodule, CRLF, dir-diff, cancel/exit semantics).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A–1C complete (t6403 core merge behavior, t6427 zdiff3, label formatting).
- ✅ Phase 2 complete (KDiff3-style fixture harness + seed fixtures + invariants).
- ✅ Phase 3A–3B complete (permutation corpus generation + invariants, including sampled default and exhaustive ignored run).
- ✅ Phase 3C now includes production-grade extraction support in `gitgpui-core` (previously test-harness-only), plus existing extraction integration tests.
- ✅ Phase 4 complete (t7610/t7800 parity via mergetool/difftool integration suites).
- ✅ Phase 5 complete (Meld-derived matcher/interval/newline behavior tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Verification run (this iteration):
- ✅ `cargo test -p gitgpui-core`
- ✅ `cargo clippy -p gitgpui-core -- -D warnings`

### Progress Snapshot (Iteration 2, Follow-up — March 2, 2026)

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ Iteration 2 component implemented: fixed `difftool --dir-diff` handling when Git provides symlink-backed directory entries (common in `git difftool --dir-diff` temp directories). `crates/gitgpui-app/src/difftool_mode.rs` now stages directory inputs into a temporary workspace and dereferences symlinked file entries before running `git diff --no-index`, so file-content deltas are shown instead of symlink-mode (`120000`) noise.
- ✅ Added regression coverage:
  - `crates/gitgpui-app/tests/difftool_git_integration.rs`: `git_difftool_dir_diff_handles_spaced_unicode_path`
  - `crates/gitgpui-app/src/difftool_mode.rs`: `run_difftool_directory_diff_dereferences_symlinked_files` (unit)
- ✅ Verification run: `cargo test -p gitgpui-app --no-default-features --features gix` passed (153 unit tests + 26 difftool integration tests + 63 mergetool integration tests + 37 standalone-tool integration tests).
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ No new portability gaps found in this iteration; previous Phase 1A–5C coverage remains complete.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### Progress Snapshot (Iteration 2 — March 2, 2026)

Full independent verification audit confirms all design document components are implemented.

- **Test suite**: 1091 passed, 0 failed, 5 ignored (3 intentionally deferred: exhaustive 161K permutation corpus, external repo extraction, fixture generation utility; 2 in vendor gpui crate)
- **Clippy**: clean (zero warnings in CI mode)
- **Audit scope**: cross-referenced every Phase (1A–5C) from `REFERENCE_TEST_PORTABILITY.md` and every section from `external_usage.md` against actual test names and source files
- **Verified test files and counts**:
  - `merge_algorithm.rs`: 41 tests (t6403 core + zdiff3 + histogram + extras)
  - `meld_algorithm_tests.rs`: 32 tests (Phase 5A/5B/5C + extras)
  - `mergetool_git_integration.rs`: 63 tests (Phase 4A E2E)
  - `difftool_git_integration.rs`: 25 tests (Phase 4B E2E)
  - `standalone_tool_mode_integration.rs`: 37 tests (standalone CLI E2E)
  - `merge_fixture_harness.rs`: 11 tests (Phase 2 harness + seed cases)
  - `merge_permutation_corpus.rs`: 1+1 tests (Phase 3A/3B, 243 sampled cases)
  - `merge_git_extraction.rs`: 8+2 tests (Phase 3C real-world extraction)
  - `conflict_label_formatting.rs`: 5 tests (Phase 1C labels)
  - `focused_merge.rs`: 20 unit tests (GUI merge window)
  - `focused_diff.rs`: 3 unit tests (GUI diff window)
  - `cli.rs`: 152 unit tests (arg parsing, validation, compat)
  - `mergetool_mode.rs`: 35 unit tests (runtime merge logic)
  - State management: 134 tests (reducers, conflict session, effects)
- **No remaining gaps**: all ✅, no 🔧 or ⬜ items

### Progress Snapshot (Iteration 1 — March 2, 2026)

External Diff/Merge Usage Design (`external_usage.md`)
- ✅ Dedicated CLI modes, robust arg/env validation, and no-subcommand KDiff3/Meld compatibility parsing are implemented.
- ✅ Git integration parity is implemented and covered for trust-exit semantics, GUI selection (`guiDefault` + `--gui/--no-gui`), `--tool-help`, path overrides (`*.path`), subdir/pathspec flows, spaced+Unicode paths, `--dir-diff`, and edge cases (no-base, delete/delete, symlink, submodule, deleted output).
- ✅ Focused runtime modes are implemented: `difftool` (`git diff --no-index --no-ext-diff` wrapper) and `mergetool` (built-in 3-way merge, conflict-style/diff-algorithm/marker-size controls, binary/non-UTF8 handling, CRLF preservation, autosolve heuristics).
- ✅ Setup automation is implemented (`gitgpui-app setup`), including headless+GUI tool entries, trust keys, prompt defaults, and shell-safe dry-run quoting.
- ✅ Iteration 1 component implemented: added standalone compatibility-mode invalid-invocation E2E coverage in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs` for strict validation paths (`--auto` without `-o/--output/--out`, diff-mode `--base` rejection, label overflow rejection, and too-many diff positionals), all asserting actionable stderr and exit code `2`.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

Reference Test Portability Plan (`docs/REFERENCE_TEST_PORTABILITY.md`)
- ✅ Phase 1A complete: core `t6403` portability coverage (identity, conflict markers, strategies, EOF/newline handling, CRLF markers, zealous coalescing, marker width, diff algorithm behavior, binary rejection).
- ✅ Phase 1B complete: all 4 `t6427` `zdiff3` portability cases.
- ✅ Phase 1C complete: conflict label formatting scenarios and runtime fallback behavior.
- ✅ Phase 2 complete: KDiff3-style fixture harness with auto-discovery, invariants, expected-result comparison, and `*_actual_result` write-on-failure behavior.
- ✅ Phase 3 complete: permutation corpus generation/invariant checks (sampled + ignored exhaustive) and real-world merge extraction harness.
- ✅ Phase 4 complete: critical `t7610`/`t7800` mergetool+difftool E2E parity suites.
- ✅ Phase 5 complete: Meld-derived matcher/interval/newline portability suites.
- ✅ Iteration 1 portability hardening: process-level tests now enforce strict compatibility-argument rejection behavior expected by Meld/KDiff3-style invocation contracts.
- 🔧 Partially implemented components: none.
- ⬜ Not-yet-started components: none.

### External Diff/Merge Usage Design (`external_usage.md`)

- ✅ CLI subcommands and argument model (`gitgpui-app difftool`, `gitgpui-app mergetool`) implemented in `crates/gitgpui-app/src/cli.rs`.
- ✅ Arg/env resolution + validation implemented for `LOCAL`, `REMOTE`, `MERGED`, `BASE`, labels, missing-input and missing-path errors. `MERGED` is treated as an output target and can be non-existent at parse time. Difftool display-path fallback now follows `--path` > `MERGED` > `BASE`.
- ✅ Arg/env validation hardening for empty values implemented in `crates/gitgpui-app/src/cli.rs`: required paths reject empty inputs early, optional display-path env vars ignore empty strings, env-provided empty `BASE` is treated as no-base (add/add-compatible), and explicit empty `--base` errors with actionable text.
- ✅ Mergetool file-path kind validation hardening implemented in `crates/gitgpui-app/src/cli.rs`: `LOCAL`/`REMOTE`/explicit `BASE` must resolve to files (not directories), and existing directory `MERGED` targets are rejected up front with actionable errors.
- ✅ Difftool path-kind validation hardening implemented in `crates/gitgpui-app/src/cli.rs`: mixed file-vs-directory inputs are rejected early with actionable errors (must be two files or two directories), covered by `cli.rs` unit test `difftool_rejects_file_vs_directory_mismatch` and standalone E2E `standalone_difftool_file_directory_mismatch_exits_two`.
- ✅ Mergetool compatibility aliases implemented in `crates/gitgpui-app/src/cli.rs`:
  - `-o`/`--output`/`--out` as aliases for `--merged`
  - `--L1`/`--L2`/`--L3` as aliases for `--label-base`/`--label-local`/`--label-remote`
  - `--auto-merge` as an alias for `--auto` in dedicated `mergetool` subcommand mode (Meld-style parity for direct invocation)
  - coverage: parser unit tests + git-invoked integration test (`git_mergetool_accepts_kdiff3_alias_flags_in_cmd`) + attached-form compatibility regression coverage for `--base=<...>` and `--out=<...>` in parser and standalone E2E tests (`compat_parses_kdiff3_style_mergetool_with_attached_output_and_base_flags`, `standalone_compat_mergetool_accepts_attached_output_and_base_flags`)
- ✅ KDiff3/Meld-style no-subcommand compatibility parser implemented in `crates/gitgpui-app/src/cli.rs`:
  - accepts direct external-tool invocation with positional paths and compatibility flags (`--auto`, `--auto-merge`, `--L1/--L2/--L3`, Meld-style `-L/--label`, `--base`, `-o/--output/--out`)
  - maps to validated `difftool`/`mergetool` app modes
  - enforces strict compatibility validation with actionable errors (`--auto`/`--auto-merge` output-path requirements, merge positional count checks, merge-mode `--L3` requires `BASE` in compatibility mode, `--base` conflict guards, diff-mode `--L3`/`--base` rejection, too-many-positional checks, and label-arity overflow rejection)
  - coverage: CLI parser tests (including Meld `-L/--label` diff+merge cases, attached-form `-L<label>` / `--label=<label>` difftool parsing, and over-arity rejection) + git-invoked `kdiff3`/`meld` path-override integration tests + standalone no-subcommand attached-label difftool E2E coverage
- ✅ Exit code constants aligned to design (`0`, `1`, `>=2`) defined in app CLI module.
- ✅ Conflict-marker label formatter and runtime integration implemented: `crates/gitgpui-core/src/conflict_labels.rs` provides `empty tree`/`<short-sha>:<path>`/merged-ancestors/rebase-parent formatting, and `crates/gitgpui-app/src/mergetool_mode.rs` now applies filename/`empty tree` fallback labels in dedicated mergetool flows.
- ✅ Focused GPUI tool windows for interactive diff/merge (`--gui` flag):
  - ✅ `focused_diff.rs` in `gitgpui-ui-gpui`: color-coded unified diff viewer with keyboard navigation (Esc/q/Ctrl+W to close), line classification (Header, HunkHeader, Add, Remove, Context), and 3 unit tests.
  - ✅ `focused_merge.rs` in `gitgpui-ui-gpui`: interactive merge conflict resolution window with conflict-marker parsing, per-segment pick buttons (Ours/Theirs/Base/Both via a/b/c/d keys), conflict navigation (F2/F3), auto-resolve action, save/cancel (Ctrl+S/Esc), live output preview, and 11 unit tests.
  - ✅ `--gui` CLI flag added to both `difftool` and `mergetool` subcommands (opt-in, defaults to false for test compatibility).
  - ✅ Headless build guard: when `ui-gpui` is not compiled, `--gui` in `difftool`/`mergetool` now returns exit `2` with actionable rebuild guidance instead of silently running in non-GUI mode.
  - ✅ Wired in `main.rs`: difftool `--gui` opens focused diff window after successful diff; mergetool `--gui` opens focused merge window when conflicts remain (non-auto mode).
  - ✅ Exit code contract preserved: focused merge returns 0 only for resolved saves, 1 for cancel/unresolved saves, and 2 on save I/O error; focused diff always returns 0.
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
- ✅ Git behavior parity matrix coverage is complete. All items covered: spaced and Unicode paths, invocation from subdirectories, pathspec-targeted invocations, no-base handling for stage extraction (including empty `BASE` file for add/add), binary/non-UTF8 content handling in both difftool and mergetool flows, trust-exit semantics, deleted output handling, writeToTemp path semantics, difftool `--dir-diff`, difftool `guiDefault` selection (`true`/`false`/`auto` + `DISPLAY`, `--gui`, `--no-gui`), difftool `--tool-help` discoverability, mergetool `guiDefault` selection (`true`/`false`/`auto` + `DISPLAY`, `--gui`, `--no-gui`), mergetool `--tool-help` discoverability, mergetool GUI fallback (no guitool → merge.tool), nonexistent tool error handling (both invalid command and missing `*.cmd`), delete/delete conflict handling, modify/delete conflict handling, symlink conflict resolution (l/r/a prompts, coexistence with normal file conflicts, difftool target diff), and submodule conflict handling (l/r/a prompt semantics, coexistence with normal file conflicts, file-vs-submodule, directory-vs-submodule, deleted-vs-modified submodule, submodule in subdirectory).
- ✅ Git-like scenario porting is complete. All listed t7610/t7800 parity items are covered: `trustExitCode`, custom cmd (`cat "$REMOTE" > "$MERGED"` + braced env variants), gui preference, writeToTemp/keepTemporaries, keepBackup delete/delete, no-base stage-file contract, difftool gui-default/trust/tool-help parity, mergetool gui-default/trust/tool-help parity, GUI fallback, nonexistent tool error (invalid command and missing `*.cmd`), delete/delete, modify/delete, pathspec-targeted runs, order-file invocation ordering (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), and submodule conflicts (l/r/a prompt semantics, deleted-vs-modified, file-vs-submodule, directory-vs-submodule, subdirectory submodule, coexistence with normal files).
- ✅ Dedicated difftool mode tests are implemented with parity-focused coverage:
  - ✅ Runtime/unit coverage in `crates/gitgpui-app/src/difftool_mode.rs` (identical files, changed files with exit normalization, display-path and explicit labels, missing-input error handling, directory diff, binary content, and non-UTF8 content).
  - ✅ Full git-invoked integration coverage in `crates/gitgpui-app/tests/difftool_git_integration.rs` (basic invocation, spaced and Unicode paths, subdirectory invocation, pathspec filtering, `--dir-diff`, `guiDefault` true/false/auto + `--gui`/`--no-gui` selection precedence, trust-exit-code matrix, `--tool-help` discoverability, symlink target diff, submodule gitlink-pointer diff, binary content, and non-UTF8 content).
- ✅ End-to-end tests that invoke `git difftool`/`git mergetool` with global-like config and `gitgpui-app` as the tool are fully implemented:
  - ✅ `git difftool` E2E in `crates/gitgpui-app/tests/difftool_git_integration.rs` (23 tests, including pathspec filtering parity, explicit `difftool.guiDefault` true/false/auto matrix coverage, submodule gitlink-pointer diff coverage, missing-tool `--tool absent` diagnostic parity, and both `kdiff3`/`meld` path-override compatibility invocation).
  - ✅ `git mergetool` E2E in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (58 tests): overlapping conflict processing, explicit custom command parity (`cat "$REMOTE" > "$MERGED"`), trust-exit-code semantics (clean merge resolved / conflict preserved), deleted-output resolution parity (`rm -f "$MERGED"` with trusted exit), no-trust exit behavior (unchanged output stays unresolved, changed output resolves), spaced and Unicode path handling, subdirectory invocation, pathspec-targeted invocation parity, add/add (no-base) conflict + explicit empty-`BASE` stage-file contract assertion, multiple conflicted files, CRLF preservation, `--tool-help` discoverability, `guiDefault` true/false/auto selection (with/without DISPLAY), `--gui` and `--no-gui` flag overrides, GUI fallback when no guitool configured, nonexistent tool error handling (invalid command + missing command config), delete/delete conflict, delete/delete with keepBackup=true (no-error parity), delete/delete abort with `keepTemporaries=true` stage-file retention parity, modify/delete conflict, explicit `mergetool.writeToTemp` `true`/`false` stage-path-shape assertions, invocation ordering parity (`diff.orderFile` and `-O` override), symlink conflicts (l/r resolution, coexistence with normal files), submodule conflicts (l/r/a prompt semantics, deleted-vs-modified, file-vs-submodule, directory-vs-submodule, subdirectory submodule, coexistence with normal files), and `kdiff3`/`meld` path-override compatibility invocation.
- ✅ Direct standalone command-mode E2E coverage for `gitgpui-app` subcommands is implemented in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - ✅ `mergetool` clean merge exits `0` and writes merged output
  - ✅ `mergetool` unresolved conflict exits `1` and writes conflict markers
  - ✅ `mergetool` CRLF conflict markers preserve `\r\n` endings in standalone conflict output
  - ✅ `mergetool` invalid input exits `2` with actionable validation error text
  - ✅ `mergetool` rejects existing directory `MERGED` targets with exit `2` and actionable error text
  - ✅ `mergetool` direct invocation handles Unicode file paths end-to-end
  - ✅ `difftool` changed-file invocation exits `0` and emits unified diff output
  - ✅ `difftool` binary content changes exit `0` and emit binary-diff output
  - ✅ `difftool` non-UTF8 content changes exit `0` and emit non-empty diff output
  - ✅ `difftool` direct invocation handles Unicode file names and display paths
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
  - ✅ nonexistent tool error: both invalid-command and missing-command-config (`--tool absent`) paths report actionable failures
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
  - ✅ full E2E via `git mergetool` command in `crates/gitgpui-app/tests/mergetool_git_integration.rs` (58 tests, including explicit add/add no-base stage-file assertions, `guiDefault` true/false/auto matrix coverage, pathspec filtering parity, binary file conflict handling, explicit custom-command parity, deleted-output resolution parity (`rm -f "$MERGED"` with trusted exit), delete/delete `keepTemporaries` abort parity, submodule `a` abort-path parity, `kdiff3`/`meld` path-override compatibility invocation, compatibility-mode git-config fallback parity, and missing-tool `--tool absent` diagnostic parity)
  - ✅ full E2E via `git difftool` command in `crates/gitgpui-app/tests/difftool_git_integration.rs` (23 tests, including `guiDefault` true/false/auto matrix coverage, pathspec filtering parity, `kdiff3` + `meld` path-override compatibility invocation, missing-tool `--tool absent` diagnostic parity, submodule gitlink-pointer diff coverage, plus binary and non-UTF8 content coverage)
- ✅ Phase 4B (critical `t7800-difftool` E2E): implemented in `crates/gitgpui-app/tests/difftool_git_integration.rs`.
  - ✅ Foundational difftool runtime with Git-compatible exit semantics and label/display-path handling.
  - ✅ Git-invoked E2E coverage for basic invocation, subdirectory execution, pathspec filtering, spaced path handling, `--dir-diff`, binary content, and non-UTF8 content.
  - ✅ Explicit `difftool.guiDefault` selection-path parity (`true`/`false`/`auto` with/without `DISPLAY`, `--gui`, `--no-gui`).
  - ✅ Dedicated trust-exit interaction matrix assertions (`difftool.trustExitCode`, `--trust-exit-code`, `--no-trust-exit-code`).
- ✅ `git difftool --tool-help` discoverability assertion for configured `gitgpui` tool.

### Latest Component Delivered (Iteration 25) — Defensive Marker Serialization Hardening

- **Defensive fix** in [`crates/gitgpui-ui-gpui/src/focused_merge.rs`](crates/gitgpui-ui-gpui/src/focused_merge.rs):
  - `render_unresolved_marker_block()` now inserts a newline before each marker line (`=======`, `|||||||`, `>>>>>>>`) when the preceding content section doesn't end with a newline character.
  - This prevents malformed conflict markers (e.g., `content=======\n`) when block content lacks trailing newlines.
  - Handles all three content sections independently: ours, base (diff3), and theirs.
  - CRLF-aware: uses the detected line ending (`\r\n` or `\n`) for inserted guards.
  - Empty content sections correctly skip the guard (no double-newline).
- Added 7 new edge-case unit tests in `focused_merge.rs` (21 total, up from 14):
  - `build_output_unresolved_content_without_trailing_newline` — 2-way markers remain well-formed
  - `build_output_unresolved_diff3_content_without_trailing_newline` — diff3 base section guard
  - `build_output_unresolved_crlf_content_without_trailing_newline` — CRLF detection + newline guard
  - `build_output_unresolved_empty_ours_and_theirs` — empty content sections
  - `build_output_unresolved_empty_base_section` — empty diff3 base section
  - `build_output_mixed_resolved_and_unresolved_blocks` — mixed resolution state output
  - `build_output_multiple_consecutive_unresolved_blocks` — adjacent unresolved blocks
- Verification:
  - `cargo test -p gitgpui-ui-gpui focused_merge` — 21 passed, 0 failed
  - `cargo clippy --workspace --no-default-features --features gix -- -D warnings` — clean
  - `cargo test --workspace --no-default-features --features gix` — 1087 passed, 0 failed, 5 ignored

### Previous Component Delivered (Iteration 67) — Fix CRLF Preservation in Autosolve Subchunk Splitting

- **Bug fix** in [`crates/gitgpui-core/src/conflict_session.rs`](crates/gitgpui-core/src/conflict_session.rs):
  - `lines_to_text` previously always appended `\n` when reconstructing text from split lines, silently converting CRLF files to LF when content went through the subchunk splitting autosolve path (`split_conflict_into_subchunks` → `per_line_merge` / `merge_line_hunks`).
  - Added `detect_subchunk_line_ending()` helper that detects the dominant line ending from input texts.
  - `lines_to_text` now takes a `line_ending` parameter, propagated from `split_conflict_into_subchunks` through `per_line_merge`, `merge_line_hunks`, and `side_content`.
- Added 9 unit tests in `conflict_session.rs`:
  - `subchunk_split_preserves_crlf_per_line_merge` — per-line merge path with CRLF content
  - `subchunk_split_preserves_crlf_diff_based_merge` — diff-based merge path with CRLF content
  - `autosolve_diff3_subchunk_split_preserves_crlf` — full autosolve pipeline with CRLF diff3 markers
  - `autosolve_crlf_identical_sides_preserves_endings` — identical-sides CRLF path
  - `autosolve_crlf_whitespace_only_diff_preserves_endings` — whitespace-only CRLF path
  - `detect_subchunk_line_ending_crlf_dominant` / `lf_dominant` / `mixed_prefers_majority` / `empty_defaults_to_lf`
- Added 1 standalone E2E test in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_mergetool_auto_crlf_subchunk_preserves_line_endings` — full `gitgpui-app mergetool --auto --conflict-style diff3` with CRLF files, verifying byte-level CRLF preservation in output.
- Verification:
  - `cargo test -p gitgpui-core --lib` — 181 passed, 0 failed
  - `cargo test -p gitgpui-app --no-default-features --features gix --test standalone_tool_mode_integration` — 30 passed, 0 failed
  - `cargo test --workspace --no-default-features --features gix` — all passed

### Previous Component Delivered (Iteration 66) — Standalone Behavior-Matrix Parity (CRLF/Binary/Non-UTF8)

- Added standalone mergetool CRLF regression coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_mergetool_conflict_markers_preserve_crlf_line_endings` verifies direct `gitgpui-app mergetool` conflict output preserves `\r\n` line endings in conflict markers.
- Added standalone difftool byte-content regression coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_difftool_binary_content_change_exits_zero` verifies binary-content diffs return exit `0` with binary-diff output.
  - `standalone_difftool_non_utf8_content_change_exits_zero` verifies non-UTF8 byte-content diffs return exit `0` with emitted output.
- Verification:
  - `cargo test -p gitgpui-app --no-default-features --features gix --test standalone_tool_mode_integration`

### Previous Component Delivered (Iteration 65) — Explicit Missing-Tool `--tool` Error Parity

- Added git-invoked mergetool regression coverage in [`crates/gitgpui-app/tests/mergetool_git_integration.rs`](crates/gitgpui-app/tests/mergetool_git_integration.rs):
  - `git_mergetool_absent_tool_reports_cmd_not_set_error` verifies `git mergetool --tool absent` fails with actionable `cmd not set for tool 'absent'` output when `mergetool.absent.cmd` is unset.
- Added git-invoked difftool regression coverage in [`crates/gitgpui-app/tests/difftool_git_integration.rs`](crates/gitgpui-app/tests/difftool_git_integration.rs):
  - `git_difftool_absent_tool_reports_cmd_not_set_error` verifies `git difftool --tool absent` fails with actionable `cmd not set for tool 'absent'` output when `difftool.absent.cmd` is unset.
- Verification:
  - `cargo test -p gitgpui-app --no-default-features --features gix --test mergetool_git_integration git_mergetool_absent_tool_reports_cmd_not_set_error -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test difftool_git_integration git_difftool_absent_tool_reports_cmd_not_set_error -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test mergetool_git_integration --test difftool_git_integration`

### Previous Component Delivered (Iteration 64) — Attached-Form KDiff3 Output/Base Compatibility Regression Coverage

- Hardened no-subcommand KDiff3 compatibility regression coverage in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - added `compat_parses_kdiff3_style_mergetool_with_attached_output_and_base_flags`, which validates attached `--base=<BASE>` and `--out=<MERGED>` parsing in merge compatibility mode with label aliases.
- Added standalone command-mode E2E regression coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_compat_mergetool_accepts_attached_output_and_base_flags` verifies attached-form flags resolve to a clean 3-way merge (`exit 0`) and write expected output bytes.
- Verification:
  - `cargo test -p gitgpui-app --no-default-features --features gix compat_parses_kdiff3_style_mergetool_with_attached_output_and_base_flags -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix standalone_compat_mergetool_accepts_attached_output_and_base_flags -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --bin gitgpui-app -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test standalone_tool_mode_integration -- --nocapture`

### Previous Component Delivered (Iteration 63) — Mergetool `--auto-merge` Subcommand Alias Parity

- Hardened dedicated `mergetool` CLI parsing in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - `--auto-merge` is now accepted as a direct alias for `--auto` on the `mergetool` subcommand (Meld-style compatibility for explicit subcommand invocation).
- Added parser regression coverage in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - `clap_parses_mergetool_auto_merge_alias_flag` verifies `--auto-merge` maps to `MergetoolArgs.auto = true`.
- Added standalone command-mode E2E regression coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_mergetool_auto_merge_alias_resolves_whitespace_conflict_exits_zero` verifies alias behavior end-to-end (`exit 0`, clean output).
- Verification:
  - `cargo test -p gitgpui-app --no-default-features --features gix clap_parses_mergetool_auto_merge_alias_flag -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix standalone_mergetool_auto_merge_alias_resolves_whitespace_conflict_exits_zero -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test standalone_tool_mode_integration -- --nocapture`

### Previous Component Delivered (Iteration 62) — Difftool Input-Kind Validation Hardening

- Hardened dedicated `difftool` argument validation in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - `resolve_difftool_with_env` now rejects mixed file-vs-directory inputs at parse time
  - error text is actionable and explicit: "Use two files or two directories."
- Added parser regression coverage in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - `difftool_rejects_file_vs_directory_mismatch`
- Added standalone command-mode E2E regression coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_difftool_file_directory_mismatch_exits_two` asserts exit code `2` and actionable stderr.
- Verification:
  - `cargo test -p gitgpui-app --no-default-features --features gix difftool_rejects_file_vs_directory_mismatch -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix standalone_difftool_file_directory_mismatch_exits_two -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test standalone_tool_mode_integration -- --nocapture`
  - `cargo test -p gitgpui-app --no-default-features --features gix --test difftool_git_integration -- --nocapture`

### Previous Component Delivered (Iteration 61) — State Crate CI Coverage

- Added `gitgpui-state` to CI workflow in `.github/workflows/rust.yml`:
  - Clippy step now includes `gitgpui-state` alongside `gitgpui-core` and `gitgpui-git-gix`
  - Build step now includes `gitgpui-state`
  - New "State management (conflict session, reducers, effects)" test step in the merge-algorithm job runs all 134 state crate tests
- This closes the gap where 134 headless-capable tests covering conflict session management, store reducers, effect pipelines, and repo monitoring were not gated in CI.
- No GPUI system dependencies required — `gitgpui-state` depends only on `gitgpui-core`, `globset`, `notify`, `smol`, `serde`, `serde_json`, and `rustc-hash`.
- Verification:
  - `cargo clippy -p gitgpui-core -p gitgpui-state -p gitgpui-git-gix -- -D warnings` — clean
  - `cargo test -p gitgpui-state --verbose` — 134 passed, 0 failed
  - `cargo test --workspace` — 1043 passed, 0 failed, 5 ignored

### Previous Component Delivered (Iteration 60) — Attached-Label Difftool Compatibility Regression Hardening

- Added explicit parser coverage for no-subcommand difftool attached label forms in [`crates/gitgpui-app/src/cli.rs`](crates/gitgpui-app/src/cli.rs):
  - `compat_parses_meld_style_difftool_attached_labels` validates `-LLEFT_LABEL` and `--label=RIGHT_LABEL` are correctly mapped to left/right pane labels.
- Added standalone end-to-end coverage in [`crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`](crates/gitgpui-app/tests/standalone_tool_mode_integration.rs):
  - `standalone_compat_difftool_accepts_attached_label_forms` invokes `gitgpui-app` in no-subcommand mode and asserts attached labels are rendered in diff headers.
- Verification:
  - `cargo test -p gitgpui-app compat_parses_meld_style_difftool_attached_labels`
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration standalone_compat_difftool_accepts_attached_label_forms`

### Previous Component Delivered (Iteration 59) — CI Workflow Portability Hardening

- Fixed CI workflow (`.github/workflows/rust.yml`) to work on ubuntu-latest without GPUI system dependencies:
  - All `gitgpui-app` jobs now build/test with `--no-default-features --features gix` (headless mode)
  - Extracted `APP_FEATURES` env var to avoid repetition across CI steps
  - Split clippy into separate core+backend and app steps for clear feature isolation
- Added `#[cfg]` feature gates in `crates/gitgpui-app/src/main.rs`:
  - `crashlog` module gated behind `#[cfg(feature = "ui")]` (only needed for interactive app)
  - `path_label` helper gated behind `#[cfg(feature = "ui-gpui")]` (only used by focused windows)
  - `std::path::Path` import removed (inlined in `path_label` signature)
- Verification:
  - `cargo clippy -p gitgpui-app --no-default-features --features gix -- -D warnings` — clean
  - `cargo clippy -p gitgpui-core -p gitgpui-app -p gitgpui-git-gix -- -D warnings` — clean
  - `cargo test -p gitgpui-app --no-default-features --features gix` — 246 passed, 0 failed
  - `cargo test --workspace` — 1041 passed, 0 failed, 5 ignored

### Previous Component Delivered (Iteration 58) — Focused Merge Exit-Code Policy Hardening

- Hardened focused merge save semantics in `crates/gitgpui-ui-gpui/src/focused_merge.rs`:
  - save now returns exit `0` only when all conflicts are resolved
  - save with unresolved conflicts now returns exit `1` (cancel/unresolved parity)
  - merged-output write failures now return exit `2` (I/O error parity)
- Added pure save-exit mapping helper and regression tests:
  - `saved_exit_code_clean_merge_is_success`
  - `saved_exit_code_all_conflicts_resolved_is_success`
  - `saved_exit_code_unresolved_conflicts_are_canceled`
- Verification:
  - `cargo test -p gitgpui-ui-gpui focused_merge` — 11 passed, 0 failed
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration` — 22 passed, 0 failed

### Previous Component Delivered (Iteration 57) — Focused Merge Malformed-Marker CRLF Preservation

- Hardened malformed-marker fallback in `crates/gitgpui-ui-gpui/src/view/conflict_resolver.rs`:
  - parser now preserves consumed marker text exactly in no-end-marker scenarios, including `|||||||` base sections and the original `=======` separator line text
  - removes synthetic `=======\n` insertion that could normalize CRLF and drop diff3 base-marker content
- Added focused regression coverage:
  - `malformed_missing_end_marker_crlf_preserved_as_text`
  - `malformed_diff3_missing_end_marker_preserved_as_text`
- This closes a remaining interactive-path gap for external behavior-matrix line-ending preservation in malformed conflict round trips.
- Verification:
  - `cargo test -p gitgpui-ui-gpui` — 207 passed, 0 failed, 2 ignored
  - `cargo test -p gitgpui-app --test standalone_tool_mode_integration` — 22 passed, 0 failed

### Previous Component Delivered (Iteration 56) — GUI Tool Variant in Setup Config

- Fixed `gitgpui-app setup` to register a separate `gitgpui-gui` tool variant whose commands include `--gui`:
  - `mergetool.gitgpui-gui.cmd` = mergetool command with `--gui` flag
  - `mergetool.gitgpui-gui.trustExitCode = true`
  - `difftool.gitgpui-gui.cmd` = difftool command with `--gui` flag
  - `difftool.gitgpui-gui.trustExitCode = true`
- Changed `merge.guitool` from `gitgpui` to `gitgpui-gui` and `diff.guitool` from `gitgpui` to `gitgpui-gui`.
- This closes the gap where `guiDefault=auto` + DISPLAY would previously select the same headless tool as no-display mode. Now:
  - With DISPLAY: git selects `gitgpui-gui` → opens focused GPUI window (interactive merge/diff)
  - Without DISPLAY: git selects `gitgpui` → runs headless algorithm-only merge/diff
- Updated `external_usage.md` recommended config to document separate headless and GUI tool entries.
- Added 4 new unit tests in `setup_mode.rs`:
  - `gui_tool_uses_separate_tool_name`
  - `gui_tool_cmd_includes_gui_flag`
  - `headless_tool_cmd_omits_gui_flag`
- Updated standalone E2E tests:
  - `setup_dry_run_prints_commands_without_writing`: now asserts GUI tool cmd/trust entries
  - `setup_dry_run_commands_execute_verbatim_in_shell`: now verifies GUI tool commands are shell-valid and `merge.guitool` = `gitgpui-gui`
  - `setup_local_writes_config_to_repo`: now verifies `gitgpui-gui` guitool name, GUI cmd with `--gui`, headless cmd without `--gui`, and GUI trust keys
- Verification:
  - `cargo test --workspace` — 1,036 passed, 0 failed, 5 ignored
  - `cargo clippy --workspace` — clean

### Previous Component Delivered (Iteration 54) — Focused GPUI Tool Windows

- Implemented `focused_diff.rs` in `crates/gitgpui-ui-gpui/src/`:
  - `FocusedDiffView` GPUI view with color-coded unified diff rendering
  - Line classification: Header (dim), HunkHeader (cyan), Add (green), Remove (red), Context (default)
  - Close actions: Esc, q, Ctrl+W — all exit with code 0
  - `run_focused_diff(FocusedDiffConfig) -> i32` entry point using `Application::run()` blocking event loop
  - 3 unit tests for diff line parsing
- Implemented `focused_merge.rs` in `crates/gitgpui-ui-gpui/src/`:
  - `FocusedMergeView` GPUI view for interactive 3-way merge conflict resolution
  - Parses conflict markers from merged text into context/conflict segments
  - Per-conflict pick buttons: Ours (a), Theirs (b), Base (c), Both (d)
  - Conflict navigation: F2 (prev), F3 (next) with wrap-around
  - Auto-resolve action attempts heuristic resolution of all remaining conflicts
  - Save (Ctrl+S) writes resolved output to MERGED path and exits 0
  - Cancel (Esc) exits 1 without writing
  - Live output preview panel shows current resolution state
  - `run_focused_merge(FocusedMergeConfig) -> i32` entry point with `Arc<AtomicI32>` exit code passing
  - 8 unit tests for output building, segment parsing, and helper functions
- Added `--gui` CLI flag to `DifftoolArgs` and `MergetoolArgs` in `crates/gitgpui-app/src/cli.rs`:
  - Opt-in flag (defaults to false) to preserve headless test compatibility
  - Propagated through `DifftoolConfig.gui` and `MergetoolConfig.gui` fields
- Wired focused windows in `crates/gitgpui-app/src/main.rs`:
  - Difftool: when `--gui` and diff output is non-empty, opens `FocusedDiffView` window
  - Mergetool: when `--gui`, conflicts remain, and not in `--auto` mode, opens `FocusedMergeView` window
  - Both gated behind `#[cfg(feature = "ui-gpui")]`
- Re-exported public API from `crates/gitgpui-ui-gpui/src/lib.rs`
- Changed `conflict_resolver` module visibility to `pub(crate)` in `view/mod.rs`
- Verification:
  - `cargo test --workspace` — 1,031 passed, 0 failed, 5 ignored
  - `cargo clippy --workspace` — clean

### Previous Component Delivered (Iteration 53) — Git Mergetool Deleted-Output E2E Parity

- Added `git_mergetool_trust_exit_code_deleted_output_resolves_conflict` in `crates/gitgpui-app/tests/mergetool_git_integration.rs`.
- The new test verifies Git-invoked external mergetool behavior when the tool resolves by deleting `$MERGED`:
  - configured command: `rm -f "$MERGED"; exit 0`
  - `mergetool.fake.trustExitCode=true`
  - assertions: command succeeds, no unresolved index entries remain, worktree file is removed, and staged deletion is present in porcelain status.
- This closes explicit app-level E2E coverage for the "deleted output (tool chooses deletion)" behavior-matrix item from `external_usage.md`.
- Verification:
  - `cargo test --offline -p gitgpui-app --test mergetool_git_integration -- --nocapture`
  - `cargo test --offline -p gitgpui-app`

### Previous Component Delivered (Iteration 52) — Mergetool Auto-Resolve Mode

- Implemented `try_autosolve_merged_text()` in `crates/gitgpui-core/src/conflict_session.rs`:
  - Parses merged text with conflict markers into alternating context/conflict spans
  - For each conflict block, tries 5 heuristic passes: identical sides, single-side change (with base), whitespace-only normalization, subchunk splitting (line-level re-merge)
  - Returns clean text if ALL conflicts resolved, `None` otherwise
  - 8 unit tests covering: no-conflicts, identical sides, whitespace-only, diff3 single-side, diff3 subchunk, true conflict, partial resolve, multiple conflicts
- Added `--auto` CLI flag to `gitgpui-app mergetool` subcommand in `crates/gitgpui-app/src/cli.rs`:
  - Boolean flag that enables heuristic auto-resolve post-processing
  - Propagated through `MergetoolConfig.auto` field
  - Wired in KDiff3/Meld compatibility mode: `--auto`/`--auto-merge` → `config.auto = true`
  - 4 CLI parser tests (clap parsing, config propagation, compat `--auto`, compat `--auto-merge`)
- Wired autosolve into mergetool runtime in `crates/gitgpui-app/src/mergetool_mode.rs`:
  - After `merge_file_bytes` produces conflicts and `config.auto` is true, calls `try_autosolve_merged_text`
  - If autosolve succeeds: overwrites MERGED with clean output, exits 0 with "Auto-resolved" message
  - If autosolve fails: keeps original markers, exits 1
  - 5 runtime unit tests covering: whitespace resolve, diff3 subchunk resolve, true conflict, disabled mode, identical sides
- Added standalone E2E tests in `crates/gitgpui-app/tests/standalone_tool_mode_integration.rs`:
  - `standalone_mergetool_auto_resolves_whitespace_conflict_exits_zero`
  - `standalone_mergetool_auto_with_diff3_resolves_subchunk_exits_zero`
  - `standalone_mergetool_auto_unresolvable_conflict_exits_one`
  - `standalone_mergetool_without_auto_does_not_autosolve`
- Refactored `merged_display_name()` helper to reduce code duplication in mergetool output messages
- Verification:
  - `cargo test -p gitgpui-core -p gitgpui-app -p gitgpui-git-gix` — 619 passed, 0 failed
  - `cargo clippy -p gitgpui-core -p gitgpui-app -p gitgpui-git-gix -- -D warnings` — clean

### Previous Component Delivered (Iteration 49) — Difftool Submodule Gitlink E2E Parity

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
