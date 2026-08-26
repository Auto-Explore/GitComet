# GitComet Themes

GitComet supports built-in themes and user-provided custom themes.

Built-in themes are embedded in the GitComet binary.

GitComet loads custom themes from JSON bundle files in your per-user themes directory.

## Theme File Location

GitComet creates the user themes directory on startup and only loads custom `.json` files from that location:

| Platform | Themes directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/gitcomet/themes` or `~/.local/share/gitcomet/themes` |
| macOS | `~/Library/Application Support/gitcomet/themes` |
| Windows | `%LOCALAPPDATA%\\gitcomet\\themes` or `%APPDATA%\\gitcomet\\themes` |

## JSON Schema

Disclaimer: The theme JSON format may change as GitComet's UI is still actively being developed.

Each theme file is a bundle with a bundle name and one or more themes. The example below includes every currently supported field:

```javascript
{
  "schema_version": 2,
  "name": "My Theme Pack",
  "author": "Example Author",                           // Optional
  "themes": [
    {
      "key": "my_theme_dark",
      "name": "My Theme Dark",
      "appearance": "dark",
      "colors": {
        "surface": {
          "canvas": "#10131aff",
          "chrome": "#171b24ff",
          "panel": "#171b24ff",
          "raised": "#1d2230ff",
          "input": "#202633ff"
        },
        "foreground": {
          "primary": "#edf1f7ff",
          "secondary": "#9ea7b8ff",
          "disabled": { "hex": "#9ea7b8ff", "alpha": 0.62 },
          "placeholder": "#ffffff59",
          "emphasis": "#ffffffff"
        },
        "stroke": {
          "subtle": "#ffffff14",
          "default": "#2c3445ff",
          "control": "#556176ff"
        },
        "interaction": {
          "hover_overlay": { "hex": "#edf1f7ff", "alpha": 0.07 },
          "pressed_overlay": { "hex": "#edf1f7ff", "alpha": 0.11 },
          "hover_background": "#222839ff",
          "pressed_background": "#3a4560d9",
          "selected_background": "#262c3bff",
          "selected_foreground": "#edf1f7ff",
          "selected_indicator": "#59b7ffff",
          "focus_ring": "#59b7ffff",
          "focus_background": { "hex": "#59b7ffff", "alpha": 0.16 }
        },
        "accent": {
          "foreground": "#59b7ffff",
          "solid": "#59b7ffff",
          "on_solid": "#08111cff",
          "subtle_background": "#183246ff"
        },
        "status": {
          "info": { "foreground": "#59b7ffff", "background": "#183246ff", "border": "#387ba7ff" },
          "success": { "foreground": "#9edb63ff", "background": "#20351fff", "border": "#59823fff" },
          "warning": { "foreground": "#ffc06aff", "background": "#3b2d1bff", "border": "#98713eff" },
          "danger": { "foreground": "#f16b73ff", "background": "#3b1d24ff", "border": "#95434aff" }
        },
        "editor": {
          "background": "#10131aff",
          "foreground": "#edf1f7ff",
          "gutter_background": "#10131aff",
          "line_number": "#9ea7b8ff",
          "cursor": "#edf1f7c7",
          "selection_background": "#59b7ff47",
          "search_match_background": "#3b2d1bff",
          "search_match_foreground": "#ffc06aff",
          "bracket_match_background": "#ffffff26",
          "indent_guide": "#ffffff14"
        },
        "diff": {
          "added": { "foreground": "#b9f2c0ff", "background": "#163322ff", "word_background": "#b9f2c038", "focused_background": "#b9f2c033" },
          "removed": { "foreground": "#ffc4ccff", "background": "#40171dff", "word_background": "#ffc4cc38", "focused_background": "#ffc4cc33" },
          "modified": { "foreground": "#ffc06aff", "background": "#3b2d1bff", "word_background": "#ffc06a38", "focused_background": "#ffc06a33" }
        },
        "tooltip": { "background": "#0b0e14ff", "foreground": "#f5f7fbff" },
        "scrollbar": {
          "thumb": { "hex": "#9ea7b8ff", "alpha": 0.30 },
          "thumb_hover": { "hex": "#9ea7b8ff", "alpha": 0.42 },
          "thumb_pressed": { "hex": "#9ea7b8ff", "alpha": 0.52 }
        },
        "shadow": "#000000ff",
        "graph_lane_palette": [                         // Optional
          "#ff6b6bff",
          "#ffd166ff",
          "#06d6a0ff",
          "#4dabf7ff"
        ],
        "graph_lane_hues": [                            // Optional
          0.00,
          0.18,
          0.42,
          0.63
        ]
      },
      "syntax": {                                       // Optional
        "comment": "#7f8aa1ff",                         // Optional
        "comment_doc": "#91a0b8ff",                     // Optional
        "string": "#ffd27aff",                          // Optional
        "string_escape": "#8ce3b4ff",                   // Optional
        "string_regex": "#ff9b8dff",                    // Optional
        "string_special": "#ffc776ff",                  // Optional
        "keyword": "#7ec5ffff",                         // Optional
        "keyword_control": "#8fd8ffff",                 // Optional
        "preproc": "#71d8ffff",                         // Optional
        "number": "#9edb63ff",                          // Optional
        "boolean": "#b4e07aff",                         // Optional
        "function": "#78c4ffff",                        // Optional
        "function_method": "#87d0ffff",                 // Optional
        "function_special": "#96dbffff",                // Optional
        "constructor": "#5cd7c7ff",                     // Optional
        "type": "#ffc06aff",                            // Optional
        "type_builtin": "#ffce87ff",                    // Optional
        "type_interface": "#ffd9a3ff",                  // Optional
        "namespace": "#9cc1ffff",                       // Optional
        "variable": "#f3f6fbff",                        // Optional
        "variable_parameter": "#c7d0deff",              // Optional
        "variable_special": "#70c5ffff",                // Optional
        "variable_builtin": "#78d8cbff",                // Optional
        "property": "#66c2ffff",                        // Optional
        "label": "#c1b4ffff",                           // Optional
        "constant": "#9edb63ff",                        // Optional
        "constant_builtin": "#bfe68bff",                // Optional
        "operator": "#c5ceddff",                        // Optional
        "punctuation": "#b4beceff",                     // Optional
        "punctuation_bracket": "#c2cadaff",             // Optional
        "punctuation_delimiter": "#a9b4c7ff",           // Optional
        "punctuation_special": "#8fd8ffff",             // Optional
        "punctuation_list_marker": "#ff9b8dff",         // Optional
        "tag": "#ffc06aff",                             // Optional
        "attribute": "#74caffff",                       // Optional
        "markup_heading": "#8fd8ffff",                  // Optional
        "markup_link": "#7ec5ffff",                     // Optional
        "text_literal": "#ffd27aff",                    // Optional
        "diff_plus": "#9edb63ff",                       // Optional
        "diff_minus": "#ff9b8dff",                      // Optional
        "diff_delta": "#7ec5ffff",                      // Optional
        "lifetime": "#80d2ffff"                         // Optional
      },
      "radii": {
        "panel": 12.0,
        "pill": 999.0,
        "row": 8.0,
        "control": 8.0,                                 // Optional
        "popover": 10.0,                                // Optional
        "window": 12.0                                  // Optional
      }
    }
  ]
}
```

In normal use, provide either `graph_lane_palette` or `graph_lane_hues`. The example shows both only so every supported field is visible in one place.

One file can define multiple themes. Theme keys must be unique within the file.

## Required Theme Fields

Each entry in `themes` must include:

| Field | Type | Notes |
| --- | --- | --- |
| `key` | string | Stable internal identifier used in settings and persistence |
| `name` | string | User-facing label shown in the UI |
| `appearance` | string | Must be `light` or `dark` |
| `colors` | object | Theme color definitions |
| `radii` | object | Radius values for UI surfaces |

The bundle root supports:

| Field | Type | Notes |
| --- | --- | --- |
| `schema_version` | number | Required. Must be `2` |
| `name` | string | Required. Bundle name |
| `author` | string | Optional |
| `themes` | array | Required. One or more theme entries |

## Colors Schema

Theme schema v2 uses semantic groups. Define every group and field below: a token
your file leaves out falls back to the bundled theme matching your `appearance`
(`gitcomet_dark` or `gitcomet_light`), which keeps older theme files loading when
new tokens are added but means the omitted token is not yours to control. A token
you misspell is still an error — the file is rejected rather than half-applied.

- `surface`: `canvas`, `chrome`, `panel`, `raised`, `input`
- `foreground`: `primary`, `secondary`, `disabled`, `placeholder`, `emphasis`
- `stroke`: `subtle`, `default`, `control`
- `interaction`: `hover_overlay`, `pressed_overlay`, `hover_background`,
  `pressed_background`, `selected_background`, `selected_foreground`,
  `selected_indicator`, `focus_ring`, `focus_background`
- `accent`: `foreground`, `solid`, `on_solid`, `subtle_background`
- `status`: `info`, `success`, `warning`, `danger`; each contains
  `foreground`, `background`, and `border`
- `editor`: `background`, `foreground`, `gutter_background`, `line_number`,
  `cursor`, `selection_background`, `search_match_background`,
  `search_match_foreground`, `bracket_match_background`,
  `occurrence_highlight_background`, `indent_guide`
- `diff`: `added`, `removed`, `modified`; each contains `foreground`,
  `background`, `word_background`, and `focused_background`
- `tooltip`: `background`, `foreground`
- `scrollbar`: `thumb`, `thumb_hover`, `thumb_pressed`
- `shadow`
- `graph_lane_palette` and `graph_lane_hues` are optional

`surface.canvas` is the central content area. `surface.chrome` is the surrounding
title/action/sidebar/status band. Necessary input and button outlines should use
`stroke.control`; `stroke.subtle` is for decorative separators.

### Color value format

Most color fields accept either:

- a hex RGBA string such as `#0d1016ff`
- an object with `hex` plus `alpha`, for example `{ "hex": "#5ac1feff", "alpha": 0.60 }`

Use `graph_lane_palette` for an explicit list of colors, or `graph_lane_hues` for a list of hue values that GitComet turns into graph lane colors automatically.

Syntax colors, graph lanes, and the documented radius extensions have fallbacks of
their own — omitting `graph_lane_palette` and `graph_lane_hues` generates lane
colors for your `appearance` rather than copying the bundled theme's. Every other
semantic UI color falls back to the bundled theme for your `appearance`. Spell
them all out anyway: a component never infers a status, editor, selection, or
control color from an unrelated token, so an omitted one is a bundled color
sitting in your theme, not a shade of it.

## Syntax Schema

The `syntax` object is optional. Supported keys are:

`comment`, `comment_doc`, `string`, `string_escape`, `string_regex`, `string_special`, `keyword`, `keyword_control`, `preproc`, `number`, `boolean`, `function`, `function_method`, `function_special`, `constructor`, `type`, `type_builtin`, `type_interface`, `namespace`, `variable`, `variable_parameter`, `variable_special`, `variable_builtin`, `property`, `label`, `constant`, `constant_builtin`, `operator`, `punctuation`, `punctuation_bracket`, `punctuation_delimiter`, `punctuation_special`, `punctuation_list_marker`, `tag`, `attribute`, `markup_heading`, `markup_link`, `text_literal`, `diff_plus`, `diff_minus`, `diff_delta`, `lifetime`

Use `type` in JSON for the main type-name color.

## Radii Schema

The `radii` object is required and must include:

- `panel` — cards and panels
- `pill` — round badges and chips
- `row` — list rows

It may also include (falling back to built-in defaults when omitted):

- `control` — buttons, inputs, and tabs (default `8.0`)
- `popover` — menus, popovers, and dialogs (default `10.0`)
- `window` — the window frame under client-side decorations (default `12.0`)

These values are numeric and control the corner radius used by major UI elements.

## Overrides And Validation Behavior

- Built-in system themes stay embedded in the GitComet binary and are not loaded from the custom themes directory.
- GitComet loads custom `.json` files from the themes directory, but ignores files whose basename matches a bundled system theme file such as `gitcomet.json`.
- Custom themes can add new theme keys, but they cannot override built-in system theme keys. Any runtime theme entry that reuses a built-in key is ignored.
- A file that cannot be read or parsed is ignored and reported with its path and reason.
- GitComet validates the structure and types of custom themes, but does not
  measure, warn about, reject, or alter their colors based on contrast.
- GitComet does not expose a separate machine-readable JSON Schema file today; the implementation in [`crates/gitcomet-ui-gpui/src/theme.rs`](crates/gitcomet-ui-gpui/src/theme.rs) is the source of truth.
