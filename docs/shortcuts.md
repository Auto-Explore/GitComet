# GitComet Shortcuts

This file documents the keyboard shortcuts currently wired in the GPUI application.

Source of truth:
- `crates/gitcomet-ui-gpui/src/app/bindings.rs`
- `crates/gitcomet-ui-gpui/src/view/command_shortcuts.rs`
- `crates/gitcomet-ui-gpui/src/view/gitcomet_view_render.rs`
- `crates/gitcomet-ui-gpui/src/view/terminal_panel.rs`
- `crates/gitcomet-ui-gpui/src/view/panels/main/diff_view.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/history/find.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/file_history.rs`
- `crates/gitcomet-ui-gpui/src/view/panes/diff/conflict_actions/`

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
| Open the command palette | `Cmd-P`, `Cmd-Shift-P` | `Ctrl-P`, `Ctrl-Shift-P` | Search for an action; its shortcut is shown beside it. |
| Show keyboard shortcuts | `Cmd-/`, `Cmd-?` | `Ctrl-/`, `Ctrl-?` | Also the keyboard button in the bottom bar. Opens the searchable cheat sheet; Escape closes it and restores focus. With terminal focus on Windows/Linux, use `Ctrl-Shift-/`. |
| Open a repository | `Cmd-O` | `Ctrl-O` | |
| Open a workspace | `Cmd-Shift-R` | `Ctrl-Shift-R` | Lists saved workspaces. In an empty window the chosen workspace opens there; otherwise its own window is focused or opened. Also in the command palette and the app menu. |
| Go to a commit | `Cmd-G` | `Ctrl-G` | Opens the Go to dialog. Accepts a full or short SHA (4+ characters, unique), a branch, a tag, or any revision such as `HEAD~3`; matches are exact. In an embedded terminal the shell keeps `Ctrl-G`. |
| Toggle open and recently closed repositories | `Ctrl-Shift-A`, `Cmd-Shift-A`, `Cmd-Shift-O`, `Cmd-Option-O` | `Ctrl-Shift-A`, `Ctrl-Shift-O` | In an embedded terminal on Windows/Linux, `Ctrl-Shift-A` keeps its terminal “Select All” behavior. |
| Open active repository in external code editor | `Cmd-Shift-E` | `Ctrl-Shift-E` | Only active when an external code editor is configured. |
| Show the open file in the file explorer | `Cmd-Shift-L` | `Ctrl-Shift-L` | Switches the sidebar to Files, expands the folders leading to the file, and scrolls it into view. |
| Open the repository's remote in a web browser | `Cmd-K` | `Ctrl-K` | Opens the remote's page on its host (GitHub, GitLab, Bitbucket, Azure DevOps, Gitea/Codeberg, AWS CodeCommit, or a self-hosted forge). With several such remotes a menu lists them, `origin` first; `1`–`9` pick directly. Also in the command palette, the app menu, and a remote's right-click menu in the sidebar. In an embedded terminal on Windows/Linux the shell keeps `Ctrl-K`. |
| Close the active repository tab, or close the window if no repo tab can close | `Cmd-W` | `Ctrl-W` | |
| Close the active window | `Cmd-Shift-W` | `Ctrl-Shift-W` | |
| Previous repository tab | `Cmd-PageUp`, `Cmd-{`, `Cmd-Option-Left` | `Ctrl-PageUp`, `Ctrl-Shift-Tab` | |
| Next repository tab | `Cmd-PageDown`, `Cmd-}`, `Cmd-Option-Right` | `Ctrl-PageDown`, `Ctrl-Tab` | |
| Toggle full screen | `Cmd-Ctrl-F` | `F11` | |
| Increase UI scale | `Cmd-+`, `Cmd-=` | `Ctrl-+`, `Ctrl-=` | |
| Decrease UI scale | `Cmd--` | `Ctrl--` | |
| Reset UI scale | `Cmd-0` | `Ctrl-0` | |
| Quit GitComet | `Cmd-Q` | `Ctrl-Q` | |

Additional window-management shortcuts:

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Minimize the active window | `Cmd-M` | None | macOS only. |
| Hide the application | `Cmd-H` | None | macOS only. |
| Hide other applications | `Cmd-Option-H` | None | macOS only. |

## Repository and layout shortcuts

These shortcuts apply in a normal repository window, including from text inputs.
They are inactive while a menu, dialog, command palette, Go to dialog, or recent-documents picker is open.
A focused embedded terminal keeps these keys for its shell or TUI, except for the
terminal toggle and shortcut cheat sheet (use their shifted terminal variants on Windows/Linux). Repository availability checks and confirmations
are the same as when running the corresponding command from the palette.
On Windows, `Ctrl-Alt` is AltGr: where the keyboard layout types a character for
that chord (AltGr-S is `ś` on Polish), the shortcut does not run.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Reload repository | `Cmd-R`, `F5` | `Ctrl-R`, `F5` | |
| Fetch all remotes | `Cmd-Shift-F` | `Ctrl-Shift-F` | |
| Pull | `Cmd-Option-Down` | None | Uses the configured upstream. Available from the palette and buttons. |
| Push | `Cmd-Option-Up` | None | Opens upstream setup when required. Available from the palette and buttons. |
| Open commit dialog | `Cmd-Option-Enter` | `Ctrl-Alt-Enter` | Review and submit in the dialog; `Cmd-Enter` / `Ctrl-Enter` still submits from the commit message input. |
| Stage all changes | `Cmd-Option-S` | `Ctrl-Alt-S` | Unresolved conflict markers still require confirmation. |
| Unstage all changes | `Cmd-Option-U` | `Ctrl-Alt-U` | |
| Create branch | `Cmd-Option-N` | `Ctrl-Alt-N` | Opens the branch creation dialog. |
| Checkout branch | `Cmd-Option-C` | `Ctrl-Alt-C` | Opens the branch picker. |
| Stash changes | `Cmd-Shift-S` | `Ctrl-Shift-S` | Opens the stash dialog. |
| Toggle sidebar | `Cmd-B` | `Ctrl-B` | |
| Toggle details pane | `Cmd-Option-B`, `Cmd-Shift-B` | `Ctrl-Alt-B`, `Ctrl-Shift-B` | |
| Toggle embedded terminal | `Cmd-J` | `Ctrl-J`, `Ctrl-Shift-J` | With terminal focus on Windows/Linux, use `Ctrl-Shift-J`. Closing an active session keeps the existing confirmation. |
| Open external terminal | `Cmd-Option-T` | None | Available from the palette and terminal controls. |

## Image preview shortcuts

These keys apply when an image viewport has focus. They also work in image diffs
and image conflict previews.

| Action | Shortcut |
| --- | --- |
| Zoom in / out | `+` / `-` |
| Fit | `0` or double-click |
| Actual size (100%) | `1` |
| Zoom around pointer | Ctrl/Cmd + scroll |
| Pan | Drag or scroll |

The sampling and shared zoom menus are in the file's bottom bar. When comparison
images are unlinked, each pane has its own zoom controls beneath the image.
Ctrl/Cmd + scroll over an image changes image zoom, and over a Mermaid diagram
the diagram's zoom (see [Diagram previews](file-management.md#diagram-previews));
elsewhere it changes application UI scale.

## Text input shortcuts

File operation shortcuts are scoped to the Files explorer. See
[File management and Documents](file-management.md#explorer) for selection,
clipboard, rename, and filesystem Undo/Redo bindings.

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
| Show the character palette | `Cmd-Ctrl-Space` | None | macOS only. |
| Delete to line start / end | `Cmd-Backspace`, `Cmd-Delete` | `Ctrl-Shift-Backspace`, `Ctrl-Shift-Delete` | With the caret at the end of a single-line input, such as a search box, deleting to the line start clears it. At the start or end of a line, deletes the line break instead. |

### Cursor movement and selection

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Move by character or line | Arrow keys | Arrow keys | `Left`, `Right`, `Up`, `Down` |
| Select by character or line | `Shift` + arrow keys | `Shift` + arrow keys | |
| Move to document start / end | `Cmd-Home`, `Cmd-End` | `Ctrl-Home`, `Ctrl-End` | Picker search inputs also select the first / last result. |
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
- Diff-navigation fallbacks stay active from focused GitComet text inputs for `F1`, `F4`, `F2`, `F3`, `F7`, and `Shift-F7` when the input does not handle those keys itself.

### Commit composer

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Commit staged changes | `Cmd-Enter` | `Ctrl-Enter` | Commit message input only, and only when the Commit action is enabled. |

### Commit message editor

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Update commit | `Cmd-Enter` | `Ctrl-Enter` | While editing a commit's message in Commit details, and only when Update commit is enabled. For a commit already on the upstream, the first press asks to confirm. |
| Cancel editing | `Escape` | `Escape` | Discards the edited message. |

## History view shortcuts

These shortcuts apply while the commit history list is showing.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Previous / next row | `Up`, `Down` | `Up`, `Down` | While the history list has focus. |
| Previous / next page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | While the history list has focus. Moves selection by one visible page, updates the selected row's details, and stops at the first / last available row. With nothing selected, selects the first row. |
| First / last row | `Home`, `End` | `Home`, `End` | While the history list has focus. Also with nothing selected. |
| Find a commit | `Cmd-F` | `Ctrl-F` | Opens the find bar over the history. Matches the commit summary or the author (each on its own), or the start of the SHA (4+ hex characters), across the whole history, not only the loaded rows. Typing selects the first match. With the bar already open, refocuses it with the query selected. |
| Find options | `Aa`, `W`, `.*` buttons | `Aa`, `W`, `.*` buttons | Match Case, Whole Word and Regex toggles, as in diff search; toggling one searches again and selects the first match. Without Match Case, plain text ignores ASCII letter case only (`É` and `é` differ), while a regex ignores Unicode case. The SHA prefix always ignores case and Whole Word, and applies only while Regex is off. An invalid regex reads "Invalid regex" and searches nothing. |
| Next match | `Enter`, `F3` | `Enter`, `F3` | Selects the next match below the selected commit, wrapping to the top. `Enter` only from the find bar; `F3` also from the list. Also the ↓ button. |
| Previous match | `Shift-Enter`, `F2` | `Shift-Enter`, `F2` | Selects the previous match above the selected commit, wrapping to the bottom. `Shift-Enter` only from the find bar; `F2` also from the list. Also the ↑ button. |
| Close the find bar | `Escape` | `Escape` | Also the × button. Focus returns to the history list. |

## Review navigation

Start in the graph with `F6` (reverse with `Shift-F6`) or Tab. Each file list is
one tab stop; Tab still reaches its filters and controls. The active file row is
outlined only while its list has focus. In a focused diff, the current change
block is outlined instead. Moving focus away hides the outline but keeps the
review position. Selecting a file previews it without moving keyboard focus.

Closing Commit details moves focus to the first nonempty working-copy file list
(Untracked/Unstaged before Staged). With no visible file rows, focus returns to
the main canvas. An open diff preview stays open.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Next / previous pane | `F6`, `Shift-F6` | `F6`, `Shift-F6` | Visible panes only; terminal input and modal dialogs keep focus. |
| Graph: focus changed files | `Enter` | `Enter` | Opens the details pane if hidden. |
| Graph: show working-copy changes | `Escape` | `Escape` | Closes the active search first; otherwise replaces Commit details with Unstaged/Staged while keeping the graph selection. |
| File list: previous / next row | `Up`, `Down` | `Up`, `Down` | Previews files and keeps focus in the list. |
| File list: previous / next page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | Uses the visible list height. |
| File list: first / last row | `Home`, `End` | `Home`, `End` | Stops at the ends. |
| File list: expand / collapse folder | `Right`, `Left` | `Right`, `Left` | Right enters an expanded folder; Left returns to its parent. Enter toggles folders and groups. |
| File list: focus the diff | `Enter` | `Enter` | The selected file stays open. |
| File list: return to graph | `Escape` | `Escape` | Closes file search first. Restores the same commit and graph position. |
| Filter the focused file list | `Cmd-F` | `Ctrl-F` | Case-insensitive path substring. Enter or Down focuses the results; Escape clears the filter. Diff navigation follows the filtered list. |
| Open the selected commit/file menu | `Shift-F10` | `Shift-F10` | Uses the existing context menu and its confirmations. |

Review a commit with `Enter` → `Up`/`Down` → `Enter`, then use `Up`/`Down`
through its changes and files. `Escape` returns one level at a time:
diff → files → graph → working-copy details. Navigation preserves the last
diff position when returning to the same file and waits for pending loads.

## Picker shortcuts

These shortcuts apply while a picker's search input has focus.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Previous / next result | `Up`, `Down` | `Up`, `Down` | `Shift-Tab` / `Tab` also navigate. |
| First / last result | `Cmd-Home`, `Cmd-End` | `Ctrl-Home`, `Ctrl-End` | `Ctrl-Home` / `Ctrl-End` also work on macOS. |
| Previous / next page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | Moves by one visible page and stops at the first / last result. |
| Activate selected result | `Enter` | `Enter` | |
| Close menu or picker | `Escape` | `Escape` | Closes a row's context menu first, keeping the picker open. |

## File history tab

"File history" opens a History tab in the left sidebar, next to Branches and Files. It opens from `Ctrl-H` in the diff, the "Working tree" revision badge in the diff, the "File history" entry in file context menus, and the command palette. The command palette entry opens a file picker: the file open in the diff is the first row, and Enter opens its history. Untracked files are not listed. The tab stays open and lists the commits of one file, following renames. The Diff / File switch in the tab header chooses between the changes of a commit and the whole file at that commit.

These shortcuts apply while the tab's filter input has focus.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Previous / next commit | `Up`, `Down` | `Up`, `Down` | Selects the commit and shows it in the main diff at once. |
| Previous / next page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | Moves by one visible page and shows the commit. |
| First / last commit | `Cmd-Home`, `Cmd-End` | `Ctrl-Home`, `Ctrl-End` | Shows the commit. `Ctrl-Home` / `Ctrl-End` also work on macOS. |
| Clear the filter | `Escape` | `Escape` | The tab stays open. |

The filter matches SHA, summary, and author while preserving history order. Right-click a row to open a revision or its parent, show the file's changes, reveal the commit in history, copy its SHA, or use the standard commit actions. The × button in the tab header closes the tab.

## Embedded terminal shortcuts

These shortcuts apply when the embedded terminal has focus.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Copy terminal selection | `Cmd-C` | `Ctrl-Shift-C` | Disabled/no-op without a terminal selection. |
| Toggle the terminal | `Cmd-J` | `Ctrl-Shift-J` | Closing a running command keeps its confirmation. Plain `Ctrl-J` reaches the shell. |
| Show keyboard shortcuts | `Cmd-/`, `Cmd-?` | `Ctrl-Shift-/`, `Ctrl-Shift-?` | Escape restores terminal focus. Plain `Ctrl-/` reaches the shell. |
| Paste clipboard text | `Cmd-V` | `Ctrl-Shift-V` | Clipboard CRLF/CR line endings are normalized to LF. Bracketed paste is used when the shell enables it. |
| Select all terminal buffer | `Cmd-A` | `Ctrl-Shift-A` | Selects the scrollback buffer plus the visible terminal grid. |
| Scroll terminal history | `Shift-PageUp`, `Shift-PageDown`, `Shift-Home`, `Shift-End` | `Shift-PageUp`, `Shift-PageDown`, `Shift-Home`, `Shift-End` | Only in the normal screen buffer. |

Shell-input note:
- Plain `Ctrl-C`, `Ctrl-V`, and `Ctrl-A` keep going to the shell instead of GitComet clipboard handling.

Mouse and menu behavior:
- Left-drag selects visible terminal text.
- Right-click focuses the terminal and opens a menu without clearing the current selection.
- The terminal menu includes Copy, Paste, Select All, Clear, and Open in External Terminal.
- Clear sends `Ctrl-L` to the shell and is disabled when the embedded terminal is disconnected.

## Diff view shortcuts

These shortcuts apply in the main diff panel, including conflict resolution views where noted.

| Action | macOS | Windows / Linux | Scope / notes |
| --- | --- | --- | --- |
| Open the file history tab | `Ctrl-H` | `Ctrl-H` | Working-tree files and files viewed at a commit; not while a text field has focus. |
| Search the current diff | `Cmd-F` | `Ctrl-F` | Rendered Markdown previews search their rendered text and expand disclosures containing the current match. |
| Insert a newline in diff search | `Shift-Enter` | `Shift-Enter` | Diff search only. The search box also has Match Case, Whole Word, and Regex toggles. |
| Previous search match | `F2` | `F2` | While diff search is open. The ↑ button in the search box does the same. |
| Next search match | `F3` | `F3` | While diff search is open. The ↓ button in the search box does the same. |
| Return to the file list | `Escape` | `Escape` | With diff focus, returns to the matching row and keeps its preview open. If no row matches, closes the preview and focuses the graph. Closes search, an editor, or an inline submodule first. Standalone diff windows retain their close behavior. |
| Previous file | `Option-Up`, `F1` | `Alt-Up`, `F1` | Option/Alt-Up from the focused diff lands at the previous file’s last change. When the open file has left the list (its last change was staged or discarded), opens the list's first file. |
| Next file | `Option-Down`, `F4` | `Alt-Down`, `F4` | Option/Alt-Down from the focused diff lands at the next file’s first change. When the open file has left the list (its last change was staged or discarded), opens the list's first file. |
| Previous / next change block | `Up`, `Down` | `Up`, `Down` | Focused diff only. At a file boundary, continues to the previous file’s last block or the next file’s first block. Stops at the outermost files. Images and files without text changes remain review stops. Conflicts retain their own navigation. |
| Scroll the diff by one page | `PageUp`, `PageDown` | `PageUp`, `PageDown` | Focused diff only, outside the conflict resolver. |
| First / last change block in this file | `Home`, `End` | `Home`, `End` | Focused diff only, outside the conflict resolver. |
| Previous change | `F2`, `Shift-F7` | `F2`, `Shift-F7` | Moves one change block (a run of consecutive changed lines) at a time, landing on its first line and outlining the block, in Full, Collapsed, and whole-commit diffs alike. Pressed outside a text field, also moves keyboard focus into the diff, so `S` and `U` act on the outlined block. The conflict resolver moves by conflict instead. |
| Next change | `F3`, `F7` | `F3`, `F7` | Same unit as Previous change. With nothing selected, goes to the first change block. |
| Switch to inline diff | `Option-I` | `Alt-I` | Raw file diff only. Conflict resolver keeps split layout. |
| Enter or leave the file editor | `Option-E` | `Alt-E` | Not while a text field has focus. Escape also leaves the editor. |
| Save the edited file | `Cmd-S` | `Ctrl-S` | While the file editor is open, wherever focus is. Outside the editor the same chord stages the file. |
| Stage the current or selected working-tree files | `Cmd-S` | `Ctrl-S` | Outside the file editor. Conflicts with unresolved markers require confirmation. |
| Unstage the current or selected files | `Cmd-U` | `Ctrl-U` | Staged file diff or status section. |
| Discard current file changes | `Cmd-D` | `Ctrl-D` | Opens the existing confirmation dialog. |
| Copy the current file path | `Cmd-Shift-C` | `Ctrl-Shift-C` | Working-tree file diff. |
| Open the current file in the code editor | `Cmd-E` | `Ctrl-E` | Requires a configured external editor. |
| Toggle blame / annotation | `Option-B` | `Alt-B` | Raw text diff. |
| Switch to split diff | `Option-S` | `Alt-S` | Raw file diff only. |
| Toggle whitespace characters | `Option-W` | `Alt-W` | Text diff / conflict diff only. |
| Stage or unstage the current working-tree file and advance to the adjacent file | `Space` | `Space` | Raw working-tree file diff only, and not while the diff search input has focus. |
| Discard the file's changes | `Cmd-U` | `Ctrl-U` | Unstaged working-tree diff or a focused unstaged file list, after a confirmation. In a staged diff or list the same chord unstages, so a menu shows one or the other, never both. `Cmd-D` / `Ctrl-D` no longer discards: it is next to `Cmd-S` / `Ctrl-S`. |
| Stage the selected lines | `S` | `S` | Unstaged working-tree diff, while the diff has focus. No modifiers. Acts on the lines selected by dragging over text, by selecting rows, or by right-clicking a line: a right-click focuses the diff, and once its menu closes the keys act on the lines that menu did until the selection changes. With nothing selected it acts on the change block outlined by change navigation or by opening the diff; a mouse selection removes that outline. Pressed outside the diff (e.g. in the file list), but not in a text field, it acts only on that outlined block; the same holds for `U`. After a plain click (a caret only) it does nothing. Once the diff reloads, the next change block is outlined, so `S`, `S`, `S` stages a file block by block. The same as "Stage lines" in the diff's right-click menu. |
| Unstage the selected lines | `U` | `U` | Staged diff, while the diff has focus. No modifiers. The same as "Unstage lines" in the right-click menu. In an unstaged diff the same key discards instead (next row). |
| Discard the selected lines | `U` | `U` | Unstaged working-tree diff, while the diff has focus. No modifiers. Reverts the selected lines, or the outlined change block, in the file without asking, as "Discard lines" in the right-click menu does. `U` rather than `D`, so discard is not next to `S`; unstage shares the key but only applies in a staged diff. `D` does nothing in a diff (in the conflict resolver it picks Both). |
| Select all diff text | `Cmd-A` | `Ctrl-A` | File preview and text-selection flows. |
| Copy selected diff text | `Cmd-C` | `Ctrl-C` | File preview and text-selection flows. |
| Pick conflict result `Base / Ours / Theirs / Both` | `A`, `B`, `C`, `D` | `A`, `B`, `C`, `D` | Conflict resolver only. |
| Pick a conflict source | `Cmd-1`, `Cmd-2`, `Cmd-3` | `Ctrl-1`, `Ctrl-2`, `Ctrl-3` | Conflict resolver only; also works from the resolved-output editor. `Ctrl` aliases work on macOS too. Source choices depend on the two-way or three-way layout. |
| Choose a conflict source everywhere | `Cmd-Shift-1`, `Cmd-Shift-2`, `Cmd-Shift-3` | `Ctrl-Shift-1`, `Ctrl-Shift-2`, `Ctrl-Shift-3` | Includes automatically selected blocks. `Ctrl` aliases work on macOS too. |
| Mark the current conflict unresolved | `U` | `U` | Conflict resolver only, outside text inputs. |
| Previous unresolved conflict | `Shift-F2` | `Shift-F2` | Conflict resolver only. Skips conflicts you have already resolved; works while the resolved-output editor has focus. |
| Next unresolved conflict | `Shift-F3` | `Shift-F3` | Conflict resolver only. Skips conflicts you have already resolved; works while the resolved-output editor has focus. |
| First / last delta | `Cmd-Home` / `Cmd-End` | `Ctrl-Home` / `Ctrl-End` | Conflict resolver only. Not active while the resolved-output editor has focus, which keeps them for cursor movement. |
| Align selected lines manually | `Cmd-Y` | `Ctrl-Y` | Conflict resolver only (kdiff3 manual diff help). |
| Clear all manual alignments | `Cmd-Shift-Y` | `Ctrl-Shift-Y` | Conflict resolver only. |

In a focused status section, `Space` also stages or unstages its single selected
file and advances when that file is shown in the diff. Keyboard focus stays in
the list, so another `Space` acts on the next file and `Ctrl-A` / `Cmd-A` still
selects the section's files. At the end of the section, advancement selects the
previous file; when the section is empty, it clears the diff.
`Up` / `Down`, `PageUp` / `PageDown` and `Home` / `End` step through the
section's files and open each file in the diff. Keyboard focus stays in the list.

When `S`, `U`, the gutter buttons, a diff menu or Discard leaves the open file
with no changes in its list, the diff opens the next file in that list, or the
previous one at the end. Settings → Change tracking → "Automatically open the
next file when the current one has no changes left" turns this off; the diff
then stays on the file, or closes if you discarded the whole file. `Space` and
the Stage/Unstage button move to the next file either way.

Preview-mode note:
- [Markdown previews](markdown-previews.md) also support text selection, link menus, and keyboard-operated disclosures in embedded descriptions.
- Rendered markdown preview hides the raw diff navigation controls and ignores the raw-diff-only view toggles, whitespace toggle, and conflict navigation hotkeys until you return to source mode.
- The function-key navigation aliases remain available from text inputs. Arrow keys and Option/Alt arrows belong to the focused input, terminal, menu, or dialog.

## Context menu shortcuts

Context-menu keyboard behavior is the same on every platform:

| Action | Shortcut | Notes |
| --- | --- | --- |
| Move menu selection | `Up`, `Down` | |
| Activate selected menu item | `Enter` | Uses the first enabled item if nothing is selected. |
| Close the menu | `Escape` | |
| Activate an inline menu shortcut | Letter or digit shown beside the item | |

Additional note:
- Some menus also show clipboard-style shortcuts such as `Ctrl+C`; those reflect the underlying view shortcut rather than the menu's generic single-letter dispatcher.

## Focused diff window

These shortcuts apply in the standalone focused diff window opened for difftool-style flows.

| Action | macOS | Windows / Linux | Notes |
| --- | --- | --- | --- |
| Close the window | `Cmd-W`, `Ctrl-W`, `Escape`, `Q` | `Ctrl-W`, `Escape`, `Q` | `Ctrl-W` remains accepted on macOS as an extra alias. |
| Previous change | `F2`, `Shift-F7` | `F2`, `Shift-F7` | One change block at a time, as in the main diff view. |
| Next change | `F3`, `F7` | `F3`, `F7` | One change block at a time, as in the main diff view. |
