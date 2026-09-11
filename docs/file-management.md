# File management and Documents

The Files sidebar operates on the working tree. Historical views remain revision
previews. File operations do not stage changes.

## Explorer

Click a file to preview it. Ctrl/Cmd-click toggles selection; Shift-click selects a
range. Folder chevrons expand and collapse folders. Right-clicking an item in the
selection preserves the selection. Keyboard focus is independent of selection.

The context menu provides New File, New Folder, Cut, Copy, Paste, Duplicate,
Rename, Add to .gitignore, Trash, and Delete permanently. Creation and rename use
inline entry with Enter to accept and Escape to cancel. Root menus provide
creation and paste. Tracked files cannot be added to .gitignore through this menu;
directory rules cover untracked contents and future files without changing the
index.

| Action with explorer focus | macOS | Windows / Linux |
| --- | --- | --- |
| Copy / Cut / Paste | Cmd-C / Cmd-X / Cmd-V | Ctrl-C / Ctrl-X / Ctrl-V |
| Move previously copied files here | Option-Cmd-V | Cut, then Paste |
| Duplicate | Cmd-D | Ctrl-D |
| Select all visible rows | Cmd-A | Ctrl-A |
| Rename | F2 | F2 |
| Open focused item | Enter | Enter |
| Move focus and selection | Up / Down / Home / End | Up / Down / Home / End |
| Extend selection | Shift + navigation | Shift + navigation |
| Move focus without changing selection | Cmd + navigation | Ctrl + navigation |
| Expand / collapse or navigate folders | Right / Left | Right / Left |
| Undo / Redo file operation | Cmd-Z / Cmd-Shift-Z | Ctrl-Z / Ctrl-Shift-Z or Ctrl-Y |
| Trash / permanent deletion | Delete / Shift-Delete | Delete / Shift-Delete |
| Cancel inline entry, pending cut, or transfer | Escape | Escape |

These bindings are scoped to explorer focus. Text editing and terminal bindings
remain available in their respective controls. Hidden files are visible by
default, ignored files are hidden, and `.git` stays hidden. The Hidden and Ignored
buttons change visibility. Explicit search can load ignored subtrees in the
background; opening a hidden or ignored file explicitly reveals it.

Copy/paste uses saved disk contents. Pending cut items and descendants of cut
folders are muted and marked until completed or cancelled. A folder receives
items inside it; a file targets its parent; empty explorer space targets the
repository root. Hovering expands folders and scrolls near viewport edges.

Internal dragging moves by default; Ctrl on Windows/Linux or Option on macOS
selects Copy. Incoming desktop file drops copy by default; Shift requests Move.
Desktop clipboard payloads must contain native files, not plain text paths.
Copying into the same folder or duplicating creates `name copy.ext`, followed by
numbered variants. A move to the current location does nothing. Other collisions
offer Keep Both, Replace, Skip, or Cancel; directory collisions also offer Merge.

## Documents

The bottom-bar Documents button is available without an open repository. Its
searchable picker provides Open File and a menu to remove each recent entry.
History contains up to 50 normalized absolute paths, shared across windows and
persisted in the session file. Removing history never deletes files. Missing
entries remain removable and report an error when opened.

Opening a file discovers its owning repository, including nested repositories
and linked worktrees. GitComet prefers an existing tab in the current window,
then another window, and opens the repository when needed. The working-tree
explorer reveals the file. Files outside repositories open on the Documents
canvas with a selectable absolute path. Text files have an explicit Edit action,
Save, and Save As; image and binary previews use the existing restrictions.

Selecting a repository tab restores its canvas. Unsaved standalone buffers remain
available above recent documents even after history removal or eviction. A
multiple-file drop records every successful file and displays the first one;
later background replies cannot steal focus after another navigation.

## Filesystem service and recovery

`gitcomet-core::filesystem` owns typed requests, cancellation, progress, per-item
results, path mappings, disk versions, and a session journal of 100 logical
operations. `gitcomet-state` submits work through a shared filesystem executor.
The process-wide filesystem lock also coordinates editor saves across windows.
Editors pause and drain outstanding saves before operations, then adopt successful
path mappings before saving resumes. Saving checks the disk version loaded by
the editor; newer external bytes require a conflict decision.

Transfers enumerate actual contents, preserve links and executable permissions,
and reject protected repository roots, Git metadata, nested repositories, and
submodule boundaries. Replacements retain the old destination before installing
new data. Cross-filesystem moves finish the copy before source removal.
Cancellation leaves completed items available to Undo. Undo and Redo verify
current contents, identities, and parent boundaries before reversing a step.
Conflicting later changes are preserved and the operation remains retryable.

Native Trash uses an exact restore receipt. A Trash error never triggers
permanent deletion. Permanent deletion requires confirmation and has no Undo.
Operations completed by another application are outside GitComet's journal.
For incoming local Linux file URLs, GitComet performs the move and acknowledges
the native transfer after completion; native completion alone does not remove
the source file.

Rollback storage uses `.gitcomet-operation-*` directories on the affected
filesystem, preferring the temporary directory, then the repository's Git
directory, then the affected parent. Before moving retained data, `recovery.log`
records both native paths with percent-escaped native path bytes. Recovery data
survives an interrupted process or an unsuccessful rollback. The log is not
fsynced: renames are not fsynced either, so after a power loss it can name a
move that did not land -- it over-reports rather than under-reports, which is
what a log read by hand wants. Recovery after a crash is manual: inspect the
recorded paths and retain conflicting current files before restoring parked
items. The session journal is not replayed after restart.

## Verification and native acceptance

Filesystem tests cover recursive transfers, conflict decisions and merges,
permissions, links, native filenames, case-only renames, cancellation, protected
paths, later external changes, journal bounds, and interrupted batches. UI/state
tests cover selection and focus, clipboard ownership, document routing and
history, missing files, unsaved buffers, and autosave coordination during rename.

Run normal checks with:

```sh
cargo test --workspace --no-fail-fast
cargo test --workspace --all-features --no-fail-fast
cargo fmt --all --check
cargo fmt --manifest-path crates/gitcomet-filesystem-native/Cargo.toml --check
cargo clippy --workspace --all-targets --all-features
```

The native Trash test is intentionally opt-in because it uses the current user's
Trash. The cross-filesystem test uses `/dev/shm` on Linux when it is a distinct
filesystem:

```sh
cargo test -p gitcomet-core native_trash_restores_the_exact_entry -- --ignored
cargo test -p gitcomet-core cross_filesystem_move_and_its_undo_redo
```

Both GPUI dependencies use the same published commit. Platform implementation and
CI results are in [GPUI PR #16](https://github.com/Havunen/gpui-ce/pull/16).

The implementation was exercised against real Nautilus on an isolated X11
display: clipboard copy/cut in both directions, conflict cancellation, outbound
dragging, inbound file/folder drops, receiver-owned Move/Undo/Redo, and document
opening. Linux Trash restoration and cross-filesystem Move/Undo/Redo were also
exercised. GPUI's native CI runs cover Windows, macOS, Wayland, and X11 builds and
automated tests. Finder, Windows
Explorer, Dolphin, WSLg, and a real Wayland desktop still require their manual
acceptance passes; a successful build does not substitute for those transfers.

The September 10, 2026 workspace run passed 6,420 tests, with 16 ignored. The final
UI run against the published GPUI pin passed 3,617 tests, including the added
native move acknowledgement/Undo regression, with four ignored. The
all-features run reached 6,650 passes and 23 failures in the optional row benchmark
tests. An untouched checkout at `5d11e006` reproduced 22 of those failures; the
remaining prepared-conflict wrap-offset test passed in isolation. Workspace
Clippy passed with existing warnings. The strict Clippy commands used by the
repository's Rust CI passed. The broader all-targets/all-features `-D warnings`
command also encounters warnings in existing tests and benchmark code.

For each desktop, test both directions, multiple files and folders, Copy versus
Move, cancellation, clipboard replacement, conflicts, source removal exactly
once, and Trash restoration. Include hidden files, links, and unsaved editors.
