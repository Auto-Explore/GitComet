# Difftastic Integration

This document records the approved Difftastic integration being implemented for GitComet.

## Behavior

The comparison engine is independent of Split/Inline layout and Collapsed/Full content. Settings > Diff selects the global default, initially Line by line. The viewer cog offers Use default, Line by line, and Difftastic. Overrides last until that viewer closes, including across file navigation, and are never persisted.

Difftastic runs as a bundled, pinned helper and supplies line alignment and token changes to the native paged viewer. Both source texts remain available, including structurally unchanged formatting. Formatting-only comparisons explain that text differs without syntactic changes. Unsupported comparisons fall back visibly without changing the preference. Merge resolution retains its existing algorithm.

Partial stage, unstage, and discard operate on whole source lines or structural blocks. They prepare an exact Git patch, preview it using the line engine, and apply the reviewed bytes only while their source identities remain current. Side-only selections remain side-only. Blocks never expand to enclosing Git hunks or display-context padding. Token staging is outside this change.

The first release also covers standalone GUI file pairs. Patch-only and directory comparisons use a labelled line-engine fallback.

## Implementation sequence

- Native engine integration: core contracts, isolated helper adapter, both native layouts, settings and viewer overrides, cancellation and explicit fallback.
- Partial actions: exact patch source, source-line mapping, immutable preview, guarded application, real Git verification.
- Distribution: focused file-pair inputs, pinned helper builds, all existing packages and notices, installed-package smoke tests.

The initial process limits are 1,000,000 bytes per source, 3,000,000 graph vertices, five seconds per comparison, two concurrent workers, 32 MiB stdout and 64 KiB stderr. Obsolete work is cancelled and reaped. The adapter validates zero-based lines and UTF-8 byte ranges, preserves carriage returns, and removes only synthetic EOF rows introduced by the helper.

## Validation

Cover the engine/layout/content matrix, preference restoration, independent viewers, source-line anchors, and stale completions. Protocol fixtures cover Unicode, tabs, CRLF, missing final newlines, wrappers, multiline changes, formatting-only comparisons, additions/deletions, parse errors and resource limits. Real repository tests verify exact index and worktree results, including reverse operations, side-only selections, multiple Git hunks, unselected formatting and stale previews. Existing performance and cross-platform checks remain required.

## Upstream contract

The pinned release is [Difftastic 0.71.0](https://github.com/Wilfred/difftastic/releases/tag/0.71.0). It exposes a binary, not a Rust library. Its [unstable JSON protocol](https://github.com/Wilfred/difftastic/blob/0.71.0/src/display/json.rs) supplies full alignment for changed text and changed-token ranges; status-only outputs require explicit handling. Difftastic does not generate applicable Git patches. Helper builds stay outside the application's Cargo workspace to isolate Tree-sitter versions. Distribution must carry Difftastic and dependency/parser notices.
