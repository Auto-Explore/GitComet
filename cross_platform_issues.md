# Cross-Platform Issues Analysis

Date: 2026-03-07

This document summarizes the current cross-platform failures and what should be changed to support Linux, macOS, and Windows consistently.

## P0 (Fix First)

### 1) Repository-dependent extraction test is not portable (Fedora failure) [DONE]

Symptoms:
- `extraction_discovers_merge_commits` fails with `NotGitRepository("/__w/GitComet/GitComet")`.

Evidence:
- Host-repo discovery in test: `crates/gitcomet-core/tests/merge_git_extraction.rs:192-198`.
- Hard expectation in test: `crates/gitcomet-core/tests/merge_git_extraction.rs:303-305`.
- Any `git rev-parse` failure is mapped to `NotGitRepository`: `crates/gitcomet-core/src/merge_extraction.rs:384-390`.

What to change:
- Make `extraction_discovers_merge_commits` hermetic (create a temp repo with at least one merge commit, like other tests in the same file).
- If a host-repo test is kept, skip cleanly when repository discovery or repo validation fails.
- Preserve `git` stderr in `ensure_git_repository` failures so safe-directory and environment issues are diagnosable instead of being reported only as `NotGitRepository`.

---

### 2) Windows test build breaks on Unix-only APIs [DONE]

Symptoms:
- Windows compile error: `could not find unix in os` in `difftool_git_integration.rs`.

Evidence:
- Unix-only symlink usage in Windows-compiled test: `crates/gitcomet-app/tests/difftool_git_integration.rs:1054,1059`.
- Same pattern also exists in mergetool tests and will fail next: `crates/gitcomet-app/tests/mergetool_git_integration.rs:2100,2105,2110,2146,2151,2156,2191,2197,2203`.

What to change:
- For symlink-specific tests, either:
  - Gate with `#[cfg(unix)]`, or
  - Add a cross-platform helper (`unix::symlink`, `windows::symlink_file/symlink_dir`) and skip gracefully when Windows symlink creation is unavailable.
- Keep Windows CI compiling all integration tests after this fix to catch the next platform-specific compile break early.

---

### 3) Linux X11 UI job is missing native link dependencies [DONE]

Symptoms:
- Linker fails with missing `-lxcb`, `-lxkbcommon`, `-lxkbcommon-x11` in Linux display-profile job.

Evidence:
- UI smoke test runs in `linux-display-profiles` without installing X11 dev libs: `.github/workflows/cross-platform-tests.yml:76-120`.

What to change:
- Install required native packages before UI smoke test (at minimum the libs reported by the linker).
- Add a preflight check (for example via `pkg-config`) and fail with a clear dependency message when missing.
- Keep headless jobs unchanged; only display-profile jobs need these deps.

## P1 (Stability and Behavior Parity)

### 4) macOS mergetool integration tests rely on Git behavior that varies by version [DONE]

Symptoms:
- Multiple macOS failures in mergetool path-override and `trustExitCode=false` scenarios:
  - `git_mergetool_kdiff3_path_override_*`
  - `git_mergetool_meld_path_override_*`
  - `git_mergetool_no_trust_exit_code_changed_output_resolves_conflict`
  - conflictstyle fallback tests

Evidence:
- No-trust tests and assumptions: `crates/gitcomet-app/tests/mergetool_git_integration.rs:1006-1044`.
- Compat path-override tests: `crates/gitcomet-app/tests/mergetool_git_integration.rs:515-620`.

What to change:
- Stop depending on one exact Git implementation detail for `trustExitCode=false` + non-zero tool exit.
- In tests that only verify argument parsing/mapping, use tool commands that exit `0` after writing output to remove Git-version-specific ambiguity.
- Add `git --version` output in each OS CI job to correlate behavior with Git version.

---

### 5) KDiff3 compat parser likely misses some invocation forms used on macOS [DONE]

Symptoms:
- KDiff3 path-override tests fail on macOS while Linux passes.

Evidence:
- Parser explicitly supports `--L1/--L2/--L3` and `--L1=...`: `crates/gitcomet-app/src/cli.rs:702-711,734-748`.
- Generic `-L...` branch treats token suffix as label text directly: `crates/gitcomet-app/src/cli.rs:803-809`.
- Existing parser tests cover `--L*` variants, not `-L1 <value>` style: `crates/gitcomet-app/src/cli.rs:2808-2875`.

What to change:
- Add explicit parsing for `-L1`, `-L2`, `-L3` (separate-value and attached-value forms).
- Add unit tests for those forms.
- Add an integration helper that records actual argv from Git mergetool on each OS so parser assumptions are verified against real invocations.

---

### 6) Conflictstyle fallback uses ambient cwd, which is fragile across tool launch contexts [DONE]

Symptoms:
- `diff3`/`zdiff3` fallback tests fail on macOS.

Evidence:
- Fallback reads config via plain `git config --get ...` with no repo context: `crates/gitcomet-app/src/cli.rs:567-574`.

What to change:
- Resolve repository context explicitly (from `MERGED`/`LOCAL` path or `git rev-parse --show-toplevel`) and read config with `git -C <repo> config --get ...`.
- Add a test where the process cwd is outside the repo to confirm fallback still works.

---

### 7) One macOS test is brittle due BSD/GNU command output formatting [DONE]

Symptoms:
- Add/add BASE size assertion fails with `BASE_SIZE=       0` instead of `BASE_SIZE=0`.

Evidence:
- `wc -c` output is used without trimming in command template: `crates/gitcomet-app/tests/mergetool_git_integration.rs:265-266`.
- Assertion expects an exact string: `crates/gitcomet-app/tests/mergetool_git_integration.rs:913-915`.

What to change:
- Normalize to an integer value before asserting:
  - Trim shell output (`tr -d '[:space:]'`) in the command, or
  - Parse numeric value in Rust and compare numerically.

## P2 (Broader Portability Hardening)

### 8) Backend mergetool command execution assumes `sh` [DONE]

Symptoms:
- Potential runtime portability issue on Windows/non-POSIX shells.

Evidence:
- Custom mergetool command executes via `Command::new("sh").arg("-c")`: `crates/gitcomet-git-gix/src/repo/mergetool.rs:49-57`.

What to change:
- Use platform-specific shell dispatch:
  - Unix: `sh -c`
  - Windows: `cmd /C` (or an explicitly configured shell path)
- Add Windows integration coverage for backend mergetool command execution.

---

### 9) Locale-sensitive assertions can fail on non-English environments [DONE]

Symptoms:
- Many tests assert English Git output substrings (`"seems unchanged"`, `"Was the merge successful"`, etc).

Evidence:
- Example assertions: `crates/gitcomet-app/tests/mergetool_git_integration.rs:983-988`.

What to change:
- Force deterministic locale for test Git commands (`LC_ALL=C`, `LANG=C`) in helper command setup.

## Recommended Implementation Order

1. P0 items 1-3 (unblock CI across Fedora/Linux/Windows).
2. P1 items 4-7 (stabilize macOS parity and remove Git-version fragility).
3. P2 items 8-9 (runtime hardening and long-term portability).

## Verification Checklist

- Linux (Ubuntu + Fedora):
  - Headless suite passes.
  - Display-profile jobs pass with X11/Wayland-specific deps installed.
- macOS (Intel + Apple Silicon):
  - Mergetool compatibility and conflictstyle tests pass consistently.
  - No assumptions tied to one Git minor version.
- Windows:
  - Full workspace test compile succeeds.
  - Symlink-related tests are gated or pass with platform helper.
  - Backend mergetool custom command path works without requiring `sh` in PATH.
