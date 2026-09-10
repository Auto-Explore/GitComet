# Appearance and push options

In **Settings → General**, choose **Compact**, **Comfortable** or **Spacious**
density. Compact is the default. Each step adds space around controls and
increases button, menu and list-row targets for pointer and trackpad use;
Spacious continues the same step further again. Density changes spacing only —
font sizes are a separate setting. Staging actions and the commit composer keep
their existing positions.

Font sizes are saved separately and apply immediately across open windows:

| Setting | Default | Range | Applies to |
| --- | --- | --- | --- |
| UI Font size | 14 px | 10–24 px | Interface text, preserving heading and label proportions |
| Editor Font size | 14 px | 8–32 px | File editors, diffs, merge resolution and terminal text |
| Markdown preview size | 16 px | 10–32 px | Rendered Markdown, preserving heading proportions |

Use the minus/plus buttons or type a whole-number value. Reset affects only that
font size. UI scale remains a separate multiplier for the whole window.

Window title bars are the one exception to all three. A title bar shares its row
with the operating system's own window controls, which do not resize, so the
bar, its buttons and the repository tabs hold a single size at every density,
font size and UI scale. This applies to the Settings window's header too, and on
macOS the traffic lights are centred on that same fixed bar. The window frame's
resize border is not part of this and still follows the UI scale.


## History context menu

Right-click anywhere on a history row, including its branch and tag badges, to
open the same menu. It includes commit actions and named groups for each local
branch, remote branch and tag on that commit. Expand a group to act on that ref;
only one group opens at a time. Sidebar ref menus remain specific to their ref.

Use Up/Down to navigate, Right to expand, Left to collapse, Enter/Space to
activate and Escape to close and return focus.

## Push with tags

The Push dropdown has two additional actions:

- **Push with annotated tags** pushes the current branch and missing annotated
  tags reachable from it (`--follow-tags`). Lightweight tags are excluded.
- **Push with all tags** pushes the current branch and all local tags, including
  lightweight tags and tags outside that branch (`--tags`).

Both show a background preview of new tags for the selected destination.
Up-to-date tags are excluded from the count; conflicting tags are listed
separately. Hover for tag names or choose **View tags to push…** to search the
full list. Multiple push URLs are checked and tag names are counted once.

Previews use a cancellable Git dry run without hooks, signing or authentication
dialogs. If a preview needs credentials or cannot reach the remote, it shows
**Preview unavailable** and you can still push. The real push uses normal
authentication, hooks and signing, and keeps the chosen tag option when setting
an upstream or retrying authentication. The option applies to that push only.

The remote can change after a preview. Conflicting tags are not force-updated.
