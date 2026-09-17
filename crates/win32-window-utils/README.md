# Win32 window utilities

This crate wraps the Win32 APIs used for GitComet's system menu, console control
handling, process termination, and thread CPU timing. Its private raw bindings
are generated for just those APIs and depend only on `windows-link`.

## Regenerating bindings

From this directory, run:

```sh
cargo run --locked --manifest-path tools/generate-bindings/Cargo.toml
```

Add API names to `tools/generate-bindings/bindings.txt` when extending the wrapper.
The generator automatically includes types required by function signatures.
Review and commit `src/bindings.rs` after regeneration; do not edit it by hand.
The generator is a separate Cargo workspace so normal library builds do not
compile `windows-bindgen` or its metadata dependencies.

Validate with `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`.
