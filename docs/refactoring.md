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

## Milestone 2: the UI kit

`gitcomet-ui-kit` holds the GPUI foundations and components; the UI host
re-exports its modules under their old `crate::` paths (`crate::kit`,
`crate::theme`, `view::components`, ...), so host code did not change shape.

- Preferences come in as plain values: `AppearancePreferences`,
  `StoredFontPreferences`, and a stored scale percent. `session_ui.rs` is the
  host's one adapter from `UiSession`.
- Runtime policy is explicit: `ui_runtime::install(UiRuntime::live())` runs in
  `UiLaunch::run`, the focused mergetool and difftool, and the benchmark
  harnesses; everything else (every test) is deterministic. The previous
  `cfg(test)` switch could not reach a dependency. The user theme folder is
  likewise installed at launch (`theme::set_user_themes_dir`).
- Window-layout and settings persistence moved out of the tooltip module to
  `view/ui_persistence.rs`; the picker's workspace colour became a generic
  `PickerSwatch` with the workspace adapter in `view/workspace_picker.rs`.
- The icon set and its generation moved with `icons`; GitComet's artwork
  (`gitcomet_mark.svg`, window icon, logo) stays in the host's asset source,
  layered over `KitAssets`, and follows the identity's branding.
- `test-support` exposes test helpers and hooks (counters, snapshots) and the
  shared visual/clipboard locks; it adds functions, never behaviour switches.
  `#![warn(unnameable_types)]` keeps every type in a public signature nameable.
- Source guards are shared: `test_support::source_guards` (clipboard access,
  unscaled icon sizes, unresolvable menu icons) run over the kit's sources in
  the kit and over the host's sources in `render_guards.rs`, as does the
  "rendering never builds its own theme" guard.
- New shared components, used by the host: settings rows, cards, headings, and
  navigation items (`settings_*`), navigation tabs (`navigation_tab*`, the
  sidebar's Branches/Files strip), selectable fields
  (`TextInputOptions::selectable*`, `selectable_field`), and the interstitial
  card (`interstitial`, the splash and Git-unavailable screens).
- Exit condition: `gitcomet-extension-example` renders and clicks a kit button
  in a GPUI test without depending on the UI host (`boundaries.py`).

Test moves: 428 tests moved from `gitcomet-ui-gpui` to `gitcomet-ui-kit`
(`inventory.py compare --split gitcomet-ui-gpui:lib:gitcomet_ui_gpui=gitcomet-ui-kit:lib:gitcomet_ui_kit`).
Three host guards became two each (kit and host scans), two window-frame tests
stayed in the host (`window_focus_tests.rs`, `resize_grip_tests.rs`), and the
asset-registry test split into the kit's icon check and the host's
brand-over-kit listing check.

## Milestone 3: extensions and host interfaces

`gitcomet-extension-api` is the contract: an `Extension` declares its
contributions through a `Registrar`; `Registry::build` validates and freezes
them (namespaced ids, no duplicate contribution ids or asset paths, bindings
and menu items naming declared commands). A product adds extensions with
`AppLaunch::extension`; `run_mode` builds the registry after the command line
parses and before any mode runs, so a broken registration exits with
`exit_code::ERROR` in every mode (help and version never register).

The host side lives in `view/extension_host.rs`:

- With no extension registered nothing is installed: no global, no
  `ExtensionWindow`, no router, no palette rows, no key bindings, and every
  lookup returns early (`without_extensions_the_host_adds_nothing`).
- Each main window owns an `ExtensionWindow`: a weak `WindowHost` that reads
  a state snapshot and theme the view publishes, never the view entity, so
  extension code can use its handles inside host updates (close guards,
  gates). Handles report `WindowClosed`/`RepositoryClosed`; repository
  handles carry the repository's lifetime, since `RepoId`s are reused.
- UI mutations from extensions are deferred: dialogs, notifications, and
  commands run after the current update.
- `WindowHost::observe_state` subscribes to a window's state: observers run
  once per update cycle however many snapshots land in it, nothing is
  scheduled while none are registered, and dropping the subscription
  unregisters it. A closed window's handles hold neither its store nor its
  last snapshot.
- Contributions: repository views (`view/repository_views.rs`, a router that
  shows History or one extension view per repository; inactive History is not
  rendered but keeps its state; views are built on first selection and
  dropped when their repository closes), status items (built once per window
  after it opens), settings pages (`settings_window/extension_pages.rs`, only
  the selected page is built), commands (palette rows fixed at construction,
  `RunExtensionCommand` action for key bindings with app- and window-level
  handlers, app menu / macOS menu bar / repository tab menu entries), assets
  (served under `extensions/<id>/` by `GitCometAssets`), hosted dialogs
  (`PopoverKind::ExtensionDialog`, the popover host's focus restoration), and
  `on_window_opened` callbacks.
- Repository entry: every entry passes `extension_host::entry_decision` once,
  before routing: command-line and forwarded requests, macOS open-URL
  requests, the chooser (Home, pickers, the open dialog), drops, and
  workspace restoration (window construction and `adopt_workspace`, which
  filter the list before any bootstrap is dispatched or deferred). A denial
  shows a warning in the relevant window. Focused tool windows never install
  extensions.
- Closing: the guards moved from `terminal_panel.rs` to `close_guards.rs`
  and run in one order: unsaved editor buffers, running terminal commands,
  running Git operations (push, fetch/pull, commit), then extension guards.
  Resolving a prompt resumes at the next stage, so no guard asks twice. Every
  repository close (tab button, Cmd+W, the tab menu's Close / Close others /
  Close to the right) goes through `request_close_repos`; the tab menu used to
  bypass the terminal guard.
- Persistence: an extension's session-wide and per-workspace namespaces
  (kept verbatim through host updates, 64 KiB each) are reached through
  `storage` and `WindowHost::{workspace_state, set_workspace_state}`.

`gitcomet-extension-example` registers one of every contribution
(`review.rs`), and the host runs it in `view/tests/extensions.rs`: two
windows with separate state, workspace persistence and restoration,
notifications, commands through the palette and a key binding, the router,
hosted dialogs, entry gates, close guards, and teardown when a window closes.
`comet-example` runs it.

Test inventory (`inventory.py compare`, the CI workspace selection): all
7,688 tests from Milestone 2 map unchanged; 19 are new (session namespaces,
extension hosting, the extension settings page and shortcut labels, and one
shared render guard). The close-guard move changed no test paths.

## Milestone 4: comparison services

- `CommitFileChange`, `SubmoduleInnerChange`, and the `DiffTarget` variants
  are non-exhaustive and built through constructors (the migration was
  compiler-driven), so the fields below were added without touching callers.
- Changes carry rename/copy sources, blob ids, and modes. Commit and range
  targets carry the rename source (equality ignores it: it is derived from
  the commit and path), and every loader reads the old side from it, so a
  renamed file shows its edit, decoded by the source path's attributes,
  instead of a whole-file addition. History's commit rows, range rows, file
  navigation, and submodule ranges pass it through.
- `GitRepository::compare_files` (direct or merge-base; untracked files
  opt-in), `merge_base`, and `is_ancestor` have defaults that delegate where
  they can and return `Unsupported` where an option cannot be honored.
  `repo/comparison.rs` holds the shared implementation, including the
  commit-to-worktree helpers that used to live in submodule code. History's
  range loads call `compare_files`; `Msg::CompareWithOptions` starts a
  comparison with options, and a merge-base comparison's file diffs start at
  the resolved base (`RangeSelection::diff_from`).
- `Msg::FetchRefspecs` fetches exact refspecs through the command pipeline a
  full fetch uses: in-flight tracking, auth prompt and replay, errors, and the
  remote-branch refresh.
- `HistoryRefFilter` (names, prefixes, globs) is part of the immutable
  `RepositoryOptions` a repository opens with; the default excludes nothing.
  The gix backend applies it where every all-branches reader collects tips
  (pages, authors, index, snapshots, and their caches). `ConfiguredBackend`
  and `AppLaunch::repository_options` let a product set it; a backend that
  cannot honor an option refuses to open.
- Watcher changes carry `ChangedPaths`: the worktree paths behind them, up to
  `MAX_CHANGED_PATHS`, or `Unknown` past the bound, on rescans and ignore-rule
  or control-file changes, and for changes not from the watcher.
  `RepoState::worktree_paths_changed_since` answers exactly one change back
  and `Unknown` beyond, so a truncated or missed list is never taken as
  complete.
- `AppStore::watch_repository` returns a counted `WatchLease` that keeps a
  non-active repository's watcher running and delivering until dropped;
  extensions get it as `WindowHost::watch_repository`. Leases never keep the
  store alive and leave with their repository.

## Milestone 5: independently owned panes

- `DiffSession`s live beside History's selected diff, keyed by `DiffViewId`
  in `RepoState::diff_sessions`. Each has its own target, encoding,
  generation, revision, content, blame, and cancellation token (a child of
  the repository's). Completions carry repository, lifetime, view, and
  generation, so a stale, cancelled, or closed-repository load is dropped.
  Sessions reuse the selected diff's backend readers and preview flags, and
  worktree changes reload only sessions that follow the worktree.
- `ChangeListSession`s load a commit's files or a comparison
  (`ChangeSource`) the same way; the base they resolve (first parent or merge
  base) turns each file into a `DiffTarget`.
- `WindowHost::create_diff_pane` / `create_snapshot_pane` / `create_file_list`
  return owning handles (`DiffPane`, `FileList`); dropping the handle and its
  mounted view closes the session and cancels its work. The views
  (`view/hosted/`) observe the window's state through the extension host and
  rebuild only when their own session's revision moves; rows are built off
  the UI thread, and a retarget drops a build still running for the old
  target.
- A pane's selection, reveal anchor, and search are file side + file line
  (`DiffLineRange`, `DiffLineSide`), never display rows. `DiffPanePolicy` is
  checked in the click and search handlers, not only when drawing.
  `DiffRowStyle` overrides row backgrounds, and a `DiffRowDecorProvider` is
  asked for gutter marks and tints only for drawn rows.
- A snapshot pane diffs two texts with no repository (`DiffSnapshot`); a
  pane keeps the kind of source it was created with.
- `FileListController` reuses the details pane's projection and tree plan,
  with its own filter, sort, collapse, selection, and scroll per list.
- `WindowHost::observe_selected` notifies only when a projection of the
  state changes; `WindowHost::highlight_line` exposes the per-line syntax
  highlighter in the window's theme.
- The example's Changes view mounts a file list (the worktree against HEAD)
  and two diff panes; UI tests click through it and check that retargeting
  or dropping one pane leaves the other, the list, and History unchanged.
  Another opens a pane per file of a commit with a rename (showing its
  edit, not a whole-file add), an addition, a deletion, and a binary change
  while History keeps its own selected diff and target revision.

Still to do, deliberately kept out of this step:

- History's main pane keeps its own selected diff (`diff_state`); moving it
  onto a `DiffSession` needs its navigation, conflict, and preview paths
  migrated with parity tests.
- The details pane's commit, range, worktree, and status lists still render
  through their own code; they share the projection with hosted lists but
  not the rendering.
- The focused difftool keeps its window; snapshot panes are the intended
  replacement once the Milestone 6 parity tests exist.

## Milestone 6: richer contributions

- Hosted diff panes take `DiffAnnotations` (indexed by file side and line,
  replaced whole), a legend, gutter actions, selection actions, and
  `DiffInset`s. One projection (`view/hosted/projection.rs`) places insets
  after their lines and maps file lines to display rows; search, selection,
  copy (`DiffPane::selected_text`), reveal, and hit testing go through it,
  so inset rows are never taken for file lines. Scrollbar markers are
  placed when rows, insets, or annotations change, never while drawing (a
  UI test counts placements across frames). Actions run after the pane's
  update, so they may read the pane.
- The example's Changes view flags lines from the gutter (annotations and
  a legend) and adds notes under a selection (insets).
- Bottom panels, details tabs, and sidebar sections are registry
  contributions built per repository with a `RepositoryViewContext`.
  Terminal, Reflog, and extension panels share one bottom tab list and
  strip (a lone panel still renders without one, and only the terminal
  renders exactly as before). Which extension panels are open lives in a
  cell the host and the root view share, so `open_bottom_panel`,
  `close_bottom_panel`, and `is_bottom_panel_open` never touch the root
  view; the view is built in a deferred root update.
- Repository views and details tabs use one `ViewRouter` (select, build on
  first use, keep until the repository closes); sidebar sections use it to
  build every section for the active repository, each collapsible. With no
  such contribution registered, the details and sidebar panes mount through
  the same cached paths as before (the invalidation guard still checks them).
- File lists have a `FileListMode` (tree, flat, grouped). Grouped lists
  keep headers at the file rows' height, so the list stays uniform and
  virtual, and pin the current group's header with a list decoration that
  reads precomputed group starts each frame; a UI test scrolls and checks
  the rows are never regrouped.
- `WindowHost::open_window` opens a pop-out window showing any view, such
  as a pane's (one window at a time per view). Pop-outs take native
  decorations like the focused difftool and the main window's app id, close
  when the window that opened them closes (observed as a window closing,
  not as the root view's release, which any stray handle would delay), and
  run their `on_closed` once however they close. The example's Changes view
  pops its current pane out and takes it back; side by side is the two
  independent panes it already lays out.

Still to do, deliberately kept out of this step:

- History's split and inline diff do not take insets yet; hosted panes have
  one (inline) layout, and their projection is the one both History layouts
  should share once History moves onto a diff session.
- History-find decoration comes with #532, which is not merged; once it is,
  its row highlighting becomes annotations drawn in the History canvas.
- The focused difftool keeps its renderer. Its contract is pinned by unit
  tests in `focused_diff.rs` (parsing, whitespace, change navigation, key
  dispatch) and by `difftool_git_integration.rs` and
  `standalone_tool_mode_integration.rs` (launch and exit codes). Moving it
  onto a hosted pane first needs a unified-patch snapshot source,
  whitespace mode, change navigation, and a pane that runs without a main
  window's extension host.

## Module splits and shared primitives

Moves only, unless noted; every public or crate path still resolves through
re-exports, and inline tests moved into child modules of their old paths.

| Module | Before | After |
|---|---:|---|
| UI application (`app.rs`) | 6,960 | launch, windows, routing, menus, bindings |
| Core file diff | 4,344 | file-backed text, planning, rows, edits |
| Backend utilities | 3,131 | process/cancellation, decoding, hook tracing, arguments/paths; one output combiner |
| Main pane helpers | 3,971 | `helpers/`: state (fields grouped by responsibility with section comments, not regrouped: field order is drop order), line index, conflict projection/segments/provenance, presentation, mergetool |
| Main pane conflict actions | 4,292 | `conflict_actions/`: sync, bootstrap, resolution, navigation, output, region edits, view mode; `sync_conflict_resolver` became 912 → 417 lines of named steps (extract-function only) |
| Panel layout | 4,158 | `layout/`: commit details, metadata, form, comparison, worktree changes, file lists, status sections |
| Sidebar rows | 3,111 | `sidebar/`: branch rows, changed-file rows, badges, branch lookup, search labels |
| Popover host | 3,588 | `host/`: construction, lifecycle, focus, opening; branch, remote, and repository prompt submission; settings sync |
| State model | 3,080 | `model/`: repository, history, diff, navigation, loads, operations, app |
| Store reducer / effects | 4,182 / 3,249 | 1,991 / 1,650; auth and retry, loads, submodule trust, Git operations, settings, watch leases; load tokens and Git-unavailable replies. Both dispatch matches stay exhaustive in place |

- `gitcomet_core::text_utils::line_starts` is the one newline-start index;
  the markdown preview, three-way deferred starts, conflict previews, and
  benchmark fixtures call it. Conflict previews keep "no lines for empty
  text" on top; tree-sitter's convention (no start after a trailing
  newline) stays separate on purpose.

Left for later: `render_sidebar_rows` (one 1,796-line function) and the tail
of `commit_details_view` (the status sections) need method extraction, not
moves; the three changed-file row renderers in `sidebar/file_rows.rs`
duplicate their directory and file rows and should share one; the
remaining auth helpers in `reducer/util.rs` belong in `reducer/auth.rs`.
