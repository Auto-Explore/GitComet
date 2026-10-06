# Native terminal grid alignment

Combining the view fixes with `perf/linked_worktrees` changes the native text
backend. The real terminal survey exposed new per-glyph hit-test/caret lookup
costs while fitting backend glyphs onto Alacritty's fixed cell grid. Plain ASCII
rows with one glyph per cell and one fragment per source run now map directly to
the cell columns. Unicode, combining sequences, ligatures and split font
fragments retain the general native cluster mapping. Fragment bounds use indexed
cell ranges instead of rescanning all cells for every coloured run.

The quiet native development runs recorded in
[terminal-grid-2026-10-06.json](terminal-grid-2026-10-06.json) observed:

| Repository | Integrated baseline draw p95 | Grid follow-up draw p95 | Follow-up frames/s |
| --- | ---: | ---: | ---: |
| GitComet | 36.13 ms | 16.92 ms | 58.53 |
| Pro | 32.71 ms | 16.15 ms | 59.36 |

Pro's original parser-lock baseline was 99.80 ms and about 13 frames/s. The
combined fixes avoid parser waits and per-glyph ASCII hit testing. Development
History selection still has frames above 16 ms, and first-use GPU/text startup
costs are not claimed to be eliminated. The integrated warm-cache History run
passed the one-second guard; that is not a cold-driver guarantee.

Validation covers coloured ASCII cells in both bundled mono/proportional fonts,
wide and combining glyphs, parser-lock contention and the full terminal panel
suite, plus the merged hosted pane suite. No GPU experiment is enabled.
