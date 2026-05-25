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
  "name": "My Theme Pack",
  "author": "Example Author",                           // Optional
  "themes": [
    {
      "key": "my_theme_dark",
      "name": "My Theme Dark",
      "appearance": "dark",
      "colors": {
        "window_bg": "#10131aff",
        "surface_bg": "#171b24ff",
        "surface_bg_elevated": "#1d2230ff",
        "active_section": "#262c3bff",
        "border": "#2c3445ff",
        "tooltip_bg": "#0b0e14ff",                      // Optional
        "tooltip_text": "#f5f7fbff",                    // Optional
        "text": "#edf1f7ff",
        "text_muted": "#9ea7b8ff",
        "accent": "#59b7ffff",
        "hover": "#222839ff",
        "active": { "hex": "#3a4560ff", "alpha": 0.85 },
        "focus_ring": { "hex": "#59b7ffff", "alpha": 0.55 },
        "focus_ring_bg": { "hex": "#59b7ffff", "alpha": 0.16 },
        "scrollbar_thumb": { "hex": "#9ea7b8ff", "alpha": 0.30 },
        "scrollbar_thumb_hover": { "hex": "#9ea7b8ff", "alpha": 0.42 },
        "scrollbar_thumb_active": { "hex": "#9ea7b8ff", "alpha": 0.52 },
        "danger": "#f16b73ff",
        "warning": "#ffc06aff",
        "success": "#9edb63ff",
        "diff_add_bg": "#163322ff",                     // Optional
        "diff_add_text": "#b9f2c0ff",                   // Optional
        "diff_remove_bg": "#40171dff",                  // Optional
        "diff_remove_text": "#ffc4ccff",                // Optional
        "input_placeholder": "#ffffff59",               // Optional
        "accent_text": "#08111cff",                     // Optional
        "emphasis_text": "#f5f7fbff",                   // Optional
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
        "panel": 2.0,
        "pill": 2.0,
        "row": 2.0
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
| `name` | string | Required. Bundle name |
| `author` | string | Optional |
| `themes` | array | Required. One or more theme entries |

## Colors Schema

### Required color fields

`window_bg`, `surface_bg`, `surface_bg_elevated`, `active_section`, `border`, `text`, `text_muted`, `accent`, `hover`, `active`, `focus_ring`, `focus_ring_bg`, `scrollbar_thumb`, `scrollbar_thumb_hover`, `scrollbar_thumb_active`, `danger`, `warning`, `success`

### Optional color fields

`tooltip_bg`, `tooltip_text`, `diff_add_bg`, `diff_add_text`, `diff_remove_bg`, `diff_remove_text`, `input_placeholder`, `accent_text`, `emphasis_text`, `graph_lane_palette`, `graph_lane_hues`

### Color value format

Most color fields accept either:

- a hex RGBA string such as `#0d1016ff`
- an object with `hex` plus `alpha`, for example `{ "hex": "#5ac1feff", "alpha": 0.60 }`

Use `graph_lane_palette` for an explicit list of colors, or `graph_lane_hues` for a list of hue values that GitComet turns into graph lane colors automatically.

If you omit optional diff colors, tooltip colors, placeholder color, accent text color, emphasis text color, or syntax colors, GitComet falls back to built-in defaults.

More generally, omitted optional values are either derived from the theme's base colors or filled with built-in defaults, depending on the field.

## Syntax Schema

The `syntax` object is optional. Supported keys are:

`comment`, `comment_doc`, `string`, `string_escape`, `string_regex`, `string_special`, `keyword`, `keyword_control`, `preproc`, `number`, `boolean`, `function`, `function_method`, `function_special`, `constructor`, `type`, `type_builtin`, `type_interface`, `namespace`, `variable`, `variable_parameter`, `variable_special`, `variable_builtin`, `property`, `label`, `constant`, `constant_builtin`, `operator`, `punctuation`, `punctuation_bracket`, `punctuation_delimiter`, `punctuation_special`, `punctuation_list_marker`, `tag`, `attribute`, `markup_heading`, `markup_link`, `text_literal`, `diff_plus`, `diff_minus`, `diff_delta`, `lifetime`

Use `type` in JSON for the main type-name color.

## Radii Schema

The `radii` object is required and must include:

- `panel`
- `pill`
- `row`

These values are numeric and control the corner radius used by major UI elements.

## Overrides And Validation Behavior

- Built-in system themes stay embedded in the GitComet binary and are not loaded from the custom themes directory.
- GitComet loads custom `.json` files from the themes directory, but ignores files whose basename matches a bundled system theme file such as `gitcomet.json`.
- Custom themes can add new theme keys, but they cannot override built-in system theme keys. Any runtime theme entry that reuses a built-in key is ignored.
- A file that cannot be read or parsed is ignored.
- GitComet does not expose a separate machine-readable JSON Schema file today; the implementation in [`crates/gitcomet-ui-gpui/src/theme.rs`](crates/gitcomet-ui-gpui/src/theme.rs) is the source of truth.
