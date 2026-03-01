# External Diff/Merge Usage Design for GitGpui

## Objective

Make GitGpui usable as a global Git difftool and mergetool so users can:

1. Run `git difftool` and open GitGpui only for diff.
2. Run `git mergetool` and open GitGpui only for merge.
3. Launch GitGpui directly in dedicated diff/merge modes outside full-repo UI flow.

This document cross-verifies behavior with three reference implementations:

1. Git (`git mergetool`, `git difftool`)
2. Meld
3. KDiff3

and defines the design and test plan for GitGpui.

## Cross-Verified Reference Behavior

### 1) Git (authoritative contract)

References:

- `/home/sampo/git/git/Documentation/git-mergetool.adoc`
- `/home/sampo/git/git/Documentation/git-difftool.adoc`
- `/home/sampo/git/git/Documentation/config/mergetool.adoc`
- `/home/sampo/git/git/Documentation/config/difftool.adoc`
- `/home/sampo/git/git/git-mergetool--lib.sh`
- `/home/sampo/git/git/git-mergetool.sh`
- `/home/sampo/git/git/git-difftool--helper.sh`
- `/home/sampo/git/git/t/t7610-mergetool.sh`
- `/home/sampo/git/git/t/t7800-difftool.sh`

Observed best practices to adopt:

1. Respect the custom tool contract exactly:
   1. Merge uses `BASE`, `LOCAL`, `REMOTE`, `MERGED`.
   2. Diff uses `LOCAL`, `REMOTE`, with `MERGED` and `BASE` compatibility.
2. Honor GUI tool precedence and auto behavior:
   1. `diff.guitool`/`merge.guitool`
   2. `difftool.guiDefault`/`mergetool.guiDefault` with `auto` + `DISPLAY` behavior.
3. Keep path override support first class:
   1. `difftool.<tool>.path`
   2. `mergetool.<tool>.path`
4. Keep `--tool-help` discoverability parity.
5. Preserve safety semantics around trusting exit codes:
   1. Git has explicit `trustExitCode` semantics.
   2. Difftool has `--trust-exit-code`.
6. Handle edge conflict classes explicitly:
   1. No base stage
   2. Delete/delete
   3. Symlink
   4. Submodule
7. Keep broad compatibility with invocation contexts:
   1. file names with spaces
   2. running from subdirectories
   3. pathspec and dir-diff flows
8. Use exhaustive integration testing as done in `t7610` and `t7800`.

### 2) Meld (CLI UX and validation quality)

Reference:

- `/home/sampo/git/meld/meld/meldapp.py`

Observed best practices to adopt:

1. Strict argument validation with clear errors:
   1. wrong number of inputs for diff mode
   2. explicit "too many arguments" handling
   3. auto-merge constraints validated early
2. Dedicated output-target concept (`--output`) for merge result file.
3. Label support for panes (`-L/--label`) so tool windows are self-describing.
4. Avoid ambiguous behavior by rejecting invalid combinations up front.

### 3) KDiff3 (merge CLI compatibility + fixture rigor)

References:

- `/home/sampo/git/git/mergetools/kdiff3`
- `/home/sampo/git/kdiff3/src/main.cpp`
- `/home/sampo/git/kdiff3/src/kdiff3.cpp`
- `/home/sampo/git/kdiff3/test/alignmenttest.cpp`
- `/home/sampo/git/kdiff3/test/testdata/README`
- `/home/sampo/git/kdiff3/test/generate_testdata_from_permutations.py`
- `/home/sampo/git/kdiff3/test/generate_testdata_from_git_merges.py`

Observed best practices to adopt:

1. Strong compatibility options in CLI:
   1. `--base`
   2. `--output` / `--out`
   3. `--L1/--L2/--L3` labeling
2. Clear auto-mode constraints (auto requires output path).
3. Exit with meaningful status from "saved vs not saved" merge outcomes.
4. Test fixtures with deterministic convention:
   1. `*_base.*`
   2. `*_contrib1.*`
   3. `*_contrib2.*`
   4. `*_expected_result.*`
5. Large corpus generation from:
   1. permutations
   2. real-world Git merges (`merge-base` based extraction)

## GitGpui External Tool Design

## CLI Modes

Add dedicated subcommands:

1. `gitgpui-app difftool`
2. `gitgpui-app mergetool`

These bypass full repository browsing flow and open focused tool windows.

### `difftool` mode

Inputs (priority order):

1. explicit flags:
   1. `--local <path>`
   2. `--remote <path>`
   3. optional `--path <display_name>`
   4. optional `--label-left`, `--label-right`
2. fallback env:
   1. `LOCAL`, `REMOTE`
   2. optional `MERGED`, `BASE` (for display compatibility)

Validation:

1. `local` and `remote` must resolve to existing file or directory.
2. invalid args return non-zero with actionable error text.

### `mergetool` mode

Inputs (priority order):

1. explicit flags:
   1. `--merged <path>` (required)
   2. `--local <path>` (required)
   3. `--remote <path>` (required)
   4. `--base <path>` (optional for add/add and other no-base cases)
   5. optional labels: `--label-base`, `--label-local`, `--label-remote`
2. fallback env:
   1. `MERGED`, `LOCAL`, `REMOTE` required
   2. `BASE` optional

Validation:

1. fail fast on missing required inputs.
2. allow base to be missing.
3. preserve binary bytes and line endings by default.

## Exit Code Policy

Align behavior with Git expectations and tool robustness:

1. `0`: user completed action and result persisted to output target.
2. `1`: user canceled or closed with unresolved result.
3. `>=2`: input/IO/internal error.

Notes:

1. When invoked by Git custom cmd, Git may additionally apply `trustExitCode` logic.
2. GitGpui should still return meaningful process statuses for standalone use.

## Git Global Config Setup

Recommended global config:

```bash
GITGPUI_BIN="/absolute/path/to/gitgpui-app"

git config --global merge.tool gitgpui
git config --global mergetool.gitgpui.cmd \
  "'$GITGPUI_BIN' mergetool --base \"\$BASE\" --local \"\$LOCAL\" --remote \"\$REMOTE\" --merged \"\$MERGED\""
git config --global mergetool.gitgpui.trustExitCode true
git config --global mergetool.prompt false

git config --global diff.tool gitgpui
git config --global difftool.gitgpui.cmd \
  "'$GITGPUI_BIN' difftool --local \"\$LOCAL\" --remote \"\$REMOTE\" --path \"\$MERGED\""
git config --global difftool.trustExitCode true
git config --global difftool.prompt false

git config --global merge.guitool gitgpui
git config --global diff.guitool gitgpui
git config --global mergetool.guiDefault auto
git config --global difftool.guiDefault auto
```

Quoting requirement:

1. Always quote all substituted vars (`"$LOCAL"`, etc.) to preserve spaces and special chars.

## Behavior Matrix (must be explicit and tested)

1. file paths with spaces and unicode
2. invocation from repo subdirectory
3. no-base conflicts (`BASE` absent)
4. binary and non-UTF8 content
5. deleted output (tool chooses deletion)
6. symlink conflicts
7. submodule path conflicts
8. CRLF preservation
9. directory diff mode (`LOCAL`/`REMOTE` dirs)
10. close/cancel behavior and exit code

## Test Strategy (port best practices)

### A. Port core Git scenarios

Create an integration suite that mirrors critical coverage from:

1. `t7610-mergetool.sh`
2. `t7800-difftool.sh`

Start with high value subsets:

1. trust-exit-code behavior
2. gui default selection and precedence
3. `--tool-help` discoverability output
4. subdir/path-with-space invocation
5. delete/delete and no-base conflict cases
6. dir-diff and symlink edge cases

### B. Keep and extend existing GitGpui tests

Reuse current helpers and coverage in:

1. `/home/sampo/gitgpui/crates/gitgpui-git-gix/tests/status_integration.rs`
2. `/home/sampo/gitgpui/crates/gitgpui-git-gix/tests/conflict_base_checkout_integration.rs`

Existing strengths to keep:

1. `launch_mergetool` trust semantics
2. unresolved marker rejection
3. braced env variable support
4. stage materialization helpers (`set_unmerged_stages`)

Gaps to add:

1. dedicated difftool-mode integration tests
2. full end-to-end tests that invoke `git difftool`/`git mergetool` with global-like config
3. explicit symlink/submodule external tool scenarios

### C. Add KDiff3-style fixture harness

Add fixture runner inspired by KDiff3 `alignmenttest`:

1. fixture naming:
   1. `*_base.*`
   2. `*_contrib1.*`
   3. `*_contrib2.*`
   4. `*_expected_result.*`
2. runner auto-discovers fixtures and reports deterministic pass/fail.
3. optional generators:
   1. permutation corpus
   2. real merge corpus extracted from Git history

This creates long-term algorithm/regression protection for merge behavior.

## Rollout Plan

### Phase 1 (MVP)

1. add CLI `difftool` and `mergetool` modes
2. add documented global config commands
3. implement robust arg/env parsing + validation
4. add E2E tests for happy-path diff/merge with Git invocation

### Phase 2 (compat parity hardening)

1. add dir-diff support and symlink handling
2. add no-base/delete/delete/submodule paths coverage
3. align GUI default behavior and tool-help output

### Phase 3 (regression suite)

1. fixture harness and generated corpora
2. parity-focused regression gates in CI

## Acceptance Criteria

1. GitGpui can be set globally as both `diff.tool` and `merge.tool`.
2. `git difftool` opens focused GitGpui diff window and exits correctly.
3. `git mergetool` opens focused GitGpui merge window, writes output, and exits correctly.
4. Behavior matrix above is covered by automated tests.
5. Regression harness catches diff/merge behavior drift deterministically.
