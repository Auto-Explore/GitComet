## Contributing

<!-- gitcomet-cla:start -->
### Contributor License Agreement

GitComet's open-source edition is licensed under AGPL-3.0-only. AutoExplore Oy also plans a paid proprietary GitComet Pro edition.

Before we merge a contribution, its rights holders must have accepted the applicable Contributor License Agreement (CLA), or AutoExplore must have verified equivalent permission under another agreement. Contributors retain ownership of their work. The CLA permits AutoExplore Oy to include their contributions in proprietary GitComet products without royalty payments while also licensing those contributions under the project's license at submission. The full agreement governs; this paragraph is a summary.

Read the [Individual CLA](CLA.md). Company-owned contributions use the separate [Entity CLA](CLA-ENTITY.md) and the authorization process below.

#### CLA signing instructions

After opening a pull request, follow the CLA Assistant comment to read and accept the Individual CLA using your GitHub account. Provide your full legal name and a contact email address. Sign only after you have read the agreement and obtained any necessary employer permission.

For an unchanged agreement, returning signers normally do not need to sign again. An agreement change may require renewed acceptance.

The acceptance record identifies the agreement version that was accepted. The text shown during signing is the agreement you accept; the repository copy must be kept consistent with it.

For company-owned work, have an authorized representative complete the Entity CLA and send it privately to info@gitcomet.dev. Include the company's legal name, the relevant GitHub usernames, and the contribution references. Do not sign the individual agreement as a substitute for obtaining the rights holder's authorization.

Contributors under eighteen should contact info@gitcomet.dev with a parent or guardian to arrange the signing process before contributing.

#### Third-party material and employer-owned contributions

Identify copied or adapted third-party code, documentation, images, and other material in the pull request. Include its source, copyright notices, license, and the affected files. A CLA cannot grant rights you do not hold, and it does not automatically change a dependency's license.

Do not submit employer-owned or other third-party-owned material as if you personally own it. Obtain the necessary permission and contact info@gitcomet.dev to resolve the appropriate agreement before the contribution is merged.

Preserve the actual authors and co-authors of contributions. Ask maintainers for help with an unrecognized GitHub account or an incorrectly attributed commit; do not remove attribution to make the check pass.

Questions about contribution permissions or signing records: info@gitcomet.dev. Do not include private signing details in public issue comments.
<!-- gitcomet-cla:end -->


### Workspace layout

See [Architecture](docs/architecture.md) for dependency boundaries, state flow,
repository lifetimes, and document ownership.

- `crates/gitcomet-base`: product identity (`identity`), per-user directories (`platform::dirs`), platform detection, file utilities, operation traces and work counters. No Git domain types; core re-exports these modules at their `gitcomet_core::` paths.
- `crates/gitcomet-text`: text foundations without GPUI itself: rope and text model, tab expansion, instrumented LRU caches, syntax token kinds, date and time formatting, and cached path display. The UI kit re-exports `rope` and `text_model`.
- `crates/gitcomet-syntax`: tree-sitter syntax highlighting without GPUI: language detection, the bundled grammars and queries, prepared documents parsed off the UI thread, live documents for editors, injections, occurrences and pairs. Renderers style its token kinds through a `SyntaxPalette`.
- `crates/gitcomet-conflict-resolver`: the conflict resolver's model without GPUI: marker parsing, two- and three-way row projections, navigation, provenance, resolution and autosolve summaries, and word highlights. Rendering and styled-text caches stay in the UI host.
- `crates/gitcomet-markdown`: the markdown preview model without GPUI: parsing, HTML and table flattening, inline spans, heading anchors, code-fence languages, row wrapping and scrollbar marker buckets. Rendering and row styling stay in the UI host.
- `crates/gitcomet-core`: domain types, Git service contracts, merge algorithm, conflict session, text utils.
- `crates/gitcomet-git-gix`: `gix`/gitoxide backend implementation.
- `crates/gitcomet-state`: MVU state store, reducers, effects, conflict session management.
- `crates/gitcomet-fs-watch`: native macOS filesystem watcher support used by the state store, plus Windows test support.
- `crates/gitcomet-diagrams`: offline Mermaid recognition and rendering, independent of GPUI.
- `crates/gitcomet-fonts`: bundled font assets shared by diagram rendering and the UI kit.
- `crates/gitcomet-tree-sitter-alloc`: installs the allocator hooks used by tree-sitter.
- `crates/gitcomet-perf`: performance measurement records (sidecars) written by benchmarks and probes, and the `perf_budget_report` binary that checks them against budgets.
- `crates/gitcomet-ui-kit`: GPUI foundations reusable without the app: runtime policy (`ui_runtime`, installed live by the app at launch, deterministic otherwise), appearance, UI scale, themes, fonts, interaction primitives, text inputs, tooltips, icons, client-side window chrome (frame, title-bar geometry and drag, resize edges, window controls), and components (buttons, pickers, menus, settings rows, navigation tabs, interstitials). Builds on `gitcomet-base`, with no dependency on core, state, the application, or the UI host.
- `crates/gitcomet-ui-terminal`: the embedded terminal widget: an alacritty backend on a PTY and the GPUI viewport that paints it, selects text and forwards input. Terminal tabs, sessions, shutdown prompts and preferences stay in the UI host.
- `crates/gitcomet-ui-media`: image and diagram previews: device-pixel image inspection with tiled resampling, SVG rasterization with the bundled fonts, asynchronous diagram surfaces and the shared source/rendered toggle. The host supplies inputs, opens the menus the toolbars request and saves sampling preferences.
- `crates/gitcomet-extension-api`: the contract for compiled-in extensions: contributions (repository views, status items, settings pages, commands, key bindings, menu entries, assets, entry gates, close guards), weak window and repository handles, and per-extension session/workspace storage. Depends on core, state, and the UI kit, never on the UI host.
- `crates/gitcomet-ui-gpui`: the GitComet window: views, panes, panels (focused diff/merge windows, conflict resolver, word diff); `UiLaunch` opens the browser window and hosts extensions (`view/extension_host.rs`). With none registered it installs nothing.
- `crates/gitcomet-app`: process launch (`AppLaunch`): identity install, crash reporting, CLI (clap), browser-instance broker, difftool/mergetool/setup/uninstall modes. GUI dependencies are optional (`ui-gpui` feature).
- `crates/gitcomet`: the executable: allocator, platform resources, packaging, and instrumentation binaries.
- `crates/gitcomet-extension-example` and `crates/gitcomet-extension-example-app`: a neutral example product built only on public upstream interfaces, with an extension registering one of every contribution; the UI host's tests run it (`view/tests/extensions.rs`), and CI builds the product in its own context.

Product names, identifiers, and links come from `gitcomet_core::identity`, never
from string literals: `scripts/ci/identity_literals.py` fails on new ones.

### Getting started

Linux prerequisites:

- Install Clang (`sudo apt-get install clang` on Ubuntu/Debian, `sudo dnf install clang` on Fedora, or `sudo pacman -S clang` on Arch).
- Install the UI development libraries. On Ubuntu/Debian, CI uses `sudo apt-get install pkg-config libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev`.
- Install mold 3.0.0 and its `ld.mold` alias on your `PATH`. The repository installer downloads checksum-pinned upstream binaries for x86_64 and ARM64 Linux:

  ```bash
  python3 scripts/install-mold.py --bin-dir "$HOME/.local/bin"
  export PATH="$HOME/.local/bin:$PATH"
  mold --version
  ```

Cargo uses `scripts/linux/mold-linker.sh` to select mold through Clang for all Linux build profiles, including tests, coverage, and profiling. CI and Linux releases use the same pinned installer. See the [mold 3.0.0 release](https://github.com/rui314/mold/releases/tag/v3.0.0) for upstream binaries and source installation on other architectures.

To verify the linker after building, run `readelf -p .comment target/debug/gitcomet`; it should include `mold 3.0.0`. To temporarily use the system linker on x86_64 Linux, run `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc cargo build` (use `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER` on ARM64).

Windows prerequisites (Windows 10/11):

- Install Visual Studio 2022 (Community or Build Tools).
- Install the `Desktop development with C++` workload.
- Ensure both MSVC tools and Windows 10/11 SDK components are installed.
- This repo configures Cargo to use `scripts/windows/windows-lld-linker.cmd` for x64 and ARM64 Windows builds. The wrapper uses the active Rust toolchain's bundled `rust-lld` linker and discovers the MSVC and Windows SDK libraries, so `cargo build` works from a regular PowerShell/CMD shell. No separate LLVM installation is needed; the Visual Studio components above are still required.

The default build includes the GPUI app and the gix backend:

```bash
cargo build --locked
```

To build the command-line app without GPUI:

```bash
cargo build --locked -p gitcomet --no-default-features --features gix
```

Run (opens the repo passed as the first arg, or falls back to the current directory):

```bash
cargo run --locked -p gitcomet --bin gitcomet -- /path/to/repo
```

### Testing

The [CI runner](scripts/ci/run.py) separates package feature contexts and runs
GPUI suites in their libtest process. Use the Python and `cargo-nextest` versions
in [CI setup](.github/actions/ci-setup/action.yml); its nextest installer is
[install-nextest.py](scripts/ci/install-nextest.py).
Install Git LFS and initialize the syntax fixture submodule before running tests:

```bash
git lfs version
git submodule update --init fixtures/syntax_test
```

The [native CI workflow](.github/workflows/cross-platform-tests.yml) lists
platform prerequisites and the separate git-annex test lane.
Compile and run the workspace tests, including the UI, with:

```bash
python3 scripts/ci/run.py compile --context workspace
python3 scripts/ci/run.py test --context workspace
python3 scripts/ci/run.py doc --context workspace
```

Use `--context core`, `diagrams`, `state`, `backend`, `app`, or `ui` for a focused
run. The `app` context disables GUI features. Run the same commands with
`--context example` to check the downstream extension product in its separate
feature context. Recompile a context after changing its code.

Large unit-test suites use private child modules. Tests move with the code they
test, including into a new crate: use [inventory.py](scripts/ci/inventory.py) to
snapshot the compiled harnesses before and after the change, then compare names
and ignore flags modulo the documented module-prefix mapping. Record intentional
additions or replacements separately from mechanical moves; retain the existing
GPUI process isolation and visual/clipboard locks.

The backend's `test-support` feature exposes test Git environments and mergetool
consent overrides. In-tree tests enable it through dev dependencies; external test
harnesses must opt in. Normal app builds leave it disabled. See the
[profiling guide](scripts/profiling/README.md#backend-measurements) for probe builds.

Formatting, Clippy, and dependency boundaries (matching CI's package contexts):

```bash
cargo fmt --all --check
cargo clippy --locked -p gitcomet-core -p gitcomet-diagrams -p gitcomet-state -p gitcomet-git-gix -p gitcomet-tree-sitter-alloc -- -D warnings
cargo clippy --locked -p gitcomet -p gitcomet-app --no-default-features --features gix -- -D warnings
cargo clippy --locked -p gitcomet-ui-kit -p gitcomet-extension-api -p gitcomet-extension-example -p gitcomet-ui-gpui -p gitcomet-app -- -D warnings
python3 scripts/ci/boundaries.py
python3 scripts/ci/ratchets.py
```

The ratchet script keeps the [refactor round 2](docs/refactor-round-2.md)
structure counts from regressing; after a change that improves one, run it with
`--write` to record the new value.

Coverage (local + CI-compatible):

```bash
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov
bash scripts/coverage.sh
```

This writes:

- `target/llvm-cov/lcov.info` (used by CI upload)
- `target/llvm-cov/html/index.html` (local detailed report)

### Release packaging

macOS packaging is handled by:

```bash
scripts/package-macos.sh --version 0.2.0 --arch arm64 --release
scripts/package-macos.sh --version 0.2.0 --arch x86_64 --release
```

Use `--skip-dmg` when running in restricted/sandboxed environments where `hdiutil create` is unavailable.

The release workflow `.github/workflows/build-release-artifacts.yml` builds and publishes:

- Windows: portable ZIP + MSI
- Linux: tar.gz + AppImage + .deb + .rpm
- macOS: DMG + tar.gz for `arm64` and `x86_64`
- Homebrew cask asset: `gitcomet.rb` (generated from macOS DMG artifacts and Linux AppImages plus their SHA256 values)

### Homebrew deployment

To push `Casks/gitcomet.rb` into a Homebrew tap repo automatically on release:

1. Create a tap repository (default expected name: `OWNER/homebrew-gitcomet`).
2. In this repo, configure:
   - secret `HOMEBREW_TAP_TOKEN`: GitHub token with `contents:write` access to the tap repository.
   - variable `HOMEBREW_TAP_REPO`: tap repository in `OWNER/REPO` form.
   - optional variable `HOMEBREW_TAP_BRANCH`: target branch (default `main`).
3. Run `.github/workflows/release-manual-main.yml` with `draft=false`.

This release flow will:

- build and upload release artifacts
- publish the GitHub release
- call `.github/workflows/deploy-homebrew-tap.yml` to update `Casks/gitcomet.rb` in the tap repo
- call `.github/workflows/deploy-aur.yml` to update `gitcomet-bin` in the AUR

You can also run `.github/workflows/deploy-homebrew-tap.yml` manually for backfills or dry-runs.

### Linux packages

`scripts/package-linux.sh` builds the release artifacts from one staged payload. The `.deb`, `.rpm` and AppImage include the binary, desktop entry, all five icon sizes, licence files and README. The tarball preserves its historical layout for third-party packagers: only the binary, README, `LICENSE-AGPL-3.0` and `NOTICE`. Package metadata lives in `packaging/linux/`: `debian-control.in` for the `.deb`, `gitcomet.spec` for the `.rpm`, and `PKGBUILD.in` for the AUR's `gitcomet-bin`.

Linked libraries are declared automatically by `dpkg-shlibdeps` and rpm AutoReq, and explicitly in `PKGBUILD.in`. Libraries the app loads at runtime (Vulkan, EGL, Wayland) are declared by hand. `scripts/check-linux-runtime-deps.sh` runs in CI and on every release build, and fails when the binary's libraries or its glibc baseline drift from those declarations. Update all three package templates when the runtime dependencies change. Arch requires `libglvnd` for EGL and offers `vulkan-icd-loader` as an optional dependency; `wayland` is optional because X11 is the fallback.

The `deb_revision` and `rpm_release` inputs of `.github/workflows/release-manual-main.yml` set the package revision (the `1` in `1.2.3-1`). Raise them when rebuilding packages for an existing version through a manual dispatch of `.github/workflows/build-release-artifacts.yml`.

The RPM supports Fedora 42 and newer.

The release workflow publishes `gitcomet-bin` after the GitHub release is public. `.github/workflows/deploy-aur.yml` can also be dispatched for backfills or with `dry_run: true` to verify sources and preview the generated PKGBUILD and `.SRCINFO` without pushing. It uses the existing `AUR_PRIVATE_SSH_KEY` and `AUR_PRIVATE_SSH_KEY_PASSPHRASE` secrets. `AUR_GIT_REPOSITORY` defaults to `ssh://aur@aur.archlinux.org/gitcomet-bin.git`; an existing `gitcomet.git` URL is redirected to `gitcomet-bin.git`. `AUR_GIT_BRANCH` defaults to `master`.

`scripts/update-aur.sh` renders the template with checksums of both release tarballs and the tagged source archive, which supplies the desktop entry and icons. Arch's `aarch64` uses the `linux-arm64` release artifact. New versions start at `pkgrel=1`; metadata changes for the same version increment `pkgrel`, while identical reruns leave it unchanged. Release candidates use `1.2.3rc.1` as the Arch version and retain `1.2.3-rc.1` in download URLs. Run `scripts/test-aur-packaging.sh` as a non-root user on Arch to verify metadata, source checksums, and the full payload for both architectures.
