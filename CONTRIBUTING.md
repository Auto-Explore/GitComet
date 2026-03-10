## Contributing

### Workspace layout

- `crates/gitcomet-core`: domain types, merge algorithm, conflict session, text utils.
- `crates/gitcomet-git`: Git abstraction + no-op backend.
- `crates/gitcomet-git-gix`: `gix`/gitoxide backend implementation.
- `crates/gitcomet-state`: MVU state store, reducers, effects, conflict session management.
- `crates/gitcomet-ui`: UI model/state (toolkit-independent).
- `crates/gitcomet-ui-gpui`: gpui views/components (focused diff/merge windows, conflict resolver, word diff).
- `crates/gitcomet-app`: binary entrypoint, CLI (clap), difftool/mergetool/setup/uninstall modes.

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
cargo build -p gitcomet-app --features ui,gix
```

To also compile the gpui-based UI crate:

```bash
cargo build -p gitcomet-app --features ui-gpui,gix
```

Run (opens the repo passed as the first arg, or falls back to the current directory):

```bash
cargo run -p gitcomet-app --features ui-gpui,gix -- /path/to/repo
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
