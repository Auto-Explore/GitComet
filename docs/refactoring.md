# Downstream-reuse refactoring record

This file records the measurements and decisions behind the refactoring that
lets a downstream application consume unmodified GitComet crates. Update it in
the same change as the milestone it describes.

## Baseline

| Item | Value |
|---|---|
| Inspected revision | `2efcfc96811331a85ab7cf38f92dad73df4ff88a` (`dev`, 2026-09-29) |
| Open integration dependency | #532 (`feat/history_find_commit`), head `17280f50e5eb6cc1605563989b86d177c105b328`, not merged |
| Test inventory | 7,668 tests in 44 harnesses, 31 ignored (`--workspace --no-default-features --features gix,gitcomet-ui-gpui/default`) |
| Test-profile build of every harness | 5 m 18 s wall, 39 m 32 s CPU, dependencies warm, workspace crates cold (32 threads, shared host) |
| `cargo build -p gitcomet` (dev), fresh target directory | 1 m 58 s wall |
| Incremental dev rebuild after touching `gitcomet-ui-gpui` / `gitcomet-core` | 8.4 s / 9.3 s |
| CI helper suite | 68 tests, 1 skipped |
| Source-size check | 7 test files over 6,500 lines; the first-`#[cfg(test)]` heuristic hid 5 production files over 4,500 (whole-file counts: `app.rs` 6,960, sidebar rows 4,858, panel layout 4,849, diff canvas 4,791, state model 4,524) |

After #532 merges, recheck the shortcut tables, History annotations, and
message routing that the extension host and History-find decoration touch.

## Checks

- `scripts/check-rust-source-size.sh` counts every file's physical lines:
  production files are limited to 4,500 and test/benchmark files (`tests/`,
  `benches/`, `tests.rs`, `*_tests.rs`) to 6,500. An inline test module counts
  against its production file, so large inline modules live in a child
  `tests.rs`. CI runs it in the formatting job.
- `scripts/ci/boundaries.py` proves the crate layering from Cargo's resolved
  graph (normal and build edges, every target). Rules for crates that do not
  exist yet are reported as skipped.
- `scripts/ci/inventory.py snapshot` records every harness's tests and ignored
  flags; `compare` maps each old test to exactly one new test in the same
  harness, allowing only moves into child modules.
- `scripts/ci/run.py` runs every GPUI harness package (`GPUI_PACKAGES`) as one
  libtest process with isolated session state; nextest excludes the ones
  present in the metadata.

## Test moves

| Change | Tests moved | Old -> new |
|---|---:|---|
| Split the seven oversized test files by behavior | 614 | `inventory.py compare`: 7,668 mapped, 0 problems |
| Extract inline test modules to child `tests.rs` | 0 renamed | Module paths unchanged |

## Milestone 1: identity and application bootstrap

- `gitcomet_core::identity::ProductIdentity` (private fields, builder,
  validation) names the product: display, executable, directory, desktop app
  id, macOS bundle id, version, Git tool name, links (website, editions,
  community, repository, new issue, releases, license, documentation, survey),
  update source, and branding artwork. `identity::install` must precede every
  read; the first read freezes GitComet's identity.
- `gitcomet_core::platform::dirs` is the only directory resolver (session,
  themes, broker, crash logs, clipboard diagnostics). Environment values stay
  `OsString`; blank values count as unset. Crash-directory values are no longer
  trimmed, so a padded `HOME` is used as written.
- `gitcomet-app` owns launch: `AppLaunch::new(identity).about(..).on_prepare(..).run()`
  installs the identity, then crash logging, parses the command line into
  `CliOutcome::Run` or `CliOutcome::Inform` (help, version), runs `on_prepare`
  for modes only, and returns the exit code instead of exiting from helpers.
- `gitcomet_ui_gpui::UiLaunch` replaces the `run_with_*` functions, which remain
  as deprecated wrappers.
- Product-aware: window titles and app ids, menus and dialogs, the Linux
  launcher (`<app id>.desktop`, icons, `StartupWMClass`), the macOS dev bundle,
  the browser-instance broker (descriptor and requests carry the app id;
  another product's broker is never forwarded to), Git tool setup and
  uninstall (`mergetool.<tool>.*`, `<tool>.backup.*`; shared selectors only
  while they select this product), crash reports (no Report button without an
  issue tracker), the update check (off without an update source), and the
  status-bar and Settings links (hidden when unset).
- Kept as counted exceptions in `scripts/ci/identity-literals.txt`: file-format
  markers (crash log headers, path encodings), on-disk rebase state and
  per-repository trust keys shared by every product, temporary-file prefixes,
  thread names, and GitComet's own identity.
- Exit condition: `comet-example` (`gitcomet-extension-example-app`) launches
  with its own name, version, Git tools, and state, data, and crash directories
  using only upstream crates and no product features.

Test moves: the executable's 241 unit tests moved with their modules from
`gitcomet:bin:gitcomet` to `gitcomet-app:lib:gitcomet_app`
(`inventory.py compare --harness gitcomet:bin:gitcomet=gitcomet-app:lib:gitcomet_app`).
Replaced by `platform::dirs` tests, which cover every platform on every host:
`session::tests::app_data_dir_*` (2) and crashlog's `crash_dir_base_*` and
`non_empty_path_trims_and_rejects_empty_values` tests (4 on Linux).
