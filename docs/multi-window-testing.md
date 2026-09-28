# Multi-Window Testing

This document is the release test plan for GitComet's multi-window support. It covers durable workspaces, launch restoration, cross-window moves, command-line routing, window placement, and platform chrome.

## Expected behavior

- A new window starts empty on the Home page (Open, Clone, Initialize, saved workspaces, recent repositories) and does not create a saved workspace until its first repository is added.
- Every non-empty window is an independent workspace with its own active tab and layout.
- The same repository can be open in several windows or saved workspaces. Opening it again within one window selects that window's existing tab.
- Quitting GitComet restores every window that was open at quit time on the next launch.
- Closing a window before quitting keeps its workspace recoverable in the repository picker but does not restore it automatically. On Linux and Windows, closing the last window quits the app, so that window is restored like any other quit.
- Closing the final repository in a window removes an anonymous empty workspace. Moving the final repository out closes the empty source window.
- A customized workspace (a name, a title-bar color, or a theme override) outlives its last repository: its window stays open on Home, keeps its title-bar chip, and the empty workspace is saved and restored until it is deleted in Settings.
- Saved windowed bounds, display, maximized state, fullscreen state, and relative stacking order are restored when possible. Bounds are rebased and clamped when the saved display is missing or smaller.
- Tiled or snapped placement (tiling window managers, Windows Snap) is owned by the window manager and cannot be requested back. The tiled edges are recorded for diagnostics and the window reopens at its last frame.
- `gitcomet <repository>` sends the request to the running browser process. The default target is the active window; the General setting can instead request a new window. Both destinations are honored even when the repository is already open in another window.
- Zoom is process-wide, so it lives in menus (the in-app menu on Linux and Windows, the Window menu on macOS), shortcuts, the command palette, and Settings, never in an individual window's footer.
- Right-clicking a workspace in the repository picker offers **Activate** (disabled for the window's own workspace), **Workspace Settings**, which opens Settings › Workspaces with that workspace selected, and a red **Delete workspace**. Deleting closes the workspace's window (after its unsaved-edits and running-terminal prompts) and forgets its name, color, theme and layout; if it was the last window, that window returns to Home instead. Workspace rows look the same in the picker, Settings and Home. Title-bar color is set there; it is per workspace, survives closing/relaunching, and **Default** returns to the theme-derived color.
- **Settings › Workspaces** explains how to start a workspace (New Window, its shortcut, and a **New Workspace** button that opens an empty window), and renames a workspace (the **Save** button beside the name is enabled once the name changes), sets its title-bar color and an optional theme override, or opens it (**Open workspace** focuses its window, including on Wayland). **Delete workspace** is a separate red button at the bottom and asks for confirmation. A theme override applies only to that workspace's window; changing the app theme leaves overridden windows alone.
- **Open Workspace** (`Ctrl/Cmd+Shift+R`, the command palette, the app menu) lists only workspaces. From an empty window the chosen workspace opens in that window; otherwise its own window is focused or opened.
- On Linux and FreeBSD, **Follow system** uses the desktop button layout and hides minimize/maximize while tiled. Explicit Show/Hide modes override that behavior. macOS keeps native traffic lights.

## Automated coverage

Run the focused feature suite while iterating:

```bash
cargo test -p gitcomet-state session::tests
cargo test -p gitcomet --bin gitcomet browser_instance::tests
cargo test -p gitcomet-ui-gpui --lib workspaces::tests
cargo test -p gitcomet-ui-gpui --lib window_controls::tests
cargo test -p gitcomet-ui-gpui --lib review_regression -- --test-threads=1
cargo test -p gitcomet-ui-gpui --lib app::tests::new_window_shortcuts_open_new_windows
cargo test -p gitcomet-ui-gpui --lib app::tests::moving_the_only_repository_to_a_new_window_closes_the_source
cargo test -p gitcomet-ui-gpui --lib settings_window::tests::workspaces
cargo test -p gitcomet-ui-gpui --lib view::tests::home
cargo test -p gitcomet-ui-gpui --lib workspace_theme_override
```

Before merge, run the broad regression gates:

```bash
cargo test -p gitcomet-ui-gpui --lib
cargo test -p gitcomet-state
cargo test -p gitcomet --bin gitcomet
cargo clippy -p gitcomet-state --lib -- -D warnings
cargo clippy -p gitcomet-ui-gpui --lib -- -D warnings
cargo clippy -p gitcomet --bin gitcomet -- -D warnings
cargo fmt --all -- --check
```

The cross-platform workflow runs the full workspace suite, including these portability regressions, on Linux, Apple Silicon and Intel macOS, and x64 and ARM64 Windows. Separate Linux display-profile smoke tests cover display selection. Platform CI proves compilation, path encoding, broker transport, and GPUI state transitions; it does not replace the native desktop checks below.

## Manual fixture

Create six small repositories named A, B, C, F, G, and H. Make at least one repository contain an uncommitted edit, and use paths containing spaces and non-ASCII characters for two repositories. Start from a disposable GitComet app-state directory or back up the existing session before migration testing.

## Manual scenarios

### 1. Creation, isolation, and recovery

1. Open A, B, and C in the first window.
2. Create a new window and verify it is empty rather than a copy of A/B/C.
3. Open F, G, and H in the second window and select a different active tab in each window.
4. Close the first window, then quit from the second window.
5. Relaunch. Verify only the F/G/H window opens with its previous active tab.
6. Open the repository picker and recover the saved A/B/C workspace. Verify it opens once and focuses if selected again.
7. Quit while both windows are open, relaunch, and verify both return.
8. Focus a new empty window, add its first repository, click back to the previous window, and quit. Verify the previously clicked window is restored frontmost.
9. With A open in the first window, use the folder picker, manual path field, and pinned/recent repository row for A from the second window. Verify A opens in the second window and repeated opens select its existing tab. Repeat with a worktree or submodule opened from both windows' pickers, and by dropping A's folder onto the second window. Select different tabs in each window, then close one copy and verify the other remains open. Quit and relaunch with overlapping repository lists and verify both workspaces restore with their own active tabs.
10. On Linux and Windows, close the only open window with its title-bar close button. Relaunch and verify that window's workspace is restored.

### 2. Window geometry and stacking

1. Put the two windows on different displays with distinct sizes. Maximize one; repeat once with fullscreen.
2. Quit and relaunch. Verify display assignment, state, size, relative placement, and frontmost ordering.
3. Quit again, disconnect the secondary display or switch to a smaller resolution/DPI, and relaunch.
4. Verify every window is fully reachable inside an available screen and no restored window is permanently off-screen.
5. Repeat with both windows overlapping to verify they reopen as separate windows rather than at an identical unusable position.
6. Change a process-wide preference in the first window, then move or resize the second window. Relaunch and verify the second window's bounds were saved without reverting the preference changed in the first.

### 3. Repository moves

1. Move B from A/B/C to F/G/H and verify B appears exactly once, the target focuses, and the source remains A/C.
2. Move C to a new window and verify the new workspace contains only C.
3. Move A, the last source tab, to another window and verify the empty source window closes and is not offered as a saved empty workspace.
4. Repeat a move while that repository has a running terminal. Cancel once, then confirm termination; verify no move occurs on cancel and the confirmed move completes after shutdown.
5. Move a repository to a window that already contains it and verify GitComet focuses the destination's existing tab and removes only the source copy after its save/terminal guards complete. A third window with the same repository is unaffected.
6. Disable auto-save, dirty both the current editor and a stashed editor in the repository, and request a move. Also dirty a file in a repository that will remain in the source window. Verify Cancel leaves all buffers in place, Save writes only files owned by the moved repository before moving it, and Discard moves it without affecting the other repository's dirty buffers.
7. Close a workspace containing repository B, open B in the source window, then move B to that saved workspace. Verify the saved workspace reopens with its original identity and B is removed from the source after its guards complete. Moving within the source's own workspace remains a no-op and must not prompt to discard edits or terminate terminals.

### 4. Command-line routing

1. Keep two windows open and active in turn. Run `gitcomet .` from a third repository and verify the default behavior adds it to the currently active GitComet window.
2. Change **Command-line repository opens** to **New window** and repeat with another repository. Verify one new window is created.
3. Run the command again for a repository already open elsewhere. With **Active window**, verify it opens in the last-focused GitComet window and repeated requests reuse that window's tab. With **New window**, verify every request opens a separate window, including requests made while an earlier copy is still loading.
4. Focus each GitComet window in turn, then focus an external terminal and run `gitcomet .`. Verify the repository opens in the last-focused GitComet window even though the app itself is no longer active.
5. Quit with a saved workspace present, keep **Command-line repository opens** set to **New window**, and launch GitComet with a repository path. Verify the requested repository opens in its own window beside the restored workspace.
6. Save a workspace with A active and B inactive, quit, then launch with `gitcomet <path-to-B>`. Verify restoration finishes with B active rather than reverting to A.
7. Launch two commands nearly simultaneously and verify only one browser process owns the session and no request is lost or duplicated.
8. Terminate GitComet uncleanly, relaunch, and repeat to exercise stale broker-descriptor recovery.
9. Repeat with relative paths, spaces, non-ASCII characters, and, on Unix, a non-UTF-8 path.
10. Close every saved workspace, quit, and launch `gitcomet <repository>`. Verify the first window contains the requested repository instead of opening empty.

### 5. Window controls, title-bar colors, and zoom

1. Verify the footer has no zoom control in any window and menu/shortcut zoom changes all windows consistently.
2. Open the repository picker and right-click the current window's workspace: **Activate** is disabled. Right-click a recoverable closed workspace: **Activate** opens it, and **Workspace Settings** opens Settings on that workspace. Choose different title-bar colors for two workspaces there and verify only the selected workspace's title bar changes.
3. Close/recover one colored workspace and quit/relaunch with the other open. Verify both colors persist. Choose **Default** and verify that workspace returns to the current theme's title-bar color.
4. On Linux/FreeBSD, test floating and tiled states with **Follow system**, including a desktop layout that places/reorders buttons on the left. Verify side and order are preserved, then test explicit Show and Hide. Hide must leave Close available.
5. On Windows, verify minimize, maximize/restore, close, title dragging, and double-click maximize in Show and Hide modes.
6. On macOS, verify native traffic lights and fullscreen remain functional, the workspace color applies to GitComet's content title bar, and the custom controls setting does not replace native chrome.

### 5a. Workspaces, Home, and Open Workspace

1. Launch with no session. Verify Home shows Open, Clone, and Initialize, empty Workspaces and Recent repositories lists, and the app menu in the title bar. Drop a repository folder on Home and verify it opens with no remaining drag highlight in the repository bar. Repeat with both an immediate drop and a pause before releasing. Drag the same folder again, cancel or leave the window, and verify the highlight clears; repeat over the repository bar and with invalid folders and files. Drop two repository folders, a non-repository folder and a file together: each folder is handled on its own, so both repositories open as tabs in drop order with the last valid one selected, the non-repository folder is discarded with its own warning, and the file is ignored. Relaunch and verify both repositories were saved.
2. Open A and B in one window. In **Settings › Workspaces** verify that workspace is preselected; rename it, pick a title-bar color, and choose a theme different from the app theme. Verify only that window re-themes and its title and chip update. Change the app theme and verify the overridden window keeps its theme while other windows follow.
3. Close B, then A. Verify the window stays on Home with the workspace's name and chip. Quit and relaunch; verify the empty workspace reopens on Home. Delete it in Settings and verify it disappears everywhere.
4. Close a window holding C and D. From a new empty window, choose that workspace on Home and verify C and D open in the same window (no second window) with the saved pane layout, while the window keeps its own position.
5. Press `Ctrl/Cmd+Shift+R` with nothing focused, type a repository name, and press Enter. Verify only workspaces are listed and filtering matches the repositories they hold. Repeat from the command palette and the app menu.
6. Resize the sidebar and immediately close the window with `Ctrl/Cmd+Shift+W`. Recover the workspace and verify the new width was kept.

### 6. Session migration and failure recovery

1. Launch with a version 3 session containing several repository tabs. Verify they migrate into one workspace without losing the active repository or layout.
2. Verify the first version 5 write creates the one-time `.v3.bak` backup (a version 4 file from an earlier build of this branch gets `.v4.bak` and its `window_groups` key is read as `workspaces`).
3. Close one migrated workspace, quit, and relaunch to verify closed-versus-restorable state survives another write.
4. Temporarily make one saved repository unavailable. Verify the rest of the workspace remains recoverable and is not erased while startup loading is pending.
5. Close a workspace, open one of its repositories in another live window, then recover the closed workspace. Verify the recovered workspace keeps its full repository list and active tab, including a separate copy of the shared repository. Repeat from an empty Home window and verify it adopts the saved workspace in place.
6. Resize the sidebar/details/status layout and immediately close or quit, without waiting for the debounce. Recover or relaunch and verify the latest layout was persisted.
7. Drop a folder that is not a Git repository and close or quit while validation is still pending. Relaunch or inspect saved workspaces and verify the provisional path was never persisted.

### 7. Process-wide startup resources

1. Arrange three workspaces to restore, then relaunch while an update or survey prompt is eligible. Verify the check runs once and at most one prompt appears, regardless of the window count.
2. Restore progressively larger numbers of workspaces while observing GitComet's thread count. Verify each window adds its reducer and repository monitors as needed, but does not add another primary, repository-load, metadata, or session-persistence worker pool.

## Native platform matrix

| Platform | Required environments | Platform-specific focus |
| --- | --- | --- |
| Linux | X11 GNOME, Wayland GNOME, Wayland KDE, one tiling WM | CSD button layout, tiled detection, non-UTF-8 paths, display rebasing |
| Windows | Windows 11 x64 and ARM64 when available | UTF-16 paths, client-titlebar hit testing, PowerShell/CMD command routing, mixed-DPI displays |
| macOS | Apple Silicon and Intel CI; current supported macOS for manual checks | Native traffic lights, Dock reopen, Cmd-N, Spaces/fullscreen, display UUID restore |

Record the OS version, display topology and scale, window-manager/session type, and whether the run used a migrated or clean session with every release result.
