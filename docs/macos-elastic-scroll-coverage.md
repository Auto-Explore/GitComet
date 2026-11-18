# macOS elastic-scroll coverage audit

This inventory records the scroll surfaces found in combined branch
`feat/macos-elastic-scrolling` at `1e419c75`, and compares the clean overscroll PR
branch at `41ba21f9` on upstream `2365d4c6`. It is a source audit, not a claim that
every listed interaction has been tested with a physical trackpad. Line numbers
refer to that baseline and can move as the follow-up changes land.

The audit inspected production overflow, list construction, scroll-handle
tracking and custom wheel handlers across the UI kit, application, media,
terminal and extension crates. There were 130 list/overflow/handle call sites in
the combined tree and 131 in the clean PR tree; these are calls, not distinct
scroll areas.

## Baseline and implementation

The audited baseline already wrapped history, branch and file-browser sidebars,
shared changed-file lists, inline/split text diffs and plain-text file previews.
Their elasticity was vertical. Sidebar sticky rows already used a sibling
overlay, but the bottom-of-list behavior still needed correction. Grouped
changed-file headers still lived inside their translated list decorations.

The implementation adds horizontal elasticity, wraps the shared `TabBar`,
`PickerPrompt` and `ScrollContainer`, and corrects the sidebar/grouped-file
header overlays. The tables record the original gaps and their integration
contracts. Ordinary lists, previews, menus, editor bodies/gutters and normal
terminal scrollback are covered. Camera gestures and terminal application input
retain their specialized ownership. Source coverage does not imply a physical
trackpad check of every surface.

## Shared components and frequently used lists

| Surface | Integration point | Required separation or behavior |
| --- | --- | --- |
| Repository/workspace picker and other shared pickers | `gitcomet-ui-kit/src/components/picker_prompt.rs:809–986` | Wrap the result list; keep query row, trailing sort control and scrollbar fixed. Retain spacer-based windowing. |
| Repository tabs | `gitcomet-ui-kit/src/components/tab_bar.rs:245–280` | Horizontal body only; retain fixed viewport, trailing add control and logical tab visibility. Programmatic reveal clears displacement. |
| Grouped changed-file sticky headers | `view/changed_file_list.rs:185–205`; `view/rows/file_list/render.rs:276–319` | Pin headers outside the translated body. Full mouse occlusion in the original decoration also needs wheel pass-through checks. Covers status, commit, range and worktree lists. |
| Reflog | `view/panes/reflog.rs:728–736` | Wrap the per-repository rows below the fixed header/filter. |
| Command palette | `view/command_palette.rs:1639–1664` | Stretch results below a fixed query field. |
| Home columns | `view/home.rs:670–681` | Wrap workspace/repository rows; retain section headings and scrollbar tracks. |
| Home outer page | `view/home.rs:895–901` | This implicit overflow area needs its own handle. Preserve nested list ownership. |
| Multiple selected commit cards | `view/panels/layout/comparison.rs:403–413`; shared `view/panels/layout/mod.rs:46–56` | Do not double wrap file lists which already obtain elasticity from `ChangedFileListView`. |
| Interactive rebase/cherry-pick | `view/panes/main/interactive_rebase.rs:1250–1295` | Uses `gpui::ListState`; requires a matching driver. Preserve drag autoscroll and the fixed footer. |
| Submodule summary | `view/panes/diff/submodule_summary.rs:283–303` | Also uses `ListState`; keep the scrollbar fixed. |

The shared picker integration covers repository/workspace, branch/ref, author,
worktree, worktree badge, submodule, stash, upstream, file-history file, document
and file-history commit pickers. Picker section headings are ordinary content
unless explicitly implemented as sticky headings; the query row is fixed chrome.

Application paths in the tables are relative to
`crates/gitcomet-ui-gpui/src`. UI-kit paths are relative to `crates`.

## Documents, diffs and conflict views

| Surface | Integration point | Required separation or behavior |
| --- | --- | --- |
| Markdown diff preview | `view/panels/main/diff.rs:822` | The rendered document has a separate ordinary `ScrollHandle`; split labels and resize handles stay fixed. |
| Worktree Markdown preview | `view/panels/main/diff_view.rs:2337` | Separate from the covered plain-text preview. |
| Conflict Markdown columns | `view/panels/main/conflict_resolver_view.rs:2712` | Coordinate displacement according to the existing column sync setting. |
| Two-/three-way conflict source columns | `view/panels/main/conflict_resolver_view.rs:1372,1400,1428,1628,1656` | Retain wheel-master bookkeeping, logical synchronization and fixed column headings. |
| Conflict comparison | `view/panels/main/diff_view.rs:2540` | Separate virtualized list below the column header. |
| Horizontal inline/split diffs | `view/panels/main/diff_text.rs:132,221,243` | Baseline wrappers were vertical only; horizontal content and scrollbar tracks need independent treatment. |
| Horizontal plain-text preview | `view/panels/main/diff_view.rs:2399` | Same distinction as horizontal diffs. |
| Markdown code/table blocks | `view/rows/markdown_document.rs:2782–2802` | Nested horizontal-only regions must leave vertical gestures to the document. Keep their bottom scrollbar tracks fixed. |
| Keep/delete conflict preview | `view/panels/main/keep_delete_conflict.rs:289–295` | Implicit scrolling needs a handle; preserve fixed conflict actions. |
| Raw/error fallbacks | `view/panels/main/diff.rs:347,528,600`; `view/panels/main/diff_view.rs:2375` | These are separate text scroll containers, despite the surrounding normal diff being covered. |

## Settings, menus and secondary content

| Surface | Integration point | Required separation or behavior |
| --- | --- | --- |
| Settings content | `view/settings_window/render.rs:523–532` | Body only; preserve the window toolbar and navigation. |
| Settings navigation | `view/settings_window/rows.rs:1273–1280` | Independent vertical area. |
| Settings option lists | Shared `view/settings_window/rows.rs:138–187` | Fonts, external editor, date/time, remote protocols/images, tracking, file-list options and diff options use this container. Preserve boundary propagation policy. |
| Settings font lists | `view/settings_window/appearance.rs:250,298` | Uses the shared dropdown container. |
| Other Settings option lists | `view/settings_window/cards.rs:135,313,346,764,810,931,965,999,1186,1236,1274,1310` | Uses the shared dropdown container. |
| Open-source licenses | `view/settings_window/render.rs:648–657` | Virtualized body below a fixed breadcrumb/header. |
| Hook activity/error details | Shared `view/panels/popover/hook_activity.rs:400–440` | Covers history, hooks, output and error detail rails/body. |
| Hook command preview | `view/panels/popover/hook_activity.rs:883–900` | A separate nested area with implicit scrolling. |
| Pinned-item overflow | `view/panels/popover/context_menu.rs:291–317` | Keep the menu title and scrollbar outside. |
| Context menu | `view/panels/popover/prompt_actions.rs:805–824` | Keep placement latching and scroll-arrow controls. |
| Picker row menu | `view/panels/popover/picker_row_menu.rs:487–496` | Same placement/scroll-arrow constraints. |
| Push-upstream remote options | `view/panels/popover/push_set_upstream_prompt.rs:191–199` | Implicit overflow; preserve the surrounding prompt. |
| History ref hover list | `view/history_refs_hover.rs:578–595` | Implicit overflow and hover-lifetime behavior. |
| Long prompt/banner details | `view/gitcomet_view_render.rs:1065–1070` | Content only; fixed prompt actions. |
| Long toast messages | `view/toast_host.rs:1639–1644` | Content only; fixed dismiss/action controls. |
| Terminal tab strip | `view/terminal_panel.rs:1214–1224` | An ordinary horizontal tab area, separate from terminal content. |

The clean PR tree adds one upstream-only overflow surface:
`view/settings_window/render.rs:232–238`, `settings_hosted_menu_scroll`, for
extension-supplied Settings menus. It has no explicit handle in the audited
version. The clean PR integration also wraps that host-supplied menu. The differing
inline diff list construction is a refactor rather than an additional scroll
area; upstream line-anchored extension overlays must remain attached while
their viewport clip stays fixed.

## Text inputs, editors and specialized gesture owners

Editors use explicit springs and reset bindings; `TextInput` is a custom GPUI
control. Internal max-height fields own their spring. External editor bodies
and number gutters share vertical displacement while caret reveal and editing
clear it. `ScrollContainer` callers bind the input's existing spring rather than
adding another nested scroller.

| Surface | Owner and constraints |
| --- | --- |
| Shared text content/description areas | `gitcomet-ui-kit/src/components/containers.rs:178–200`, `ScrollContainer`. Used by commit details/reword, squash/rebase descriptions, tag messages and gitignore inputs. Keep borders, labels and scrollbar tracks outside the body. |
| Internally scrolling multiline input | `gitcomet-ui-kit/src/text_input/render.rs:200–229`. The input owns caret reveal, selection and its internal handle. Coordinate elasticity with those mechanisms. |
| Document editor/gutter | `view/documents.rs:1276–1368`. Editor and virtualized line-number gutter must move together; toolbar and scrollbars remain fixed. |
| Repository file editor/gutter | `view/panes/diff/file_editor.rs:2039–2167`. Same requirement, including blame/annotation geometry. |
| Conflict output editor/streamed output/gutter | `view/panels/main/conflict_resolver_view.rs:2037,2248,2290`. Preserve output/source synchronization, streaming and caret reveal. |
| Multiline diff search input | `view/panels/main/diff_view.rs:1571–1585`. Input-body elasticity must preserve the surrounding search controls. |
| Terminal content | `gitcomet-ui-terminal/src/viewport.rs:985`. Its wheel handler owns terminal mouse reporting, alternate-screen behavior and scrollback. A dedicated fractional-intent adapter adds elasticity only to normal scrollback. A live input gate retains PTY mouse reporting and alternate-screen behavior, including mode changes before repaint. The scrollbar stays fixed and canvas selection/cursor/IME painting follows displacement. Rendering remains row based. |
| Image preview | `gitcomet-ui-media/src/image_preview/surface.rs:330`. Image camera pan/zoom owns its gestures. |
| Diagram preview | `gitcomet-ui-media/src/diagram_preview/surface.rs:229`. Diagram pan/zoom and inline diagram/document handoff own their gestures. |
| Extension-provided arbitrary views | The extension owns internal UI unless the host explicitly supplies a scrolling container, such as the Settings menu above. Host-provided changed-file lists inherit the shared list behavior. |

## Routing and regression coverage

Settings dropdowns deliberately chain wheel events to the enclosing page at a
boundary (`view/settings_window/mod.rs:972–999`). Elasticity must define precise
gesture ownership without silently changing the existing nonprecise-wheel
policy. Nested horizontal Markdown blocks restrict scrolling to their axis;
their wrapper must not claim vertical input intended for the document.

Meaningful checks should cover:

- Top/bottom and left/right pulls, clamped logical offsets, unchanged extents,
  reversal, release/cancellation and eventual settlement without idle frames.
- Sticky headers at both edges, including a header whose natural position is
  visible near the bottom; wheel input directly over a pinned header; header
  clicks while the body is stretched.
- Fixed query/header/footer/add controls and scrollbar tracks, while rows move.
- Nested horizontal-in-vertical ownership and the converse route, including
  release after the pointer crosses into another area.
- Linked conflict/diff columns and editor gutters, with sync both enabled and
  disabled; preservation of wheel-master callbacks.
- Picker windowing, virtual-list row bounds and scroll extent under stretch;
  keyboard/programmatic reveal during a rebound; list contents changing.
- Drag autoscroll, caret/selection/IME geometry and scrollbar dragging, which
  must not treat elastic paint displacement as a logical scroll position.
- Reduced Motion, ordinary line wheels, repository/document switches,
  viewport resizing and closing a stretched view.

`gitcomet-ui-kit/src/elastic_scroll/surface_tests.rs` adds focused GPUI cases for
horizontal boundaries and settlement, nested-axis ownership including small
mixed-axis packets in one gesture, the real shared tab bar's trailing
control/reveal behavior, and the real shared picker's fixed query row/scrollbar
with windowed rows. Differently sized linked horizontal surfaces must retain
equal displacement throughout a shared stretch and return. The tests also cover
persistence and rebound of a tracked implicit overflow area, and fractional
scrolling, boundary clamping and virtualization through the `ListState` adapter.
Additional regressions cover programmatic reveal, shared resize with zero
extent, post-reversal logical forwarding, TextInput caret/IME geometry, editor
gutters, independent conflict output, grouped-header wheel forwarding, dropdown
boundary policy and live terminal mode handoff. These complement motion and
vertical-wrapper tests. The user accepted the spring tuning with a physical
trackpad on the earlier installed build; the expanded surfaces need their own
interaction checks.
