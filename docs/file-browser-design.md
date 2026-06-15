# File Browser — Design & Implementation Document

## 1. Overview

Adds a new **"Files" tab** to the left sidebar pane, providing a virtualized, expandable directory tree with inline search filtering. The file source is pluggable: working directory (HEAD), a specific commit SHA, or a branch name.

**Inspiration from Zed:** The file tree rendering, flat-list-with-depth approach, and expand/collapse semantics are modeled after Zed's Project Panel (`/home/sampo/git/zed/crates/project_panel/src/project_panel.rs`, 7479 lines).

| Zed Source | Concept borrowed |
|---|---|
| `project_panel.rs:4036-4357` | `update_visible_entries()` — flat list building with expansion filtering |
| `project_panel.rs:95-99` | `VisibleEntriesForWorktree` — flat entry storage with lazy path index |
| `project_panel.rs:101-135` | `State` — expanded_dirs tracking |
| `file_icons.rs:20-82` | `FileIcons::get_icon()` — multi-strategy icon resolution (planned, not yet ported) |
| `icon_theme.rs:74-310` | `FILE_SUFFIXES_BY_ICON_KEY` — extension-to-icon mapping (planned) |
| `gpui/src/elements/uniform_list.rs:58-865` | `UniformList` — virtualization implementation |

---

## 2. Data Model

### 2.1 Domain types (`gitcomet-core/src/domain.rs`)

`FileEntry`, `FileEntryKind`, and `FileSource` live in the domain crate so the `GitRepository` trait (also in core) can reference them without depending on the state or gix crates.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileEntryKind { File, Directory }

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileSource {
    WorkingDirectory,
    Commit(CommitId),
    Branch(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub name: String,        // e.g. "main.rs"
    pub path: Arc<PathBuf>,  // e.g. "src/main.rs"
    pub kind: FileEntryKind,
    pub depth: usize,        // nesting level for indentation
}
```

### 2.2 State types (`gitcomet-state/src/model.rs`)

**`FileBrowserState`** — stored per repository in `RepoState`:
```rust
pub struct FileBrowserState {
    pub source: FileSource,
    pub entries: Loadable<Arc<Vec<FileEntry>>>,
    pub expanded_dirs: HashSet<Arc<PathBuf>>,
    pub search_query: String,
    pub file_browser_rev: u64,  // bumped on any change, drives UI notifications
}
```

**`SidebarMode`** — stored in `AppState` (not local UI state) so the reducer can react to mode switches:
```rust
pub enum SidebarMode { Branches, Files }
```

**Rationale:** `sidebar_mode` lives in `AppState` rather than `SidebarPaneView` so that switching to Files mode can automatically trigger `LoadFileBrowser` as a side effect in the reducer, keeping all state transitions centralized.

### 2.3 Message variants (`gitcomet-state/src/msg/message.rs`)

```rust
// External (dispatched by UI)
LoadFileBrowser { repo_id, source: FileSource }
ToggleFileBrowserDir { repo_id, path: PathBuf }
SetFileBrowserSearch { repo_id, query: String }
SetFileBrowserSource { repo_id, source: FileSource }
SetSidebarMode { mode: SidebarMode }

// Internal (result of async git operation)
InternalMsg::FileBrowserLoaded { repo_id, source, result: Result<Vec<FileEntry>> }
```

### 2.4 Effect variant (`gitcomet-state/src/msg/effect.rs`)

```rust
Effect::LoadFileBrowser { repo_id, source: FileSource }
```

This triggers the background git operation; the result comes back as `InternalMsg::FileBrowserLoaded`.

---

## 3. Data Loading (gix tree walk)

### 3.1 Trait (`gitcomet-core/src/services.rs`)

Two methods on `GitRepository` so the caller can choose based on `FileSource`:

```rust
fn list_tree_files(&self) -> Result<Vec<FileEntry>>;           // HEAD
fn list_tree_files_at_commit(&self, commit_id: &CommitId) -> Result<Vec<FileEntry>>;
```

### 3.2 Implementation (`gitcomet-git-gix/src/repo/file_browser.rs`)

**Recursive tree walking using gix** (not CLI git):

1. Resolve tree OID from commit via `find_commit(oid)?.tree_id()`.
2. `find_object(tree_oid)?.peel_to_tree()?` to get a `gix::Tree`.
3. Iterate `tree.iter()` (returns `Result<EntryRef>` — each must be unwrapped with `?`).
4. Collect entries into a `Vec<(name, mode, oid)>`.
5. **Sort**: directories first, then alphabetical — matches typical file tree convention.
6. For each entry:
   - If `mode.is_tree()`: push a `FileEntryKind::Directory`, then recursively walk the child tree.
   - If `mode.is_blob() || mode.is_link()`: push a `FileEntryKind::File`.
7. Track `depth` through the recursion.

**Why not `repo.index_from_tree()`:** That API flattens everything via an intermediate index, losing directory boundaries. The recursive walk preserves the tree structure needed for expand/collapse.

**Why not CLI `git ls-tree -r`:** GitComet2 already uses `gix` for status, log, diff, blame, etc. Using gix for tree walking avoids spawning subprocesses and follows the existing codebase convention. Zed uses CLI git; we match our codebase, not Zed's.

### 3.3 Effect chain

```
UI dispatches Msg::LoadFileBrowser
  → reducer sets entries to Loading, returns Effect::LoadFileBrowser
    → effects runner calls schedule_load_file_browser()
      → spawns background task calling repo.list_tree_files() or repo.list_tree_files_at_commit()
        → result sent as InternalMsg::FileBrowserLoaded
          → reducer stores entries (Ready or Error), bumps file_browser_rev
            → UI re-renders via SidebarNotifyFingerprint
```

The `file_browser_rev` field on `RepoState` is bumped whenever entries, expanded_dirs, search_query, or source change. `SidebarNotifyFingerprint` includes this rev, so the `SidebarPaneView` automatically re-renders when file browser state changes.

---

## 4. UI Architecture

### 4.1 Sidebar Mode System

`SidebarMode` is stored in `AppState.sidebar_mode` (the shared data model, not local UI state). When the reducer processes `SetSidebarMode` and the new mode is `Files`, it automatically dispatches `LoadFileBrowser` if the file entries haven't been loaded yet. This centralizes the "load on first view" logic.

### 4.2 Tab Bar

Rendered at the top of the sidebar (`SidebarPaneView::render_tab_bar`):

```
┌──────────────────────┐
│ Branches    Files    │  ← tab bar
├──────────────────────┤
│ (content below)      │
└──────────────────────┘
```

- Active tab: `theme.colors.active_section` background, `theme.colors.text` color.
- Inactive tab: transparent background, `theme.colors.text_muted` color.
- Uses existing theme colors only (no new theme fields needed).

**Note:** Tab click handlers (`on_click`) were deferred — the `gpui-ce` fork used by this project has a different `InteractiveElement` API surface than upstream GPUI. The `on_click` builder method works correctly inside `UniformList` processors but not on standalone `Div` elements built within `&mut self` methods. This will be addressed in a follow-up using the correct API for this GPUI version.

### 4.3 File Browser Content

`SidebarPaneView::render_file_browser_content()`:

1. If no repo, shows "No repository selected."
2. If entries are `Loading`/`NotLoaded`, shows "Loading files..."
3. If entries are `Error(e)`, shows the error message.
4. If entries are `Ready([])`, shows "Empty repository."
5. Otherwise, renders a `UniformList` backed by `file_browser_scroll` handle.

The `UniformList` uses `cx.processor(Self::render_file_browser_rows)` — the same pattern as the existing branch sidebar and commit file lists.

### 4.4 Visible Rows Builder

`SidebarPaneView::file_browser_visible_rows()`:

**Two modes:**

- **Normal mode (no search):** Calls `file_browser_visible_mask()` which walks entries, marks each as visible, and when it encounters a directory NOT in `expanded_dirs`, skips all children (entries with depth > directory's depth) until it reaches a sibling. This mirrors Zed's `advance_to_sibling()` approach.

- **Search mode (non-empty query):** Case-insensitive path match. Collects matching entry indices AND all ancestor directory paths. The final list includes any entry that either matches the query or is an ancestor of a match. This preserves the directory tree structure during search — the user always sees the path context for each result.

Both modes produce `Vec<FileBrowserVisibleRow>` — a flat structure with `entry_index`, `depth`, `is_directory`, and `is_expanded`.

### 4.5 Row Rendering

`SidebarPaneView::render_file_browser_rows()` — the `UniformList` processor:

```
[↓] [📁] src/         ← expanded directory (chevron_down.svg, folder.svg)
      [→] [📁] components/  ← collapsed directory (arrow_right.svg, folder.svg)
            [📄] Button.tsx  ← file (no chevron, file.svg)
      [📄] main.rs           ← file
```

Layout:
1. **Left padding**: `depth * 12px` (scaled by UI scale).
2. **Chevron slot** (14px): `chevron_down.svg` for expanded dirs, `arrow_right.svg` for collapsed dirs, empty for files.
3. **Icon slot** (16px): `folder.svg` for directories, `file.svg` for files (language-specific icons planned in Phase 7).
4. **Label**: entry `name` in `theme.colors.text`, 11.5px font, overflow-hidden with text ellipsis.

**Click handling:**
- **Directory**: dispatches `ToggleFileBrowserDir` — toggles the path in `expanded_dirs`.
- **File (left click)**: dispatches `OpenFileContent { repo_id, source, path }` — opens the file's **full content** in the main pane (see §10), for both `WorkingDirectory` (working tree) and `Commit` sources.
- **File (right click)**: opens a context menu (`PopoverKind::FileBrowserFileMenu`) via the shared popover system — Open, Open diff, Open file / Open file location (working dir only), File history, Copy path. Model builder: `view/panels/popover/context_menu/file_browser_file.rs`.

---

## 5. File Icons

**Implementation:** `crates/gitcomet-ui-gpui/src/view/file_icons.rs`

To match Zed exactly, the icon data is **ported verbatim** from Zed's default icon
theme (`zed/crates/theme/src/icon_theme.rs`) rather than hand-rolled:

- **Full icon set**: all of Zed's `assets/icons/file_icons/` SVGs are synced into
  `crates/gitcomet-ui-gpui/assets/icons/file_icons/` so every icon key resolves
  (an earlier hand-rolled `match` referenced ~18 SVGs that were never copied,
  rendering broken icons).
- **Data tables** (transcribed from `icon_theme.rs`): `FILE_STEMS_BY_ICON_KEY`,
  `FILE_SUFFIXES_BY_ICON_KEY` (extension/full-name → icon key), and `FILE_ICONS`
  (icon key → SVG path, with intentional `file.svg` fallbacks for keys without a
  dedicated glyph, e.g. `csharp`, `solidity`). Built into `LazyLock<HashMap>`s.
- **Resolution order** mirrors Zed's `FileIcons::get_icon` (`file_icons.rs:20-82`):
  full file name → progressive `split_once('.')` suffixes → multiple-extensions →
  extension/hidden-file name → extension → `default`. The `extension_or_hidden_file_name`
  and `multiple_extensions` helpers are ports of Zed's `PathExt`.
- **Folder icons**: `folder.svg` / `folder_open.svg`. **Chevrons**: `chevron_down.svg`
  / `arrow_right.svg`. File and folder icons are tinted with `theme.colors.text_muted`
  (neutral), matching Zed rather than a bright accent.
- Covered by unit tests in `file_icons.rs` (`resolves_zed_specific_mappings`, etc.).

**File:** `crates/gitcomet-ui-gpui/src/view/file_icons.rs`
```rust
pub fn file_icon_for_path(path: &Path) -> &'static str;
pub fn folder_icon(expanded: bool) -> &'static str;
pub fn chevron_icon(expanded: bool) -> &'static str;
```

## 6. Row Caching

**Implementation:** `SidebarPaneView` stores a `RefCell<Option<(u64, Rc<[FileBrowserVisibleRow]>)>>`.

- **Fingerprint**: `repo.file_browser.file_browser_rev` — bumped whenever entries, expanded_dirs, search_query, or source changes.
- **Cache lookup**: On each call to `file_browser_visible_rows()`, compare the cached revision against the current `file_browser_rev`. If equal, return the cached rows directly (no recomputation).
- **Cache storage**: After computing visible rows, store `(file_browser_rev, Rc::from(rows))` in the cache.
- **Invalidation**: Any state change that bumps `file_browser_rev` automatically invalidates the cache. The next UI frame triggers a recompute.

This follows the existing fingerprint-based cache pattern used by `BranchSidebarCache` (`caches.rs:938`) but simplified to a single-tier check since the file browser's computation is lighter (O(n) visible mask + filter vs O(n²) branch grouping).

## 7. Performance

| Concern | Approach | Status |
|---|---|---|
| **Virtualization** | `UniformList` — only ~30 rows rendered regardless of total count | Implemented |
| **Background loading** | gix tree walk runs in background via `TaskExecutor` / `spawn_with_repo_or_else` | Implemented |
| **Directory collapse** | Children of collapsed dirs excluded from visible list before UniformList sees them | Implemented |
| **Search** | Computed synchronously on the visible-rows builder | Implemented |
| **Row caching** | Fingerprint-based single-tier cache keyed on `file_browser_rev` | Implemented |
| **Icon resolution** | Compiled `match` statement for O(1) extension lookup | Implemented |

---

## 8. Implementation Status (All Phases Complete)

| Phase | Scope | Key files |
|---|---|---|
| **Phase 1** | `FileSource`, `FileEntry`, `FileEntryKind`, `FileBrowserState`, `SidebarMode`, `Msg` variants | `gitcomet-core/src/domain.rs`, `gitcomet-state/src/model.rs`, `gitcomet-state/src/msg/message.rs`, `gitcomet-state/src/msg/effect.rs` |
| **Phase 2** | gix tree listing: `list_tree_files`, `list_tree_files_at_commit` | `gitcomet-core/src/services.rs`, `gitcomet-git-gix/src/repo/file_browser.rs` (new), `gitcomet-git-gix/src/repo/mod.rs` |
| **Phase 3** | Reducer + effect chain: all Msg handlers, `InternalMsg::FileBrowserLoaded`, `Effect::LoadFileBrowser`, background scheduling | `gitcomet-state/src/store/reducer.rs`, `gitcomet-state/src/store/reducer/effects.rs`, `gitcomet-state/src/store/effects.rs`, `gitcomet-state/src/store/effects/repo_load.rs` |
| **Phase 4** | `SidebarMode` in `AppState`, tab bar UI, mode-based rendering dispatch | `gitcomet-ui-gpui/src/view/panes/sidebar.rs`, `gitcomet-state/src/model.rs` (AppState field) |
| **Phase 5** | File browser `UniformList`, visible rows builder, row rendering (chevrons, icons, indentation), directory toggle clicks, commit file diff clicks, search filtering | `gitcomet-ui-gpui/src/view/panes/sidebar.rs` |
| **Phase 6** | Tab click handlers (`on_mouse_down`), search input widget (`TextInput`), source indicator label | `gitcomet-ui-gpui/src/view/panes/sidebar.rs` |
| **Phase 7** | 43 SVG file icons from Zed, `file_icons.rs` with suffix-to-icon mapping, dynamic folder/chevron icons | `gitcomet-ui-gpui/assets/icons/file_icons/` (43 files), `gitcomet-ui-gpui/src/view/file_icons.rs` (new) |
| **Phase 8** | Visible rows cache with `file_browser_rev` fingerprint, `RefCell`-based cache in `SidebarPaneView` | `gitcomet-ui-gpui/src/view/panes/sidebar.rs` |

### Deferred / Future

| Item | Reason |
|---|---|
| Source selector dropdown | Popovers are complex; current label provides basic feedback |
| Branch source resolution | `FileSource::Branch` defined but not yet wired through effect chain |
| Background search for 500k+ file repos | Currently synchronous; fine for typical repos |
| Sticky directory headers | Like Zed's project panel; cosmetic improvement |
| Indent guides | Vertical lines connecting parent/child rows; cosmetic |

---

## 9. Key Design Decisions

1. **gix over CLI git**: GitComet2 uses gix pervasively for status, log, diff, blame. Tree walking follows the same convention. The gix `TreeRefIter` is lazy (iterator-based), though we do materialize the full `Vec<FileEntry>` once for the visible-rows builder.

2. **Flat list + depth over recursive rendering**: A single `Vec<FileEntry>` with depth values, plus a `HashSet<Arc<PathBuf>>` for expanded directories. Collapsed children are simply excluded from the visible list. This composes naturally with `UniformList` and avoids nested layout complexity.

3. **`FileSource` in domain, not state**: `FileSource` references `CommitId` and is used by the `GitRepository` trait. Placing it in `gitcomet-core::domain` avoids a dependency from core → state and keeps the trait independent of the state crate.

4. **`SidebarMode` in `AppState`**: Storing mode in the shared state (not local UI state) lets the reducer auto-trigger `LoadFileBrowser` on mode switch. This keeps the "load on first view" logic in one place.

5. **Search maintains directory structure**: Unlike a flat fuzzy finder, the search filter preserves ancestor directories for each match so users retain spatial context. This matches the user's explicit preference.

6. **Expand/collapse via HashSet**: `expanded_dirs` is a `HashSet<Arc<PathBuf>>`. Directories are collapsed by default (shown, but children hidden). Expanding adds the path to the set. This is simpler than Zed's dual-set approach (collapsed + expanded overrides) and sufficient for a file browser that shows all entries.

7. **Reuse existing patterns**: The `UniformList`, `UniformListScrollHandle`, `svg_icon`, `components::Scrollbar`, `components::empty_state`, fingerprint-based notification, and `Msg`/`Effect`/`InternalMsg` dispatch follow the exact same patterns as the existing branch sidebar, commit file list, and diff search.

---

## 10. File Content Viewer

Left-clicking a file opens its **full content** in the main pane. Rather than a
bespoke renderer, it **reuses the existing added/removed-file preview** (syntax
highlighting, correct line numbers, image rendering, no green/red diff coloring)
by forcing that preview for any file.

**Mechanism — a `content_preview` flag on `DiffState`:**
`Msg::OpenFileContent { repo_id, source, path }` → `diff_selection::open_file_content`
maps the source to a `DiffTarget` (`WorkingDirectory`→`WorkingTree{Unstaged}`,
`Commit`→`Commit{path}`) and runs the normal diff selection with
`content_preview = true` (threaded through `fill_select_diff_inline`). The flag:
- **Load plan** (`reducer/util.rs::selected_diff_load_plan`): forces `preview_only`
  (no patch diff) and `preview_text_side = New` for commits (working-tree content
  is read straight from disk). Images load via the existing `load_file_image`.
- **Render gating** (`view/panes/main/preview.rs`): `is_file_preview_active()`
  returns true for `content_preview`, and `ensure_selected_file_preview_loaded`
  drives the existing worktree-preview renderer via `content_preview_abs_path()`
  (display) + `content_preview_source_path()` (the disk path for working-tree
  content, or the New-side blob temp file for commit content).

Because the content view now *is* the diff target, the diff machinery already
clears it on the right transitions (`select_commit`, repo reload,
`ClearDiffSelection`, Esc) — there is no parallel state machine, custom renderer,
message, effect, or `read_file_content`.

**Asset registration (important):** GitComet serves SVGs through a **manual**
`include_bytes!` registry in `assets.rs` (gpui's `svg()` loads via `AssetSource`).
The `icons/file_icons/*` glyphs were never registered, so file-browser icons
rendered blank. `build.rs` now generates `file_icon_bytes()` / `FILE_ICON_ASSETS`
(embedding every `assets/icons/file_icons/*.svg`), `include!`d into `assets.rs` and
served from `load_static`/`list_static`. Any new top-level icon still needs a manual
`assets.rs` entry.

### Notes

| Item | Note |
|---|---|
| `WorkingDirectory` content | Read from the **working tree on disk** (like the added/removed-file preview), so it reflects uncommitted edits even though the tree listing is HEAD's. |
| Images | Rendered as a **single** image — `diff_file_image_loaded` drops the `old` side for content preview, so there is no before/after. SVG keeps the existing rendered/text toggle. |
| Branch folders | The branch sidebar's group/remote folders use the same `file_icons/folder.svg` + `folder_open.svg` (open/closed) as the file browser. |
| Branch source | `FileSource::Branch` is unsupported (the tree listing is also unwired). |

---

## 11. Browse Repository at a Historical Commit

The current "browse point" is `file_browser.source == Commit(sha)`
(`RepoState::browsing_commit()`); `RepoState.browse_history: Vec<CommitId>` is the
session **stack** of browsed commits. Two reducers (`store/reducer/effects.rs`),
fired by `Msg::BrowseRepositoryAtCommit` / `Msg::ResetBrowseToLive`:
- `browse_repository_at_commit` — dedup-push the commit, switch the sidebar to
  Files, `set_file_browser_source(Commit)`, and **re-target any open file** to that
  commit (`open_file_content`), kept open.
- `reset_browse_to_live` — clear the stack, `set_file_browser_source(WorkingDirectory)`,
  re-target the open file to its live version.

**Entry points** (all dispatch `Msg::BrowseRepositoryAtCommit`):
- A commit's right-click menu (`context_menu/commit.rs`).
- The commit-SHA link component (`components/commit_sha_hover_menu.rs`) — used in the
  details pane for the **Commit SHA** (own id; `allow_navigate = false`, so no
  "Navigate") and **Parent commit SHA** (`allow_navigate = true`, keeps "Navigate").
  Icon `icons/browse_at_commit.svg` (registered in `assets.rs` + the
  `context_menu_icon_path` allow-list).

While browsing (any commit in the stack):
- A **purple badge** (current short SHA, `theme::historical_outline`) sits by the
  Repository/Branch selectors (`action_bar.rs`). Clicking it opens
  `PopoverKind::BrowseHistoryMenu` (`context_menu/browse_history.rs`): the whole
  stack (current marked `●`) + **Go live** (`ResetBrowseToLive`). Jumping between
  entries keeps the stack.
- The **file directory** (`sidebar.rs::render_file_browser_content`) and the
  **content pane** (`panes/main.rs`) get a purple border.

All three surfaces re-render off `file_browser.file_browser_rev` (bumped by
`set_file_browser_source`): added to `ActionBarView::notify_fingerprint` and
`MainPaneView::notify_fingerprint_for`; the sidebar already hashes it.
