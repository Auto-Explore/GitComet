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

Only the changed tokens are highlighted; a changed line keeps the plain background, and its line number shows the change. A line whose own side has no changed tokens is not shown as removed or added: Inline shows such a pair once, from the side that changed, with both line numbers. An added or removed final newline is not a token either, so it changes no row. Split keeps both source texts side by side, including formatting that Difftastic considers structurally unchanged. In collapsed mode, a formatting-only comparison shows **Difftastic - Collapsed mode** and **No syntactic changes; text differs** on the main canvas, with **Show full text** and **Show line changes** actions. Ignore whitespace is unavailable while a structural result is displayed; the saved whitespace preference remains intact. Show line changes selects the line engine with whitespace shown; when the viewer's engine changes again or the viewer closes, the whitespace mode returns to what it was.

If Difftastic cannot compare a file, the viewer uses the line engine and displays the reason. Unsupported languages, parse errors, resource limits (size, complexity, nesting depth, time), and internal Difftastic failures can cause a fallback. Unlike the Difftastic command-line tool, GitComet does not compare Verilog or Fortran structurally. The user's engine preference stays selected. Image/binary views and merge resolution retain their existing behavior.

Standalone GUI file comparisons also use full sources and support both engines. Regular UTF-8 file pairs up to the source limit are eligible. Directory, patch-only, non-UTF-8, binary, symlink, oversized, mode-change, and BOM-change inputs retain the patch view. Before/After labels are preserved separately from the language-detection filename.

## Partial stage, unstage, and discard

Select whole source lines or act on a structural change block. Token-level patch actions are not supported. A side-only selection includes only that side. A gutter button, or a right-click without a selection, on a pair changed on one side acts on both of its lines. Structural blocks exclude display context and do not expand to enclosing Git hunks.

GitComet prepares the exact Git patch for those whole lines and applies it at once, as the line engine applies its line and hunk actions. A queued action is rejected if its repository, target, viewer, or source generation changes before the worker starts applying it. The worker then revalidates the sources and attributes; a changed snapshot is rejected with a notice to try again on the current diff. An application lock excludes GitComet's other index and worktree changes and editor saves during validation and application; fetches and pushes do not hold it. External programs do not participate in that lock.

These actions preserve unrelated edits, line endings, and missing final newlines. Existing restrictions on managed payloads and linked-worktree partial actions remain in place.

## Building and packaging

Difftastic is [0.71.0](https://github.com/Wilfred/difftastic/releases/tag/0.71.0). It and its grammars are listed under **Settings → Links → Open source licenses**. `vendor/README.md` records the import recipe and how to upgrade.

## Runtime contract

GitComet links a trimmed copy of Difftastic as a Rust library (`vendor/difftastic`; its changes are listed in `vendor/README.md`) and runs it in-process through `crates/gitcomet-difftastic`. It renders native paged rows from Difftastic's line alignment and changed-token byte ranges. Difftastic does not supply applicable Git patches; partial actions use GitComet's separate Git patch pipeline.

Each comparison allows 1,000,000 bytes per source, 3,000,000 graph vertices, syntax trees 1,000 levels deep, and five seconds. Queue wait is excluded. One comparison runs at a time, on its own thread with a 4 MiB stack, since a comparison can use about 1 GB of memory. Obsolete work is cancelled and stops within about half a second. Difftastic reads no environment variables. A panic inside Difftastic becomes a fallback, not a crash report. The adapter validates zero-based source lines, UTF-8 byte ranges, and complete source coverage, and preserves carriage returns.

The hosted pane API exposes `DiffEngine`, `DiffPaneOptions.engine_override`, and `DiffPane::set_engine_override`. Hosted and repository viewers share prepared engine results, source-line anchors, cancellation, and engine-aware cache identity.
