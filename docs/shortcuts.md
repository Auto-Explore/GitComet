# GitComet Shortcuts

This file documents the keyboard shortcuts currently wired in the GPUI application.

Source of truth:
- `crates/gitcomet-ui-gpui/src/app.rs`
- `crates/gitcomet-ui-gpui/src/focused_diff.rs`
- `crates/gitcomet-ui-gpui/src/view/panels/main/diff_view.rs`
- `crates/gitcomet-ui-gpui/src/view/conflict_resolver.rs`

Notes:
- `Cmd` and `Option` are the macOS names. `Ctrl` and `Alt` are the Windows/Linux equivalents.
- Some controls keep extra compatibility aliases in addition to the primary platform shortcut.
- Context menus also display per-entry shortcuts inline.

## App window shortcuts

These shortcuts apply in the normal GitComet window.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Open a new window | `Cmd-N`, `Cmd-Shift-N` | `Ctrl-N`, `Ctrl-Shift-N` | |
| Open Settings | `Cmd-,` | `Ctrl-,` | |
| Open a repository | `Cmd-O` | `Ctrl-O` | |
| Open recent repositories | `Cmd-Shift-O`, `Option-Cmd-O` | `Ctrl-Shift-O` | |
| Close the active repository tab, or close the window if no repo tab can close | `Cmd-W` | `Ctrl-W` | |
| Close the active window | `Cmd-Shift-W` | `Ctrl-Shift-W` | |
| Previous repository tab | `Cmd-PageUp`, `Cmd-{`, `Option-Cmd-Left` | `Ctrl-PageUp` | |
| Next repository tab | `Cmd-PageDown`, `Cmd-}`, `Option-Cmd-Right` | `Ctrl-PageDown` | |
| Toggle full screen | `Ctrl-Cmd-F` | `F11` | |
| Quit GitComet | `Cmd-Q` | `Ctrl-Q` | |

macOS-only window-management shortcuts:
- `Cmd-M`: Minimize the active window.
- `Cmd-H`: Hide GitComet.
- `Option-Cmd-H`: Hide other applications.

## Text input shortcuts

These shortcuts apply when a GitComet text input has focus.

### Editing

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Select all | `Cmd-A` | `Ctrl-A` | `Ctrl-A` is also accepted on macOS. |
| Copy | `Cmd-C` | `Ctrl-C` | `Ctrl-C` is also accepted on macOS. |
| Paste | `Cmd-V` | `Ctrl-V` | `Ctrl-V` is also accepted on macOS. |
| Cut | `Cmd-X` | `Ctrl-X` | `Ctrl-X` is also accepted on macOS. |
| Undo | `Cmd-Z` | `Ctrl-Z` | |
| Redo | `Cmd-Shift-Z` | `Ctrl-Shift-Z` | |
| Show the character palette | `Ctrl-Cmd-Space` | None | macOS only. |

### Cursor movement and selection

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Move by character or line | Arrow keys | Arrow keys | `Left`, `Right`, `Up`, `Down` |
| Select by character or line | `Shift` + arrow keys | `Shift` + arrow keys | |
| Move to line start / end | `Cmd-Left`, `Cmd-Right`, `Home`, `End` | `Home`, `End` | |
| Select to line start / end | `Cmd-Shift-Left`, `Cmd-Shift-Right`, `Shift-Home`, `Shift-End` | `Shift-Home`, `Shift-End` | |
| Move by page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | |
| Select by page | `Shift-PageUp`, `Shift-PageDown` | `Shift-PageUp`, `Shift-PageDown` | |

### Word movement and word deletion

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Move left / right by word | `Option-Left`, `Option-Right` | `Ctrl-Left`, `Ctrl-Right` | |
| Select left / right by word | `Option-Shift-Left`, `Option-Shift-Right` | `Ctrl-Shift-Left`, `Ctrl-Shift-Right` | |
| Delete word to the left / right | `Option-Backspace`, `Option-Delete` | `Ctrl-Backspace`, `Ctrl-Delete` | |

Compatibility note:
- GitComet also keeps the opposite modifier family wired in text inputs where practical, so `Alt`-based word movement and `Ctrl`-based editing aliases remain available as portability fallbacks.
- Diff-navigation fallbacks stay active from focused GitComet text inputs for `F1`, `F4`, `F2`, `F3`, `F7`, `Shift-F7`, `Alt-Up`, and `Alt-Down` when the input does not handle those keys itself.

### Commit composer

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Commit staged changes | `Cmd-Enter` | `Ctrl-Enter` | Commit message input only, and only when the Commit action is enabled. |

## Diff view shortcuts

These shortcuts apply in the main diff panel, including conflict resolution views where noted.

| Action | macOS | Windows / Linux | Scope / notes |
| --- | --- | --- | --- |
| Search the current diff | `Cmd-F` | `Ctrl-F` | If rendered markdown preview is open, GitComet switches back to source mode before opening search. |
| Previous search match | `F2` | `F2` | While diff search is open. |
| Next search match | `F3` | `F3` | While diff search is open. |
| Close search, clear selection, or close the current diff | `Escape` | `Escape` | Exact behavior depends on the current diff state. |
| Previous file in the status list | `F1` | `F1` | Working tree and conflict-oriented diff flows. |
| Next file in the status list | `F4` | `F4` | Working tree and conflict-oriented diff flows. |
| Previous change | `F2`, `Shift-F7`, `Option-Up` | `F2`, `Shift-F7`, `Alt-Up` | Raw diff and conflict diff views. |
| Next change | `F3`, `F7`, `Option-Down` | `F3`, `F7`, `Alt-Down` | Raw diff and conflict diff views. |
| Switch to inline diff | `Option-I` | `Alt-I` | Raw file diff only. Conflict resolver keeps split layout. |
| Switch to split diff | `Option-S` | `Alt-S` | Raw file diff only. |
| Toggle whitespace visibility | `Option-W` | `Alt-W` | Raw diff / conflict diff only. |
| Open the hunk picker | `Option-H` | `Alt-H` | When hunk navigation is available. |
| Stage or unstage the current working-tree file and advance to the adjacent file | `Space` | `Space` | Raw working-tree file diff only, and not while the diff search input has focus. |
| Select all diff text | `Cmd-A` | `Ctrl-A` | File preview and text-selection flows. |
| Copy selected diff text | `Cmd-C` | `Ctrl-C` | File preview and text-selection flows. |
| Pick conflict result `Base / Ours / Theirs / Both` | `A`, `B`, `C`, `D` | `A`, `B`, `C`, `D` | Conflict resolver only. |

Preview-mode note:
- Rendered markdown preview hides the raw diff navigation controls and ignores the raw-diff-only view toggles, hunk picker, whitespace toggle, and conflict navigation hotkeys until you return to source mode.
- The diff navigation keys above still work while a GitComet text input has focus, but search activation, `Escape`, view toggles, hunk picker, and staging `Space` remain tied to the active diff surface rather than text inputs.

## Context menu shortcuts

Context-menu keyboard behavior is the same on every platform:
- `Up` / `Down`: move the selection.
- `Enter`: activate the selected item, or the first enabled item if nothing is selected.
- `Escape`: close the menu.
- Single-letter shortcuts shown inline activate the matching entry.

Additional note:
- Some menus also show clipboard-style shortcuts such as `Ctrl+C`; those reflect the underlying view shortcut rather than the menu's generic single-letter dispatcher.

## Focused diff window

These shortcuts apply in the standalone focused diff window opened for difftool-style flows.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Close the window | `Cmd-W`, `Ctrl-W`, `Escape`, `Q` | `Ctrl-W`, `Escape`, `Q` | `Ctrl-W` remains accepted on macOS as an extra alias. |
