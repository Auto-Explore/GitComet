# Difftastic

GitComet offers two diff engines: **Line by line** and **Difftastic**. Difftastic compares parsed syntax and highlights changed tokens. The existing line engine remains the default.

## Choosing an engine

Select **Settings → Diff → Diff Engine** to change the global default. The diff viewer's cog offers **Use default**, **Line by line**, and **Difftastic**. A viewer override survives file navigation, stays local to that viewer, and resets when the viewer closes. It is not saved with the session.

The engine is independent of presentation:

| Control | Choices | Effect |
| --- | --- | --- |
| Diff Engine | Line by line / Difftastic | How changes and line alignment are computed |
| Layout | Split / Inline | How the two sides are arranged |
| Content | Collapsed / Full | Whether unchanged regions are folded |

Both source texts remain available, including formatting that Difftastic considers structurally unchanged. A formatting-only comparison shows a notice with **Show full text** and **Show line changes** actions. Ignore whitespace is unavailable while a structural result is displayed; the saved whitespace preference remains intact. Show line changes temporarily selects the line engine with whitespace shown.

If Difftastic cannot compare a file, the viewer uses the line engine and displays the reason. Unsupported languages, parse errors, resource limits, unavailable helpers, and invalid helper output can cause a fallback. The user's engine preference stays selected. Image/binary views and merge resolution retain their existing behavior.

Standalone GUI file comparisons also use full sources and support both engines. Regular UTF-8 file pairs up to the source limit are eligible. Directory, patch-only, non-UTF-8, binary, symlink, oversized, mode-change, and BOM-change inputs retain the patch view. Before/After labels are preserved separately from the language-detection filename.

## Partial stage, unstage, and discard

Select whole source lines or act on a structural change block. Token-level patch actions are not supported. A side-only selection includes only that side. Structural blocks exclude display context and do not expand to enclosing Git hunks.

GitComet prepares the exact Git patch and opens a read-only preview using the line engine. Applying the preview uses those reviewed bytes. If the file, index, attributes, target, or viewer changes before application, the action is rejected and must be prepared again. The worker revalidates the source snapshot, and an application lock excludes other GitComet repository mutations and editor saves during validation and application. External programs do not participate in that lock.

These actions preserve unrelated edits, line endings, and missing final newlines. Existing restrictions on managed payloads and linked-worktree partial actions remain in place.

## Building and packaging

Release packages bundle [Difftastic 0.71.0](https://github.com/Wilfred/difftastic/releases/tag/0.71.0). Development builds need Python 3.12 or newer, Cargo, and the platform's Rust/C build toolchain:

```sh
python3 scripts/build-difftastic.py --profile debug --output target/debug/helpers
python3 scripts/check-difftastic.py --helper target/debug/helpers/difft --notices target/debug/helpers/difftastic-NOTICES.txt
```

Use `python` and `difft.exe` on Windows. For release builds use `--profile release --output target/release/helpers`. `--target` supports an explicit Rust target with the appropriate cross-compilation toolchain. The builder verifies the pinned source archive checksum and builds with upstream's lockfile in an isolated Cargo workspace. It also generates `difftastic-NOTICES.txt`, including parser/dependency licenses and an inventory.

The app resolves a bundled helper relative to its executable, without searching `PATH`:

| Distribution | Helper location |
| --- | --- |
| Development / portable | `helpers/difft` beside `gitcomet` (`difft.exe` on Windows) |
| Linux installation / AppImage | `../libexec/gitcomet/difft` relative to the app's `bin` directory |
| macOS app | `Contents/Helpers/difft` |
| Windows MSI | `bin/helpers/difft.exe` |

The Linux installer and macOS packager build the helper unless `--no-build` is supplied. All package paths require the helper and notices. Release CI checks extracted Linux packages, the installed Windows MSI, and the mounted macOS disk image. The Windows and macOS signing flows also sign the helper.

## Runtime contract

Difftastic exposes an executable rather than a Rust library. GitComet consumes its [unstable JSON protocol](https://github.com/Wilfred/difftastic/blob/0.71.0/src/display/json.rs) through a version-pinned adapter. It renders native paged rows from line alignment and changed-token byte ranges, without parsing terminal colors. Difftastic does not supply applicable Git patches; partial actions use GitComet's separate Git patch pipeline.

Each comparison allows 1,000,000 bytes per source, 3,000,000 graph vertices, a shared five-second deadline for the version probe and comparison, 32 MiB stdout, and 64 KiB stderr. Queue wait is excluded. At most two workers run concurrently. Obsolete work is cancelled and reaped. The child environment removes inherited `DFT_*` and `GIT_*` settings. The adapter validates zero-based source lines, UTF-8 byte ranges, and complete source coverage; it preserves carriage returns and removes only the helper's synthetic EOF row.

The hosted pane API exposes `DiffEngine`, `DiffPaneOptions.engine_override`, and `DiffPane::set_engine_override`. Hosted and repository viewers share prepared engine results, source-line anchors, cancellation, and engine-aware cache identity.
