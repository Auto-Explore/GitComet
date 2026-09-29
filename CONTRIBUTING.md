## Contributing

### Workspace layout

- `crates/gitcomet-core`: domain types, Git service contracts, product identity (`identity`), per-user directories (`platform::dirs`), merge algorithm, conflict session, text utils.
- `crates/gitcomet-git-gix`: `gix`/gitoxide backend implementation.
- `crates/gitcomet-state`: MVU state store, reducers, effects, conflict session management.
- `crates/gitcomet-ui-kit`: GPUI foundations reusable without the app: runtime policy (`ui_runtime`, installed live by the app at launch, deterministic otherwise), appearance, UI scale, themes, fonts, interaction primitives, text inputs, tooltips, icons, and components (buttons, pickers, menus, settings rows, navigation tabs, interstitials). No dependency on state, the application, or the UI host.
- `crates/gitcomet-ui-gpui`: the GitComet window: views, panes, panels (focused diff/merge windows, conflict resolver, word diff); `UiLaunch` opens the browser window.
- `crates/gitcomet-app`: process launch (`AppLaunch`): identity install, crash reporting, CLI (clap), browser-instance broker, difftool/mergetool/setup/uninstall modes. GUI dependencies are optional (`ui-gpui` feature).
- `crates/gitcomet`: the executable: allocator, platform resources, packaging, and instrumentation binaries.
- `crates/gitcomet-extension-example` and `-app`: a neutral example product built only on public upstream interfaces; CI builds it in its own context.

Product names, identifiers, and links come from `gitcomet_core::identity`, never
from string literals: `scripts/ci/identity_literals.py` fails on new ones.

### Getting started

Windows prerequisites (Windows 10/11):

- Install Visual Studio 2022 (Community or Build Tools).
- Install the `Desktop development with C++` workload.
- Ensure both MSVC tools and Windows 10/11 SDK components are installed.
- This repo configures Cargo to use `scripts/windows/msvc-linker.cmd` for x64 and ARM64 Windows builds. The wrapper uses the active Rust toolchain's bundled `rust-lld` linker and discovers the MSVC and Windows SDK libraries, so `cargo build` works from a regular PowerShell/CMD shell. No separate LLVM installation is needed; the Visual Studio components above are still required.

Offline-friendly default build (does not build the UI or the Git backend):

```bash
cargo build
```

To build the actual app you'll enable features (requires network for dependencies):

```bash
cargo build -p gitcomet --features ui,gix
```

To also compile the gpui-based UI crate:

```bash
cargo build -p gitcomet --features ui-gpui,gix
```

Run (opens the repo passed as the first arg, or falls back to the current directory):

```bash
cargo run -p gitcomet --features ui-gpui,gix -- /path/to/repo
```

### Testing

Full headless test suite (CI mode):

```bash
cargo test --workspace --no-default-features --features gix
```

Clippy (CI mode):

```bash
cargo clippy --workspace --no-default-features --features gix -- -D warnings
```

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

You can also run `.github/workflows/deploy-homebrew-tap.yml` manually for backfills or dry-runs.

### Linux packages

`scripts/package-linux.sh` builds every Linux package from one staged payload. The `.deb`, `.rpm` and AppImage include the binary, desktop entry, all five icon sizes, licence files and README. The tarball preserves its historical layout for third-party packagers: only the binary, README, `LICENSE-AGPL-3.0` and `NOTICE`. Package metadata lives in `packaging/linux/`: `debian-control.in` for the `.deb` and `gitcomet.spec` for the `.rpm`.

Linked libraries are declared automatically by `dpkg-shlibdeps` and rpm AutoReq. Libraries the app loads at runtime (Vulkan, EGL, Wayland) are declared by hand. `scripts/check-linux-runtime-deps.sh` runs in CI and on every release build, and fails when the binary's libraries or its glibc baseline drift from those declarations.

The `deb_revision` and `rpm_release` inputs of `.github/workflows/release-manual-main.yml` set the package revision (the `1` in `1.2.3-1`). Raise them when rebuilding packages for an existing version through a manual dispatch of `.github/workflows/build-release-artifacts.yml`.

The RPM supports Fedora 42 and newer. This repository no longer publishes to the AUR; Arch users are served by the community-maintained `gitcomet-bin` package.
