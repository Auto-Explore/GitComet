## Contributing

### Workspace layout

- `crates/gitcomet-core`: domain types, merge algorithm, conflict session, text utils.
- `crates/gitcomet-git`: Git abstraction + no-op backend.
- `crates/gitcomet-git-gix`: `gix`/gitoxide backend implementation.
- `crates/gitcomet-state`: MVU state store, reducers, effects, conflict session management.
- `crates/gitcomet-ui`: UI model/state (toolkit-independent).
- `crates/gitcomet-ui-gpui`: gpui views/components (focused diff/merge windows, conflict resolver, word diff).
- `crates/gitcomet`: binary entrypoint, CLI (clap), difftool/mergetool/setup/uninstall modes.

### Getting started

Windows prerequisites (Windows 10/11):

- Install Visual Studio 2022 (Community or Build Tools).
- Install the `Desktop development with C++` workload.
- Ensure both MSVC tools and Windows 10/11 SDK components are installed.
- This repo configures Cargo to use `scripts/windows/msvc-linker.cmd`, so `cargo build` works from a regular PowerShell/CMD shell when those components are present.

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
- Linux: tar.gz + AppImage + .deb + Flatpak bundle
- macOS: DMG + tar.gz for `arm64` and `x86_64`
- Flathub assets: `gitcomet-v<VERSION>-source.tar.gz`, `gitcomet-v<VERSION>-cargo-vendor.tar.gz`, `dev.gitcomet.GitComet.yaml`, and `flathub.json`
- Homebrew cask asset: `gitcomet.rb` (generated from macOS DMG artifacts and their SHA256 values)
- Homebrew CLI formula asset: `gitcomet-cli.rb` (generated from macOS + Linux x86_64 tarballs and their SHA256 values)

### Homebrew deployment

To push `Casks/gitcomet.rb` and `Formula/gitcomet-cli.rb` into a Homebrew tap repo automatically on release:

1. Create a tap repository (default expected name: `OWNER/homebrew-gitcomet`).
2. In this repo, configure:
   - secret `HOMEBREW_TAP_TOKEN`: GitHub token with `contents:write` access to the tap repository.
   - variable `HOMEBREW_TAP_REPO`: tap repository in `OWNER/REPO` form.
   - optional variable `HOMEBREW_TAP_BRANCH`: target branch (default `main`).
3. Run `.github/workflows/release-manual-main.yml` with `draft=false`.

This release flow will:

- build and upload release artifacts
- publish the GitHub release
- call `.github/workflows/deploy-homebrew-tap.yml` to update `Casks/gitcomet.rb` and `Formula/gitcomet-cli.rb` in the tap repo

You can also run `.github/workflows/deploy-homebrew-tap.yml` manually for backfills or dry-runs.

### AUR deployment

To push `PKGBUILD` and `.SRCINFO` into the live AUR repository automatically on release:

1. Ensure the `gitcomet` AUR package repository exists.
2. In this repo, configure:
   - secret `AUR_PRIVATE_SSH_KEY`: the AUR-authorized SSH private key.
   - secret `AUR_PRIVATE_SSH_KEY_PASSPHRASE`: the passphrase for that SSH key.
   - optional variable `AUR_GIT_REPOSITORY`: AUR Git remote URL (default: `ssh://aur@aur.archlinux.org/gitcomet.git`).
   - optional variable `AUR_GIT_BRANCH`: AUR branch for that remote (default `master`).
3. Run `.github/workflows/release-manual-main.yml` with `draft=false`.

This release flow will:

- download the published Linux release tarball and source tarball
- update `PKGBUILD` `pkgver` and `sha256sums`
- regenerate `.SRCINFO`
- validate sources with `makepkg --verifysource`
- clone the configured AUR repo over HTTPS for read access
- push the updated metadata into the configured AUR repo over SSH using the configured key

The previous `AUR_REPO_TOKEN` secret is no longer used.

You can also run `.github/workflows/deploy-aur.yml` manually for backfills or dry-runs.

### Flathub deployment

The app ID used for Flatpak/Flathub packaging is `dev.gitcomet.GitComet`.

To push `dev.gitcomet.GitComet.yaml` and `flathub.json` into the Flathub packaging repo automatically on release:

1. Complete the one-time Flathub onboarding so the app has a packaging repo, typically `flathub/dev.gitcomet.GitComet`.
2. In this repo, configure:
   - secret `FLATHUB_TOKEN`: GitHub token with `contents:write` access to the Flathub packaging repository.
   - optional variable `FLATHUB_REPO`: packaging repository in `OWNER/REPO` form. Defaults to `flathub/dev.gitcomet.GitComet`.
   - optional variable `FLATHUB_BRANCH`: target branch. If unset, the packaging repo default branch is used.
   - optional variable `FLATHUB_MODE`: `pull_request` (default, opens/updates a PR against the packaging repo) or `push` (commits straight to the target branch).
3. Run `.github/workflows/release-manual-main.yml` with `draft=false`.

This release flow will:

- build and upload the Flatpak bundle plus Flathub source/manifest assets
- publish the GitHub release so the source tarball URL is public
- call `.github/workflows/deploy-flathub.yml` to update the Flathub packaging repo, either by opening/updating a PR or by pushing directly depending on `FLATHUB_MODE`

You can also run `.github/workflows/deploy-flathub.yml` manually for backfills or dry-runs.
